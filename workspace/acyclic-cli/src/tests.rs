use std::fmt::Write as _;

use acyclic_core::archive::ArchiveError;
use acyclic_core::import::{ApplyError, ParseError};
use acyclic_core::locate::LocateError;
use acyclic_core::model::{RunState, WriteError};
use acyclic_core::scope::ScopeError;
use acyclic_core::state::{Error as StateError, Key, ReplicaId};
use acyclic_core::store::StoreError;

use clap::Parser;

use super::*;
use crate::error::CliError;

fn env(vars: &[(&str, &str)]) -> Env {
    Env {
        vars: vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        cwd: PathBuf::from("/"),
    }
}

fn actor(line: &str, vars: &[(&str, &str)]) -> Result<String, &'static str> {
    let cli = args::Cli::try_parse_from(line.split(' ')).unwrap();
    context::actor(&cli, &env(vars)).map_err(|e| e.code)
}

#[test]
fn the_user_is_never_inherited_from_the_environment() {
    let user = [("ACYCLIC_ACTOR", "actor/user")];
    let session = [("ACYCLIC_ACTOR", "session/mac")];
    assert_eq!(actor("acyclic needs", &[]), Ok("agent/undeclared".into()));
    assert_eq!(actor("acyclic needs", &session), Ok("session/mac".into()));
    assert_eq!(actor("acyclic needs", &user), Err("user-from-environment"));
    assert_eq!(actor("acyclic needs --as-user", &user), Ok("actor/user".into()));
    assert_eq!(actor("acyclic needs --actor actor/user", &session), Ok("actor/user".into()));
    assert_eq!(actor("acyclic needs --actor nobody", &[]), Err("invalid-actor"));
}

#[test]
fn a_tracker_clone_needs_no_home_or_mapping() {
    let dir = tempfile::TempDir::new().unwrap();
    let git = |args: &[&str]| {
        let status =
            std::process::Command::new("git").arg("-C").arg(dir.path()).args(args).status();
        assert!(status.unwrap().success(), "git {args:?}");
    };
    git(&["init", "-q", "clone"]);
    git(&["-C", "clone", "config", "acyclic.replica", "mac"]);
    let inside = dir.path().join("clone/deeper");
    std::fs::create_dir(&inside).unwrap();
    let cli = args::Cli::try_parse_from(["acyclic", "list"]).unwrap();
    // No HOME, so no mapping could be read.
    let found = context::tracker(&cli, &Env { cwd: inside, ..Env::default() }).unwrap();
    assert_eq!(found.canonicalize().unwrap(), dir.path().join("clone").canonicalize().unwrap());
}

