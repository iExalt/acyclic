//! The repository layer against Git itself: what it writes must be what Git would have written, and what it reads must
//! be what Git reads.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

use super::{Git, NewCommit};

#[test]
fn sectionless_config_values_only_block_the_originless_exemption() {
    let parses = |text: &str| {
        gix::config::File::from_bytes_no_includes(
            text.as_bytes(),
            gix::config::file::Metadata::api(),
            gix::config::file::init::Options::default(),
        )
        .expect("the config fixture parses")
    };

    assert!(!super::config_has_sectionless_values(&parses("# comment\n \n")));
    assert!(!super::config_has_sectionless_values(&parses(
        "[core]\nrepositoryformatversion = 0\n"
    )));
    assert!(super::config_has_sectionless_values(&parses(
        "key = value\n[core]\nrepositoryformatversion = 0\n"
    )));
    assert!(!super::config_has_sectionless_values(&parses(
        "[core]\nrepositoryformatversion = 0\nkey = value\n"
    )));

    let mut with_section = parses("[core]\nrepositoryformatversion = 0\n");
    with_section
        .append(parses("key = value\n[core]\nrepositoryformatversion = 0\n"))
        .expect("the parsed config appends");
    assert!(super::config_has_sectionless_values(&with_section));

    let mut with_comments = parses("[core]\nrepositoryformatversion = 0\n");
    with_comments
        .append(parses("# comment\n \n[core]\nrepositoryformatversion = 0\n"))
        .expect("the parsed config appends");
    assert!(!super::config_has_sectionless_values(&with_comments));
}

type Files = BTreeMap<String, Option<Vec<u8>>>;

/// Git in `dir`, away from the machine's configuration; its trimmed output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs in the test environment");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Git in `dir` with `input` on its standard input; its output.
fn git_with(dir: &Path, args: &[&str], envs: &[(&str, &str)], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git runs in the test environment");
    child.stdin.take().expect("stdin is piped").write_all(input).expect("git reads its input");
    let output = child.wait_with_output().expect("git finishes");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repository(hash: &str) -> (TempDir, Git) {
    let dir = TempDir::new().expect("a temporary directory");
    git(dir.path(), &["init", "-q", "--object-format", hash, "."]);
    let store = Git::new(dir.path());
    (dir, store)
}

#[test]
fn first_parent_commit_threshold_counts_the_tip_and_stops_at_the_boundary() {
    let (_dir, store) = repository("sha1");
    let changes = files(&[("state", Some("same"))]);
    let tree = store.write_tree(None, &changes).expect("a tree");
    let first = store.commit(&tree, None, "test", 1, "first").expect("a commit");
    let second = store.commit(&tree, Some(&first), "test", 2, "second").expect("a commit");
    let third = store.commit(&tree, Some(&second), "test", 3, "third").expect("a commit");

    assert!(store.first_parent_commits_at_least(&third, 3).expect("the history reads"));
    assert!(!store.first_parent_commits_at_least(&third, 4).expect("the history reads"));
    assert!(store.first_parent_commits_at_least(&third, 0).expect("zero is immediate"));
}

fn files(entries: &[(&str, Option<&str>)]) -> Files {
    entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.map(|c| c.as_bytes().to_vec())))
        .collect()
}

