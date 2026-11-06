use std::fmt::Write as _;

use acyclic_core::archive::ArchiveError;
use acyclic_core::delete::DeleteError;
use acyclic_core::import::{ApplyError, ParseError, ValidationError};
use acyclic_core::locate::LocateError;
use acyclic_core::model::WriteError;
use acyclic_core::state::{Error as StateError, Key, ReplicaId};
use acyclic_core::store::StoreError;

use clap::Parser;

use super::*;
use crate::error::CliError;

mod contract;

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
fn the_actor_is_the_flag_else_the_environment_else_the_user() {
    let session = [("ACYCLIC_ACTOR", "session/mac")];
    assert_eq!(actor("acyclic needs", &[]), Ok("actor/user".into()));
    assert_eq!(actor("acyclic needs", &session), Ok("session/mac".into()));
    assert_eq!(actor("acyclic needs --actor session/pi", &session), Ok("session/pi".into()));
    assert_eq!(actor("acyclic needs --actor nobody", &[]), Err("invalid-actor"));
    assert_eq!(actor("acyclic needs", &[("ACYCLIC_ACTOR", "no one")]), Err("invalid-actor"));
}

#[test]
fn output_is_pretty_at_a_terminal_and_json_in_a_pipe_unless_said_otherwise() {
    use args::Format::{Json, Pretty};
    let output = |line: &str, terminal: Option<u16>| {
        let cli = args::Cli::try_parse_from(line.split(' ')).unwrap();
        context::output(&cli, &Env { terminal, ..env(&[]) })
    };
    let (tty, pipe) = (Some(80), None);
    // An agent's tool reads a pipe, so agents get JSON without asking.
    assert_eq!(output("acyclic item list", pipe), Json);
    assert_eq!(output("acyclic item list", tty), Pretty);
    // The flag overrides the terminal.
    assert_eq!(output("acyclic item list -o json", tty), Json);
    assert_eq!(output("acyclic item list -o pretty", pipe), Pretty);
    // At a terminal, an error is pretty's line on standard error.
    let args = ["acyclic", "item", "show", "a1", "--tracker", "/nonexistent"].map(Into::into);
    let out = execute(args, &Env { terminal: tty, ..env(&[]) });
    assert!(out.stdout.is_empty() && out.stderr.starts_with("error: "), "{out:?}");
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
    let cli = args::Cli::try_parse_from(["acyclic", "item", "list"]).unwrap();
    // No HOME, so no mapping could be read.
    let found = context::tracker(&cli, &Env { cwd: inside, ..Env::default() }).unwrap();
    assert_eq!(found.canonicalize().unwrap(), dir.path().join("clone").canonicalize().unwrap());
}

