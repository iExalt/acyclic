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
    // A bare command or group prints its help.
    assert_eq!(run("acyclic"), help);
    let out = run("acyclic item");
    assert_eq!((out.code, out.stdout.starts_with("Adds, edits")), (exit::OK, true), "{out:?}");
}

#[test]
fn the_help_sections_hold_every_command_once() {
    let mut named: Vec<&str> =
        args::SECTIONS.iter().flat_map(|(_, names)| names.iter().copied()).collect();
    named.sort_unstable();
    let command = args::command();
    let mut commands: Vec<&str> = command.get_subcommands().map(clap::Command::get_name).collect();
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