/// What `git fast-import` made of the commits, as the store used to ask for it: the ids in order.
fn fast_import(dir: &Path, commits: &[NewCommit<'_>]) -> Vec<String> {
    let mut stream: Vec<u8> = b"feature done\n".to_vec();
    for (n, commit) in commits.iter().enumerate() {
        let name = format!("acyclic {}", commit.replica);
        let email = format!("{}@acyclic.invalid", commit.replica);
        stream.extend_from_slice(
            format!(
                "reset refs/scratch/{n}\ncommit refs/scratch/{n}\nmark :{}\nauthor {name} <{email}> {seconds} +0000\ncommitter {name} \
                 <{email}> {seconds} +0000\ndata {}\n{}\n",
                n + 1,
                commit.message.len(),
                commit.message,
                seconds = commit.seconds,
            )
            .as_bytes(),
        );
        if let Some(parent) = commit.parent {
            stream.extend_from_slice(format!("from {parent}\n").as_bytes());
        }
        for (path, content) in commit.files {
            match content {
                Some(content) => {
                    stream.extend_from_slice(
                        format!("M 100644 inline {path}\ndata {}\n", content.len()).as_bytes(),
                    );
                    stream.extend_from_slice(content);
                    stream.push(b'\n');
                },
                None => stream.extend_from_slice(format!("D {path}\n").as_bytes()),
            }
        }
        stream.push(b'\n');
    }
    for n in 1..=commits.len() {
        stream.extend_from_slice(format!("get-mark :{n}\n").as_bytes());
    }
    stream.extend_from_slice(b"done\n");
    let out = git_with(
        dir,
        &["-c", "fastimport.unpackLimit=0", "fast-import", "--quiet", "--force"],
        &[],
        &stream,
    );
    out.lines().map(str::to_owned).collect()
}

/// What `git read-tree`, `update-index` and `write-tree` made of `changes` on `base`, as the store used to ask.
fn git_tree(dir: &Path, base: Option<&str>, changes: &Files) -> String {
    let index = dir.join("oracle-index");
    let env = [("GIT_INDEX_FILE", index.to_str().expect("a UTF-8 path"))];
    match base {
        Some(base) => git_with(dir, &["read-tree", base], &env, b""),
        None => git_with(dir, &["read-tree", "--empty"], &env, b""),
    };
    let zero =
        "0".repeat(if git(dir, &["rev-parse", "--show-object-format=storage"]) == "sha256" {
            64
        } else {
            40
        });
    let mut info = String::new();
    for (path, content) in changes {
        match content {
            Some(content) => {
                let id = git_with(dir, &["hash-object", "-w", "--stdin"], &[], content);
                let _ = writeln!(info, "100644 {id}\t{path}");
            },
            None => {
                let _ = writeln!(info, "0 {zero}\t{path}");
            },
        }
    }
    git_with(dir, &["update-index", "--index-info"], &env, info.as_bytes());
    let tree = git_with(dir, &["write-tree"], &env, b"");
    std::fs::remove_file(index).expect("the oracle index exists");
    tree
}

fn child<'a>(parent: &'a str, files: &'a Files) -> [NewCommit<'a>; 1] {
    [NewCommit {
        parent: Some(parent),
        files,
        replica: "mac",
        seconds: 1_792_022_401,
        message: "join",
    }]
}

#[test]
fn imported_commits_are_byte_for_byte_what_fast_import_writes_and_make_one_pack() {
    for hash in ["sha1", "sha256"] {
        let (ours_dir, ours) = repository(hash);
        let (theirs_dir, _) = repository(hash);
        let first_files = files(&[
            ("header.json", Some("{\"context\":{}}")),
            ("a/b.json", Some("{}")),
            ("c.json", Some("1")),
        ]);
        let log_files = files(&[("_/15.jsonl", Some("{\"at\":1}\n"))]);
        let first = |files| {
            [
                NewCommit {
                    parent: None,
                    files: &first_files,
                    replica: "mac",
                    seconds: 1_792_022_400,
                    message: "batch",
                },
                NewCommit {
                    parent: None,
                    files,
                    replica: "mac",
                    seconds: 1_792_022_400,
                    message: "events",
                },
            ]
        };
        let ids = ours.import_commits(&first(&log_files)).expect("the import writes");
        assert_eq!(
            ids,
            fast_import(theirs_dir.path(), &first(&log_files)),
            "{hash}: two parentless commits, one with nested paths and one with a log"
        );
        let counts = git(ours_dir.path(), &["count-objects", "-v"]);
        assert!(
            counts.contains("count: 0") && counts.contains("packs: 1"),
            "{hash}: the batch's objects are one pack and no loose files: {counts}"
        );

        // A child that changes, removes and adds below an existing directory, and empties another.
        let second_files =
            files(&[("a/b.json", Some("{\"x\":1}")), ("c.json", None), ("a/d/e.json", Some("[]"))]);
        let ours_child =
            ours.import_commits(&child(&ids[0], &second_files)).expect("the child imports");
        assert_eq!(
            ours_child,
            fast_import(theirs_dir.path(), &child(&ids[0], &second_files)),
            "{hash}: a child that changes, removes and adds files"
        );
        git(ours_dir.path(), &["fsck", "--strict", "--full"]);
        assert_eq!(
            ours.read_tree(&ours_child[0]).expect("the tree reads"),
            [
                ("a/b.json".to_owned(), b"{\"x\":1}".to_vec()),
                ("a/d/e.json".to_owned(), b"[]".to_vec()),
                ("header.json".to_owned(), b"{\"context\":{}}".to_vec()),
            ]
            .into(),
            "{hash}: the store reads back what it wrote, with the removed file gone"
        );
    }
}

