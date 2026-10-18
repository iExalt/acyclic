//! Starting and ending runs. Starting one records it in the tracker and mints it a token naming it and its thread;
//! the server checks that token on every request the run makes, and applies its writes as the run. Ending a run
//! revokes its token, whatever the token's own expiry, since the server refuses a run that isn't under way.

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
