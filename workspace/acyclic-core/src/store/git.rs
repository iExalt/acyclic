//! The repository operations the store needs, over `gix`: plumbing only, never a work tree, and no `git` process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use gix::refs::Target;
use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

use super::{Maintenance, StoreError};

/// An error with its causes, as one line.
pub(crate) fn describe(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

fn failed(err: impl std::error::Error + 'static) -> StoreError {
    StoreError::Git(describe(&err))
}

fn parse_id(text: &str) -> Result<gix::ObjectId, StoreError> {
    gix::ObjectId::from_hex(text.as_bytes())
        .map_err(|e| StoreError::Git(format!("{text} isn't an object id: {e}")))
}

/// Finds the repository from the path alone, as `git -C <path>` does, taking no repository or settings from the
/// environment, and without reflogs for the tracker's refs.
fn discover(path: &Path) -> Result<gix::ThreadSafeRepository, String> {
    let overrides = || {
        let mut overrides: Vec<String> = vec![
            "core.logAllRefUpdates=false".into(),
            "gitoxide.core.nativeLocalTransport=true".into(),
            // A transfer slower than 1,000 bytes a second for 30 seconds fails over HTTP rather than hanging the sync,
            // and the server's writes queued behind it.
            "http.lowSpeedLimit=1000".into(),
            "http.lowSpeedTime=30".into(),
        ];
        if let Some(dir) = credential_exec_path() {
            overrides.push(format!("gitoxide.credentials.execPath={}", dir.display()));
        }
        overrides
    };
    let options = || {
        gix::open::Options::default().ignore_repository_environment(true).cli_overrides(overrides())
    };
    let mapping = gix::sec::trust::Mapping { full: options(), reduced: options() };
    gix::ThreadSafeRepository::discover_opts(
        path,
        gix::discover::upwards::Options::default(),
        mapping,
    )
    .map_err(|e| format!("can't open {}: {}", path.display(), describe(&e)))
}

/// Where Git keeps its helpers on a Mac, which `gix` can't ask `git` for: the first of these that holds the keychain
/// helper, so a configured `osxkeychain` credential helper is found without a `git` on the `PATH`.
const MACOS_EXEC_PATHS: [&str; 4] = [
    "/Library/Developer/CommandLineTools/usr/libexec/git-core",
    "/Applications/Xcode.app/Contents/Developer/usr/libexec/git-core",
    "/opt/homebrew/opt/git/libexec/git-core",
    "/usr/local/opt/git/libexec/git-core",
];

/// The first of `candidates` that holds a `git-credential-osxkeychain`.
fn first_holding_keychain_helper<'a>(candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .copied()
        .find(|dir| Path::new(dir).join("git-credential-osxkeychain").is_file())
}

/// The directory of Git's own helpers to search for credential helpers: only on a Mac, and only when the user hasn't
/// pointed `GIT_EXEC_PATH` somewhere. Elsewhere Git's helpers are found on the `PATH`, as Git finds them.
fn credential_exec_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") && std::env::var_os("GIT_EXEC_PATH").is_none() {
        return first_holding_keychain_helper(&MACOS_EXEC_PATHS).map(PathBuf::from);
    }
    None
}

/// A repository addressed by its path.
///
/// It is opened once and found by that path alone, taking no repository or settings from the environment, even when
/// the process runs inside a git hook, and without reflogs for the tracker's refs: each would be another fsynced append
/// per batch, the remote holds the history, and a squash makes it moot.
#[derive(Debug)]
pub(crate) struct Git {
    pub(crate) repo: PathBuf,
    opened: OnceLock<Result<gix::ThreadSafeRepository, String>>,
}

impl Git {
    pub(crate) fn new(repo: &Path) -> Self {
        Self { repo: repo.to_owned(), opened: OnceLock::new() }
    }

    /// The repository on this thread.
    pub(crate) fn handle(&self) -> Result<gix::Repository, StoreError> {
        let opened = self.opened.get_or_init(|| discover(&self.repo));
        match opened {
            Ok(repo) => Ok(repo.to_thread_local()),
            Err(message) => Err(StoreError::Git(message.clone())),
        }
    }

    /// The repository as it is now, with the packs that were written since it was opened. A fetch or a push judges
    /// what the local refs lead to without looking for new packs, so it needs this.
    fn fresh_handle(&self) -> Result<gix::Repository, StoreError> {
        match discover(&self.repo) {
            Ok(repo) => Ok(repo.to_thread_local()),
            Err(message) => Err(StoreError::Git(message)),
        }
    }

    /// A ref's commit id, or `None` when the ref doesn't exist or doesn't lead to a commit.
    pub(crate) fn resolve(&self, reference: &str) -> Result<Option<String>, StoreError> {
        let repo = self.handle()?;
        let Some(mut found) = repo.try_find_reference(reference).map_err(failed)? else {
            return Ok(None);
        };
        let id = found.peel_to_id().map_err(failed)?.detach();
        let header = repo.find_header(id).map_err(failed)?;
        Ok((header.kind() == gix::object::Kind::Commit).then(|| id.to_string()))
    }

