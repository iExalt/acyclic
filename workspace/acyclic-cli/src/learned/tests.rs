use std::os::unix::fs::MetadataExt as _;
use std::process::Command;
use std::rc::Rc;

use acyclic_core::model::{Repository, Value};
use tempfile::TempDir;

use super::*;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// A temporary home, with a clone of a synthetic github.com repository in it.
struct Machine {
    temp: TempDir,
}

impl Machine {
    fn new() -> Self {
        Self { temp: TempDir::new().unwrap() }
    }

    fn root(&self) -> PathBuf {
        std::fs::canonicalize(self.temp.path()).unwrap()
    }

    fn clone_of(&self, dir: &str, origin: &str) -> PathBuf {
        let path = self.root().join(dir);
        std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q", "-b", "main"]);
        git(&path, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(&path, &["remote", "add", "origin", origin]);
        path
    }

    fn env(&self, cwd: &Path) -> Env {
        Env {
            vars: [
                ("HOME".to_owned(), self.root().join("home").display().to_string()),
                ("XDG_STATE_HOME".to_owned(), self.root().join("state").display().to_string()),
            ]
            .into(),
            cwd: cwd.to_owned(),
            terminal: None,
        }
    }

    fn cache(&self) -> PathBuf {
        self.root().join("state/acyclic/clones.json")
    }

    fn learned(&self) -> BTreeMap<String, String> {
        read::<Cache>(&self.cache()).clones
    }
}

fn record(owner: &str, name: &str) -> Repository {
    Repository {
        id: "repo".to_owned(),
        owner: Value::One(owner.to_owned()),
        name: Value::One(name.to_owned()),
        default_branch: Value::One("main".to_owned()),
    }
}

fn found(machine: &Machine, owner: &str, name: &str) -> Option<PathBuf> {
    let clones = crate::context::clones(&machine.env(&machine.root())).unwrap();
    clones(&record(owner, name))
}

#[test]
fn a_worktree_records_its_main_checkout() {
    let machine = Machine::new();
    let main = machine.clone_of("main", "https://github.com/Me/Garden.git");
    let worktree = machine.root().join("tree");
    git(&main, &["worktree", "add", "-q", "-b", "side", worktree.to_str().unwrap()]);
    learn(&machine.env(&worktree));
    assert_eq!(
        machine.learned(),
        BTreeMap::from([("me/garden".to_owned(), main.display().to_string())])
    );
    assert_eq!(found(&machine, "me", "garden"), Some(main));
}

#[test]
fn only_a_github_origin_naming_owner_and_name_is_learned() {
    let machine = Machine::new();
    for (dir, origin) in [
        ("other-host", "https://example.test/me/garden.git"),
        ("deep", "https://github.com/me/garden/extra.git"),
        ("local", "/srv/me/garden"),
    ] {
        let clone = machine.clone_of(dir, origin);
        learn(&machine.env(&clone));
    }
    let no_origin = machine.root().join("none");
    std::fs::create_dir_all(&no_origin).unwrap();
    git(&no_origin, &["init", "-q"]);
    git(&no_origin, &["remote", "add", "upstream", "https://github.com/me/garden.git"]);
    learn(&machine.env(&no_origin));
    let bare = machine.root().join("bare.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare"]);
    git(&bare, &["remote", "add", "origin", "https://github.com/me/garden.git"]);
    learn(&machine.env(&bare));
    assert!(!machine.cache().exists());
}

#[test]
fn a_fork_is_not_learned_under_the_upstream_name() {
    let machine = Machine::new();
    let fork = machine.clone_of("fork", "git@github.com:someone/garden.git");
    git(&fork, &["remote", "add", "upstream", "https://github.com/me/garden.git"]);
    learn(&machine.env(&fork));
    assert_eq!(machine.learned().keys().collect::<Vec<_>>(), ["someone/garden"]);
    assert_eq!(found(&machine, "me", "garden"), None);
    assert_eq!(found(&machine, "someone", "garden"), Some(fork));
}