#[test]
fn a_commit_is_what_commit_tree_makes_with_its_newline() {
    for hash in ["sha1", "sha256"] {
        let (dir, ours) = repository(hash);
        let tree = ours.write_tree(None, &files(&[("a/b", Some("x"))])).expect("a tree");
        let parent = ours.commit(&tree, None, "mac", 1_792_022_400, "first").expect("a commit");
        let child =
            ours.commit(&tree, Some(&parent), "mac", 1_792_022_401, "squash").expect("a child");
        let env = |seconds: &str| {
            let date = format!("@{seconds} +0000");
            [
                ("GIT_AUTHOR_NAME", "acyclic mac".to_owned()),
                ("GIT_AUTHOR_EMAIL", "mac@acyclic.invalid".to_owned()),
                ("GIT_AUTHOR_DATE", date.clone()),
                ("GIT_COMMITTER_NAME", "acyclic mac".to_owned()),
                ("GIT_COMMITTER_EMAIL", "mac@acyclic.invalid".to_owned()),
                ("GIT_COMMITTER_DATE", date),
            ]
        };
        let run = |args: &[&str], seconds: &str| {
            let vars = env(seconds);
            let borrowed: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            git_with(dir.path(), args, &borrowed, b"")
        };
        assert_eq!(
            parent,
            run(&["commit-tree", &tree, "-m", "first"], "1792022400"),
            "{hash}: parentless"
        );
        assert_eq!(
            child,
            run(&["commit-tree", &tree, "-m", "squash", "-p", &parent], "1792022401"),
            "{hash}: with a parent"
        );
        assert_eq!(ours.commit_seconds(&child).expect("a time"), 1_792_022_401);
        assert_eq!(ours.tree_of(&child).expect("a tree"), tree);
    }
}

#[test]
fn trees_are_what_git_writes_including_the_directories_a_removal_empties() {
    for hash in ["sha1", "sha256"] {
        let (dir, ours) = repository(hash);
        let kept = files(&[
            ("log/a/1.json", Some("1")),
            ("log/a/2.json", Some("2")),
            ("log/b/3.json", Some("3")),
            ("z.json", Some("z")),
        ]);
        let tree = ours.write_tree(None, &kept).expect("a tree");
        assert_eq!(tree, git_tree(dir.path(), None, &kept), "{hash}: from nothing");

        // Removing the last file of a directory takes the directory along, in both.
        let commit = ours.commit(&tree, None, "mac", 1, "base").expect("a commit");
        let removals = files(&[("log/b/3.json", None), ("z.json", Some("changed"))]);
        assert_eq!(
            ours.write_tree(Some(&commit), &removals).expect("an edited tree"),
            git_tree(dir.path(), Some(&commit), &removals),
            "{hash}: a directory emptied by a removal vanishes, a change replaces"
        );
        let nothing = files(&[]);
        assert_eq!(
            ours.write_tree(None, &nothing).expect("the empty tree"),
            git_tree(dir.path(), None, &nothing),
            "{hash}: the empty tree"
        );
    }
}

#[test]
fn an_abbreviated_commit_id_resolves_only_when_it_names_one_commit() {
    let (dir, ours) = repository("sha1");
    let tree = ours.write_tree(None, &files(&[("a", Some("x"))])).expect("a tree");
    let commit = ours.commit(&tree, None, "mac", 1, "only").expect("a commit");
    assert_eq!(ours.full_commit(&commit[..7]), Some(commit.clone()), "a unique abbreviation");
    assert_eq!(ours.full_commit("0123456"), None, "no object at all");
    assert_eq!(ours.full_commit(&tree[..7]), None, "the abbreviation of a tree isn't a commit");

    // Objects that share their first four digits: Git calls the abbreviation ambiguous, and so does the store.
    let repo = ours.handle().expect("the repository opens");
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut shared = None;
    for n in 0..4_000 {
        let id = repo.write_blob(format!("blob {n}")).expect("a blob").to_string();
        if let Some(earlier) = seen.insert(id[..4].to_owned(), id.clone()) {
            shared = Some((id[..4].to_owned(), earlier));
            break;
        }
    }
    let (prefix, _) = shared.expect("4,000 objects share four digits somewhere");
    assert_eq!(ours.full_commit(&prefix), None, "an ambiguous abbreviation names no commit");
    let ambiguous = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["rev-parse", "--verify", "--quiet", &format!("{prefix}^{{commit}}")])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(!ambiguous.status.success(), "git agrees that it names no commit");
}

