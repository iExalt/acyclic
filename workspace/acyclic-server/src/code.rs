//! Runs' code repositories, over git's smart HTTP protocol on the runners' listener. A run never holds a credential
//! for the forge: it fetches each repository of its thread, and pushes to its own refs, `refs/runs/<run>/*`, in a
//! repository its thread alone uses.
//!
//! Each repository on the forge has a bare mirror, fetched from its `refs/heads/*` and `refs/tags/*` alone, which no
//! run reaches. Each thread has a repository of its own per repository it works on, borrowing the mirror's objects and
//! holding its runs' pushes, so a run reaches the forge's history, its own thread's runs' refs and nothing of any other
//! thread's. Every request is checked against the run's thread's repository set as it stands, so a repository added
//! to the thread is reachable at once and one removed is refused. Pushes go through a hook ([`hook`]) before git takes
//! them.
//!
//! The mirror keeps every object it fetched, since the threads' repositories borrow them. Once a thread is finished,
//! its repositories are removed, and a mirror no thread borrows from any more drops what the forge no longer has.
//!
//! Git runs with an environment of the server's making, nothing inherited, and a home of its own, so none of the
//! user's settings or credentials reaches the forge or a run. Each request runs git in a process group of its own,
//! killed whole when the request ends early or takes too long.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acyclic_cli::{Env, Reach};
use acyclic_core::model::Repository;
use acyclic_core::private;
use axum::body::{Body, Bytes};
use axum::extract::{Path as Route, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router};
use futures_util::StreamExt as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use acyclic_operator::token::Claims;

use crate::{Credential, Listener};

pub mod hook;

/// The largest push, in bytes.
pub const MOST_PUSHED: u64 = 512 * 1024 * 1024;

/// The largest fetch request, in bytes, once decompressed: a fetch's request lists what the run wants and has.
const MOST_ASKED: usize = 64 * 1024 * 1024;

/// How long one request may take, git's work included.
pub const TIME_LIMIT: Duration = Duration::from_secs(600);

/// How many fetches may run at once.
const FETCHES: usize = 8;

/// How long a fetch waits for one of the [`FETCHES`] before it is refused.
const FETCH_WAIT: Duration = Duration::from_secs(30);

/// The upstream every mirror is fetched from, `<upstream>/<owner>/<name>.git`, unless `ACYCLIC_UPSTREAM` names another.
pub const UPSTREAM: &str = "https://github.com";

/// What the code endpoint needs.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The environment, which says where the mirrors are and what they are fetched from.
    pub env: Env,
    /// The `acyclic` executable, which the pushes' hook runs as `acyclic receive-hook`.
    pub executable: PathBuf,
    /// The deny-list file, which the hook reads.
    pub deny: PathBuf,
}

/// The code endpoint's state.
#[derive(Debug)]
pub struct Code {
    settings: Settings,
    /// The mirrors' directory, readable by the user alone, which holds the hooks and git's home too.
    root: PathBuf,
    upstream: String,
    path: String,
    /// When each mirror's last successful fetch began, by record id.
    fetched: Mutex<BTreeMap<String, Instant>>,
    /// One lock per repository, held while its mirror or a thread's repository of it is made, fetched or removed.
    fetching: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The runs with a push under way, with the repository's record: `<run> <record>`.
    pushing: Arc<Mutex<BTreeSet<String>>>,
    fetches: Arc<tokio::sync::Semaphore>,
}

/// A refusal or failure, as a status and a line of text, which git shows the run.
type Failure = (StatusCode, String);