#[test]
fn a_deleted_path_or_a_changed_origin_is_dropped() {
    let machine = Machine::new();
    let gone = machine.clone_of("gone", "https://github.com/me/gone.git");
    let moved = machine.clone_of("moved", "https://github.com/me/moved.git");
    let kept = machine.clone_of("kept", "https://github.com/me/kept.git");
    for clone in [&gone, &moved, &kept] {
        learn(&machine.env(clone));
    }
    assert_eq!(machine.learned().len(), 3);
    std::fs::remove_dir_all(&gone).unwrap();
    git(&moved, &["remote", "set-url", "origin", "https://github.com/me/renamed.git"]);
    assert_eq!(found(&machine, "me", "gone"), None);
    assert_eq!(found(&machine, "me", "moved"), None);
    assert_eq!(found(&machine, "me", "kept"), Some(kept));
    assert_eq!(machine.learned().keys().collect::<Vec<_>>(), ["me/kept"]);
}

#[test]
fn a_lookup_is_memoized_for_the_commands_lifetime() {
    let machine = Machine::new();
    let clone = machine.clone_of("clone", "https://github.com/me/garden.git");
    learn(&machine.env(&clone));
    let learned = Rc::new(Learned::default());
    let clones = crate::context::clones_with(&machine.env(&clone), &learned).unwrap();
    assert_eq!(clones(&record("me", "garden")), Some(clone.clone()));
    std::fs::remove_dir_all(&clone).unwrap();
    let again = crate::context::clones_with(&machine.env(&machine.root()), &learned).unwrap();
    assert_eq!(again(&record("me", "garden")), Some(clone), "one command sees one answer");
}

#[test]
fn a_corrupt_or_unwritable_cache_is_harmless() {
    let machine = Machine::new();
    let clone = machine.clone_of("clone", "https://github.com/me/garden.git");
    std::fs::create_dir_all(machine.cache().parent().unwrap()).unwrap();
    std::fs::write(machine.cache(), "{ not json").unwrap();
    assert_eq!(found(&machine, "me", "garden"), None);
    learn(&machine.env(&clone));
    assert_eq!(found(&machine, "me", "garden"), Some(clone.clone()));

    // The state directory can't be created: a file is in its way.
    let blocked = machine.root().join("blocked");
    std::fs::write(&blocked, "").unwrap();
    let mut env = machine.env(&clone);
    env.vars.insert("XDG_STATE_HOME".to_owned(), blocked.display().to_string());
    learn(&env);
    let clones = crate::context::clones(&env).unwrap();
    assert_eq!(clones(&record("me", "garden")), None);
}

#[test]
fn nothing_is_rewritten_when_nothing_changed() {
    let machine = Machine::new();
    let clone = machine.clone_of("clone", "https://github.com/me/garden.git");
    learn(&machine.env(&clone));
    let first = std::fs::metadata(machine.cache()).unwrap().ino();
    learn(&machine.env(&clone));
    assert_eq!(std::fs::metadata(machine.cache()).unwrap().ino(), first, "the file was replaced");
    // A lookup that finds the entry valid leaves it alone too.
    assert_eq!(found(&machine, "me", "garden"), Some(clone));
    assert_eq!(std::fs::metadata(machine.cache()).unwrap().ino(), first);
    // Atomic: no temporary file is left behind.
    let left: Vec<_> = std::fs::read_dir(machine.cache().parent().unwrap()).unwrap().collect();
    assert_eq!(left.len(), 1);
}

#[test]
fn the_cache_defaults_to_the_home_state_directory() {
    let env = Env {
        vars: [("HOME".to_owned(), "/h".to_owned())].into(),
        cwd: PathBuf::from("/"),
        terminal: None,
    };
    assert_eq!(cache_path(&env), Some(PathBuf::from("/h/.local/state/acyclic/clones.json")));
}

#[test]
fn clones_toml_wins_over_a_learned_clone() {
    let machine = Machine::new();
    let clone = machine.clone_of("clone", "https://github.com/me/garden.git");
    learn(&machine.env(&clone));
    assert_eq!(found(&machine, "me", "garden"), Some(clone));
    let config = machine.root().join("home/.config/acyclic");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("clones.toml"), "[clones]\n\"Me/Garden\" = \"/mapped/garden\"\n")
        .unwrap();
    assert_eq!(found(&machine, "me", "garden"), Some(PathBuf::from("/mapped/garden")));
}
