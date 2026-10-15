use std::fmt::Write as _;

use acyclic_core::archive::ArchiveError;
use acyclic_core::import::{ApplyError, ParseError};
use acyclic_core::locate::LocateError;
use acyclic_core::model::{RunState, WriteError};
use acyclic_core::scope::ScopeError;
use acyclic_core::state::{Error as StateError, Key, ReplicaId};
use acyclic_core::store::StoreError;

use super::*;

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
    ];
    let mut table = String::new();
    for error in errors {
        writeln!(table, "{} {}", error.exit, error.code).unwrap();
    }
    insta::assert_snapshot!(table);
}

#[test]
fn usage_errors_use_the_envelope_and_help_is_text() {
    let (code, out) = execute(["acyclic", "nonsense"].map(Into::into), &env(&[]));
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!((code, out["error"]["code"].as_str()), (exit::USAGE, Some("usage")));
    let (code, out) = execute(["acyclic", "--version"].map(Into::into), &env(&[]));
    assert_eq!(
        (code, out.trim()),
        (exit::OK, format!("acyclic {}", acyclic_core::VERSION).as_str())
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
    }
    assert!(!may_claim(None, me));
}

#[test]
fn prime_is_text_matched_to_the_version() {
    let (code, out) = execute(["acyclic", "prime"].map(Into::into), &env(&[]));
    assert_eq!(code, exit::OK);
    assert!(out.contains(&format!("acyclic {})", acyclic_core::VERSION)));
    insta::assert_snapshot!(out.replace(acyclic_core::VERSION, "[version]"));
}
