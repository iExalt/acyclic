use std::fmt::Write as _;

use acyclic_core::archive::ArchiveError;
use acyclic_core::delete::DeleteError;
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
        terminal: None,
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
    // Nor is the operator, from anywhere.
    let operator = [("ACYCLIC_ACTOR", "operator/mac")];
    assert_eq!(actor("acyclic needs", &operator), Err("operator-actor"));
    assert_eq!(actor("acyclic needs --actor operator/mac", &[]), Err("operator-actor"));
}

#[test]
fn output_is_pretty_at_a_terminal_and_json_in_a_pipe_unless_said_otherwise() {
    use args::Format::{Json, Pretty};
    let output = |line: &str, vars: &[(&str, &str)], terminal: Option<u16>| {
        let cli = args::Cli::try_parse_from(line.split(' ')).unwrap();
        context::output(&cli, &Env { terminal, ..env(vars) }).map_err(|e| e.code)
    };
    let (tty, pipe) = (Some(80), None);
    let (json, pretty) = ([("ACYCLIC_OUTPUT", "json")], [("ACYCLIC_OUTPUT", "pretty")]);
    // An agent's tool reads a pipe, so agents get JSON without asking.
    assert_eq!(output("acyclic list", &[], pipe), Ok(Json));
    assert_eq!(output("acyclic list", &[], tty), Ok(Pretty));
    // The environment overrides the terminal, and the flag overrides both.
    assert_eq!(output("acyclic list", &json, tty), Ok(Json));
    assert_eq!(output("acyclic list", &pretty, pipe), Ok(Pretty));
    assert_eq!(output("acyclic list -o json", &pretty, tty), Ok(Json));
    assert_eq!(output("acyclic list -o pretty", &json, pipe), Ok(Pretty));
    assert_eq!(output("acyclic list", &[("ACYCLIC_OUTPUT", "yaml")], tty), Err("usage"));
    // At a terminal, an error is pretty's line on standard error, and a bad setting is still JSON on standard output.
    let at = |vars: &[(&str, &str)]| {
        let args = ["acyclic", "show", "a1", "--tracker", "/nonexistent"].map(Into::into);
        execute(args, &Env { terminal: tty, ..env(vars) })
    };
    let out = at(&[]);
    assert!(out.stdout.is_empty() && out.stderr.starts_with("error: "), "{out:?}");
    let out = at(&[("ACYCLIC_OUTPUT", "yaml")]);
    assert!(out.stdout.contains("ACYCLIC_OUTPUT is yaml") && out.stderr.is_empty(), "{out:?}");
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
#[expect(clippy::too_many_lines, reason = "one line per error")]
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
        StateError::NoThread("t".into()).into(),
        StateError::LeaseRequired("session/s".into()).into(),
        StateError::ResourceHeld {
            resource: "gpu".into(),
            thread: "t".into(),
            holder: "session/s".into(),
        }
        .into(),
        StateError::Format("bad".into()).into(),
        WriteError::InvalidName("A".into()).into(),
        WriteError::NoItem("a1".into()).into(),
        WriteError::Deleted("a1".into()).into(),
        WriteError::ParentCycle("a1".into()).into(),
        WriteError::LinkCycle(vec!["a1".into()]).into(),
        WriteError::Checked { field: "state".into(), write: "set_state" }.into(),
        WriteError::UserOnly { field: "approved".into(), actor: "session/s".into() }.into(),
        WriteError::SelfLink("a1".into()).into(),
        WriteError::RunTransition { run: "r".into(), to: RunState::Running }.into(),
        WriteError::RunActive { thread: "t".into(), run: "r".into() }.into(),
        WriteError::RunWriter("run/r".into()).into(),
        ScopeError::NotTheUser("session/s".into()).into(),
        ScopeError::NoProposal("p".into()).into(),
        ScopeError::Decided("p".into()).into(),
        DeleteError::InUse { item: "a1".into(), reasons: vec![] }.into(),
        DeleteError::NotAllowed { actor: "session/s".into(), thread: "t".into() }.into(),
        ArchiveError::Ineligible { item: "a1".into(), reasons: vec![] }.into(),
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
        // Raised by the commands themselves.
        CliError::refused("not-ready", ""),
        CliError::refused("already-exists", ""),
        CliError::refused("already-done", ""),
        CliError::refused("not-in-thread", ""),
        CliError::refused("not-the-runs", ""),
        CliError::refused("incomplete-repository", ""),
        CliError::refused("unwritable", ""),
        CliError::refused("unopenable", ""),
        CliError::refused("no-renderer", ""),
        CliError::new("not-found", exit::NOT_FOUND, ""),
        CliError::new("usage", exit::USAGE, ""),
        CliError::new("invalid-commit", exit::USAGE, ""),
        CliError::new("invalid-actor", exit::USAGE, ""),
        CliError::new("user-from-environment", exit::USAGE, ""),
        CliError::new("operator-actor", exit::USAGE, ""),
        CliError::new("config", exit::CONFIG, ""),
        CliError::new("no-remote", exit::CONFIG, ""),
        CliError::new("no-server", exit::CONFIG, ""),
    ];
    let mut table = String::new();
    for error in &errors {
        // Whether a run's operation that fails this way spends its number: a new code takes a side here.
        let spent = if spends(error) { " spends" } else { "" };
        writeln!(table, "{} {}{spent}", error.exit, error.code).unwrap();
    }
    insta::assert_snapshot!(table);
    // Every code a command can raise is in the table above, so the next one added there must be added here.
    let known: std::collections::BTreeSet<&str> = errors.iter().map(|error| error.code).collect();
    let mut scanned = 0;
    for file in ["commands.rs", "context.rs", "edit.rs", "lib.rs", "args.rs", "hints.rs", "page.rs"]
    {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file),
        )
        .unwrap();
        let tests = source.find("#[cfg(test)]").unwrap_or(source.len());
        for call in ["CliError::new(", "CliError::refused("] {
            for (at, _) in source[..tests].match_indices(call) {
                let rest = source[at + call.len()..].trim_start();
                let code = rest.strip_prefix('"').and_then(|r| r.split('"').next());
                // A code that isn't a literal, such as one passed in, is checked where it is written.
                if let Some(code) = code {
                    assert!(known.contains(code), "{file}: {code} is raised but not in the table");
                    scanned += 1;
                }
            }
        }
    }
    assert!(scanned > 10, "the scan found only {scanned} codes");
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
    let mut out = json!({"eligible_at": 0, "members": 0,
                         "claims": [{"lease_expires_at": null, "lease_started_at": 0}]});
    timestamps(&mut out);
    assert_eq!(
        out,
        json!({"eligible_at": "1970-01-01T00:00:00Z", "members": 0,
               "claims": [{"lease_expires_at": null,
                           "lease_started_at": "1970-01-01T00:00:00Z"}]})
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
        thread: "other".into(),
        holder: holder.into(),
    };
    let contested = |first: &str, second: &str| Status::Contested {
        holders: vec![first.into(), second.into()],
    };
    for (status, may) in [
        (Status::Ready, true),
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
    let user = By { actor: acyclic_core::scope::USER, at: 1, seen: None };
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
    // An agent's add that raced the approval reaches the thread unproposed, and the approval didn't see it.
    let late = model::item_key("late").unwrap();
    mac.write(&late, "thread", "session/mac", 2, Some(json!("ok"))).unwrap();
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
        assert_eq!(crate::view::approval(&model, item), expected, "{item}");
    }
}

