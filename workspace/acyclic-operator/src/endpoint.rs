//! The operator's git endpoint: where a run pushes its queued operations, over git's smart HTTP protocol.
//!
//! It serves `git-receive-pack` alone, at `/tracker.git`, and checks the run's token on every request, since the
//! protocol keeps no session. Each run has an inbox of its own, a bare repository under the operator's directory that
//! refuses deletions and rewrites and runs only the operator's hook ([`crate::hook`]), which checks and forwards the
//! push before git takes it. `git receive-pack` runs with an environment of the endpoint's making, nothing inherited,
//! and one at a time per run, within a time limit; past it, receive-pack and its hook are killed together, in their own
//! process group.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acyclic_core::private;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use crate::forward::{self, Tracker as _};
use crate::hook;
use crate::token::{Claims, Verifier};

/// The largest push, in bytes; the inbox refuses a larger pack too.
pub const MOST_BYTES: usize = 16 * 1024 * 1024;

/// How long one request may take, receiving the push and forwarding it included.
pub const TIME_LIMIT: Duration = Duration::from_secs(120);

const SERVICE: &str = "git-receive-pack";

/// What the endpoint needs.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The operator's id, as in `operator/<id>`.
    pub id: String,
    /// What checks run tokens.
    pub verifier: Verifier,
    /// The tracker the operations go to.
    pub tracker: forward::Clone,
    /// The operator's own directory, readable by the user alone: the inboxes and the hook live here.
    pub dir: PathBuf,
    /// The deny-list file, readable by the user alone.
    pub deny: PathBuf,
    /// The home directory holding the operator's key.
    pub home: PathBuf,
    /// The `acyclic` executable, which the hook runs as `acyclic operator-hook`.
    pub executable: PathBuf,
}

/// The endpoint's state.
#[derive(Debug)]
pub struct Endpoint {
    settings: Settings,
    path: String,
    /// The runs with a push under way.
    busy: Mutex<BTreeSet<String>>,
}

impl Endpoint {
    /// An endpoint keeping its inboxes and hook under `settings.dir`, which it makes private if it isn't.
    ///
    /// # Errors
    ///
    /// When the directory is open to others or the hook can't be written, or the executable's path can't be quoted
    /// for the hook.
    pub fn new(settings: Settings) -> Result<Self, String> {
        private::dir(&settings.dir).map_err(|err| err.to_string())?;
        let executable = settings.executable.display().to_string();
        if executable.contains('\'') || !settings.executable.is_absolute() {
            return Err(format!(
                "the hook can't run {executable}: give an absolute path without quotes"
            ));
        }
        let hooks = settings.dir.join("hooks");
        std::fs::create_dir_all(&hooks).map_err(|err| err.to_string())?;
        let script = format!("#!/bin/sh\nexec '{executable}' operator-hook\n");
        private::place(&hooks.join("pre-receive"), script.as_bytes(), true)
            .map_err(|err| err.to_string())?;
        std::fs::set_permissions(
            hooks.join("pre-receive"),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(|err| err.to_string())?;
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
        Ok(Self { settings, path, busy: Mutex::new(BTreeSet::new()) })
    }

    fn now(&self) -> acyclic_core::state::Millis {
        acyclic_cli::now(&self.settings.tracker.env)
    }

    /// The run's token, checked, and its run under way; otherwise why the request is refused.
    fn authorize(&self, headers: &HeaderMap) -> Result<(String, Claims), (StatusCode, String)> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|token| !token.is_empty());
        let unauthorized = |message: String| (StatusCode::UNAUTHORIZED, message);
        let token = token.ok_or_else(|| unauthorized("a run token is required".to_owned()))?;
        let claims = self
            .settings
            .verifier
            .verify(token, self.now())
            .map_err(|err| unauthorized(err.to_string()))?;
        if !claims.run.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Err(unauthorized("the token names no run".to_owned()));
        }
        match self.settings.tracker.active(&claims.run, &claims.thread) {
            Ok(true) => Ok((token.to_owned(), claims)),
            Ok(false) => {
                Err((StatusCode::FORBIDDEN, format!("run {} isn't under way", claims.run)))
            },
            Err(_) => {
                Err((StatusCode::SERVICE_UNAVAILABLE, "the tracker can't be read".to_owned()))
            },
        }
    }

    /// The run's inbox, made on first use.
    fn inbox(&self, run: &str) -> Result<PathBuf, String> {
        let inboxes = self.settings.dir.join("inbox");
        std::fs::create_dir_all(&inboxes).map_err(|err| err.to_string())?;
        let path = inboxes.join(format!("{run}.git"));
        if path.exists() {
            return Ok(path);
        }
        let staged = inboxes.join(format!(".{run}.{}.new", std::process::id()));
        let _ = std::fs::remove_dir_all(&staged);
        let hooks = self.settings.dir.join("hooks");
        let hooks = hooks.display().to_string();
        let maximum = MOST_BYTES.to_string();
        let staged_path = staged.display().to_string();
        let mut steps: Vec<Vec<&str>> =
            vec![vec!["init", "-q", "--bare", "--template=", staged_path.as_str()]];
        for (key, value) in [
            ("receive.denyNonFastForwards", "true"),
            ("receive.denyDeletes", "true"),
            ("receive.advertisePushOptions", "false"),
            ("receive.fsckObjects", "true"),
            ("receive.maxInputSize", maximum.as_str()),
            ("core.hooksPath", hooks.as_str()),
        ] {
            steps.push(vec!["-C", staged_path.as_str(), "config", key, value]);
        }
        for step in steps {
            let status = self
                .git()
                .args(&step)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|err| format!("git can't run: {err}"))?;
            if !status.success() {
                let _ = std::fs::remove_dir_all(&staged);
                return Err(format!("git {} failed for the inbox", step[0]));
            }
        }
        if std::fs::rename(&staged, &path).is_err() {
            // Another request made it first.
            let _ = std::fs::remove_dir_all(&staged);
        }
        if path.exists() { Ok(path) } else { Err("the inbox couldn't be made".to_owned()) }
    }

    /// Git with nothing of this process's environment.
    fn git(&self) -> Command {
        let mut command = Command::new("git");
        command
            .env_clear()
            .env("PATH", &self.path)
            .env("HOME", &self.settings.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        command
    }

    fn receive_pack(
        &self,
        token: &str,
        inbox: &Path,
        advertise: bool,
        input: Vec<u8>,
        deadline: Instant,
    ) -> Result<Vec<u8>, String> {
        let vars: BTreeMap<String, String> = hook::environment(
            token,
            &self.settings.tracker.tracker,
            &self.settings.id,
            &self.settings.deny,
            &self.settings.home,
            &self.path,
            self.settings.tracker.env.vars.get("ACYCLIC_NOW").map(String::as_str),
        );
        let mut command = self.git();
        command.envs(vars).args(["receive-pack", "--stateless-rpc"]);
        if advertise {
            command.arg("--advertise-refs");
        }
        let mut child = command
            .arg(inbox)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("git can't run: {err}"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stdout.read_to_end(&mut output);
            output
        });
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                },
                _ => {
                    // The hook too, which would otherwise carry on forwarding.
                    let group = format!("-{}", child.id());
                    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the push took too long".to_owned());
                },
            }
        }
        let _ = writer.join();
        // Receive-pack reports a refused push in its output, which the client reads, so its exit status says nothing
        // more.
        reader.join().map_err(|_| "git's output was lost".to_owned())
    }
}