#[test]
fn a_fetch_follows_the_tags_into_what_it_receives_as_git_does() {
    let dir = TempDir::new().expect("a temporary directory");
    let remote = dir.path().join("remote.git");
    git(dir.path(), &["init", "-q", "--bare", "remote.git"]);
    let source = dir.path().join("source");
    git(dir.path(), &["init", "-q", "source"]);
    let identity = [
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@e.invalid"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@e.invalid"),
    ];
    git_with(&source, &["commit", "-q", "--allow-empty", "-m", "a"], &identity, b"");
    git_with(&source, &["tag", "-a", "-m", "t", "v1"], &identity, b"");
    git(
        &source,
        &[
            "push",
            "-q",
            remote.to_str().expect("UTF-8"),
            "HEAD:refs/heads/state/mac",
            "refs/tags/v1",
        ],
    );

    let spec = vec!["+refs/heads/state/*:refs/acyclic/remotes/x/state/*".to_owned()];
    let ours_dir = dir.path().join("ours");
    git(dir.path(), &["init", "-q", "ours"]);
    Git::new(&ours_dir)
        .fetch(remote.to_str().expect("UTF-8"), &spec)
        .expect("the native fetch works");
    let theirs_dir = dir.path().join("theirs");
    git(dir.path(), &["init", "-q", "theirs"]);
    git(&theirs_dir, &["fetch", "-q", "--prune", remote.to_str().expect("UTF-8"), &spec[0]]);
    let refs = |path: &Path| git(path, &["for-each-ref", "--format=%(refname) %(objectname)"]);
    assert_eq!(refs(&ours_dir), refs(&theirs_dir), "the same refs, tags among them");
    assert!(refs(&ours_dir).contains("refs/tags/v1"));

    // The remote loses the ref: both prune it.
    git(&remote, &["update-ref", "-d", "refs/heads/state/mac"]);
    Git::new(&ours_dir)
        .fetch(remote.to_str().expect("UTF-8"), &spec)
        .expect("a pruning fetch works");
    git(&theirs_dir, &["fetch", "-q", "--prune", remote.to_str().expect("UTF-8"), &spec[0]]);
    assert_eq!(refs(&ours_dir), refs(&theirs_dir), "the pruned tracking ref is gone from both");
}

#[test]
fn a_transaction_tells_a_lost_race_from_a_held_lock_as_it_runs_in_production() {
    use crate::store::StoreError;
    let (dir, ours) = repository("sha1");
    let tree = ours.write_tree(None, &files(&[("a", Some("x"))])).expect("a tree");
    let first = ours.commit(&tree, None, "mac", 1, "first").expect("a commit");
    let second = ours.commit(&tree, None, "mac", 2, "second").expect("another commit");
    let create = |name: &str, id: &str| {
        let mut transaction = ours.transaction().expect("a transaction");
        transaction.update(name, id, None).expect("an update");
        transaction.commit(None)
    };
    create("refs/heads/x", &first).expect("a new ref is created");

    let result = create("refs/heads/x", &second);
    assert!(
        matches!(result, Err(StoreError::Conflict)),
        "creating what exists is a lost race: {result:?}"
    );
    let mut transaction = ours.transaction().expect("a transaction");
    transaction.update("refs/heads/x", &second, Some(&second)).expect("an update");
    let result = transaction.commit(None);
    assert!(
        matches!(result, Err(StoreError::Conflict)),
        "a wrong old value is a lost race: {result:?}"
    );
    let mut transaction = ours.transaction().expect("a transaction");
    transaction.delete("refs/heads/x", &second).expect("a deletion");
    let result = transaction.commit(None);
    assert!(
        matches!(result, Err(StoreError::Conflict)),
        "so is deleting at a wrong value: {result:?}"
    );
    assert_eq!(git(dir.path(), &["rev-parse", "refs/heads/x"]), first, "nothing moved");

    let lock = dir.path().join(".git/refs/heads/x.lock");
    std::fs::write(&lock, "").expect("a lock file");
    let mut transaction = ours.transaction().expect("a transaction");
    transaction.update("refs/heads/x", &second, Some(&first)).expect("an update");
    match transaction.commit(None) {
        Err(StoreError::Locked { path, age_seconds }) => {
            assert_eq!(path, dir.path().join(".git/refs/heads/x.lock"));
            assert!(age_seconds.is_some());
        },
        other => panic!("a held lock is reported as one: {other:?}"),
    }
    std::fs::remove_file(&lock).expect("the lock goes");
    let mut transaction = ours.transaction().expect("a transaction");
    transaction.update("refs/heads/x", &second, Some(&first)).expect("an update");
    transaction.commit(None).expect("with the right old value and no lock it commits");
    assert_eq!(git(dir.path(), &["rev-parse", "refs/heads/x"]), second);
}