#[test]
fn a_thread_lists_its_members_in_run_order_and_a_raced_member_in_both_threads() {
    use acyclic_core::model::{By, LinkKind, Model, add_item, link};
    use acyclic_core::scope::{ScopeEdit, USER, edit};
    use acyclic_core::state::{Replica, State};
    let user = By { actor: USER, at: 1, seen: None };
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
    // needs a rule there before it joins this list. `serve` and `up` are never deserialized.
    let mut found = Vec::new();
    for sub in
        args::command().get_subcommands().filter(|sub| !["serve", "up"].contains(&sub.get_name()))
    {
        // `completions` is never deserialized either, but has nothing to check.
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
            "proposal show: proposals",
            "repo show: repositories",
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
    assert_eq!(refused(json!({"done": {"item": "a", "commit": "c"}})), Err("usage"));
    assert_eq!(refused(json!({"import": {}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"file": "p", "schema": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true, "dry-run": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true}})), Ok(()));
    assert_eq!(refused(json!({"import": {"file": "p", "dry-run": true}})), Ok(()));
    assert_eq!(refused(json!({"repo": {"show": {"repositories": []}}})), Err("usage"));
    assert_eq!(refused(json!({"proposal": {"show": {"proposals": []}}})), Err("usage"));
    for lease in [0, 481] {
        assert_eq!(refused(json!({"claim": {"thread": "t", "lease": lease}})), Err("usage"));
    }
    for lease in [1, 480] {
        assert_eq!(refused(json!({"claim": {"thread": "t", "lease": lease}})), Ok(()));
    }
    assert_eq!(refused(json!({"done": {"item": "a", "repository": "r", "commit": "c"}})), Ok(()));
    // Unknown fields and the server's own command are refused when deserializing.
    assert!(
        serde_json::from_value::<args::Command>(json!({"show": {"items": ["a"], "force": true}}))
            .is_err()
    );
    assert!(serde_json::from_value::<args::Command>(json!({"serve": {"login": "x"}})).is_err());
    assert!(
        serde_json::from_value::<args::Command>(json!({"completions": {"shell": "zsh"}})).is_err()
    );
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
    let user = By { actor: acyclic_core::scope::USER, at: 1, seen: None };
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
        assert!(err.to_string().contains("--lease takes 1 to 480 minutes"), "{err}");
        assert!(!err.to_string().contains("1..="), "{err}");
    }
    assert!(args::Cli::try_parse_from(["acyclic", "claim", "a", "--lease", "480"]).is_ok());
    // The help text says the same bound.
    let help = run("acyclic help claim").stdout;
    assert!(help.contains(&format!("1 to {}", args::LEASE_MAX_MINUTES)), "{help}");
}