    /// Refs under a prefix with their object ids.
    pub(crate) fn refs(&self, prefix: &str) -> Result<BTreeMap<String, String>, StoreError> {
        self.refs_under(&[prefix])
    }

    /// Refs under any of several prefixes (ending in `/`), or named exactly, with their object ids.
    pub(crate) fn refs_under(
        &self,
        prefixes: &[&str],
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let repo = self.handle()?;
        let platform = repo.references().map_err(failed)?;
        let mut found = BTreeMap::new();
        for pattern in prefixes {
            let name = pattern.trim_end_matches('/');
            for reference in platform.under(name).map_err(failed)? {
                let reference = reference.map_err(failed)?;
                if let Some(id) = reference.try_id() {
                    found.insert(reference.name().as_bstr().to_string(), id.to_string());
                }
            }
        }
        Ok(found)
    }

    /// Every file in a commit's tree, by path.
    pub(crate) fn read_tree(&self, commit: &str) -> Result<BTreeMap<String, Vec<u8>>, StoreError> {
        let repo = self.handle()?;
        let tree = repo.find_commit(parse_id(commit)?).map_err(failed)?.tree().map_err(failed)?;
        let mut files = BTreeMap::new();
        read_blobs(&repo, &tree, &mut String::new(), commit, &mut files)?;
        Ok(files)
    }

    /// The contents of the given paths in a commit's tree; missing paths are left out.
    pub(crate) fn read_paths(
        &self,
        commit: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, StoreError> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let repo = self.handle()?;
        let Some(commit) = repo.try_find_object(parse_id(commit)?).map_err(failed)? else {
            return Ok(BTreeMap::new());
        };
        let tree = commit.try_into_commit().map_err(failed)?.tree().map_err(failed)?;
        let mut files = BTreeMap::new();
        for path in paths {
            let Some(entry) = tree.lookup_entry_by_path(path).map_err(failed)? else { continue };
            if entry.mode().is_blob() {
                let blob = repo.find_object(entry.object_id()).map_err(failed)?;
                files.insert(path.clone(), blob.detach().data);
            }
        }
        Ok(files)
    }

    /// Writes a tree from `base` (a commit, or none) with `changes` applied: `Some` content adds or replaces a file and
    /// `None` removes it. No index, work tree or shared file is touched.
    pub(crate) fn write_tree(
        &self,
        base: Option<&str>,
        changes: &BTreeMap<String, Option<Vec<u8>>>,
    ) -> Result<String, StoreError> {
        let repo = self.handle()?;
        Ok(edit_tree(&repo, &repo, base, changes)?.to_string())
    }

    /// The tree of a commit.
    pub(crate) fn tree_of(&self, commit: &str) -> Result<String, StoreError> {
        let repo = self.handle()?;
        Ok(repo
            .find_commit(parse_id(commit)?)
            .map_err(failed)?
            .tree_id()
            .map_err(failed)?
            .to_string())
    }

    /// When a commit was committed, in seconds since the epoch; zero when that can't be read.
    pub(crate) fn commit_seconds(&self, commit: &str) -> Result<i64, StoreError> {
        let repo = self.handle()?;
        let commit = repo.find_commit(parse_id(commit)?).map_err(failed)?;
        Ok(commit.time().map_or(0, |time| time.seconds))
    }

    /// The repository's common Git directory, shared by its worktrees.
    pub(crate) fn common_dir(&self) -> Result<PathBuf, StoreError> {
        Ok(self.handle()?.common_dir().to_owned())
    }

