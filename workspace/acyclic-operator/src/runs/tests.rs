use std::cell::RefCell;

use super::*;

const NOW: Millis = 1_792_022_400_000;
const HOUR: Millis = 60 * 60 * 1000;

/// A tracker that records what it was asked.
#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
}

impl Tracker for Fake {
    fn start(&self, operator: &str, run: &str, thread: &str) -> Result<(), Failure> {
        self.calls.borrow_mut().push(format!("start {operator} {run} {thread}"));
        Ok(())
    }

    fn set_state(&self, operator: &str, run: &str, state: RunState) -> Result<(), Failure> {
        self.calls.borrow_mut().push(format!("{state:?} {operator} {run}"));
        Ok(())
    }
}

#[test]
fn a_run_is_started_by_the_operator_and_given_a_token_for_itself() {
    let operator = Operator::new("mac", Signer::from_seed([5; 32]), Fake::default());
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
fn a_runs_git_sends_its_owners_repositories_by_any_url_to_the_server_and_others_to_the_forge() {
    let vars = environment("http://host.example:8/", "v1.token", &["o1".to_owned()].into());
    assert_eq!(vars["ACYCLIC_REMOTE"], "http://host.example:8");
    let url = |url: &str| {
        let output = std::process::Command::new("git")
            .args(["ls-remote", "--get-url", url])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .envs(&vars)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    for sent in
        ["https://github.com/o1/app", "git@github.com:o1/app", "ssh://git@github.com/o1/app"]
    {
        assert_eq!(url(sent), "http://host.example:8/git/o1/app", "{sent}");
    }
    assert_eq!(url("https://github.com/o2/app"), "https://github.com/o2/app");
}