#[test]
fn links_are_named_waits_for_and_relates_to_and_the_old_words_say_so() {
    use acyclic_core::model::LinkKind;
    for (word, kind) in [
        (args::LinkWord::WaitsFor, LinkKind::Blocks),
        (args::LinkWord::RelatesTo, LinkKind::Relates),
    ] {
        assert_eq!(word.kind(), kind);
    }
    // Exhaustive, so a new word must be listed.
    for word in args::LinkWord::ALL {
        match word {
            args::LinkWord::WaitsFor | args::LinkWord::RelatesTo => {},
        }
    }
    let out = run("acyclic link remove a2 follows a1");
    assert!(out.stderr.contains("use `acyclic link remove a2 relates-to a1`"), "{out:?}");
    // Soft links are retired, under either name, with the words to use instead.
    for retired in ["follows", "after"] {
        let out = run(&format!("acyclic link add a2 {retired} a1"));
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{retired}");
        assert!(
            out.stderr.contains(&format!(
                "`{retired}` links are retired: use `acyclic link add a2 waits-for a1` when a2 can't start until a1 \
                 is done, or `relates-to` for context"
            )),
            "{out:?}"
        );
    }
    for (old, new) in [("blocks", "waits-for"), ("relates", "relates-to")] {
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
    assert!(help.contains("`<FROM>` waits for or relates to `<TO>`"), "{help}");
    assert!(!help.contains("follows"), "{help}");
    assert!(help.contains("acyclic link add plant waits-for beds"), "{help}");
}

#[test]
fn completions_are_text_that_reaches_nested_commands_and_short_flags() {
    for (shell, nested, short) in [
        ("bash", "subcmd__item__subcmd__edit", "-t"),
        ("zsh", "(edit)", "'-t+["),
        ("fish", "from edit\" -s t", "-l dry-run"),
    ] {
        let out = run(&format!("acyclic completions {shell}"));
        assert_eq!((out.code, out.stderr.as_str()), (exit::OK, ""), "{shell}");
        assert!(out.stdout.contains(nested) && out.stdout.contains(short), "{shell}");
        assert!(out.stdout.contains("dry-run") && out.stdout.contains("completions"), "{shell}");
    }
    assert_eq!(run("acyclic completions tcsh").code, exit::USAGE);
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

/// Every command below `command`, as `(path, command)`, leaving out the server and the built-in help.
fn walk<'a>(command: &'a clap::Command, path: &str, found: &mut Vec<(String, &'a clap::Command)>) {
    for sub in command.get_subcommands().filter(|sub| !["serve", "help"].contains(&sub.get_name()))
    {
        let path = format!("{path} {}", sub.get_name()).trim_start().to_owned();
        found.push((path.clone(), sub));
        walk(sub, &path, found);
    }
}

#[test]
fn every_command_follows_the_input_conventions() {
    let root = args::command();
    let mut commands = Vec::new();
    walk(&root, "", &mut commands);
    // Positionals are the subject's id, named for the entity; these are the only others.
    let exceptions = [
        ("init", "path"),
        ("completions", "shell"),
        ("import", "file"),
        ("level", "level"),
        ("link add", "from"),
        ("link add", "kind"),
        ("link add", "to"),
        ("link remove", "from"),
        ("link remove", "kind"),
        ("link remove", "to"),
    ];
    let entities = ["ITEM", "THREAD", "REPOSITORY", "PROPOSAL"];
    for (path, command) in &commands {
        let longs: Vec<&str> = command.get_arguments().filter_map(clap::Arg::get_long).collect();
        for arg in command.get_arguments() {
            let id = arg.get_id().as_str();
            if arg.is_positional() {
                let shown = arg
                    .get_value_names()
                    .and_then(|names| names.first())
                    .map_or_else(|| id.to_uppercase(), ToString::to_string);
                assert!(
                    entities.contains(&shown.as_str()) || exceptions.contains(&(path.as_str(), id)),
                    "{path}: positional <{shown}> is neither an entity's id nor a listed exception"
                );
            }
            // A short flag is its long's initial, apart from the usual `-n` for a dry run.
            if let (Some(short), Some(long)) = (arg.get_short(), arg.get_long()) {
                assert!(
                    long.starts_with(short) || (short, long) == ('n', "dry-run"),
                    "{path}: -{short} for --{long}"
                );
            }
            // A flag that clears or removes has the flag that sets or adds.
            if let Some(field) = arg.get_long().and_then(|long| long.strip_prefix("no-")) {
                assert!(longs.contains(&field), "{path}: --no-{field} without --{field}");
            }
            // An input that takes one of the model's enums lists its values.
            if ["size", "state", "level", "kind", "format", "renderer", "shell"].contains(&id) {
                assert!(!arg.get_possible_values().is_empty(), "{path}: {id} lists no values");
            }
        }
    }
    // The global options sit under their own heading on every page.
    for arg in root.get_arguments().filter(|arg| arg.is_global_set()) {
        assert_eq!(arg.get_help_heading(), Some("Global options"), "{:?}", arg.get_id());
    }
    // Every group has the verbs that make it uniform where it has the entity.
    let group = |name: &str| -> Vec<&str> {
        commands.iter().filter_map(|(path, _)| path.strip_prefix(name)?.strip_prefix(' ')).collect()
    };
    for (name, verbs) in [
        ("item", &["add", "edit"][..]),
        ("thread", &["add", "edit", "list", "show"]),
        ("repo", &["add", "edit", "list", "show"]),
    ] {
        for verb in verbs {
            assert!(group(name).contains(verb), "{name} {verb}");
        }
    }
}

#[test]
fn removed_commands_say_what_replaced_them_and_valid_ones_are_left_alone() {
    let hint = |line: &str| {
        let args: Vec<OsString> = line.split(' ').map(Into::into).collect();
        hints::hint(&args)
    };
    for (line, instead) in [
        ("acyclic item move a1 b1", "acyclic item edit <ITEM> --parent <PARENT>"),
        ("acyclic item state a1 parked", "acyclic item edit <ITEM> --state <STATE>"),
        ("acyclic thread set t1 --title T", "acyclic thread add <THREAD> --title <TITLE>"),
        ("acyclic thread add a1 t1", "acyclic item edit <ITEM> --thread <THREAD>"),
        ("acyclic --actor x thread remove a1", "acyclic item edit <ITEM> --no-thread"),
        ("acyclic repo set r1 --name n", "acyclic repo add <REPOSITORY>"),
        ("acyclic repo attach r1 --item a1", "acyclic item edit <ITEM> --repository <REPOSITORY>"),
        ("acyclic repo detach r1 --thread t1", "acyclic thread edit <THREAD> --no-repository"),
        ("acyclic level --thread t1 blockers", "acyclic thread edit <THREAD> --level <LEVEL>"),
        ("acyclic graph --output graph.dot", "acyclic graph --file <FILE>"),
        ("acyclic -o pretty item move a1 b1", "acyclic item edit <ITEM> --parent <PARENT>"),
        ("acyclic item edit a1 --state done", "acyclic done <ITEM>"),
        ("acyclic item add a1 --title T --state=done", "acyclic done <ITEM>"),
    ] {
        assert!(hint(line).is_some_and(|text| text.contains(instead)), "{line}: {:?}", hint(line));
    }
    // Commands that kept their names in a new shape get no hint.
    for line in [
        "acyclic thread add t1 --title T",
        "acyclic thread add t1",
        "acyclic item edit a1 --parent b1",
        "acyclic item edit a1 --state parked",
        "acyclic level blockers",
        "acyclic thread edit t1 --level blockers",
        "acyclic repo edit r1 --private",
        // A way to print parses as one, so a graph line failing with it failed for another reason.
        "acyclic graph --output pretty --bogus",
        "acyclic --output=json graph --bogus",
    ] {
        assert_eq!(hint(line), None, "{line}");
    }
    let out = run("acyclic thread set t1 --title T");
    assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""));
    assert!(
        out.stderr.starts_with("error: `acyclic thread set` is gone: use `acyclic thread add"),
        "{out:?}"
    );
    // The old form of a command that is still there is caught as the failed parse it is.
    let old_add = run("acyclic thread add a1 t1").stderr;
    assert!(old_add.starts_with("error: `thread add <ITEM> <THREAD>` is gone: use"), "{old_add}");
    assert!(old_add.contains("acyclic item edit <ITEM> --thread"));
}