    /// Whether `ancestor` is `descendant` or one of its ancestors; an object that can't be read is no ancestor.
    pub(crate) fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let (Ok(ancestor), Ok(descendant)) = (parse_id(ancestor), parse_id(descendant)) else {
            return false;
        };
        self.handle()
            .and_then(|repo| repo.is_ancestor(ancestor, descendant).map_err(failed))
            .unwrap_or(false)
    }

    /// Whether `ancestor` is `descendant` or one of its ancestors, failing when either can't be read.
    pub(crate) fn is_ancestor_checked(
        &self,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, StoreError> {
        self.handle()?.is_ancestor(parse_id(ancestor)?, parse_id(descendant)?).map_err(failed)
    }

    /// A commit of `tree` with an optional parent, authored as the replica at `seconds` since the epoch; written as
    /// `git commit-tree -m` does, with a newline after the message.
    pub(crate) fn commit(
        &self,
        tree: &str,
        parent: Option<&str>,
        replica: &str,
        seconds: i64,
        message: &str,
    ) -> Result<String, StoreError> {
        let repo = self.handle()?;
        new_commit(&repo, tree, parent, replica, seconds, &format!("{message}\n"))
    }

    /// Writes commits, so a batch's objects land in a single pack and no loose files are written. Returns the commits'
    /// ids in order. The commits are what `git fast-import` made of the same input, byte for byte: the message has no
    /// newline added.
    ///
    /// # Errors
    ///
    /// Fails when the objects can't be written, and refuses an empty path.
    pub(crate) fn import_commits(
        &self,
        commits: &[NewCommit<'_>],
    ) -> Result<Vec<String>, StoreError> {
        if let Some(path) = commits
            .iter()
            .flat_map(|c| c.files.keys())
            .find(|path| path.is_empty() || path.contains('\n') || path.starts_with('"'))
        {
            return Err(StoreError::Git(format!("can't import the path {path:?}")));
        }
        // Objects are made in memory first, then written as one pack.
        let mut staging = self.handle()?.with_object_memory();
        let mut ids = Vec::with_capacity(commits.len());
        for commit in commits {
            let tree = edit_tree(&staging, &staging, commit.parent, commit.files)?;
            ids.push(new_commit(
                &staging,
                &tree.to_string(),
                commit.parent,
                commit.replica,
                commit.seconds,
                commit.message,
            )?);
        }
        let objects: Vec<(gix::object::Kind, Vec<u8>)> = staging
            .objects
            .take_object_memory()
            .map(|storage| storage.values().cloned().collect())
            .unwrap_or_default();
        let repo = self.handle()?;
        repo.write_objects_as_pack(objects, &std::sync::atomic::AtomicBool::new(false))
            .map_err(failed)?;
        Ok(ids)
    }

    /// Starts a transaction on refs, which applies all of its changes or none.
    pub(crate) fn transaction(&self) -> Result<Transaction, StoreError> {
        let repo = self.handle()?;
        let common = repo.common_dir().to_owned();
        Ok(Transaction { repo, edits: Vec::new(), common })
    }

    /// Points `reference` at `new`, whatever it held.
    pub(crate) fn set_ref(&self, reference: &str, new: &str) -> Result<(), StoreError> {
        let mut transaction = self.transaction()?;
        transaction.set(reference, new)?;
        transaction.commit(None)
    }

    /// Deletes `reference`, which must be at `old` when given, and does nothing when it doesn't exist.
    pub(crate) fn delete_ref(&self, reference: &str, old: Option<&str>) -> Result<(), StoreError> {
        if self.resolve(reference)?.is_none() {
            return Ok(());
        }
        let mut transaction = self.transaction()?;
        match old {
            Some(old) => transaction.delete(reference, old)?,
            None => transaction.delete_any(reference)?,
        }
        transaction.commit(None)
    }
}

/// Reads the blobs below `tree` into `files`, by their paths below `prefix`.
fn read_blobs(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    prefix: &mut String,
    commit: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), StoreError> {
    for entry in tree.iter() {
        let entry = entry.map_err(failed)?;
        let name = entry.filename().to_string();
        let length = prefix.len();
        prefix.push_str(&name);
        if entry.mode().is_tree() {
            prefix.push('/');
            let subtree = repo.find_tree(entry.oid()).map_err(failed)?;
            read_blobs(repo, &subtree, prefix, commit, files)?;
        } else {
            // A tree names its own objects, so one missing is a damaged object store: read on, and the state would
            // lose that record with nothing to say so.
            let Some(object) = repo.try_find_object(entry.oid()).map_err(failed)? else {
                return Err(StoreError::Git(format!(
                    "{commit} lists {prefix}, but its object {} is missing from the repository",
                    entry.oid()
                )));
            };
            files.insert(prefix.clone(), object.detach().data);
        }
        prefix.truncate(length);
    }
    Ok(())
}

/// A tree made from the tree of `base` (a commit, or none for an empty tree) with `changes` applied, written to
/// `repo`'s object database (or its memory).
fn edit_tree(
    reading: &gix::Repository,
    writing: &gix::Repository,
    base: Option<&str>,
    changes: &BTreeMap<String, Option<Vec<u8>>>,
) -> Result<gix::ObjectId, StoreError> {
    let start = match base {
        Some(base) => reading
            .find_commit(parse_id(base)?)
            .map_err(failed)?
            .tree_id()
            .map_err(failed)?
            .detach(),
        None => gix::ObjectId::empty_tree(reading.object_hash()),
    };
    let mut editor = reading.edit_tree(start).map_err(failed)?;
    for (path, content) in changes {
        match content {
            Some(content) => {
                let blob = writing.write_blob(content).map_err(failed)?.detach();
                editor
                    .upsert(path.as_str(), gix::object::tree::EntryKind::Blob, blob)
                    .map_err(failed)?;
            },
            None => {
                editor.remove(path.as_str()).map_err(failed)?;
            },
        }
    }
    Ok(editor.write().map_err(failed)?.detach())
}

fn new_commit(
    repo: &gix::Repository,
    tree: &str,
    parent: Option<&str>,
    replica: &str,
    seconds: i64,
    message: &str,
) -> Result<String, StoreError> {
    let name = format!("acyclic {replica}");
    let email = format!("{replica}@acyclic.invalid");
    let time = format!("{seconds} +0000");
    let signature = gix::actor::SignatureRef {
        name: name.as_str().into(),
        email: email.as_str().into(),
        time: &time,
    };
    let parents = parent.map(parse_id).transpose()?;
    let id = repo
        .new_commit_as(signature, signature, message, parse_id(tree)?, parents)
        .map_err(failed)?;
    Ok(id.id.to_string())
}

/// A commit for [`Git::import_commits`]: `files` applied to `parent`'s tree (or to an empty tree), where `Some`
/// content adds or replaces a file and `None` removes it.
pub(crate) struct NewCommit<'a> {
    pub(crate) parent: Option<&'a str>,
    pub(crate) files: &'a BTreeMap<String, Option<Vec<u8>>>,
    pub(crate) replica: &'a str,
    pub(crate) seconds: i64,
    pub(crate) message: &'a str,
}