#[test]
fn a_slow_http_transfer_fails_rather_than_hangs() {
    let (_dir, ours) = repository("sha1");
    let values =
        ours.config_values(&["http.lowSpeedLimit", "http.lowSpeedTime"]).expect("config reads");
    assert_eq!(values.get("http.lowSpeedLimit").map(String::as_str), Some("1000"));
    assert_eq!(values.get("http.lowSpeedTime").map(String::as_str), Some("30"));
}

#[test]
fn a_fetch_never_prompts_on_a_terminal_for_credentials() {
    let dir = TempDir::new().expect("a temporary directory");
    git(dir.path(), &["init", "-q", "--object-format", "sha1", "."]);
    git(dir.path(), &["config", "gitoxide.ssh.nonInteractive", "false"]);
    let ours = Git::new(dir.path());
    let cached = ours.handle().expect("the cached repository opens");
    let fresh = ours.fresh_handle().expect("a fresh repository opens");
    let strict = super::discover_with_config(dir.path(), true)
        .expect("the strict repository opens")
        .to_thread_local();
    let policy = [
        cached.config_snapshot().boolean("gitoxide.ssh.nonInteractive"),
        fresh.config_snapshot().boolean("gitoxide.ssh.nonInteractive"),
        strict.config_snapshot().boolean("gitoxide.ssh.nonInteractive"),
    ];
    assert_eq!(
        policy,
        [Some(true); 3],
        "the SSH policy overrides the local false value on cached, fresh and strict handles"
    );
    let values =
        ours.config_values(&["gitoxide.credentials.terminalPrompt"]).expect("config reads");
    assert_eq!(
        values.get("gitoxide.credentials.terminalPrompt").map(String::as_str),
        Some("false")
    );
}

#[test]
fn the_keychain_helper_is_found_in_the_first_directory_that_holds_it() {
    let dir = TempDir::new().expect("a temporary directory");
    let (empty, holding) = (dir.path().join("empty"), dir.path().join("holding"));
    std::fs::create_dir(&empty).expect("a directory");
    std::fs::create_dir(&holding).expect("a directory");
    std::fs::write(holding.join("git-credential-osxkeychain"), "").expect("a helper");
    let (empty, holding) = (empty.to_str().expect("UTF-8"), holding.to_str().expect("UTF-8"));
    assert_eq!(
        super::first_holding_keychain_helper(&[empty, "/nonexistent", holding]),
        Some(holding)
    );
    assert_eq!(super::first_holding_keychain_helper(&[empty]), None);
}

#[test]
fn a_remote_with_relative_paths_keeps_its_name_and_its_push_url() {
    let dir = TempDir::new().expect("a temporary directory");
    git(dir.path(), &["init", "-q", "--bare", "remote.git"]);
    git(dir.path(), &["init", "-q", "ours"]);
    let ours_dir = dir.path().join("ours");
    // Fetches would go to a place that doesn't exist; only the push URL leads anywhere, and both count from the clone.
    git(&ours_dir, &["config", "remote.origin.url", "../nowhere.git"]);
    git(&ours_dir, &["config", "remote.origin.pushurl", "../remote.git"]);
    let ours = Git::new(&ours_dir);
    let tree = ours.write_tree(None, &files(&[("a", Some("x"))])).expect("a tree");
    let commit = ours.commit(&tree, None, "mac", 1, "first").expect("a commit");
    let pushed = ours
        .push(
            "origin",
            &[super::PushRef {
                name: "refs/heads/state/mac".to_owned(),
                lease: None,
                new: Some(commit.clone()),
                rewrite: false,
            }],
        )
        .expect("the push runs");
    assert!(matches!(pushed, Ok(1)), "one ref was pushed to the push URL");
    assert_eq!(git(&dir.path().join("remote.git"), &["rev-parse", "refs/heads/state/mac"]), commit);
}