#[test]
fn flags_that_contradict_each_other_are_usage_errors() {
    let check = |line: &str| {
        let cli = args::Cli::try_parse_from(line.split(' ')).unwrap();
        cli.command.validate().map_err(|err| err.code)
    };
    for line in [
        "acyclic item edit a --notes x --no-notes",
        "acyclic item edit a --parent b --no-parent",
        "acyclic item edit a --thread t --no-thread",
        "acyclic item edit a --size m --no-size",
        "acyclic item edit a --gate g --no-gate",
        "acyclic item edit a --area x --no-area x",
        "acyclic item edit a --repository r --no-repository r",
        "acyclic thread edit t --brief b --no-brief",
        "acyclic thread edit t --level blockers --no-level",
        "acyclic thread edit t --repository r --no-repository r",
        "acyclic repo edit r --public --private",
        "acyclic repo add r --owner o --name n --default-branch main",
        "acyclic repo add r --owner o --name n --default-branch main --public --private",
    ] {
        assert_eq!(check(line), Err("usage"), "{line}");
    }
    for line in [
        "acyclic item edit a --area x --no-area y --no-notes --parent b",
        "acyclic repo add r --owner o --name n --default-branch main --private",
        "acyclic repo edit r --public",
    ] {
        assert_eq!(check(line), Ok(()), "{line}");
    }
}