/// Ref changes, applied together by [`Transaction::commit`].
pub(crate) struct Transaction {
    repo: gix::Repository,
    edits: Vec<RefEdit>,
    common: PathBuf,
}

impl Transaction {
    fn log() -> LogChange {
        LogChange {
            mode: RefLog::AndReference,
            force_create_reflog: false,
            message: "acyclic".into(),
        }
    }

    fn push(&mut self, reference: &str, change: Change) -> Result<(), StoreError> {
        let name = gix::refs::FullName::try_from(reference).map_err(failed)?;
        self.edits.push(RefEdit { change, name, deref: false });
        Ok(())
    }

    /// Moves `reference` from `old` (or creates it when `None`) to `new`.
    pub(crate) fn update(
        &mut self,
        reference: &str,
        new: &str,
        old: Option<&str>,
    ) -> Result<(), StoreError> {
        let expected = match old {
            Some(old) => PreviousValue::MustExistAndMatch(Target::Object(parse_id(old)?)),
            None => PreviousValue::MustNotExist,
        };
        let change =
            Change::Update { log: Self::log(), expected, new: Target::Object(parse_id(new)?) };
        self.push(reference, change)
    }

    /// Points `reference` at `new`, whatever it held.
    pub(crate) fn set(&mut self, reference: &str, new: &str) -> Result<(), StoreError> {
        let change = Change::Update {
            log: Self::log(),
            expected: PreviousValue::Any,
            new: Target::Object(parse_id(new)?),
        };
        self.push(reference, change)
    }

    /// Deletes `reference`, which must be at `old`.
    pub(crate) fn delete(&mut self, reference: &str, old: &str) -> Result<(), StoreError> {
        let expected = PreviousValue::MustExistAndMatch(Target::Object(parse_id(old)?));
        self.push(reference, Change::Delete { expected, log: RefLog::AndReference })
    }

    /// Deletes `reference`, whatever it holds.
    pub(crate) fn delete_any(&mut self, reference: &str) -> Result<(), StoreError> {
        self.push(
            reference,
            Change::Delete { expected: PreviousValue::Any, log: RefLog::AndReference },
        )
    }

    /// Locks every ref, checks every old value and applies the changes, running the `reference-transaction` hook as
    /// `git update-ref` does. `between` runs after the refs are locked and checked, before anything changes; it
    /// is for tests that crash there, and skips the hook.
    pub(crate) fn commit(self, between: Option<&dyn Fn()>) -> Result<(), StoreError> {
        let Self { repo, edits, common } = self;
        let names: Vec<String> = edits.iter().map(|edit| edit.name.as_bstr().to_string()).collect();
        let fallback = gix::actor::SignatureRef {
            name: "acyclic".into(),
            email: "acyclic@acyclic.invalid".into(),
            time: "0 +0000",
        };
        let result = match between {
            None => repo.edit_references_with_hooks(edits, Some(fallback)).map(|_| ()),
            Some(between) => {
                let lock = gix::lock::acquire::Fail::AfterDurationWithBackoff(
                    std::time::Duration::from_millis(100),
                );
                let prepared = repo
                    .ref_store()
                    .transaction()
                    .prepare(edits, gix::refs::view::transaction::LockOptions::new(lock, lock));
                match prepared {
                    Ok(prepared) => {
                        between();
                        prepared.commit(Some(fallback)).map(|_| ())
                    },
                    Err(e) => Err(e),
                }
            },
        };
        result.map_err(|e| classify(&e, &common, &names))
    }
}