/// A remote whose state ref points at a commit with a tree that names a file twice, which `git fetch` refuses with `fetch.fsckObjects` and takes
/// without it: with the setting the fetch fails as the class `ObjectsRefused` and keeps nothing, without it the fetch takes the
/// commit, as Git does.
#[test]
fn a_flawed_object_is_refused_when_the_configuration_asks_for_the_check_as_git_does() {
    let dir = TempDir::new().expect("a temporary directory");
    let remote = dir.path().join("remote.git");
    git(dir.path(), &["init", "-q", "--bare", "remote.git"]);
    let blob_id = git_with(&remote, &["hash-object", "-w", "--stdin"], &[], b"x\n");
    let blob_bytes: Vec<u8> = (0..40)
        .step_by(2)
        .map(|i| u8::from_str_radix(&blob_id.trim()[i..i + 2], 16).expect("a hex object id"))
        .collect();
    let mut entries = Vec::new();
    for _ in 0..2 {
        entries.extend_from_slice(b"100644 a\0");
        entries.extend_from_slice(&blob_bytes);
    }
    let tree_id = git_with(
        &remote,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &[],
        &entries,
    );
    let flawed = format!(
        "tree {}\nauthor A <a@e.invalid> 1 +0000\ncommitter C <c@e.invalid> 1 +0000\n\nduplicate entries\n",
        tree_id.trim()
    );
    let commit_id = git_with(
        &remote,
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        &[],
        flawed.as_bytes(),
    );
    git(&remote, &["update-ref", "refs/heads/state/mac", commit_id.trim()]);
    let url = remote.to_str().expect("UTF-8");
    let spec = vec!["+refs/heads/state/*:refs/acyclic/remotes/x/state/*".to_owned()];
    let refs = |path: &Path| git(path, &["for-each-ref", "--format=%(refname)"]);

    // The oracle must not read the machine's configuration: a global `transfer.fsckObjects` would decide the control.
    let isolated_git = |dir: &Path| {
        let mut command = Command::new("git");
        command
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        command
    };
    let fetched_by_git = |dir: &Path| {
        isolated_git(dir).args(["fetch", "-q", url, &spec[0]]).output().expect("git runs")
    };

    for key in ["fetch.fsckObjects", "transfer.fsckObjects"] {
        let theirs = dir.path().join(format!("theirs-{key}"));
        git(dir.path(), &["init", "-q", theirs.to_str().expect("UTF-8")]);
        git(&theirs, &["config", key, "true"]);
        let output = fetched_by_git(&theirs);
        assert!(!output.status.success(), "{key}: Git refuses the commit");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("duplicateEntries"),
            "{key}: Git names the check"
        );

        let ours = dir.path().join(format!("ours-{key}"));
        git(dir.path(), &["init", "-q", ours.to_str().expect("UTF-8")]);
        git(&ours, &["config", key, "true"]);
        match Git::new(&ours).fetch(url, &spec) {
            Err(super::StoreError::ObjectsRefused(report)) => {
                assert!(
                    report.contains("duplicateEntries"),
                    "{key}: the report names the check: {report}"
                );
            },
            other => panic!("{key}: expected the objects to be refused, got {other:?}"),
        }
        assert_eq!(refs(&ours), "", "{key}: a refused fetch moves no ref");
        let kept =
            std::fs::read_dir(ours.join(".git/objects/pack")).expect("the pack directory").count();
        assert_eq!(
            kept, 0,
            "{key}: a refused fetch keeps no pack, nor the scratch directory of one"
        );
        assert!(
            !isolated_git(&ours)
                .args(["cat-file", "-e", commit_id.trim()])
                .status()
                .expect("git runs")
                .success(),
            "{key}: the flawed commit is not in the repository"
        );
    }

    // The control: both settings false, which also wins over a machine's global `transfer.fsckObjects`.
    let theirs = dir.path().join("theirs-unchecked");
    git(dir.path(), &["init", "-q", theirs.to_str().expect("UTF-8")]);
    git(&theirs, &["config", "fetch.fsckObjects", "false"]);
    assert!(fetched_by_git(&theirs).status.success(), "Git takes the commit without the check");
    let ours = dir.path().join("ours-unchecked");
    git(dir.path(), &["init", "-q", ours.to_str().expect("UTF-8")]);
    git(&ours, &["config", "fetch.fsckObjects", "false"]);
    Git::new(&ours).fetch(url, &spec).expect("without the check the commit is taken, as Git does");
    assert_eq!(refs(&ours).trim(), "refs/acyclic/remotes/x/state/mac");
}