#[test]
fn every_error_has_a_stable_code_and_exit() {
    let key = Key::new("item", "a1").unwrap();
    // One of each variant, so a renamed code shows here.
    let id = || ReplicaId::new("mac").unwrap();
    let errors: Vec<CliError> = vec![
        StateError::InvalidName("A".into()).into(),
        StateError::InvalidValue.into(),
        StateError::ClockJump { now: 0, last: 1 }.into(),
        StateError::NotContinuous(id()).into(),
        StateError::IdReused { replica: id(), mine: 1, theirs: 2 }.into(),
        StateError::Gap { run: "r".into(), expected: 1, got: 2 }.into(),
        StateError::WrongReplica { run: "r".into(), applied_by: "mac".into() }.into(),
        StateError::Claimed(key.clone()).into(),
        StateError::NotALeaf(key.clone()).into(),
        StateError::LeaseRequired("session/s".into()).into(),
        StateError::ResourceHeld { resource: "gpu".into(), item: key, holder: "session/s".into() }
            .into(),
        StateError::Format("bad".into()).into(),
        WriteError::InvalidName("A".into()).into(),
        WriteError::NoItem("a1".into()).into(),
        WriteError::ParentCycle("a1".into()).into(),
        WriteError::LinkCycle(vec!["a1".into()]).into(),
        WriteError::Checked { field: "state".into(), write: "set_state" }.into(),
        WriteError::UserOnly { field: "approved".into(), actor: "session/s".into() }.into(),
        WriteError::SelfLink("a1".into()).into(),
        WriteError::RunTransition { run: "r".into(), to: RunState::Running }.into(),
        WriteError::RunActive { thread: "t".into(), run: "r".into() }.into(),
        ScopeError::NotTheUser("session/s".into()).into(),
        ScopeError::NoProposal("p".into()).into(),
        ScopeError::Decided("p".into()).into(),
        ArchiveError::Ineligible(vec![]).into(),
        ArchiveError::NotDone("a1".into()).into(),
        ArchiveError::NotAllowed("run/r".into()).into(),
        StoreError::Git("failed".into()).into(),
        StoreError::Conflict.into(),
        StoreError::Locked { path: "x".into(), age_seconds: None }.into(),
        StoreError::Timeout.into(),
        StoreError::Config("no replica".into()).into(),
        StoreError::RemoteMoved.into(),
        StoreError::Rejected { reference: "r".into(), reason: "no".into() }.into(),
        LocateError::Config("bad".into()).into(),
        LocateError::NoRemote.into(),
        LocateError::Git("failed".into()).into(),
        LocateError::NotMapped(vec![]).into(),
        ParseError::NotAnImport.into(),
        ParseError::UnsupportedVersion { found: 9 }.into(),
        ApplyError::Invalid(vec![]).into(),
        ApplyError::UserOnly("approved threads".into()).into(),
        ApplyError::Cycles(vec![]).into(),
        CliError::refused("not-ready", ""),
        CliError::refused("already-exists", ""),
        CliError::refused("unwritable", ""),
        CliError::refused("unopenable", ""),
        CliError::refused("no-renderer", ""),
    ];
    let mut table = String::new();
    for error in errors {
        writeln!(table, "{} {}", error.exit, error.code).unwrap();
    }
    insta::assert_snapshot!(table);
}

fn run(line: &str) -> Outcome {
    execute(line.split(' ').map(Into::into), &env(&[]))
}

#[test]
fn usage_errors_and_help_are_text() {
    // A command line that doesn't parse explains itself on standard error, as other tools do.
    for line in ["acyclic nonsense", "acyclic show", "acyclic link add a1 sideways a2"] {
        let out = run(line);
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{line}");
        assert!(
            out.stderr.starts_with("error: ") && out.stderr.contains("--help"),
            "{line}: {out:?}"
        );
    }
    let out = run("acyclic --version");
    assert_eq!(
        (out.code, out.stdout.trim()),
        (exit::OK, format!("acyclic {}", acyclic_core::VERSION).as_str())
    );
    // The top level holds what every session uses, apart from the groups and the rest.
    let help = run("acyclic --help");
    assert_eq!(help.code, exit::OK);
    insta::assert_snapshot!(help.stdout);
    // A bare command or group prints its help, and so does `help`, with the global options under their own heading.
    assert_eq!(run("acyclic"), help);
    assert_eq!(run("acyclic help"), help);
    let item = run("acyclic item");
    assert_eq!(run("acyclic help item"), item);
    let edit = run("acyclic help item edit");
    assert_eq!(edit, run("acyclic item edit --help"));
    assert!(edit.stdout.contains("\nGlobal options:\n"), "{edit:?}");
    assert_eq!((item.code, item.stdout.starts_with("Adds, edits")), (exit::OK, true), "{item:?}");
    // An enum-valued input lists its values in help, and a wrong one names them.
    assert!(edit.stdout.contains("[possible values: xs, s, m, l, xl]"), "{edit:?}");
    let wrong = run("acyclic item edit a --size huge");
    assert_eq!(wrong.code, exit::USAGE);
    assert!(wrong.stderr.contains("[possible values: xs, s, m, l, xl]"), "{wrong:?}");
}

#[test]
fn the_help_sections_hold_every_command_once() {
    let mut named: Vec<&str> =
        args::SECTIONS.iter().flat_map(|(_, names)| names.iter().copied()).collect();
    named.sort_unstable();
    let command = args::command();
    let mut commands: Vec<&str> = command
        .get_subcommands()
        .map(clap::Command::get_name)
        .filter(|name| *name != "help")
        .collect();
    commands.sort_unstable();
    assert_eq!(named, commands);
}