/// A failed ref transaction as the store reports it: a lost race, a held lock or another failure. The lock file is the
/// one of the first ref that has one under `common`, the directory the refs live in.
fn classify(err: &gix::Error, common: &Path, names: &[String]) -> StoreError {
    use gix::refs::file::transaction::prepare::{MustNotExist, ReferenceOutOfDate};
    if err.downcast_any_ref::<ReferenceOutOfDate>().is_some()
        || err.downcast_any_ref::<MustNotExist>().is_some()
    {
        return StoreError::Conflict;
    }
    let held = err
        .downcast_any_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists);
    if held {
        // The lock file that exists: one of the refs being changed, or the one that guards `packed-refs`.
        let path = names
            .iter()
            .map(|name| common.join(format!("{name}.lock")))
            .chain(std::iter::once(common.join("packed-refs.lock")))
            .find(|path| path.exists())
            .or_else(|| names.first().map(|name| common.join(format!("{name}.lock"))));
        if let Some(path) = path {
            let age = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs());
            return StoreError::Locked { path, age_seconds: age };
        }
    }
    StoreError::Git(describe(err))
}

impl Git {
    /// The first value, in git's order of its sources, of each of `keys`, as git lists them; a key without a value
    /// isn't in the map.
    pub(crate) fn config_values(
        &self,
        keys: &[&str],
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let repo = self.handle()?;
        let config = repo.config_snapshot();
        let mut values = BTreeMap::new();
        for key in keys {
            if let Ok(found) = config.plumbing().raw_values(key)
                && let Some(first) = found.first()
            {
                values.insert((*key).to_owned(), first.to_string());
            }
        }
        Ok(values)
    }

    /// Sets keys in the clone's own configuration file, as `git config <key> <value>` does.
    pub(crate) fn set_config(&self, values: &[(&str, &str)]) -> Result<(), StoreError> {
        let repo = self.handle()?;
        let mut file = repo.config_file_mut(repo.common_dir().join("config")).map_err(failed)?;
        for (key, value) in values {
            file.set_raw_value(key, *value).map_err(failed)?;
        }
        file.commit().map_err(failed)
    }

    /// Removes a key from the clone's own configuration file, as `git config --unset` does.
    pub(crate) fn unset_config(&self, key: &str) -> Result<(), StoreError> {
        let repo = self.handle()?;
        let mut file = repo.config_file_mut(repo.common_dir().join("config")).map_err(failed)?;
        if let Ok(mut values) = file.raw_values_mut(key) {
            values.delete_all();
        }
        file.commit().map_err(failed)
    }

    /// Repacks when the store's thresholds are crossed, as `git gc --auto` does, in the way `how` says. Best effort.
    pub(crate) fn auto_maintenance(&self, how: Maintenance) {
        match how {
            Maintenance::Inline => {
                run_gc(&self.repo);
            },
            Maintenance::Thread => self.detached(|path| {
                let path = path.to_owned();
                std::thread::spawn(move || run_gc_and_clear_marker(&path));
            }),
            Maintenance::Process => self.detached(|path| {
                let Ok(exe) = std::env::current_exe() else { return };
                let mut child = std::process::Command::new(exe);
                child
                    .arg(MAINTAIN_ARG)
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt as _;
                    // Its own process group, so the terminal's signals for the command don't reach it.
                    child.process_group(0);
                }
                // Best effort: a binary replaced on disk since it started can't be run again from its old path.
                let _ = child.spawn();
            }),
        }
    }

    /// Starts `start` unless a run began lately: the marker file's age says a run is under way or failed, and either
    /// way the next command shouldn't start another.
    fn detached(&self, start: impl FnOnce(&Path)) {
        let Ok(common) = self.common_dir() else { return };
        let marker = common.join("acyclic").join("maintenance");
        let lately = std::fs::metadata(&marker)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age < BACKOFF);
        if lately {
            return;
        }
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&marker, b"").is_ok() {
            start(&self.repo);
        }
    }
}

/// How long after a detached run began another isn't started.
const BACKOFF: std::time::Duration = std::time::Duration::from_secs(30);

/// The longest a detached run goes on before it is told to stop.
const BOUND: std::time::Duration = std::time::Duration::from_secs(120);

/// The argument that makes the binary run the repack for the clone that follows it instead of a command. It is not one of
/// the commands' arguments, and it isn't in the environment, which a hook of the user's would inherit.
pub(crate) const MAINTAIN_ARG: &str = "--maintain-store-in-background";