impl Code {
    /// The code endpoint, making the mirrors' directory private and writing the pushes' hook into it.
    ///
    /// # Errors
    ///
    /// When there is no home for the mirrors, the directory is open to others, the hook can't be written, or the
    /// executable's path can't be quoted for the hook.
    pub fn new(settings: Settings) -> Result<Self, String> {
        let root = acyclic_cli::mirrors(&settings.env)
            .ok_or("HOME isn't set, so the mirrors have nowhere to live")?;
        std::fs::create_dir_all(root.parent().unwrap_or(&root)).map_err(|err| err.to_string())?;
        private::dir(&root).map_err(|err| err.to_string())?;
        let executable = settings.executable.display().to_string();
        if executable.contains('\'') || !settings.executable.is_absolute() {
            return Err(format!(
                "the hook can't run {executable}: give an absolute path without quotes"
            ));
        }
        for dir in [".hooks", ".home"] {
            std::fs::create_dir_all(root.join(dir)).map_err(|err| err.to_string())?;
        }
        let hook = root.join(".hooks/pre-receive");
        let script = format!("#!/bin/sh\nexec '{executable}' {}\n", hook::COMMAND);
        private::place(&hook, script.as_bytes(), true).map_err(|err| err.to_string())?;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700))
            .map_err(|err| err.to_string())?;
        let upstream = settings
            .env
            .vars
            .get("ACYCLIC_UPSTREAM")
            .filter(|upstream| !upstream.is_empty())
            .map_or(UPSTREAM, String::as_str)
            .trim_end_matches('/')
            .to_owned();
        let path = settings.env.vars.get("PATH").cloned().unwrap_or_else(|| "/usr/bin:/bin".into());
        Ok(Self {
            settings,
            root,
            upstream,
            path,
            fetched: Mutex::default(),
            fetching: Mutex::default(),
            pushing: Arc::default(),
            fetches: Arc::new(tokio::sync::Semaphore::new(FETCHES)),
        })
    }

    /// Git with nothing of this process's environment: no settings, no credentials, no prompts.
    fn git(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("git");
        command
            .env_clear()
            .env("PATH", &self.path)
            .env("HOME", self.root.join(".home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        command
    }

    /// Runs git to completion within the time limit, killing its process group past it.
    async fn run(&self, mut command: tokio::process::Command, what: &str) -> Result<(), Failure> {
        let failed = |why: String| (StatusCode::BAD_GATEWAY, format!("{what}: {why}"));
        let child = command.spawn().map_err(|err| failed(format!("git can't run: {err}")))?;
        let mut group = Group::new(child);
        match tokio::time::timeout(TIME_LIMIT, group.child.wait()).await {
            Ok(Ok(status)) if status.success() => {
                group.done();
                Ok(())
            },
            Ok(Ok(status)) => {
                group.done();
                Err(failed(format!("git failed: {status}")))
            },
            Ok(Err(err)) => Err(failed(err.to_string())),
            Err(_) => Err(failed("git took too long".to_owned())),
        }
    }

    /// The lock of `record`'s mirror and its threads' repositories, held while any of them is made, fetched or
    /// removed.
    async fn lock(&self, record: &str) -> Result<tokio::sync::OwnedMutexGuard<()>, Failure> {
        let lock = {
            let mut fetching = self.fetching.lock().map_err(|_| internal())?;
            Arc::clone(fetching.entry(record.to_owned()).or_default())
        };
        Ok(lock.lock_owned().await)
    }

    /// The repository `thread`'s runs use for `record`: made if missing, borrowing the mirror's objects, and brought
    /// up to date with the mirror, which is fetched first if `refresh`.
    async fn workspace(
        &self,
        record: &Repository,
        thread: &str,
        refresh: bool,
    ) -> Result<PathBuf, Failure> {
        let unavailable = |message: String| (StatusCode::FORBIDDEN, message);
        let (Some(owner), Some(name)) = (record.owner.one(), record.name.one()) else {
            return Err(unavailable(format!("{} has no single owner and name", record.id)));
        };
        if record.public.one() != Some(&true) {
            return Err(unavailable(format!(
                "{owner}/{name} is private, and private repositories can't be mirrored yet"
            )));
        }
        if !(segment(owner) && segment(name) && segment(thread)) {
            return Err(unavailable(format!("{owner}/{name} isn't a repository's name")));
        }
        let mirror = acyclic_cli::mirror(&self.settings.env, &record.id)
            .filter(|mirror| mirror.parent() == Some(self.root.as_path()))
            .ok_or_else(|| unavailable(format!("{} can't name a mirror", record.id)))?;
        let threads = self.root.join(format!("{}.threads", record.id));
        let repository = threads.join(format!("{thread}.git"));
        let asked = Instant::now();
        let _held = self.lock(&record.id).await?;
        self.fetch(record, owner, name, &mirror, refresh, asked).await?;
        let made = repository.is_dir();
        if made && !refresh {
            return Ok(repository);
        }
        let staged = threads.join(format!(".{thread}.{}.new", std::process::id()));
        let target = if made { repository.clone() } else { staged.clone() };
        if !made {
            let _ = std::fs::remove_dir_all(&staged);
            let mut init = self.git();
            init.args(["init", "-q", "--bare", "--template="]).arg(&staged);
            self.run(init, "making the thread's repository").await?;
            let info = staged.join("objects/info");
            let borrowed = format!("{}\n", mirror.join("objects").display());
            std::fs::create_dir_all(&info)
                .and_then(|()| std::fs::write(info.join("alternates"), borrowed))
                .map_err(|err| {
                    let _ = std::fs::remove_dir_all(&staged);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("making the thread's repository: {err}"),
                    )
                })?;
        }
        // The mirror's objects are the thread's own already, so this moves refs alone.
        let mut fetch = self.git();
        fetch
            .arg("-C")
            .arg(&target)
            .args(["fetch", "-q", "--prune", "--no-write-fetch-head", "--"])
            .arg(&mirror)
            .args(["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"]);
        let fetched = self.run(fetch, "updating the thread's repository").await;
        let named = match (&fetched, record.default_branch.one().filter(|b| segment(b))) {
            (Ok(()), Some(branch)) => {
                let mut head = self.git();
                head.arg("-C").arg(&target).args(["symbolic-ref", "HEAD"]);
                head.arg(format!("refs/heads/{branch}"));
                self.run(head, "naming the default branch").await
            },
            _ => fetched,
        };
        if let Err(failure) = named {
            if !made {
                let _ = std::fs::remove_dir_all(&staged);
            }
            return Err(failure);
        }
        if !made && std::fs::rename(&staged, &repository).is_err() {
            let _ = std::fs::remove_dir_all(&staged);
            if !repository.is_dir() {
                return Err(internal());
            }
        }
        Ok(repository)
    }

    /// Fetches the mirror at `mirror` from the forge if it is missing, or if `refresh` and no fetch begun since
    /// `asked` finished meanwhile, so fetches asked for together share one. A new mirror is fetched where it is made
    /// and moved into place only once it holds the forge's branches; it never prunes objects on its own, since the
    /// threads' repositories borrow them. Called with the record's lock held.
    async fn fetch(
        &self,
        record: &Repository,
        owner: &str,
        name: &str,
        mirror: &Path,
        refresh: bool,
        asked: Instant,
    ) -> Result<(), Failure> {
        let made = mirror.is_dir();
        let fresh = self
            .fetched
            .lock()
            .map_err(|_| internal())?
            .get(&record.id)
            .is_some_and(|began| *began >= asked);
        if made && (fresh || !refresh) {
            return Ok(());
        }
        let staged = self.root.join(format!(".{}.{}.new", record.id, std::process::id()));
        let target = if made { mirror.to_path_buf() } else { staged.clone() };
        if !made {
            let _ = std::fs::remove_dir_all(&staged);
            let mut init = self.git();
            init.args(["init", "-q", "--bare", "--template="]).arg(&staged);
            self.run(init, "making the mirror").await?;
            let mut keep = self.git();
            keep.arg("-C").arg(&staged).args(["config", "gc.pruneExpire", "never"]);
            self.run(keep, "making the mirror").await?;
        }
        let began = Instant::now();
        let mut fetch = self.git();
        fetch
            .arg("-C")
            .arg(&target)
            .args(["fetch", "-q", "--prune", "--no-write-fetch-head", "--"])
            .arg(format!("{}/{owner}/{name}.git", self.upstream))
            .args(["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"]);
        if let Err(failure) = self.run(fetch, &format!("fetching {owner}/{name}")).await {
            if made {
                // The forge being away shouldn't stop runs: they get the mirror as it was last fetched.
                eprintln!("acyclic: {}; serving the mirror as last fetched", failure.1);
                return Ok(());
            }
            let _ = std::fs::remove_dir_all(&staged);
            return Err(failure);
        }
        if !made && std::fs::rename(&staged, mirror).is_err() {
            // Another server made it first.
            let _ = std::fs::remove_dir_all(&staged);
            if !mirror.is_dir() {
                return Err(internal());
            }
        }
        self.fetched.lock().map_err(|_| internal())?.insert(record.id.clone(), began);
        Ok(())
    }

    /// Fetches every mirror that exists of a public repository, for readiness's checks of what is merged, and removes
    /// the repositories of threads that are finished.
    pub(crate) async fn refresh_all(&self, tracker: &Path) {
        let Ok(records) = acyclic_cli::repositories(tracker) else { return };
        for record in records {
            let Some(mirror) = acyclic_cli::mirror(&self.settings.env, &record.id)
                .filter(|mirror| mirror.parent() == Some(self.root.as_path()) && mirror.is_dir())
            else {
                continue;
            };
            if let (Some(owner), Some(name), Some(true)) =
                (record.owner.one(), record.name.one(), record.public.one())
                && let Ok(_held) = self.lock(&record.id).await
            {
                let _ = self.fetch(&record, owner, name, &mirror, true, Instant::now()).await;
            }
            let threads = self.root.join(format!("{}.threads", record.id));
            let present = borrowers(&threads);
            let tracker = tracker.to_path_buf();
            let finished = tokio::task::spawn_blocking(move || {
                acyclic_cli::finished(&present, &tracker).unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            if finished.is_empty() {
                continue;
            }
            if let Ok(_held) = self.lock(&record.id).await {
                let git = self.blocking_git();
                let _ = tokio::task::spawn_blocking(move || {
                    collect(&git, &mirror, &threads, &finished);
                })
                .await;
            }
        }
    }

    /// [`Self::git`], for the blocking pool.
    fn blocking_git(&self) -> impl Fn() -> std::process::Command + Send + 'static {
        let (path, home) = (self.path.clone(), self.root.join(".home"));
        move || {
            let mut command = std::process::Command::new("git");
            command
                .env_clear()
                .env("PATH", &path)
                .env("HOME", &home)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        }
    }

    /// Git's `upload-pack` or `receive-pack` in a thread's repository for `run`, the pushes' hook in place.
    fn pack(
        &self,
        service: Service,
        repository: &Path,
        run: &str,
        advertise: bool,
        public: bool,
    ) -> tokio::process::Command {
        let mut command = self.git();
        for (key, value) in [
            ("core.hooksPath", self.root.join(".hooks").display().to_string()),
            ("receive.advertisePushOptions", "false".to_owned()),
            ("receive.fsckObjects", "true".to_owned()),
            ("receive.maxInputSize", MOST_PUSHED.to_string()),
        ] {
            command.arg("-c").arg(format!("{key}={value}"));
        }
        command.arg(service.command()).arg("--stateless-rpc");
        if advertise {
            command.arg("--advertise-refs");
        }
        command
            .arg(repository)
            .env(hook::RUN, run)
            .env(hook::PUBLIC, if public { "1" } else { "0" })
            .env(hook::DENY, &self.settings.deny)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        command
    }
}

/// Git's two services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    Upload,
    Receive,
}