#[test]
fn an_internal_import_error_keeps_each_violation_in_its_details() {
    let violations = vec![
        acyclic_core::write::Violation {
            change: None,
            at: None,
            rule: acyclic_core::write::Rule::LinkCycle(vec!["a".into(), "b".into()]),
        },
        acyclic_core::write::Violation {
            change: None,
            at: None,
            rule: acyclic_core::write::Rule::EmptyRecord(acyclic_core::write::Record::Thread(
                "t".into(),
            )),
        },
    ];
    let expected: Vec<String> = violations.iter().map(ToString::to_string).collect();
    let error: CliError = ApplyError::Internal(violations).into();
    assert_eq!(error.code, "internal");
    assert!(error.message.contains("internal error"));
    assert_eq!(error.details["violations"], json!(expected));
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
        StateError::IdReused { replica: id(), mine: 1, theirs: 2 }.into(),
        StateError::Claimed(key.clone()).into(),
        StateError::NoThread("t".into()).into(),
        StateError::Format("bad".into()).into(),
        StateError::Version(2).into(),
        WriteError::InvalidName("A".into()).into(),
        WriteError::NoItem("a1".into()).into(),
        WriteError::Deleted("a1".into()).into(),
        WriteError::ParentCycle("a1".into()).into(),
        WriteError::LinkCycle(vec!["a1".into()]).into(),
        WriteError::SelfLink("a1".into()).into(),
        DeleteError::InUse { item: "a1".into(), reasons: vec![] }.into(),
        ArchiveError::Ineligible { item: "a1".into(), reasons: vec![] }.into(),
        ArchiveError::NotDone("a1".into()).into(),
        StoreError::Git("failed".into()).into(),
        StoreError::Conflict.into(),
        StoreError::Duplicate(id()).into(),
        StoreError::Locked { path: "x".into(), age_seconds: None }.into(),
        StoreError::Config("no replica".into()).into(),
        StoreError::RemoteMoved.into(),
        StoreError::Rejected { reference: "r".into(), reason: "no".into() }.into(),
        StoreError::ObjectsRefused("error: bad".into()).into(),
        LocateError::Config("bad".into()).into(),
        LocateError::NoRemote.into(),
        LocateError::Git("failed".into()).into(),
        LocateError::NotMapped(vec![]).into(),
        ParseError::NotAnImport.into(),
        ParseError::UnsupportedVersion { found: 9 }.into(),
        ApplyError::Invalid(vec![]).into(),
        ApplyError::Invalid(vec![ValidationError::InvalidId {
            at: "item id".into(),
            id: "a1".into(),
        }])
        .into(),
        ApplyError::Cycles(vec![]).into(),
        ApplyError::Internal(vec![]).into(),
        DeleteError::ThreadInUse { thread: "t".into(), reasons: vec![] }.into(),
        DeleteError::NoThread("t".into()).into(),
        // Raised by the commands themselves.
        CliError::refused("not-ready", ""),
        CliError::refused("changed-since", ""),
        CliError::refused("already-exists", ""),
        CliError::refused("already-done", ""),
        CliError::refused("incomplete-repository", ""),
        CliError::refused("nested-repository", ""),
        CliError::new("not-found", exit::NOT_FOUND, ""),
        CliError::new("usage", exit::USAGE, ""),
        CliError::new("invalid-commit", exit::USAGE, ""),
        CliError::new("invalid-id", exit::USAGE, ""),
        CliError::new("ambiguous", exit::USAGE, ""),
        CliError::new("invalid-actor", exit::USAGE, ""),
        CliError::new("config", exit::CONFIG, ""),
        CliError::new("no-remote", exit::CONFIG, ""),
        CliError::new("no-server", exit::CONFIG, ""),
    ];
    let mut table = String::new();
    for error in &errors {
        writeln!(table, "{} {}", error.exit, error.code).unwrap();
    }
    insta::assert_snapshot!(table);
    // Every code a command can raise is in the table above, so the next one added there must be added here.
    let known: std::collections::BTreeSet<&str> = errors.iter().map(|error| error.code).collect();
    let mut scanned = 0;
    for file in
        ["commands.rs", "context.rs", "edit.rs", "lib.rs", "args.rs", "resolve.rs", "error.rs"]
    {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file),
        )
        .unwrap();
        let tests = source.find("#[cfg(test)]").unwrap_or(source.len());
        // error.rs builds its errors as `Self`.
        let calls: &[&str] = if file == "error.rs" {
            &["Self::new(", "Self::refused("]
        } else {
            &["CliError::new(", "CliError::refused("]
        };
        for call in calls {
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
    for line in ["acyclic nonsense", "acyclic item show", "acyclic item edit a1 --state started"] {
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
    // The top level holds the groups, then the commands that act on the tracker or the tool.
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
fn prime_is_text_matched_to_the_version() {
    let Outcome { code, stdout: out, .. } = run("acyclic prime");
    assert_eq!(code, exit::OK);
    assert!(out.contains(&format!("acyclic {})", acyclic_core::VERSION)));
}

#[test]
fn a_thread_lists_its_members_in_run_order_and_a_raced_member_in_both_threads() {
    use acyclic_core::model::{By, Model, add_item, link, set};
    use acyclic_core::schema::{kind, thread};
    use acyclic_core::scope::{ScopeEdit, USER, edit};
    use acyclic_core::state::{Key, Replica, State};
    let user = By { actor: USER, at: 1 };
    let mut mac = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    for id in ["t", "u", "v"] {
        set(&mut mac, &Key::new(kind::THREAD, id).unwrap(), thread::TITLE, Some(&id), user)
            .unwrap();
    }
    let join = |replica: &mut Replica, item: &str, thread: &str| {
        let member = ScopeEdit::Member { item: item.to_owned(), thread: Some(thread.to_owned()) };
        edit(replica, &member, user).unwrap();
    };
    for id in ["s-1", "s-2", "s-10", "c-1", "c-2", "raced", "cleared"] {
        add_item(&mut mac, id, id, user).unwrap();
        join(&mut mac, id, "t");
    }
    // By id, s-10 would come before s-2; its links run it last.
    link(&mut mac, "s-2", "s-1", user).unwrap();
    link(&mut mac, "s-10", "s-2", user).unwrap();
    // Each replica closes half of a cycle, and each puts the raced item in another thread.
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    link(&mut mac, "c-1", "c-2", user).unwrap();
    link(&mut pi, "c-2", "c-1", user).unwrap();
    join(&mut mac, "raced", "u");
    join(&mut pi, "raced", "v");
    // One replica moves it on while the other takes it out of every thread.
    join(&mut mac, "cleared", "u");
    edit(&mut pi, &ScopeEdit::Member { item: "cleared".to_owned(), thread: None }, user).unwrap();
    mac.join(pi.state()).unwrap();
    // A thread an item names but that has no record is no thread: the item's reference dangles.
    add_item(&mut mac, "ghostly", "ghostly", user).unwrap();
    join(&mut mac, "ghostly", "w");
    let model = Model::read(mac.state()).unwrap();
    let threads = commands::thread_members(&model);
    assert!(!threads.contains_key("w"));
    let listed = |thread: &str| -> Vec<(&str, bool)> {
        threads[thread].iter().map(|m| (m.item, m.conflict)).collect()
    };
    assert_eq!(
        listed("t"),
        [("s-1", false), ("s-2", false), ("s-10", false), ("c-1", false), ("c-2", false)]
    );
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
            "import: dry_run",
            "import: file",
            "import: schema",
            "item show: items",
            "repository show: repositories",
            "thread show: threads",
        ]
    );
    let refused = |value: Value| {
        let command: args::Command = serde_json::from_value(value).unwrap();
        command.validate().map_err(|err| err.code)
    };
    assert_eq!(refused(json!({"item": {"show": {"items": []}}})), Err("usage"));
    assert_eq!(refused(json!({"thread": {"show": {"threads": []}}})), Err("usage"));
    assert_eq!(refused(json!({"import": {}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"file": "p", "schema": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true, "dry-run": true}})), Err("usage"));
    assert_eq!(refused(json!({"import": {"schema": true}})), Ok(()));
    assert_eq!(refused(json!({"import": {"file": "p", "dry-run": true}})), Ok(()));
    assert_eq!(refused(json!({"repository": {"show": {"repositories": []}}})), Err("usage"));
    for lease in [0, 481] {
        assert_eq!(
            refused(json!({"thread": {"claim": {"thread": "t", "lease": lease}}})),
            Err("usage")
        );
    }
    for lease in [1, 480] {
        assert_eq!(refused(json!({"thread": {"claim": {"thread": "t", "lease": lease}}})), Ok(()));
    }
    for count in [0, 1001] {
        assert_eq!(refused(json!({"id": {"count": count}})), Err("usage"), "{count}");
    }
    assert_eq!(refused(json!({"id": {}})), Ok(()));
    for state in ["backlog", "parked"] {
        assert!(
            serde_json::from_value::<args::Command>(json!({
                "item": {"add": {"title": "T", "state": state}}
            }))
            .is_ok(),
            "{state}"
        );
        assert!(
            serde_json::from_value::<args::Command>(json!({
                "item": {"edit": {"item": "a", "state": state}}
            }))
            .is_ok(),
            "{state}"
        );
    }
    for state in ["done", "in-progress", "unknown"] {
        assert!(
            serde_json::from_value::<args::Command>(json!({
                "item": {"add": {"title": "T", "state": state}}
            }))
            .is_err(),
            "{state}"
        );
        assert!(
            serde_json::from_value::<args::Command>(json!({
                "item": {"edit": {"item": "a", "state": state}}
            }))
            .is_err(),
            "{state}"
        );
    }
    // Unknown fields and the server's own command are refused when deserializing.
    assert!(
        serde_json::from_value::<args::Command>(
            json!({"item": {"show": {"items": ["a"], "force": true}}})
        )
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
fn a_lease_outside_its_range_is_a_usage_error_naming_the_range() {
    for lease in ["0", "481", "999"] {
        let err = args::Cli::try_parse_from(["acyclic", "thread", "claim", "a", "--lease", lease])
            .unwrap_err();
        assert!(err.to_string().contains("--lease takes 1 to 480 minutes"), "{err}");
        assert!(!err.to_string().contains("1..="), "{err}");
    }
    assert!(
        args::Cli::try_parse_from(["acyclic", "thread", "claim", "a", "--lease", "480"]).is_ok()
    );
    // The help text says the same bound.
    let help = run("acyclic help thread claim").stdout;
    assert!(help.contains(&format!("1 to {}", args::LEASE_MAX_MINUTES)), "{help}");
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
    let exceptions =
        [("init", "path"), ("completions", "shell"), ("import", "file"), ("id", "count")];
    let entities = ["ITEM", "THREAD", "REPOSITORY"];
    // A long flag has the same short form, or none, in every command that takes it.
    let mut shorts = std::collections::BTreeMap::new();
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
            if let Some(long) = arg.get_long().filter(|_| !arg.is_global_set()) {
                let (short, first) = shorts.entry(long).or_insert((arg.get_short(), path.as_str()));
                assert_eq!(*short, arg.get_short(), "--{long} in {path} and in {first}");
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
        ("item", &["add", "edit", "list", "show"][..]),
        ("thread", &["add", "edit", "list", "show"]),
        ("repository", &["add", "edit", "list", "show"]),
    ] {
        for verb in verbs {
            assert!(group(name).contains(verb), "{name} {verb}");
        }
    }
}

#[test]
fn a_removed_or_moved_command_is_a_usage_error_at_its_old_place() {
    for line in ["acyclic graph", "acyclic thread set t1 --title T", "acyclic thread renumber t1"] {
        let out = run(line);
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{line}");
        assert!(out.stderr.starts_with("error: unrecognized subcommand"), "{out:?}");
    }
    // Each command that acts on one kind of record lives in its group alone, with no alias at the top.
    for (group, line) in [
        ("item", "show a"),
        ("item", "list"),
        ("item", "done a -e shipped"),
        ("thread", "ready"),
        ("thread", "claim t"),
        ("thread", "release t"),
        ("repository", "list"),
    ] {
        let top = format!("acyclic {line}");
        assert!(args::Cli::try_parse_from(top.split(' ')).is_err(), "{top}");
        let grouped = format!("acyclic {group} {line}");
        assert!(args::Cli::try_parse_from(grouped.split(' ')).is_ok(), "{grouped}");
    }
    assert!(args::Cli::try_parse_from(["acyclic", "repo", "list"]).is_err());
}

#[test]
fn removed_fields_the_link_group_and_the_in_progress_state_are_refused_as_unknown() {
    for line in [
        "acyclic item add -t T --area x",
        "acyclic item add -t T --resource x",
        "acyclic item add -t T --gate x",
        "acyclic item edit a --no-gate",
        "acyclic link add a waits-for b",
        "acyclic item edit a --state in-progress",
    ] {
        let out = run(line);
        assert_eq!((out.code, out.stdout.as_str()), (exit::USAGE, ""), "{line}");
        let unknown = ["unexpected argument", "unrecognized subcommand", "invalid value"];
        assert!(unknown.iter().any(|said| out.stderr.contains(said)), "{line}: {out:?}");
    }
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
        "acyclic item edit a --waits-for b --no-waits-for b",
        "acyclic item edit a --repository r --no-repository r",
        "acyclic thread edit t --brief b --no-brief",
        "acyclic thread edit t --repository r --no-repository r",
    ] {
        assert_eq!(check(line), Err("usage"), "{line}");
    }
    for line in [
        "acyclic item edit a --waits-for b --no-waits-for c --no-notes --parent b",
        "acyclic repository add --name o/n --default-branch main",
        "acyclic repository edit r --default-branch trunk",
    ] {
        assert_eq!(check(line), Ok(()), "{line}");
    }
}