/// Repacks the clone at `path` when git's thresholds are crossed, here and now. Best effort. Whether the attempt is
/// settled: it ran, or nothing was due. A run that failed, found another one busy, was declined by the `pre-auto-gc` hook
/// or met a recent `gc.log` isn't, and is tried again by the first command 30 seconds on, for as long as the cause lasts.
fn run_gc(path: &Path) -> bool {
    let git = Git::new(path);
    let Ok(repo) = git.fresh_handle() else { return false };
    let mut options = gix::maintenance::gc::Options::default();
    options.auto = true;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stop);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = std::sync::Arc::clone(&done);
    // A repack that goes on past its bound is told to stop, so a stuck one doesn't hold the clone for good.
    let watchdog = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while !finished.load(std::sync::atomic::Ordering::Relaxed) {
            if started.elapsed() > BOUND {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    let outcome = repo.gc(options, &stop);
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = watchdog.join();
    matches!(outcome, Ok(outcome) if matches!(outcome.skipped, None | Some(gix::maintenance::gc::Skipped::NotDue)))
}

/// Repacks, and then lets the next command start another repack; after a failure it keeps that from the marker's age.
fn run_gc_and_clear_marker(path: &Path) {
    if run_gc(path)
        && let Ok(common) = Git::new(path).common_dir()
    {
        let _ = std::fs::remove_file(common.join("acyclic").join("maintenance"));
    }
}

/// Runs the repack a parent asked for with [`MAINTAIN_ARG`], and says whether this process was started for that.
pub(crate) fn maintenance_child() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_none_or(|first| first != MAINTAIN_ARG) {
        return false;
    }
    let Some(path) = args.next() else { return false };
    run_gc_and_clear_marker(Path::new(&path));
    true
}

/// A ref for [`Git::push`]: `new` is the id it ends at, or `None` to delete it; `lease` is the id the remote's copy
/// has, or `None` when it has none. `rewrite` lets the update replace history, as a forced push does.
pub(crate) struct PushRef {
    pub(crate) name: String,
    pub(crate) lease: Option<String>,
    pub(crate) new: Option<String>,
    pub(crate) rewrite: bool,
}

/// Why a push moved nothing.
pub(crate) enum Refused {
    /// The remote's ref isn't what the push expected.
    Stale(String),
    /// The remote refused a ref, with its reason.
    Rejected(String, String),
}

impl Git {
    /// Makes the relative paths that the remote `name` is configured with, `url` and `pushurl`, absolute in `repo`'s
    /// configuration in memory, counting from the top of the work tree where git runs a fetch or a push. The remote keeps
    /// its name and every other setting, so it is found as Git finds it.
    fn absolutize_urls(repo: &mut gix::Repository, name: &str) -> Result<(), StoreError> {
        if repo.try_find_remote(name).is_none() {
            return Ok(());
        }
        let top = repo.workdir().unwrap_or_else(|| repo.git_dir()).to_owned();
        let mut changes = Vec::new();
        for key in ["url", "pushurl"] {
            let full = format!("remote.{name}.{key}");
            let Some(value) = repo.config_snapshot().string(full.as_str()).map(|v| v.to_string())
            else {
                continue;
            };
            let relative = gix::url::parse(gix::bstr::BStr::new(value.as_bytes()))
                .ok()
                .filter(|url| url.scheme == gix::url::Scheme::File)
                .and_then(|url| gix::path::from_byte_slice(&url.path).ok().map(Path::to_owned))
                .filter(|path| path.is_relative());
            if let Some(path) = relative {
                changes.push((full, top.join(path).to_string_lossy().into_owned()));
            }
        }
        if !changes.is_empty() {
            let mut snapshot = repo.config_snapshot_mut();
            for (key, value) in &changes {
                snapshot.set_raw_value(key.as_str(), value.as_str()).map_err(failed)?;
            }
            snapshot.commit().map_err(failed)?;
        }
        Ok(())
    }