impl Service {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "git-upload-pack" => Some(Self::Upload),
            "git-receive-pack" => Some(Self::Receive),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Upload => "git-upload-pack",
            Self::Receive => "git-receive-pack",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Upload => "upload-pack",
            Self::Receive => "receive-pack",
        }
    }
}

/// Whether `text` can be one segment of a repository's path or a branch name the mirror is pointed at: letters,
/// digits, `-`, `_` and `.`, not starting with a dot.
fn segment(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('.')
        && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

/// Whether a run's id can go into git's settings and the hook's environment as it is.
fn run_id(run: &str) -> bool {
    !run.is_empty() && run.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The threads with a repository under `threads`.
fn borrowers(threads: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(threads) else { return BTreeSet::new() };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".git").map(str::to_owned))
        .filter(|thread| segment(thread))
        .collect()
}

/// Removes the repositories under `threads` of the `finished` threads whose runs' refs are all on the forge, as the
/// mirror at `mirror` last fetched it, then, once none is left to borrow from the mirror, lets it drop what the forge
/// no longer has. A run's refs hold its work until it lands, so a thread with work the forge lacks keeps its
/// repository. Called with the record's lock held.
fn collect(
    git: &dyn Fn() -> std::process::Command,
    mirror: &Path,
    threads: &Path,
    finished: &BTreeSet<String>,
) {
    let mut removed = false;
    for thread in finished.iter().filter(|thread| segment(thread)) {
        let repository = threads.join(format!("{thread}.git"));
        let unlanded = git()
            .arg("-C")
            .arg(&repository)
            .args(["rev-list", "--max-count=1", "--glob=refs/runs/*", "--not", "--alternate-refs"])
            .stdout(Stdio::piped())
            .output();
        match unlanded {
            Ok(output) if output.status.success() && output.stdout.is_empty() => {
                removed |= std::fs::remove_dir_all(&repository).is_ok();
            },
            _ => eprintln!(
                "acyclic: keeping {}: its runs' work isn't all on the forge",
                repository.display()
            ),
        }
    }
    if removed && borrowers(threads).is_empty() {
        let _ = git()
            .arg("-C")
            .arg(mirror)
            .args(["-c", "gc.pruneExpire=now", "gc", "-q", "--prune=now"])
            .status();
    }
}