#[test]
fn readiness_checks_a_repository_against_its_mapped_clone_alone() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().to_str().unwrap();
    let one = |text: &str| acyclic_core::model::Value::One(text.to_owned());
    let record = |owner: &str, name: &str| acyclic_core::model::Repository {
        id: "01900000-0000-7000-8000-000000000001".to_owned(),
        owner: one(owner),
        name: one(name),
        default_branch: acyclic_core::model::Value::None,
        installation: acyclic_core::model::Value::None,
    };
    let clones = temp.path().join(".config/acyclic/clones.toml");
    let missing = context::clones(&env(&[("HOME", home)])).unwrap();
    assert_eq!(missing(&record("me", "app")), None);
    // A mirror where older servers kept one is no longer read.
    std::fs::create_dir_all(temp.path().join(".local/share/acyclic/mirrors/app.git")).unwrap();
    std::fs::create_dir_all(temp.path().join(".config/acyclic")).unwrap();
    std::fs::write(&clones, "").unwrap();
    let empty = context::clones(&env(&[("HOME", home)])).unwrap();
    assert_eq!(empty(&record("me", "app")), None);

    std::fs::write(&clones, "[clones]\n\"me/Mapped\" = \"/code/mapped\"\n").unwrap();
    let clone_of = context::clones(&env(&[("HOME", home)])).unwrap();
    assert_eq!(clone_of(&record("me", "app")), None);
    // Keyed by owner/name, in any case.
    assert_eq!(clone_of(&record("Me", "mapped")), Some(PathBuf::from("/code/mapped")));
    // A key that isn't an owner/name, as a record's id once was, is refused rather than left unchecked.
    std::fs::write(&clones, "[clones]\nmapped = \"/code/mapped\"\n").unwrap();
    let Err(refused) = context::clones(&env(&[("HOME", home)])) else {
        panic!("an id key accepted")
    };
    assert_eq!(refused.code, "config");
    assert!(
        refused.message.contains("clones.toml") && refused.message.contains("owner/name"),
        "{refused}"
    );

    std::fs::write(&clones, "[clones\n").unwrap();
    let Err(parse_error) = context::clones(&env(&[("HOME", home)])) else {
        panic!("malformed TOML accepted")
    };
    assert_eq!((parse_error.code, parse_error.exit), ("config", exit::CONFIG));
    std::fs::remove_file(&clones).unwrap();
    std::fs::create_dir(&clones).unwrap();
    let Err(read_error) = context::clones(&env(&[("HOME", home)])) else {
        panic!("an unreadable clones.toml was treated as empty")
    };
    assert_eq!((read_error.code, read_error.exit), ("config", exit::CONFIG));
    assert!(read_error.message.contains("clones.toml"), "{read_error}");
    assert_ne!(read_error.message, parse_error.message, "read and parse failures stay distinct");
}

#[test]
fn id_prints_new_v7s_one_per_line_outside_any_tracker() {
    let out = run("acyclic id 3");
    assert_eq!(out.code, exit::OK);
    let ids: std::collections::BTreeSet<&str> = out.stdout.lines().collect();
    assert_eq!(ids.len(), 3, "{out:?}");
    assert!(ids.iter().all(|id| acyclic_core::ids::v7(id).as_deref() == Some(*id)), "{out:?}");
    assert_eq!(run("acyclic id").stdout.lines().count(), 1);
    for count in ["0", "1001"] {
        let out = run(&format!("acyclic id {count}"));
        assert_eq!(out.code, exit::USAGE, "{count}");
        assert!(out.stderr.contains("1 to 1000"), "{out:?}");
    }
}