    /// The remote by name, or at a URL or path; a relative path counts from the repository.
    fn remote<'r>(
        &self,
        repo: &'r gix::Repository,
        remote: &str,
    ) -> Result<gix::Remote<'r>, StoreError> {
        if let Some(found) = repo.try_find_remote(remote) {
            return found.map_err(failed);
        }
        let path = Path::new(remote);
        let at = if path.is_relative() && self.repo.join(path).exists() {
            self.repo.join(path).to_string_lossy().into_owned()
        } else {
            remote.to_owned()
        };
        repo.remote_at(at).map_err(failed)
    }

    /// The refs of `remote` named by `patterns`: exactly, or under one ending in `/`; by name with their ids.
    pub(crate) fn list_remote(
        &self,
        remote: &str,
        patterns: &[String],
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let mut repo = self.handle()?;
        Self::absolutize_urls(&mut repo, remote)?;
        self.list_remote_with(&repo, remote, patterns)
    }

    fn list_remote_with(
        &self,
        repo: &gix::Repository,
        remote: &str,
        patterns: &[String],
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let mut remote = self.remote(repo, remote)?;
        let specs: Vec<String> = patterns
            .iter()
            .map(|pattern| match pattern.strip_suffix('/') {
                Some(_) => {
                    format!("{pattern}*:refs/acyclic/listing/{}*", &pattern["refs/".len()..])
                },
                None => format!("{pattern}:refs/acyclic/listing/{}", &pattern["refs/".len()..]),
            })
            .collect();
        remote
            .replace_refspecs(specs.iter().map(String::as_str), gix::remote::Direction::Fetch)
            .map_err(failed)?;
        let connection = remote.connect(gix::remote::Direction::Fetch).map_err(failed)?;
        let map = connection
            .ref_map(gix::progress::Discard, gix::remote::ref_map::Options::default())
            .map_err(failed)?;
        let mut found = BTreeMap::new();
        for reference in &map.0.remote_refs {
            let (name, id, _) = reference.unpack();
            let Some(id) = id else { continue };
            let name = name.to_string();
            let wanted = patterns.iter().any(|pattern| {
                if pattern.ends_with('/') {
                    name.starts_with(pattern.as_str())
                } else {
                    name == *pattern
                }
            });
            if wanted {
                found.insert(name, id.to_string());
            }
        }
        Ok(found)
    }

    /// Fetches `refspecs` from `remote`, pruning the local refs that the specs map and the remote no longer has,
    /// and following the tags that point into what it receives, as `git fetch` does.
    pub(crate) fn fetch(&self, remote: &str, refspecs: &[String]) -> Result<(), StoreError> {
        let mut repo = self.fresh_handle()?;
        Self::absolutize_urls(&mut repo, remote)?;
        let mut remote = self.remote(&repo, remote)?;
        remote
            .replace_refspecs(refspecs.iter().map(String::as_str), gix::remote::Direction::Fetch)
            .map_err(failed)?;
        let connection = remote.connect(gix::remote::Direction::Fetch).map_err(failed)?;
        connection
            .prepare_fetch(gix::progress::Discard, gix::remote::ref_map::Options::default())
            .map_err(failed)?
            .with_prune(true)
            .receive(gix::progress::Discard, &std::sync::atomic::AtomicBool::new(false))
            .map_err(|err| match err.downcast_any_ref::<gix::remote::fetch::ObjectsRefused>() {
                Some(refused) => StoreError::ObjectsRefused(refused.report.to_string()),
                None => failed(err),
            })?;
        Ok(())
    }

    /// Pushes `refs` to `remote` in one atomic push, running the `pre-push` hook. Returns what was refused when the
    /// push moved nothing, and the number of refs it updated or deleted otherwise.
    pub(crate) fn push(
        &self,
        remote: &str,
        refs: &[PushRef],
    ) -> Result<Result<usize, Vec<Refused>>, StoreError> {
        use gix::remote::push::{Lease, Refusal, Status, Update};
        let mut repo = self.fresh_handle()?;
        Self::absolutize_urls(&mut repo, remote)?;
        let remote = self.remote(&repo, remote)?;
        let mut updates = Vec::with_capacity(refs.len());
        for r in refs {
            let name = gix::refs::FullName::try_from(r.name.as_str()).map_err(failed)?;
            let lease = r.lease.as_deref().map(parse_id).transpose()?;
            let new = r.new.as_deref().map(parse_id).transpose()?;
            let update = match (lease, new) {
                (None, Some(new)) => Update::create(name, new),
                (Some(lease), Some(new)) => Update::change(name, lease, new),
                (Some(lease), None) => Update::delete(name, lease),
                (None, None) => continue,
            };
            let update = if r.rewrite {
                update.with_policy(gix::remote::push::Policy::Rewrite)
            } else {
                update
            };
            updates.push(update);
        }
        let _ = Lease::Absent;
        let connection = remote.connect(gix::remote::Direction::Push).map_err(failed)?;
        let prepared = connection
            .prepare_push(gix::progress::Discard, updates, gix::remote::push::Options::default())
            .map_err(failed)?
            .with_pre_push_hook();
        // An atomic push moves nothing when any update is refused.
        let refused: Vec<Refused> = prepared
            .plan()
            .refused()
            .map(|(entry, refusal)| {
                let name = entry.update.name().as_bstr().to_string();
                match refusal {
                    Refusal::StaleLease { .. } => Refused::Stale(name),
                    Refusal::NotFastForward => {
                        Refused::Rejected(name, "non-fast-forward".to_owned())
                    },
                    Refusal::LeaseObjectMissing => Refused::Rejected(
                        name,
                        "the remote's value isn't in this clone: fetch first".to_owned(),
                    ),
                    Refusal::TagUpdate => {
                        Refused::Rejected(name, "a tag already exists".to_owned())
                    },
                }
            })
            .collect();
        if !refused.is_empty() {
            return Ok(Err(refused));
        }
        let outcome = prepared.send(&std::sync::atomic::AtomicBool::new(false)).map_err(failed)?;
        let mut pushed = 0;
        for entry in &outcome.entries {
            match &entry.status {
                Status::Rejected { reason } => {
                    return Ok(Err(vec![Refused::Rejected(
                        entry.update.name().as_bstr().to_string(),
                        reason.to_string(),
                    )]));
                },
                Status::Applied { .. } => pushed += 1,
                _ => {},
            }
        }
        if let Some(uncertain) = &outcome.uncertain {
            return Err(StoreError::Git(describe(uncertain)));
        }
        Ok(Ok(pushed))
    }
}