fn internal() -> Failure {
    (StatusCode::INTERNAL_SERVER_ERROR, "the server failed".to_owned())
}

fn text((status, message): Failure) -> Response {
    (status, [(CONTENT_TYPE, "text/plain; charset=utf-8")], format!("{message}\n")).into_response()
}

/// A child in a process group of its own, which is killed whole when this is dropped before [`Group::done`].
struct Group {
    child: tokio::process::Child,
    armed: bool,
}

impl Group {
    fn new(child: tokio::process::Child) -> Self {
        Self { child, armed: true }
    }

    fn done(&mut self) {
        self.armed = false;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if self.armed
            && let Some(id) = self.child.id()
        {
            // The hook and what it runs too, which killing git alone would leave running. Git leads its group, and
            // until it is reaped, which `id` says it isn't, its id can't be another's.
            if let Ok(group) = libc::pid_t::try_from(id) {
                // SAFETY: killpg takes plain integers and touches no memory of this process.
                unsafe { libc::killpg(group, libc::SIGKILL) };
            }
            let _ = self.child.start_kill();
        }
    }
}

/// The routes, under `/git/<owner>/<name>` with or without `.git`.
pub(crate) fn routes() -> Router<Arc<Listener>> {
    use axum::routing::{get, post};
    Router::new()
        .route("/git/{owner}/{name}/info/refs", get(advertise))
        .route("/git/{owner}/{name}/{service}", post(exchange))
}