#[test]
fn a_refusal_whose_number_was_spent_meanwhile_is_a_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env { cwd: dir.path().to_path_buf(), ..env(&[("ACYCLIC_NOW", "1792022400000")]) };
    let init = execute(["acyclic", "init", "tracker", "--replica", "mac"].map(Into::into), &env);
    assert_eq!(init.code, 0, "{}", init.stdout);
    let tracker = dir.path().join("tracker");
    let cli = |command| Cli {
        tracker: Some(tracker.clone()),
        actor: Some("run/r1".into()),
        as_user: false,
        output: Some(args::Format::Json),
        command,
    };
    let op = commands::Operation {
        run: "r1".into(),
        thread: "t".into(),
        seq: 1,
        seen: acyclic_core::state::Context::default(),
    };
    // The refused command's first batch found number 1 unspent; before its second, the same number was applied.
    let add = || -> Command {
        serde_json::from_value(json!({"thread": {"add": {"thread": "t", "title": "T"}}})).unwrap()
    };
    commands::apply(&cli(add()), &env, op.clone()).unwrap();
    let refusal = CliError::refused("refused", "");
    let err = commands::refuse(&cli(add()), &env, op, &refusal).unwrap_err();
    assert_eq!((err.code, err.details), (commands::DUPLICATE, json!({"ok": true})));
}