impl Git {
    /// The root of the work tree, which is an error for a bare repository.
    pub(crate) fn toplevel(&self) -> Result<PathBuf, StoreError> {
        let repo = self.handle()?;
        // As `git rev-parse --show-toplevel` prints it: the real path, symbolic links resolved.
        repo.workdir()
            .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned()))
            .ok_or_else(|| StoreError::Git("this operation must be run in a work tree".to_owned()))
    }

    /// The commits reachable from `tip`, at most `limit` of them.
    pub(crate) fn recent_commits(
        &self,
        tip: &str,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        let repo = self.handle()?;
        let walk = repo.rev_walk([parse_id(tip)?]).all().map_err(failed)?;
        let mut commits = Vec::new();
        for info in walk.take(limit) {
            commits.push(info.map_err(failed)?.id.to_string());
        }
        Ok(commits)
    }

    /// The commit an abbreviated id names, in full, or `None` when it names none, several objects or another kind.
    pub(crate) fn full_commit(&self, abbreviated: &str) -> Option<String> {
        let repo = self.handle().ok()?;
        let prefix = gix::hash::Prefix::from_hex(abbreviated).ok()?;
        let id = repo.objects.lookup_prefix(prefix, None).ok()??.ok()?;
        let header = repo.find_header(id).ok()?;
        (header.kind() == gix::object::Kind::Commit).then(|| id.to_string())
    }

    /// The `remote.<name>.url` values of the repository's own configuration file, by remote name, and whether a key
    /// is set there.
    pub(crate) fn local_urls_and_key(
        &self,
        key: (&str, &str),
    ) -> Result<(Vec<(String, String)>, bool), StoreError> {
        let repo = self.handle()?;
        let config = repo.config_snapshot();
        let file = config.plumbing();
        let mut urls = Vec::new();
        for section in file.sections_by_name("remote").into_iter().flatten() {
            if section.meta().source != gix::config::Source::Local {
                continue;
            }
            let Some(name) = section.header().subsection_name() else { continue };
            for value in section.values("url") {
                urls.push((name.to_string(), value.to_string()));
            }
        }
        let mut found = false;
        for section in file.sections_by_name(key.0).into_iter().flatten() {
            found |= section.meta().source == gix::config::Source::Local
                && section.value(key.1).is_some();
        }
        Ok((urls, found))
    }
}

impl Git {
    /// The value of a key as git reads it, from every source: the last one.
    pub(crate) fn config_value(&self, key: &str) -> Result<Option<String>, StoreError> {
        let repo = self.handle()?;
        Ok(repo.config_snapshot().string(key).map(|value| value.to_string()))
    }

    /// The URL of a remote as git fetches it, with `url.<base>.insteadOf` applied.
    pub(crate) fn remote_url(&self, name: &str) -> Result<Option<String>, StoreError> {
        let repo = self.handle()?;
        let Some(found) = repo.try_find_remote(name) else { return Ok(None) };
        let remote = found.map_err(failed)?;
        Ok(remote.url(gix::remote::Direction::Fetch).map(|url| url.to_bstring().to_string()))
    }

    /// Points the remote `name` at `url`, adding it when it doesn't exist, as `git remote add` and `set-url` do.
    pub(crate) fn set_remote_url(&self, name: &str, url: &str) -> Result<(), StoreError> {
        let repo = self.handle()?;
        let exists = repo.try_find_remote(name).is_some();
        let mut file = repo.config_file_mut(repo.common_dir().join("config")).map_err(failed)?;
        file.set_raw_value(&format!("remote.{name}.url"), url).map_err(failed)?;
        if !exists {
            file.set_raw_value(
                &format!("remote.{name}.fetch"),
                format!("+refs/heads/*:refs/remotes/{name}/*").as_str(),
            )
            .map_err(failed)?;
        }
        file.commit().map_err(failed)
    }

    /// The repository's own directory, absolute.
    pub(crate) fn git_dir(&self) -> Result<PathBuf, StoreError> {
        Ok(self.handle()?.git_dir().to_owned())
    }
}

/// Creates a repository with a work tree at `path`, as `git init` does.
pub(crate) fn init(path: &Path) -> Result<(), StoreError> {
    gix::init(path).map(|_| ()).map_err(failed)
}

/// The refs of `remote` named by `patterns`, asked from a repository that exists only for the question, so that it
/// works anywhere. A relative `remote` counts from `base`.
pub(crate) fn list_remote_anywhere(
    base: &Path,
    remote: &str,
    patterns: &[String],
) -> Result<BTreeMap<String, String>, StoreError> {
    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let scratch = std::env::temp_dir().join(format!(
        "acyclic-listing-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _cleanup = RemoveOnDrop(scratch.clone());
    gix::init_bare(&scratch).map_err(failed)?;
    let options = gix::open::Options::default()
        .ignore_repository_environment(true)
        .cli_overrides(["gitoxide.core.nativeLocalTransport=true"]);
    let repo = gix::open_opts(&scratch, options).map_err(failed)?;
    Git::new(base).list_remote_with(&repo, remote, patterns)
}

#[cfg(test)]
mod tests;