/// The code endpoint, the run's claims, and the record it named, once the run is checked: under way, and the repository
/// in its thread's set.
async fn authorize(
    listener: &Arc<Listener>,
    credential: Credential,
    owner: String,
    name: String,
) -> Result<(Arc<Code>, Claims, Repository), Failure> {
    let Credential::Run(claims) = credential else {
        return Err((StatusCode::FORBIDDEN, "only runs reach code here".to_owned()));
    };
    let crate::Access::Run { code: Some(code), .. } = &listener.access else {
        return Err((StatusCode::NOT_FOUND, "this server serves no code".to_owned()));
    };
    let code = Arc::clone(code);
    if !run_id(&claims.run) || !segment(&claims.thread) {
        return Err((StatusCode::FORBIDDEN, "the token names no run".to_owned()));
    }
    let name = name.strip_suffix(".git").unwrap_or(&name).to_owned();
    let not_in_thread = || (StatusCode::FORBIDDEN, format!("{owner}/{name} is not in this thread"));
    if !(segment(&owner) && segment(&name)) {
        return Err(not_in_thread());
    }
    let tracker = listener.server.config.tracker.clone();
    let (run, thread) = (claims.run.clone(), claims.thread.clone());
    let (owner_, name_) = (owner.clone(), name.clone());
    let reached = tokio::task::spawn_blocking(move || {
        acyclic_cli::reach(&run, &thread, &owner_, &name_, &tracker)
    })
    .await
    .map_err(|_| internal())?;
    match reached {
        Ok(Reach::Found(record)) => Ok((code, claims, record)),
        Ok(Reach::Inactive) => {
            Err((StatusCode::FORBIDDEN, "the token's run isn't under way".to_owned()))
        },
        Ok(Reach::NotInThread) => Err(not_in_thread()),
        Ok(Reach::Ambiguous) => Err((
            StatusCode::FORBIDDEN,
            format!("more than one repository record names {owner}/{name}"),
        )),
        Err(_) => Err((StatusCode::SERVICE_UNAVAILABLE, "the tracker can't be read".to_owned())),
    }
}

