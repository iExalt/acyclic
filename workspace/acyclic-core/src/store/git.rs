//! A thin layer over the `git` command: plumbing only, never a work tree.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use super::StoreError;

/// A repository addressed by its path, with an optional private index for building trees.
#[derive(Debug, Clone)]
pub(crate) struct Git {
    pub(crate) repo: PathBuf,
    zero: std::sync::OnceLock<String>,
    common: std::sync::OnceLock<PathBuf>,
}

impl Git {
    pub(crate) fn new(repo: &Path) -> Self {
        Self {
            repo: repo.to_owned(),
            zero: std::sync::OnceLock::new(),
            common: std::sync::OnceLock::new(),
        }
    }

    fn command(&self) -> Command {
        let mut command = super::git_at(&self.repo);
        command.args(["-c", "core.quotePath=false"]);
        // No reflogs for the tracker's refs: each would be another fsynced append per batch, the remote holds the
        // history, and a squash makes it moot.
        command.args(["-c", "core.logAllRefUpdates=false"]);
        // Errors are classified by their text, so keep git's messages untranslated.
        command.env("LC_ALL", "C").env("LANGUAGE", "");
        command
    }

    /// Runs git and returns its standard output, failing on a nonzero exit.
    pub(crate) fn run<I, S>(
        &self,
        args: I,
        input: Option<&[u8]>,
        env: &[(&str, &OsStr)],
    ) -> Result<Vec<u8>, StoreError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command();
        command.args(args).envs(env.iter().copied());
        command.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child =
            command.spawn().map_err(|e| StoreError::Git(format!("git can't run: {e}")))?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("stdin is piped");
            let input = input.to_vec();
            // Write on a thread so a large input can't deadlock against a full output pipe.
            let writer = std::thread::spawn(move || stdin.write_all(&input));
            let output = child.wait_with_output().map_err(|e| StoreError::Git(e.to_string()))?;
            writer
                .join()
                .expect("the writer thread doesn't panic")
                .map_err(|e| StoreError::Git(e.to_string()))?;
            return check(output);
        }
        check(child.wait_with_output().map_err(|e| StoreError::Git(e.to_string()))?)
    }

    /// Runs git and returns its output whatever the exit status.
    pub(crate) fn output<I, S>(&self, args: I) -> Result<std::process::Output, StoreError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| StoreError::Git(format!("git can't run: {e}")))
    }

    pub(crate) fn text<I, S>(&self, args: I) -> Result<String, StoreError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Ok(String::from_utf8_lossy(&self.run(args, None, &[])?).trim().to_owned())
    }

    /// A ref's object id, or `None` when the ref doesn't exist.
    /// The first call also learns the common Git directory, saving [`Git::common_dir`] a process.
    pub(crate) fn resolve(&self, reference: &str) -> Result<Option<String>, StoreError> {
        let learn = self.common.get().is_none();
        let mut command = self.command();
        command.arg("rev-parse");
        if learn {
            command.arg("--git-common-dir");
        }
        let output = command
            .args(["--verify", "--quiet", &format!("{reference}^{{commit}}")])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        if learn && let Some(dir) = lines.next() {
            let dir = PathBuf::from(dir);
            let _ = self.common.set(if dir.is_absolute() { dir } else { self.repo.join(dir) });
        }
        Ok(output.status.success().then(|| lines.next().unwrap_or_default().trim().to_owned()))
    }

    /// Refs under a prefix with their object ids.
    pub(crate) fn refs(&self, prefix: &str) -> Result<BTreeMap<String, String>, StoreError> {
        self.refs_under(&[prefix])
    }

    /// Refs under any of several prefixes, or named exactly, with their object ids, in one process.
    pub(crate) fn refs_under(
        &self,
        prefixes: &[&str],
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let mut args = vec!["for-each-ref", "--format=%(refname) %(objectname)"];
        args.extend(prefixes);
        let text = self.text(args)?;
        Ok(text
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(name, id)| (name.to_owned(), id.to_owned()))
            .collect())
    }

    /// Every file in a commit's tree, by path.
    pub(crate) fn read_tree(&self, commit: &str) -> Result<BTreeMap<String, Vec<u8>>, StoreError> {
        let listing = self.run(["ls-tree", "-r", "-z", "--full-tree", commit], None, &[])?;
        let mut entries = Vec::new();
        for record in listing.split(|&b| b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            let (meta, path) =
                record.split_once('\t').ok_or_else(|| StoreError::Git("bad ls-tree".into()))?;
            let id = meta.rsplit(' ').next().unwrap_or_default().to_owned();
            entries.push((path.to_owned(), id));
        }
        if entries.is_empty() {
            return Ok(BTreeMap::new());
        }
        let request = entries.iter().fold(String::new(), |mut out, (_, id)| {
            let _ = writeln!(out, "{id}");
            out
        });
        let output = self.run(["cat-file", "--batch"], Some(request.as_bytes()), &[])?;
        let mut rest = &output[..];
        let mut files = BTreeMap::new();
        for (path, _) in entries {
            let newline = rest
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(|| StoreError::Git("bad cat-file".into()))?;
            let header = String::from_utf8_lossy(&rest[..newline]).into_owned();
            let size: usize = header
                .split_whitespace()
                .nth(2)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| StoreError::Git(format!("bad cat-file header: {header}")))?;
            let start = newline + 1;
            let content = rest
                .get(start..start + size)
                .ok_or_else(|| StoreError::Git("short cat-file".into()))?;
            files.insert(path, content.to_vec());
            rest = &rest[(start + size + 1).min(rest.len())..];
        }
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
        let request = paths.iter().fold(String::new(), |mut out, path| {
            let _ = writeln!(out, "{commit}:{path}");
            out
        });
        let output = self.run(["cat-file", "--batch"], Some(request.as_bytes()), &[])?;
        let mut rest = &output[..];
        let mut files = BTreeMap::new();
        for path in paths {
            let newline = rest
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(|| StoreError::Git("bad cat-file".into()))?;
            let header = String::from_utf8_lossy(&rest[..newline]).into_owned();
            if header.ends_with(" missing") {
                rest = &rest[newline + 1..];
                continue;
            }
            let size: usize = header
                .split_whitespace()
                .nth(2)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| StoreError::Git(format!("bad cat-file header: {header}")))?;
            let start = newline + 1;
            let content = rest
                .get(start..start + size)
                .ok_or_else(|| StoreError::Git("short cat-file".into()))?;
            files.insert(path.clone(), content.to_vec());
            rest = &rest[(start + size + 1).min(rest.len())..];
        }
        Ok(files)
    }

    /// Writes a tree from `base` (a commit, or none) with `changes` applied: `Some` content adds or replaces a file and
    /// `None` removes it. Uses a private index, so no work tree or shared index is touched.
    pub(crate) fn write_tree(
        &self,
        base: Option<&str>,
        changes: &BTreeMap<String, Option<Vec<u8>>>,
    ) -> Result<String, StoreError> {
        let scratch = tempdir()?;
        let _cleanup = RemoveOnDrop(scratch.clone());
        let index = scratch.join("index");
        let env = [("GIT_INDEX_FILE", index.as_os_str())];
        match base {
            Some(base) => self.run(["read-tree", base], None, &env)?,
            None => self.run(["read-tree", "--empty"], None, &env)?,
        };
        let mut info = String::new();
        let blobs: Vec<(&String, &Vec<u8>)> = changes
            .iter()
            .filter_map(|(path, content)| content.as_ref().map(|c| (path, c)))
            .collect();
        if !blobs.is_empty() {
            let mut list = String::new();
            for (n, (_, content)) in blobs.iter().enumerate() {
                let file = scratch.join(format!("blob{n}"));
                std::fs::write(&file, content).map_err(|e| StoreError::Git(e.to_string()))?;
                let _ = writeln!(list, "{}", file.display());
            }
            let ids = self.run(
                ["hash-object", "-w", "--stdin-paths", "--no-filters"],
                Some(list.as_bytes()),
                &[],
            )?;
            for ((path, _), id) in blobs.iter().zip(String::from_utf8_lossy(&ids).lines()) {
                let _ = writeln!(info, "100644 {id}\t{path}");
            }
        }
        let zero = self.zero_id();
        for path in changes.iter().filter(|(_, content)| content.is_none()).map(|(path, _)| path) {
            let _ = writeln!(info, "0 {zero}\t{path}");
        }
        self.run(["update-index", "--index-info"], Some(info.as_bytes()), &env)?;
        self.run(["write-tree"], None, &env)
            .map(|out| String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// The all-zero object id of the repository's hash.
    fn zero_id(&self) -> &str {
        self.zero.get_or_init(|| {
            let format =
                self.text(["rev-parse", "--show-object-format=storage"]).unwrap_or_default();
            "0".repeat(if format == "sha256" { 64 } else { 40 })
        })
    }

    /// The repository's common Git directory, shared by its worktrees.
    pub(crate) fn common_dir(&self) -> Result<PathBuf, StoreError> {
        if let Some(dir) = self.common.get() {
            return Ok(dir.clone());
        }
        let dir = PathBuf::from(self.text(["rev-parse", "--git-common-dir"])?);
        let dir = if dir.is_absolute() { dir } else { self.repo.join(dir) };
        Ok(self.common.get_or_init(|| dir).clone())
    }

    /// A commit of `tree` with an optional parent, authored as the replica at `seconds` since the epoch.
    pub(crate) fn commit(
        &self,
        tree: &str,
        parent: Option<&str>,
        replica: &str,
        seconds: i64,
        message: &str,
    ) -> Result<String, StoreError> {
        let mut args =
            vec!["commit-tree".to_owned(), tree.to_owned(), "-m".to_owned(), message.to_owned()];
        if let Some(parent) = parent {
            args.extend(["-p".to_owned(), parent.to_owned()]);
        }
        let name = format!("acyclic {replica}");
        let email = format!("{replica}@acyclic.invalid");
        let date = format!("@{seconds} +0000");
        let env = [
            ("GIT_AUTHOR_NAME", OsStr::new(&name)),
            ("GIT_AUTHOR_EMAIL", OsStr::new(&email)),
            ("GIT_AUTHOR_DATE", OsStr::new(&date)),
            ("GIT_COMMITTER_NAME", OsStr::new(&name)),
            ("GIT_COMMITTER_EMAIL", OsStr::new(&email)),
            ("GIT_COMMITTER_DATE", OsStr::new(&date)),
        ];
        let out = self.run(&args, None, &env)?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Writes commits with one `git fast-import`, so a batch's objects land in a single pack and no temporary
    /// files are written. Each commit goes to its own scratch ref, forced, since only the
    /// holder of the batch lock writes them. Returns the commits' ids in order.
    ///
    /// # Errors
    ///
    /// Fails when git fails, and refuses a path fast-import would read differently: an empty one, or one holding a
    /// newline or starting with a quote.
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
        let mut stream: Vec<u8> = b"feature done\n".to_vec();
        for (n, commit) in commits.iter().enumerate() {
            let name = format!("acyclic {}", commit.replica);
            let email = format!("{}@acyclic.invalid", commit.replica);
            let _ = write!(
                Bytes(&mut stream),
                "reset {scratch}\ncommit {scratch}\nmark :{}\nauthor {name} <{email}> {seconds} +0000\ncommitter {name} <{email}> {seconds} \
                 +0000\ndata {}\n{}\n",
                n + 1,
                commit.message.len(),
                commit.message,
                scratch = commit.scratch,
                seconds = commit.seconds,
            );
            if let Some(parent) = commit.parent {
                let _ = writeln!(Bytes(&mut stream), "from {parent}");
            }
            for (path, content) in commit.files {
                match content {
                    Some(content) => {
                        let _ = write!(
                            Bytes(&mut stream),
                            "M 100644 inline {path}\ndata {}\n",
                            content.len()
                        );
                        stream.extend_from_slice(content);
                        stream.push(b'\n');
                    },
                    None => {
                        let _ = writeln!(Bytes(&mut stream), "D {path}");
                    },
                }
            }
            stream.push(b'\n');
        }
        for n in 1..=commits.len() {
            let _ = writeln!(Bytes(&mut stream), "get-mark :{n}");
        }
        stream.extend_from_slice(b"done\n");
        // Keep even a small batch as a pack: unpacking a few objects into loose files costs more than the pack,
        // and the store's upkeep repacks once packs pile up.
        let out = self.run(
            ["-c", "fastimport.unpackLimit=0", "fast-import", "--quiet", "--force"],
            Some(&stream),
            &[],
        )?;
        let ids: Vec<String> = String::from_utf8_lossy(&out).lines().map(str::to_owned).collect();
        if ids.len() == commits.len() {
            Ok(ids)
        } else {
            Err(StoreError::Git(format!(
                "fast-import returned {} ids for {} commits",
                ids.len(),
                commits.len()
            )))
        }
    }

    /// Starts an interactive `git update-ref --stdin` transaction.
    pub(crate) fn transaction(&self) -> Result<Transaction, StoreError> {
        let mut child = self
            .command()
            .args(["update-ref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| StoreError::Git(format!("git can't run: {e}")))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut transaction =
            Transaction { child, stdin: Some(stdin), stdout, repo: self.repo.clone() };
        transaction.send("start")?;
        transaction.expect("start")?;
        Ok(transaction)
    }
}

/// A commit for [`Git::import_commits`]: `files` applied to `parent`'s tree (or to an empty tree), where `Some`
/// content adds or replaces a file and `None` removes it.
pub(crate) struct NewCommit<'a> {
    /// The scratch ref fast-import moves to the commit.
    pub(crate) scratch: &'a str,
    pub(crate) parent: Option<&'a str>,
    pub(crate) files: &'a BTreeMap<String, Option<Vec<u8>>>,
    pub(crate) replica: &'a str,
    pub(crate) seconds: i64,
    pub(crate) message: &'a str,
}

/// Lets `write!` append to a byte buffer.
struct Bytes<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Bytes<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// An open `update-ref --stdin` transaction.
pub(crate) struct Transaction {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    repo: PathBuf,
}

impl Transaction {
    fn send(&mut self, line: &str) -> Result<(), StoreError> {
        let stdin = self.stdin.as_mut().expect("the transaction is open");
        writeln!(stdin, "{line}").and_then(|()| stdin.flush()).map_err(|_| self.failure())
    }

    fn expect(&mut self, verb: &str) -> Result<(), StoreError> {
        let mut line = String::new();
        self.stdout.read_line(&mut line).map_err(|e| StoreError::Git(e.to_string()))?;
        if line.trim() == format!("{verb}: ok") { Ok(()) } else { Err(self.failure()) }
    }

    /// Reads git's error and classifies it as a lost race, a held lock or another failure.
    fn failure(&mut self) -> StoreError {
        drop(self.stdin.take());
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            pipe.read_to_string(&mut stderr).ok();
        }
        let _ = self.child.wait();
        classify(&stderr, &self.repo)
    }

    /// Moves `reference` from `old` (or creates it when `None`) to `new`.
    pub(crate) fn update(
        &mut self,
        reference: &str,
        new: &str,
        old: Option<&str>,
    ) -> Result<(), StoreError> {
        match old {
            Some(old) => self.send(&format!("update {reference} {new} {old}")),
            None => self.send(&format!("create {reference} {new}")),
        }
    }

    /// Points `reference` at `new`, whatever it held.
    pub(crate) fn set(&mut self, reference: &str, new: &str) -> Result<(), StoreError> {
        self.send(&format!("update {reference} {new}"))
    }

    /// Deletes `reference`, which must be at `old`.
    pub(crate) fn delete(&mut self, reference: &str, old: &str) -> Result<(), StoreError> {
        self.send(&format!("delete {reference} {old}"))
    }

    /// Locks every ref and checks every old value.
    pub(crate) fn prepare(&mut self) -> Result<(), StoreError> {
        self.send("prepare")?;
        self.expect("prepare")
    }

    /// Applies the prepared updates.
    pub(crate) fn commit(mut self) -> Result<(), StoreError> {
        self.send("commit")?;
        self.expect("commit")?;
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| StoreError::Git(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(StoreError::Git(format!("update-ref failed: {status}")))
        }
    }
}

fn classify(stderr: &str, repo: &Path) -> StoreError {
    if let Some(start) = stderr.find("Unable to create '") {
        let rest = &stderr[start + "Unable to create '".len()..];
        let path = PathBuf::from(rest.split('\'').next().unwrap_or_default());
        let path = if path.is_absolute() { path } else { repo.join(path) };
        let age = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs());
        return StoreError::Locked { path, age_seconds: age };
    }
    if stderr.contains("but expected")
        || stderr.contains("reference already exists")
        || stderr.contains("is at ")
    {
        return StoreError::Conflict;
    }
    StoreError::Git(stderr.trim().to_owned())
}

fn check(output: std::process::Output) -> Result<Vec<u8>, StoreError> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(StoreError::Git(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
    }
}

/// Removes a directory when dropped, on every return path.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh private directory under the system's temporary directory.
fn tempdir() -> Result<PathBuf, StoreError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "acyclic-store-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)
        .map_err(|e| StoreError::Git(format!("can't create {}: {e}", path.display())))?;
    Ok(path)
}