#[test]
fn times_print_as_rfc3339_in_utc() {
    let mut out =
        json!({"until": 0, "rank": 0, "claims": [{"lease_expires": null, "lease_start": 0}]});
    timestamps(&mut out);
    assert_eq!(
        out,
        json!({"until": "1970-01-01T00:00:00Z", "rank": 0,
               "claims": [{"lease_expires": null, "lease_start": "1970-01-01T00:00:00Z"}]})
    );
}

#[test]
fn an_actor_may_claim_past_its_own_claims_only() {
    use acyclic_core::ready::Status;
    use commands::may_claim;
    let me = "session/mac";
    let claimed = |holder: &str| Status::Claimed { holder: holder.into() };
    let busy = |holder: &str| Status::ResourceBusy {
        resource: "gpu".into(),
        item: "other".into(),
        holder: holder.into(),
    };
    let contested = |first: &str, second: &str| Status::Contested {
        holders: vec![first.into(), second.into()],
    };
    for (status, may) in [
        (Status::Ready { rank: 0 }, true),
        (claimed(me), true),
        (claimed("run/r1"), false),
        (contested(me, "run/r1"), true),
        (contested("run/r1", me), false),
        (busy(me), true),
        (busy("run/r1"), false),
        (Status::Parent, false),
    ] {
        assert_eq!(may_claim(Some(&status), me), may, "{status:?}");
        assert_eq!(
            commands::may_finish(Some(&status), me),
            may,
            "{status:?}: finishing as claiming"
        );
    }
    assert!(!may_claim(None, me));
    // Finishing again changes nothing, so it needs no claim; a blocked item can't be finished.
    let blocked = Status::Blocked { waiting: std::collections::BTreeMap::new() };
    for (status, may) in [(Status::Done, true), (Status::InReview, true), (blocked, false)] {
        assert_eq!(commands::may_finish(Some(&status), me), may, "{status:?}");
    }
}

#[test]
fn prime_is_text_matched_to_the_version() {
    let Outcome { code, stdout: out, .. } = run("acyclic prime");
    assert_eq!(code, exit::OK);
    assert!(out.contains(&format!("acyclic {})", acyclic_core::VERSION)));
    insta::assert_snapshot!(out.replace(acyclic_core::VERSION, "[version]"));
}

#[test]
fn ready_says_which_thread_an_item_is_in_and_whether_its_work_is_approved() {
    use acyclic_core::model::{self, By, Model, add_item};
    use acyclic_core::schema::{kind, thread};
    use acyclic_core::scope::ScopeEdit;
    use acyclic_core::state::Replica;
    let user = By { actor: acyclic_core::scope::USER, at: 1 };
    let thread_key = |id: &str| Key::new(kind::THREAD, id).unwrap();
    let mut mac = Replica::open(ReplicaId::new("mac").unwrap(), acyclic_core::state::State::new());
    for id in ["x", "y", "z", "late", "raced"] {
        add_item(&mut mac, id, id, user).unwrap();
    }
    let join = |replica: &mut Replica, item: &str, thread: &str| {
        let member = ScopeEdit::Member { item: item.to_owned(), thread: Some(thread.to_owned()) };
        acyclic_core::scope::edit(replica, &member, user).unwrap();
    };
    join(&mut mac, "x", "ok");
    join(&mut mac, "y", "draft");
    join(&mut mac, "raced", "both");
    acyclic_core::scope::approve(&mut mac, "ok", true, user).unwrap();
    model::set(&mut mac, &thread_key("draft"), thread::APPROVED, Some(&false), user).unwrap();
    // Joined after the approval, so the approval didn't see it.
    join(&mut mac, "late", "ok");
    // One replica approves while another doesn't know of it and says no: a conflict.
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    model::set(&mut mac, &thread_key("both"), thread::APPROVED, Some(&true), user).unwrap();
    model::set(&mut pi, &thread_key("both"), thread::APPROVED, Some(&false), user).unwrap();
    mac.join(pi.state()).unwrap();
    let model = Model::read(mac.state()).unwrap();
    let thread = |t: &str| Some(t.to_owned());
    for (item, expected) in [
        ("x", (thread("ok"), true)),
        ("y", (thread("draft"), false)),
        ("z", (None, false)),
        ("late", (thread("ok"), false)),
        ("raced", (thread("both"), false)),
    ] {
        assert_eq!(commands::approval(&model, item), expected, "{item}");
    }
}