/// `GET …/info/refs?service=<service>`: what the mirror holds that the run may see. A fetch refreshes the mirror
/// first; a push only makes it if it is missing.
async fn advertise(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
    Route((owner, name)): Route<(String, String)>,
    uri: Uri,
) -> Response {
    let service = uri.query().and_then(|query| query.strip_prefix("service="));
    let Some(service) = service.and_then(Service::parse) else {
        return text((
            StatusCode::FORBIDDEN,
            "this server speaks git's smart protocol alone".to_owned(),
        ));
    };
    let (code, claims, record) = match authorize(&listener, credential, owner, name).await {
        Ok(authorized) => authorized,
        Err(failure) => return text(failure),
    };
    let refresh = service == Service::Upload;
    let repository = match code.workspace(&record, &claims.thread, refresh).await {
        Ok(repository) => repository,
        Err(failure) => return text(failure),
    };
    let public = record.public.one() == Some(&true);
    let command = code.pack(service, &repository, &claims.run, true, public);
    let first = format!("# service={}\n", service.name());
    let preamble = format!("{:04x}{first}0000", first.len() + 4);
    match stream(command, Input::None, Some(Bytes::from(preamble)), None, None) {
        Ok(body) => (
            [
                (CONTENT_TYPE, format!("application/x-{}-advertisement", service.name())),
                (CACHE_CONTROL, "no-cache".to_owned()),
            ],
            body,
        )
            .into_response(),
        Err(failure) => text(failure),
    }
}

/// `POST …/git-upload-pack` or `…/git-receive-pack`: a fetch's negotiation or a push.
async fn exchange(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
    Route((owner, name, service)): Route<(String, String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(service) = Service::parse(&service) else {
        return text((StatusCode::NOT_FOUND, "no such service".to_owned()));
    };
    let expected = format!("application/x-{}-request", service.name());
    if headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok()) != Some(expected.as_str()) {
        return text((StatusCode::UNSUPPORTED_MEDIA_TYPE, "not git's request".to_owned()));
    }
    let (code, claims, record) = match authorize(&listener, credential, owner, name).await {
        Ok(authorized) => authorized,
        Err(failure) => return text(failure),
    };
    let run = claims.run;
    let repository = match code.workspace(&record, &claims.thread, false).await {
        Ok(repository) => repository,
        Err(failure) => return text(failure),
    };
    let encoding = headers.get(CONTENT_ENCODING).and_then(|value| value.to_str().ok());
    let public = record.public.one() == Some(&true);
    let command = code.pack(service, &repository, &run, false, public);
    let streamed = match service {
        Service::Upload => {
            let Ok(permit) =
                tokio::time::timeout(FETCH_WAIT, Arc::clone(&code.fetches).acquire_owned()).await
            else {
                return text((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "too many fetches; try again shortly".to_owned(),
                ));
            };
            let input = match asked(body, encoding).await {
                Ok(input) => input,
                Err(failure) => return text(failure),
            };
            stream(command, Input::Bytes(input), None, permit.ok(), None)
        },
        Service::Receive => {
            if encoding.is_some_and(|encoding| encoding != "identity") {
                return text((
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "send the push uncompressed".to_owned(),
                ));
            }
            let Some(busy) = Busy::take(&code.pushing, &format!("{run} {}", record.id)) else {
                return text((
                    StatusCode::TOO_MANY_REQUESTS,
                    "this run has a push to this repository under way".to_owned(),
                ));
            };
            stream(command, Input::Body(body), None, None, Some(busy))
        },
    };
    match streamed {
        Ok(body) => (
            [
                (CONTENT_TYPE, format!("application/x-{}-result", service.name())),
                (CACHE_CONTROL, "no-cache".to_owned()),
            ],
            body,
        )
            .into_response(),
        Err(failure) => text(failure),
    }
}