/// Marks a run's push under way until dropped.
struct Busy<'a>(&'a Endpoint, String);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.0.busy.lock() {
            busy.remove(&self.1);
        }
    }
}

/// The endpoint's routes.
pub fn router(endpoint: Arc<Endpoint>) -> Router {
    Router::new()
        .route("/tracker.git/info/refs", get(refs))
        .route(&format!("/tracker.git/{SERVICE}"), post(receive))
        .route("/tracker.git/git-upload-pack", post(|| async { forbidden() }))
        .layer(axum::extract::DefaultBodyLimit::max(MOST_BYTES))
        .with_state(endpoint)
}

/// A refusal as a response, asking for a token when it is one.
fn refusal((status, message): (StatusCode, String)) -> Response {
    let mut response = (status, format!("{message}\n")).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    response
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "this endpoint takes pushes alone\n").into_response()
}

async fn refs(State(endpoint): State<Arc<Endpoint>>, uri: Uri, headers: HeaderMap) -> Response {
    if uri.query() != Some(&format!("service={SERVICE}")) {
        return forbidden();
    }
    let deadline = Instant::now() + TIME_LIMIT;
    let work = tokio::task::spawn_blocking(move || {
        let (token, claims) = match endpoint.authorize(&headers) {
            Ok(authorized) => authorized,
            Err(refused) => return refusal(refused),
        };
        let advertised = endpoint
            .inbox(&claims.run)
            .and_then(|inbox| endpoint.receive_pack(&token, &inbox, true, Vec::new(), deadline));
        match advertised {
            Ok(output) => {
                let first = format!("# service={SERVICE}\n");
                let mut body = format!("{:04x}{first}0000", first.len() + 4).into_bytes();
                body.extend(output);
                ([(header::CONTENT_TYPE, format!("application/x-{SERVICE}-advertisement"))], body)
                    .into_response()
            },
            Err(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{message}\n")).into_response()
            },
        }
    });
    work.await.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn receive(
    State(endpoint): State<Arc<Endpoint>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let deadline = Instant::now() + TIME_LIMIT;
    if headers.get(header::CONTENT_ENCODING).is_some_and(|value| value != "identity") {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "send the push uncompressed\n")
            .into_response();
    }
    if headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
        != Some(&format!("application/x-{SERVICE}-request"))
    {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "not a push\n").into_response();
    }
    // The token first, before reading a byte of the push.
    let checked = {
        let endpoint = Arc::clone(&endpoint);
        let headers = headers.clone();
        tokio::task::spawn_blocking(move || endpoint.authorize(&headers)).await
    };
    let (token, claims) = match checked {
        Ok(Ok(authorized)) => authorized,
        Ok(Err(refused)) => return refusal(refused),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let left = deadline.saturating_duration_since(Instant::now());
    let Ok(Ok(input)) = tokio::time::timeout(left, axum::body::to_bytes(body, MOST_BYTES)).await
    else {
        return (StatusCode::REQUEST_TIMEOUT, "the push was too slow or too large\n")
            .into_response();
    };
    let work = tokio::task::spawn_blocking(move || {
        {
            let Ok(mut busy) = endpoint.busy.lock() else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            if !busy.insert(claims.run.clone()) {
                return (StatusCode::TOO_MANY_REQUESTS, "this run has a push under way\n")
                    .into_response();
            }
        }
        let _busy = Busy(&endpoint, claims.run.clone());
        let received = endpoint.inbox(&claims.run).and_then(|inbox| {
            endpoint.receive_pack(&token, &inbox, false, input.to_vec(), deadline)
        });
        match received {
            Ok(output) => {
                ([(header::CONTENT_TYPE, format!("application/x-{SERVICE}-result"))], output)
                    .into_response()
            },
            Err(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{message}\n")).into_response()
            },
        }
    });
    work.await.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