#[test]
fn a_thread_lists_its_members_in_run_order_and_a_raced_member_in_both_threads() {
    use acyclic_core::model::{By, LinkKind, Model, add_item, link};
    use acyclic_core::scope::{ScopeEdit, USER, edit};
    use acyclic_core::state::{Replica, State};
    let user = By { actor: USER, at: 1 };
    let mut mac = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    let join = |replica: &mut Replica, item: &str, thread: &str| {
        let member = ScopeEdit::Member { item: item.to_owned(), thread: Some(thread.to_owned()) };
        edit(replica, &member, user).unwrap();
    };
    for id in ["s-1", "s-2", "s-10", "c-1", "c-2", "raced", "cleared"] {
        add_item(&mut mac, id, id, user).unwrap();
        join(&mut mac, id, "t");
    }
    // By id, s-10 would come before s-2; its links run it last.
    link(&mut mac, "s-2", "s-1", LinkKind::Blocks, user).unwrap();
    link(&mut mac, "s-10", "s-2", LinkKind::Blocks, user).unwrap();
    // Each replica closes half of a cycle, and each puts the raced item in another thread.
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    link(&mut mac, "c-1", "c-2", LinkKind::Blocks, user).unwrap();
    link(&mut pi, "c-2", "c-1", LinkKind::Blocks, user).unwrap();
    join(&mut mac, "raced", "u");
    join(&mut pi, "raced", "v");
    // One replica moves it on while the other takes it out of every thread.
    join(&mut mac, "cleared", "u");
    edit(&mut pi, &ScopeEdit::Member { item: "cleared".to_owned(), thread: None }, user).unwrap();
    mac.join(pi.state()).unwrap();
    let model = Model::read(mac.state()).unwrap();
    let threads = commands::thread_members(&model);
    let listed = |thread: &str| -> Vec<(&str, bool)> {
        threads[thread].iter().map(|m| (m.item, m.conflict)).collect()
    };
    assert_eq!(
        listed("t"),
        [("s-1", false), ("s-2", false), ("s-10", false), ("c-1", false), ("c-2", false)]
    );
    // Neither u nor v has fields of its own: an item names each.
    assert!(!model.threads.contains_key("u") && !model.threads.contains_key("v"));
    assert_eq!(listed("u"), [("cleared", true), ("raced", true)]);
    assert_eq!(listed("v"), [("raced", true)]);
}

/// Every argument under `command` that the parser constrains and deserializing doesn't, as `<command path>:
/// <argument>`. Deserializing already refuses a missing field that isn't optional or a list. Argument groups aren't
/// walked: the command line has none.
fn constrained(command: &clap::Command, path: &str, found: &mut Vec<String>) {
    // The global options are checked once, by `actor`.
    for arg in command.get_arguments().filter(|arg| !arg.is_global_set()) {
        let debug = format!("{arg:?}");
        let constrained = (arg.is_required_set()
            && matches!(arg.get_action(), clap::ArgAction::Append))
            || !command.get_arg_conflicts_with(arg).is_empty()
            || ["requires: []", "r_unless: []", "r_ifs: []"]
                .iter()
                .any(|empty| !debug.contains(empty));
        if constrained {
            found.push(format!("{path}: {}", arg.get_id()));
        }
    }
    for sub in command.get_subcommands() {
        constrained(sub, format!("{path} {}", sub.get_name()).trim_start(), found);
    }
}