/// A fetch's request, decompressed if git compressed it, within [`MOST_ASKED`].
async fn asked(body: Body, encoding: Option<&str>) -> Result<Bytes, Failure> {
    let too_large =
        || (StatusCode::PAYLOAD_TOO_LARGE, "the fetch's request is too large".to_owned());
    let sent = tokio::time::timeout(TIME_LIMIT, axum::body::to_bytes(body, MOST_ASKED))
        .await
        .map_err(|_| (StatusCode::REQUEST_TIMEOUT, "the request was too slow".to_owned()))?
        .map_err(|_| too_large())?;
    match encoding {
        None | Some("identity") => Ok(sent),
        Some("gzip" | "x-gzip") => {
            let mut decoded = Vec::new();
            let limit = u64::try_from(MOST_ASKED).unwrap_or(u64::MAX) + 1;
            flate2::read::GzDecoder::new(sent.as_ref())
                .take(limit)
                .read_to_end(&mut decoded)
                .map_err(|_| (StatusCode::BAD_REQUEST, "the request isn't gzip".to_owned()))?;
            if decoded.len() > MOST_ASKED {
                return Err(too_large());
            }
            Ok(Bytes::from(decoded))
        },
        Some(_) => {
            Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "an encoding git doesn't use".to_owned()))
        },
    }
}

/// What git reads.
enum Input {
    None,
    Bytes(Bytes),
    Body(Body),
}

/// Marks a run's push to a repository under way until dropped.
struct Busy(Arc<Mutex<BTreeSet<String>>>, String);

