//! Starting and ending runs. Starting one records it in the tracker and mints it a token naming it and its thread;
//! the server checks that token on every request the run makes, and applies its writes as the run. Ending a run
//! revokes its token, whatever the token's own expiry, since the server refuses a run that isn't under way.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use acyclic_cli::Env;
use acyclic_core::model::RunState;
use acyclic_core::state::Millis;
use serde_json::Value;

use crate::token::{self, Signer, Verifier};

/// A failure the tracker reported: its exit code and envelope.
pub type Failure = (u8, Value);

/// What the operator needs of the tracker.
pub trait Tracker {
    /// Starts `run` on `thread` as `operator`.
    ///
    /// # Errors
    ///
    /// When the thread has a run under way, or the tracker fails.
    fn start(&self, operator: &str, run: &str, thread: &str) -> Result<(), Failure>;

    /// Moves `run` to `state` as `operator`.
    ///
    /// # Errors
    ///
    /// When the run can't move that way, or the tracker fails.
    fn set_state(&self, operator: &str, run: &str, state: RunState) -> Result<(), Failure>;
}

/// A tracker clone on this machine, written through the CLI's own code.
#[derive(Debug, Clone)]
pub struct Clone {
    /// The clone.
    pub tracker: PathBuf,
    /// The environment the CLI runs in.
    pub env: Env,
}

impl Tracker for Clone {
    fn start(&self, operator: &str, run: &str, thread: &str) -> Result<(), Failure> {
        acyclic_cli::start_run(operator, run, thread, &self.tracker, &self.env).map(|_| ())
    }

    fn set_state(&self, operator: &str, run: &str, state: RunState) -> Result<(), Failure> {
        acyclic_cli::set_run_state(operator, run, state, &self.tracker, &self.env).map(|_| ())
    }
}

/// Why a run couldn't be started or ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    /// The token couldn't be minted.
    Token(token::Error),
    /// The tracker refused or failed.
    Tracker(Failure),
}

/// The operator: it starts runs, minting each a token, and ends them.
pub struct Operator<T> {
    /// Its own actor, `operator/<id>`.
    name: String,
    signer: Signer,
    verifier: Verifier,
    tracker: T,
}

impl<T: Tracker> Operator<T> {
    /// An operator named `operator/<id>`, signing with `signer`, writing to `tracker`.
    #[must_use]
    pub fn new(id: &str, signer: Signer, tracker: T) -> Self {
        let verifier = signer.verifier();
        Self { name: format!("{}{id}", acyclic_core::scope::OPERATOR), signer, verifier, tracker }
    }

    /// What checks this operator's tokens.
    #[must_use]
    pub fn verifier(&self) -> &Verifier {
        &self.verifier
    }

    /// Starts a new run on `thread` and returns its id and its token, issued at `now` to live `life` milliseconds.
    ///
    /// # Errors
    ///
    /// When the thread has a run under way or the tracker fails, or the token can't be minted.
    pub fn start(
        &self,
        thread: &str,
        now: Millis,
        life: Millis,
    ) -> Result<(String, String), Refused> {
        let mut bytes = [0; 6];
        getrandom::fill(&mut bytes)
            .map_err(|err| Refused::Token(token::Error::Random(err.to_string())))?;
        let run = format!("r{}", token::hex(&bytes));
        let token = self.signer.mint(&run, thread, now, life).map_err(Refused::Token)?;
        self.tracker.start(&self.name, &run, thread).map_err(Refused::Tracker)?;
        Ok((run, token))
    }

    /// Ends `run` as `state`, done or stopped, which revokes its token.
    ///
    /// # Errors
    ///
    /// When the run can't move that way or the tracker fails.
    pub fn end(&self, run: &str, state: RunState) -> Result<(), Refused> {
        self.tracker.set_state(&self.name, run, state).map_err(Refused::Tracker)
    }
}

#[cfg(test)]
mod tests;

/// The environment a run works in, given the runners' listener at `server`, such as `http://<host>:<port>`, the run's
/// `token`, and the `owners` of the tracker's repository records: the CLI's remote mode, and git settings, as
/// `GIT_CONFIG_*` variables, that send every repository of those owners on the forge, by any of its URLs, to the
/// server with the run's token. Other repositories, such as public dependencies, go to the forge as they are, with no
/// credentials.
#[must_use]
pub fn environment(
    server: &str,
    token: &str,
    owners: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let server = server.trim_end_matches('/');
    let mut settings =
        vec![(format!("http.{server}/.extraHeader"), format!("Authorization: Bearer {token}"))];
    for owner in owners {
        for forge in ["https://github.com/", "git@github.com:", "ssh://git@github.com/"] {
            settings
                .push((format!("url.{server}/git/{owner}/.insteadOf"), format!("{forge}{owner}/")));
        }
    }
    let mut vars = BTreeMap::from([
        ("ACYCLIC_REMOTE".to_owned(), server.to_owned()),
        ("ACYCLIC_TOKEN".to_owned(), token.to_owned()),
        ("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned()),
        ("GIT_CONFIG_COUNT".to_owned(), settings.len().to_string()),
    ]);
    for (index, (key, value)) in settings.into_iter().enumerate() {
        vars.insert(format!("GIT_CONFIG_KEY_{index}"), key);
        vars.insert(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    vars
}