#[test]
fn deserialized_commands_are_checked_as_the_parser_checks_them() {
    // `Command::validate` repeats these for commands that arrive deserialized, as the server's do; a new constraint
    // needs a rule there before it joins this list. `serve` is never deserialized.
    let mut found = Vec::new();
    for sub in args::command().get_subcommands().filter(|sub| sub.get_name() != "serve") {
        constrained(sub, sub.get_name(), &mut found);
    }
    found.sort();
    assert_eq!(
        found,
        [
            "done: commit",
            "done: repository",
            "import: dry_run",
            "import: file",
            "import: schema",
            "repo attach: item",
            "repo detach: item",
            "show: items",
            "thread show: threads",
        ]
    );
    let refused = |value: Value| {
        let command: args::Command = serde_json::from_value(value).unwrap();
        command.validate().map_err(|err| err.code)
    };
    assert_eq!(refused(json!({"show": {"items": []}})), Err("usage"));
    assert_eq!(refused(json!({"thread": {"show": {"threads": []}}})), Err("usage"));
    assert_eq!(refused(json!({"done": {"id": "a", "commit": "c"}})), Err("usage"));
    assert_eq!(refused(json!({"import": {}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"file": "p", "schema": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true, "dry-run": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true}})), Ok(()));
    assert_eq!(refused(json!({"import": {"file": "p", "dry-run": true}})), Ok(()));
    assert_eq!(refused(json!({"repo": {"attach": {"repository": "r"}}})), Err("usage"));
    assert_eq!(
        refused(json!({"repo": {"detach": {"repository": "r", "item": "a", "thread": "t"}}})),
        Err("usage")
    );
    for lease in [0, 481] {
        assert_eq!(refused(json!({"claim": {"id": "a", "lease": lease}})), Err("usage"));
    }
    for lease in [1, 480] {
        assert_eq!(refused(json!({"claim": {"id": "a", "lease": lease}})), Ok(()));
    }
    assert_eq!(refused(json!({"done": {"id": "a", "repository": "r", "commit": "c"}})), Ok(()));
    assert_eq!(refused(json!({"repo": {"attach": {"repository": "r", "thread": "t"}}})), Ok(()));
    // Unknown fields and the server's own command are refused when deserializing.
    assert!(
        serde_json::from_value::<args::Command>(json!({"show": {"items": ["a"], "force": true}}))
            .is_err()
    );
    assert!(serde_json::from_value::<args::Command>(json!({"serve": {"login": "x"}})).is_err());
}

#[test]
fn a_closed_pipe_is_an_error_to_return_not_a_panic() {
    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let err = emit(&mut Closed, "text").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    let mut ok = Vec::new();
    emit(&mut ok, "text").unwrap();
    assert_eq!(ok, b"text");
}

#[test]
fn evidence_must_be_able_to_verify_and_is_not_silently_replaced() {
    use acyclic_core::model::{By, Evidence, ItemState, Model, add_item, set, set_state};
    use acyclic_core::schema::{item, kind, repository};
    use acyclic_core::state::{Replica, State};
    let user = By { actor: acyclic_core::scope::USER, at: 1 };
    let mut mac = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    add_item(&mut mac, "a", "A", user).unwrap();
    let repo = Key::new(kind::REPOSITORY, "r").unwrap();
    set(&mut mac, &repo, repository::OWNER, Some(&"o"), user).unwrap();
    let evidence = |repository: &str, commit: &str| Evidence {
        repository: Some(repository.to_owned()),
        commit: Some(commit.to_owned()),
        note: None,
    };
    let code = |model: &Model, e: &Evidence| commands::check_evidence(model, e).map_err(|e| e.code);
    let model = Model::read(mac.state()).unwrap();
    assert_eq!(code(&model, &evidence("ghost", "abc1234")), Err("not-found"));
    assert_eq!(code(&model, &evidence("r", "abc1234")), Err("incomplete-repository"));
    for (name, value) in [
        (repository::NAME, json!("n")),
        (repository::PUBLIC, json!(true)),
        (repository::DEFAULT_BRANCH, json!("main")),
    ] {
        set(&mut mac, &repo, name, Some(&value), user).unwrap();
    }
    let model = Model::read(mac.state()).unwrap();
    for bad in ["123", "abc123", "abc123g", &"a".repeat(65), ""] {
        assert_eq!(code(&model, &evidence("r", bad)), Err("invalid-commit"), "{bad:?}");
    }
    for good in ["abc1234", "ABC1234", &"a".repeat(64)] {
        assert_eq!(code(&model, &evidence("r", good)), Ok(()), "{good}");
    }
    assert_eq!(code(&model, &Evidence { note: Some("n".into()), ..Evidence::default() }), Ok(()));

    // Only done items with other evidence are overwritten.
    let first = evidence("r", "abc1234");
    let overwrites = |replica: &Replica, e: &Evidence| {
        commands::overwrites(&Model::read(replica.state()).unwrap().items["a"], e)
    };
    assert!(!overwrites(&mac, &first), "an item that isn't done");
    set(&mut mac, &Key::new(kind::ITEM, "a").unwrap(), item::EVIDENCE, Some(&first), user).unwrap();
    assert!(!overwrites(&mac, &evidence("r", "def5678")), "an item that isn't done");
    set_state(&mut mac, "a", ItemState::Done, user).unwrap();
    assert!(!overwrites(&mac, &first), "the same evidence");
    assert!(overwrites(&mac, &evidence("r", "def5678")));
    assert!(overwrites(&mac, &Evidence { note: Some("n".into()), ..Evidence::default() }));
}

