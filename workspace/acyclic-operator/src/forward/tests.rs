use std::cell::RefCell;

use serde_json::json;

use super::*;

const NOW: Millis = 1_792_022_400_000;
const HOUR: Millis = 60 * 60 * 1000;

/// A tracker that records what it was asked, with run `r1` under way on `t1`, and fails an operation's number if
/// told to.
#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    fail_at: Option<u64>,
}

impl Tracker for Fake {
    fn active(&self, run: &str, thread: &str) -> Result<bool, Failure> {
        Ok(run == "r1" && thread == "t1")
    }

    fn start(&self, operator: &str, run: &str, thread: &str) -> Result<(), Failure> {
        self.calls.borrow_mut().push(format!("start {operator} {run} {thread}"));
        Ok(())
    }

    fn set_state(&self, operator: &str, run: &str, state: RunState) -> Result<(), Failure> {
        self.calls.borrow_mut().push(format!("{state:?} {operator} {run}"));
        Ok(())
    }

    fn apply(&self, operation: Operation, _: Command) -> Result<Applied, Failure> {
        if self.fail_at == Some(operation.seq) {
            return Err((4, json!({"ok": false})));
        }
        self.calls.borrow_mut().push(format!("apply {} {}", operation.run, operation.seq));
        Ok(Applied::Ran(0, json!({"ok": true})))
    }

    fn refuse(&self, operation: Operation, refusal: &CliError) -> Result<Applied, Failure> {
        self.calls
            .borrow_mut()
            .push(format!("refuse {} {} {}", operation.run, operation.seq, refusal.code));
        Ok(Applied::Ran(refusal.exit, refusal.envelope()))
    }
}

fn operator(fake: Fake) -> Operator<Fake> {
    Operator::new("mac", Signer::from_seed([5; 32]), fake)
}

fn sent(seq: u64, command: &Value) -> Sent {
    serde_json::from_value(json!({"seq": seq, "seen": Context::default(), "command": command}))
        .unwrap()
}

fn edit() -> Value {
    json!({"item": {"edit": {"item": "a", "title": "T"}}})
}

#[test]
fn a_run_is_started_by_the_operator_and_given_a_token_for_itself() {
    let operator = operator(Fake::default());
    let (run, token) = operator.start("t1", NOW, HOUR).unwrap();
    assert!(run.starts_with('r') && run.len() == 13, "{run}");
    let claims = operator.verifier().verify(&token, NOW).unwrap();
    assert_eq!((claims.run.as_str(), claims.thread.as_str()), (run.as_str(), "t1"));
    operator.end(&run, RunState::Done).unwrap();
    assert_eq!(
        *operator.tracker.calls.borrow(),
        [format!("start operator/mac {run} t1"), format!("Done operator/mac {run}")]
    );
}

#[test]
fn operations_are_the_tokens_runs_and_only_a_runs_own_commands_run() {
    let operator = operator(Fake::default());
    let token = operator.signer.mint("r1", "t1", NOW, HOUR).unwrap();
    let approve = json!({"thread": {"approve": {"thread": "t1"}}});
    let forwarded =
        operator.forward(&token, vec![sent(1, &edit()), sent(2, &approve)], NOW).unwrap();
    assert_eq!(forwarded.applied.len(), 2);
    assert_eq!(forwarded.failed, None);
    assert_eq!(*operator.tracker.calls.borrow(), ["apply r1 1", "refuse r1 2 not-for-runs"]);
}

#[test]
fn forwarding_stops_at_the_first_failure() {
    let operator = operator(Fake { fail_at: Some(2), ..Fake::default() });
    let token = operator.signer.mint("r1", "t1", NOW, HOUR).unwrap();
    let forwarded =
        operator.forward(&token, (1..=3).map(|seq| sent(seq, &edit())).collect(), NOW).unwrap();
    assert_eq!(forwarded.applied.len(), 1);
    assert_eq!(forwarded.failed, Some((4, json!({"ok": false}))));
    assert_eq!(*operator.tracker.calls.borrow(), ["apply r1 1"]);
}

#[test]
fn nothing_is_forwarded_without_a_valid_token_for_a_run_under_way() {
    let operator = operator(Fake::default());
    let expired = operator.signer.mint("r1", "t1", NOW - 2 * HOUR, HOUR).unwrap();
    let foreign = Signer::from_seed([6; 32]).mint("r1", "t1", NOW, HOUR).unwrap();
    let ended = operator.signer.mint("r2", "t1", NOW, HOUR).unwrap();
    let elsewhere = operator.signer.mint("r1", "t2", NOW, HOUR).unwrap();
    assert_eq!(
        operator.forward(&expired, vec![sent(1, &edit())], NOW),
        Err(Refused::Token(token::Error::Expired))
    );
    assert_eq!(
        operator.forward(&foreign, vec![sent(1, &edit())], NOW),
        Err(Refused::Token(token::Error::Signature))
    );
    assert_eq!(
        operator.forward(&ended, vec![sent(1, &edit())], NOW),
        Err(Refused::Inactive { run: "r2".into() })
    );
    assert_eq!(
        operator.forward(&elsewhere, vec![sent(1, &edit())], NOW),
        Err(Refused::Inactive { run: "r1".into() })
    );
    assert!(operator.tracker.calls.borrow().is_empty());
}

#[test]
fn an_operation_names_no_run() {
    let named = json!({"seq": 1, "seen": Context::default(), "command": edit(), "run": "r9"});
    assert!(serde_json::from_value::<Sent>(named).is_err());
}

#[test]
fn a_run_sends_only_the_writes_of_its_own_work() {
    let command = |value: Value| serde_json::from_value::<Command>(value).unwrap();
    for ok in [
        json!({"claim": {"thread": "t"}}),
        json!({"release": {"thread": "t"}}),
        json!({"done": {"item": "a", "note": "n"}}),
        json!({"item": {"add": {"item": "a", "title": "A"}}}),
        edit(),
        json!({"item": {"reopen": {"item": "a"}}}),
        json!({"link": {"add": {"from": "a", "kind": "waits-for", "to": "b"}}}),
        json!({"link": {"remove": {"from": "a", "kind": "waits-for", "to": "b"}}}),
    ] {
        assert!(allowed(&command(ok.clone())), "{ok}");
    }
    for refused in [
        json!("list"),
        json!({"show": {"items": ["a"]}}),
        json!({"level": {"level": "blockers"}}),
        json!({"item": {"archive": {"item": "a"}}}),
        json!({"item": {"delete": {"item": "a"}}}),
        json!({"thread": {"add": {"thread": "t", "title": "T"}}}),
        json!({"thread": {"approve": {"thread": "t"}}}),
        json!({"repo": {"add": {"repository": "r", "owner": "o", "name": "n", "default-branch": "main", "public": true}}}),
        json!({"proposal": {"keep": {"proposal": "p"}}}),
        json!({"sync": {}}),
        json!({"import": {"file": "/tmp/x"}}),
    ] {
        assert!(!allowed(&command(refused.clone())), "{refused}");
    }
}