impl Busy {
    fn take(pushing: &Arc<Mutex<BTreeSet<String>>>, run: &str) -> Option<Self> {
        pushing
            .lock()
            .ok()?
            .insert(run.to_owned())
            .then(|| Self(Arc::clone(pushing), run.to_owned()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut pushing) = self.0.lock() {
            pushing.remove(&self.1);
        }
    }
}

/// A response's stream: git's output, and what the response holds until git is done.
struct Streaming {
    stdout: tokio::process::ChildStdout,
    group: Group,
    _fetch: Option<tokio::sync::OwnedSemaphorePermit>,
    _busy: Option<Busy>,
    preamble: Option<Bytes>,
}

/// Runs `command`, feeding it `input`, and streams its output after `preamble`. Git, and whatever it runs, is killed
/// when the response is dropped before git finishes, as when the run hangs up, and when [`TIME_LIMIT`] passes.
fn stream(
    mut command: tokio::process::Command,
    input: Input,
    preamble: Option<Bytes>,
    fetch: Option<tokio::sync::OwnedSemaphorePermit>,
    busy: Option<Busy>,
) -> Result<Body, Failure> {
    let deadline = tokio::time::Instant::now() + TIME_LIMIT;
    let mut child = command
        .spawn()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, format!("git can't run: {err}")))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or_else(internal)?;
    let group = Group::new(child);
    if let Some(mut stdin) = stdin {
        tokio::spawn(async move {
            match input {
                Input::None => {},
                Input::Bytes(bytes) => {
                    let _ = stdin.write_all(&bytes).await;
                },
                Input::Body(body) => {
                    let mut chunks = body.into_data_stream();
                    while let Ok(Some(Ok(chunk))) =
                        tokio::time::timeout_at(deadline, chunks.next()).await
                    {
                        if stdin.write_all(&chunk).await.is_err() {
                            break;
                        }
                    }
                },
            }
            // Dropping stdin tells git the request is over.
        });
    }
    let state = Streaming { stdout, group, _fetch: fetch, _busy: busy, preamble };
    let chunks = futures_util::stream::unfold(Some(state), move |state| async move {
        let mut state = state?;
        if let Some(preamble) = state.preamble.take() {
            return Some((Ok::<_, std::io::Error>(preamble), Some(state)));
        }
        let mut buffer = vec![0; 64 * 1024];
        match tokio::time::timeout_at(deadline, state.stdout.read(&mut buffer)).await {
            Ok(Ok(0)) => {
                if let Ok(Ok(_)) = tokio::time::timeout_at(deadline, state.group.child.wait()).await
                {
                    state.group.done();
                }
                None
            },
            Ok(Ok(read)) => {
                buffer.truncate(read);
                Some((Ok(Bytes::from(buffer)), Some(state)))
            },
            Ok(Err(err)) => Some((Err(err), None)),
            Err(_) => Some((Err(std::io::Error::other("git took too long")), None)),
        }
    });
    Ok(Body::from_stream(chunks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> std::process::Command {
        let mut command = std::process::Command::new("git");
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Run")
            .env("GIT_AUTHOR_EMAIL", "run@example.com")
            .env("GIT_COMMITTER_NAME", "Run")
            .env("GIT_COMMITTER_EMAIL", "run@example.com")
            .stdin(Stdio::null());
        command
    }

    /// Git in `dir` with `args`, and what it printed.
    fn run(dir: &Path, args: &[&str]) -> String {
        let output = git().arg("-C").arg(dir).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Writes `text` as an object in `dir`, returning its id.
    fn object(dir: &Path, kind: &str, text: &[u8]) -> String {
        let mut child = git()
            .arg("-C")
            .arg(dir)
            .args(["hash-object", "-w", "-t", kind, "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), text).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn a_finished_threads_repository_goes_once_its_work_is_on_the_forge_and_then_the_mirror_drops_what_it_dropped()
     {
        let temp = tempfile::TempDir::new().unwrap();
        let (mirror, threads) = (temp.path().join("app.git"), temp.path().join("app.threads"));
        run(temp.path(), &["init", "-q", "--bare", "app.git"]);
        run(&mirror, &["config", "gc.pruneExpire", "never"]);
        let tree = object(&mirror, "tree", b"");
        let main = run(&mirror, &["commit-tree", &tree, "-m", "main"]);
        run(&mirror, &["update-ref", "refs/heads/main", &main]);
        // An object no ref reaches, as a branch the forge deleted leaves behind.
        let dropped = object(&mirror, "blob", b"dropped\n");
        let kept = || {
            git()
                .arg("-C")
                .arg(&mirror)
                .args(["cat-file", "-e", &dropped])
                .status()
                .unwrap()
                .success()
        };
        for thread in ["t0", "t1"] {
            let repository = threads.join(format!("{thread}.git"));
            run(temp.path(), &["init", "-q", "--bare", repository.to_str().unwrap()]);
            let borrowed = format!("{}\n", mirror.join("objects").display());
            std::fs::write(repository.join("objects/info/alternates"), borrowed).unwrap();
        }
        // t0's run's work is on the forge; t1's run's is its own alone.
        run(&threads.join("t0.git"), &["update-ref", "refs/runs/r0/work", &main]);
        let t1 = threads.join("t1.git");
        let work = run(&t1, &["commit-tree", &tree, "-p", &main, "-m", "work"]);
        run(&t1, &["update-ref", "refs/runs/r1/work", &work]);
        let finished: BTreeSet<String> = ["t0".to_owned(), "t1".to_owned()].into();
        collect(&git, &mirror, &threads, &finished);
        assert_eq!(borrowers(&threads), ["t1".to_owned()].into(), "t1's work isn't on the forge");
        assert!(kept(), "t1 may still borrow it");
        // t1's work lands.
        run(&mirror, &["fetch", "-q", t1.to_str().unwrap(), "refs/runs/r1/work:refs/heads/main"]);
        collect(&git, &mirror, &threads, &finished);
        assert!(borrowers(&threads).is_empty());
        assert!(!kept());
    }
}