#[test]
fn a_lease_outside_its_range_is_a_usage_error_naming_the_range() {
    for lease in ["0", "481", "999"] {
        let err =
            args::Cli::try_parse_from(["acyclic", "claim", "a", "--lease", lease]).unwrap_err();
        assert!(err.to_string().contains("1..=480"), "{err}");
    }
    assert!(args::Cli::try_parse_from(["acyclic", "claim", "a", "--lease", "480"]).is_ok());
    // The help text says the same bound.
    let help = run("acyclic help claim").stdout;
    assert!(help.contains(&format!("1 to {}", args::LEASE_MAX_MINUTES)), "{help}");
}

#[test]
fn creating_a_repository_record_needs_every_field_and_editing_any() {
    let (o, n, main) = ("o".to_owned(), "n".to_owned(), "main".to_owned());
    let check = |exists, owner, name, public, branch| {
        commands::check_new_repository("r", exists, owner, name, public, branch)
    };
    let err = check(false, Some(&o), None, None, Some(&main)).unwrap_err();
    assert_eq!((err.code, err.exit), ("usage", error::exit::USAGE));
    assert!(err.message.contains("creating repository r needs"), "{err}");
    assert!(err.message.contains("(missing: --name, --public)"), "{err}");
    assert!(check(false, Some(&o), Some(&n), Some(true), Some(&main)).is_ok());
    assert!(check(true, Some(&o), None, None, None).is_ok());
}

#[test]
fn links_are_named_waits_for_follows_and_relates_to_and_the_old_words_say_so() {
    use acyclic_core::model::LinkKind;
    for (word, kind) in [
        (args::LinkWord::WaitsFor, LinkKind::Blocks),
        (args::LinkWord::Follows, LinkKind::After),
        (args::LinkWord::RelatesTo, LinkKind::Relates),
    ] {
        assert_eq!(word.kind(), kind);
    }
    // Exhaustive, so a new word must be listed.
    for word in args::LinkWord::ALL {
        match word {
            args::LinkWord::WaitsFor | args::LinkWord::Follows | args::LinkWord::RelatesTo => {},
        }
    }
    for (old, new) in [("blocks", "waits-for"), ("after", "follows"), ("relates", "relates-to")] {
        let out = run(&format!("acyclic link add a2 {old} a1"));
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{old}");
        assert!(
            out.stderr
                .contains(&format!("`{old}` is now `{new}`: use `acyclic link add a2 {new} a1`")),
            "{out:?}"
        );
    }
    // The hint follows a global option, and a word that was never a kind gets the usual error.
    let out = run("acyclic --actor session/mac link remove a2 blocks a1");
    assert!(out.stderr.contains("use `acyclic link remove a2 waits-for a1`"), "{out:?}");
    assert!(run("acyclic link add a2 sideways a1").stderr.contains("[possible values: waits-for"));
    let help = run("acyclic help link add").stdout;
    assert!(help.contains("`<FROM>` waits for, follows, or relates to `<TO>`"), "{help}");
    assert!(help.contains("acyclic link add plant waits-for beds"), "{help}");
}

#[test]
fn import_schema_prints_an_example_as_text_and_takes_no_file() {
    let out = run("acyclic import --schema");
    assert_eq!((out.code, out.stdout.as_str()), (exit::OK, acyclic_core::import::SCHEMA));
    assert!(run("acyclic help import").stdout.contains("--schema"));
    for line in
        ["acyclic import --schema plan.json", "acyclic import", "acyclic import --schema --dry-run"]
    {
        let out = run(line);
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{line}");
    }
}
