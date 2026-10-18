//! Starting runs and forwarding their operations to the tracker.
//!
//! A run sends its writes as operations: each one of the CLI's commands, numbered from 1, with the run's view of the
//! tracker when it made it. The operator checks the run's token first, then that the run is still under way, and
//! takes the run from the token alone: an operation names no run. It forwards only the commands a run may send
//! ([`Command::run_may_send`]); the tracker applies each as the run, through the CLI's own checks, once. Ending a run therefore
//! revokes its token, whatever the token's own expiry.
//!
//! What a run says it had seen is its own word: a run could claim to have seen everything and so supersede a value it
//! never saw. Keeping such values as conflicts protects against a stale run's mistakes, not against a hostile one.

use std::path::PathBuf;

use acyclic_cli::args::Command;
use acyclic_cli::error::CliError;
use acyclic_cli::{Applied, Env, Operation};
use acyclic_core::model::RunState;
use acyclic_core::state::{Context, Millis};
use serde::Deserialize;
use serde_json::Value;

use crate::token::{self, Signer, Verifier};

/// A failure the tracker reported: its exit code and envelope.
pub type Failure = (u8, Value);

/// One operation as a run sends it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sent {
    /// Its number in the run's sequence, from 1.
    pub seq: u64,
    /// What the run had seen when it made it.
    pub seen: Context,
    /// The command, as `POST /api/command` takes it.
    pub command: Command,
}

/// What the operator needs of the tracker.
pub trait Tracker {
    /// Whether `run` is under way on `thread`.
    ///
    /// # Errors
    ///
    /// When the tracker can't be read.
    fn active(&self, run: &str, thread: &str) -> Result<bool, Failure>;

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

    /// Applies an operation's command as its run, once.
    ///
    /// # Errors
    ///
    /// A failure that leaves the operation's number unspent.
    fn apply(&self, operation: Operation, command: Command) -> Result<Applied, Failure>;

    /// Spends an operation's number with `refusal` as its outcome, running nothing.
    ///
    /// # Errors
    ///
    /// A failure that leaves the operation's number unspent.
    fn refuse(&self, operation: Operation, refusal: &CliError) -> Result<Applied, Failure>;
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
    fn active(&self, run: &str, thread: &str) -> Result<bool, Failure> {
        acyclic_cli::run_active(run, thread, &self.tracker)
    }

    fn start(&self, operator: &str, run: &str, thread: &str) -> Result<(), Failure> {
        acyclic_cli::start_run(operator, run, thread, &self.tracker, &self.env).map(|_| ())
    }

    fn set_state(&self, operator: &str, run: &str, state: RunState) -> Result<(), Failure> {
        acyclic_cli::set_run_state(operator, run, state, &self.tracker, &self.env).map(|_| ())
    }

    fn apply(&self, operation: Operation, command: Command) -> Result<Applied, Failure> {
        acyclic_cli::apply(operation, command, self.tracker.clone(), &self.env)
    }

    fn refuse(&self, operation: Operation, refusal: &CliError) -> Result<Applied, Failure> {
        acyclic_cli::refuse(operation, refusal, self.tracker.clone(), &self.env)
    }
}

/// Why a run's operations weren't forwarded at all.
#[derive(Debug, Clone, PartialEq)]
pub enum Refused {
    /// The token was refused.
    Token(token::Error),
    /// The token's run isn't under way on its thread: it ended, or never started.
    Inactive {
        /// The run.
        run: String,
    },
    /// The tracker failed before anything was forwarded.
    Tracker(Failure),
}

/// What forwarding a run's operations did: each one's result, in order, until the first failure, which leaves that
/// operation and those after it unspent for the run to send again.
#[derive(Debug, Clone, PartialEq)]
pub struct Forwarded {
    /// The operations applied, or refused and spent, or found already applied.
    pub applied: Vec<Applied>,
    /// The failure that stopped forwarding, if one did.
    pub failed: Option<Failure>,
}

/// The operator: it starts runs, minting each a token, and forwards their operations.
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

    /// Forwards a run's operations, sent with `token`, at `now`: each as the token's run, in order, stopping at the
    /// first failure.
    ///
    /// # Errors
    ///
    /// When the token is refused, its run isn't under way, or the tracker can't say whether it is.
    pub fn forward(&self, token: &str, sent: Vec<Sent>, now: Millis) -> Result<Forwarded, Refused> {
        let claims = self.verifier.verify(token, now).map_err(Refused::Token)?;
        if !self.tracker.active(&claims.run, &claims.thread).map_err(Refused::Tracker)? {
            return Err(Refused::Inactive { run: claims.run });
        }
        let mut forwarded = Forwarded { applied: Vec::new(), failed: None };
        for Sent { seq, seen, command } in sent {
            let operation =
                Operation { run: claims.run.clone(), thread: claims.thread.clone(), seq, seen };
            let result = if command.run_may_send() {
                self.tracker.apply(operation, command)
            } else {
                let refusal = CliError::refused("not-for-runs", "a run may not send this command");
                self.tracker.refuse(operation, &refusal)
            };
            match result {
                Ok(applied) => forwarded.applied.push(applied),
                Err(failure) => {
                    forwarded.failed = Some(failure);
                    break;
                },
            }
        }
        Ok(forwarded)
    }
}

#[cfg(test)]
mod tests;
