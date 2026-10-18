//! The operator against a real tracker clone: it starts a run, forwards the run's writes as the run whatever the run
//! claims, applies each once, refuses what runs may not send, and stops forwarding once the run has ended.

use acyclic_cli::{Applied, Env, execute};
use acyclic_core::model::RunState;
use acyclic_core::state::{Key, State};
use acyclic_core::store::Tracker;
use acyclic_operator::forward::{Clone, Operator, Refused, Sent};
use acyclic_operator::token::Signer;
use serde_json::{Value, json};

/// 2026-10-15T00:00:00Z.
const NOW: i64 = 1_792_022_400_000;
const HOUR: i64 = 60 * 60 * 1000;

fn state(tracker: &std::path::Path) -> State {
    Tracker::open(tracker).unwrap().0.load().unwrap().1
}

fn sent(seq: u64, seen: &State, command: Value) -> Sent {
    serde_json::from_value(json!({"seq": seq, "seen": seen.context, "command": command})).unwrap()
}

#[test]
fn the_operator_forwards_a_runs_writes_once_as_the_run_until_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    // The environment names another actor, which a run's writes never take.
    let env = Env {
        vars: [
            ("ACYCLIC_NOW", NOW.to_string()),
            ("ACYCLIC_ACTOR", "session/other".into()),
            ("HOME", "/nonexistent".into()),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect(),
        cwd: dir.path().to_path_buf(),
        terminal: None,
    };
    let tracker = dir.path().join("tracker");
    let line = |line: &str| {
        let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
        let outcome = execute(args, &env);
        assert_eq!(outcome.code, 0, "{line}: {}", outcome.stdout);
    };
    line("init tracker --replica mac");
    line("--tracker tracker thread add t1 --title T --as-user");
    line("--tracker tracker item add a --title A --thread t1 --as-user");

    let operator = Operator::new(
        "mac",
        Signer::from_seed([7; 32]),
        Clone { tracker: tracker.clone(), env: env.clone() },
    );
    let (run, token) = operator.start("t1", NOW, HOUR).unwrap();
    let base = state(&tracker);
    let edit = json!({"item": {"edit": {"item": "a", "title": "From the run"}}});
    let forwarded = operator.forward(&token, vec![sent(1, &base, edit.clone())], NOW).unwrap();
    assert!(matches!(forwarded.applied[..], [Applied::Ran(0, _)]), "{forwarded:?}");
    let key = Key::new("item", "a").unwrap();
    let actors = |state: &State| -> Vec<String> {
        state.field(&key, "title").into_iter().flatten().map(|(_, v)| v.actor.clone()).collect()
    };
    assert_eq!(actors(&state(&tracker)), [format!("run/{run}")]);

    // Sent again, nothing changes; a command runs may not send is refused and spent.
    let before = state(&tracker);
    let approve = json!({"thread": {"approve": {"thread": "t1"}}});
    let forwarded =
        operator.forward(&token, vec![sent(1, &base, edit), sent(2, &base, approve)], NOW).unwrap();
    assert_eq!(forwarded.applied[0], Applied::Duplicate(Some(json!({"ok": true}))));
    let Applied::Ran(1, refused) = &forwarded.applied[1] else { panic!("{forwarded:?}") };
    assert_eq!(refused["error"]["code"], "not-for-runs");
    assert_eq!(actors(&state(&tracker)), actors(&before));

    // Once the run has ended, its token forwards nothing, and its thread can start another.
    operator.end(&run, RunState::Done).unwrap();
    let late = json!({"done": {"item": "a", "note": "late"}});
    assert_eq!(
        operator.forward(&token, vec![sent(3, &base, late)], NOW),
        Err(Refused::Inactive { run })
    );
    operator.start("t1", NOW, HOUR).unwrap();
}
