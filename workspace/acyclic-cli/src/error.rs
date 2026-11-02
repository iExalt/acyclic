//! Errors as the contract reports them: a stable kebab-case code, an exit code and a message.

use std::fmt;

use acyclic_core::archive::ArchiveError;
use acyclic_core::delete::DeleteError;
use acyclic_core::import::{ApplyError, ParseError};
use acyclic_core::locate::LocateError;
use acyclic_core::model::WriteError;
use acyclic_core::renumber::RenumberError;
use acyclic_core::scope::ScopeError;
use acyclic_core::state::Error as StateError;
use acyclic_core::store::{BatchError, StoreError};
use serde_json::{Value as Json, json};

/// Exit codes. Stable: agents branch on them.
pub mod exit {
    /// Success.
    pub const OK: u8 = 0;
    /// The tracker refused the request.
    pub const REFUSED: u8 = 1;
    /// The command line is wrong.
    pub const USAGE: u8 = 2;
    /// Something named doesn't exist.
    pub const NOT_FOUND: u8 = 3;
    /// The store failed: a lock, lost races, or git.
    pub const STORE: u8 = 4;
    /// The tracker or replica isn't set up.
    pub const CONFIG: u8 = 5;
    /// The state can't be read; `acyclic doctor` explains it.
    pub const UNREADABLE: u8 = 6;
}

/// A failed command.
#[derive(Debug, Clone, PartialEq)]
pub struct CliError {
    /// The stable code.
    pub code: &'static str,
    /// The exit code.
    pub exit: u8,
    /// What happened.
    pub message: String,
    /// Anything structured that helps, or `null`.
    pub details: Json,
}

impl CliError {
    /// An error with no details.
    pub fn new(code: &'static str, exit: u8, message: impl Into<String>) -> Self {
        Self { code, exit, message: message.into(), details: Json::Null }
    }

    /// Adds details.
    #[must_use]
    pub fn with(mut self, details: Json) -> Self {
        self.details = details;
        self
    }

    /// A refusal.
    pub fn refused(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, exit::REFUSED, message)
    }

    /// The envelope printed for this error.
    #[must_use]
    pub fn envelope(&self) -> Json {
        json!({"ok": false, "error": {"code": self.code, "message": self.message, "details": self.details}})
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<StateError> for CliError {
    fn from(err: StateError) -> Self {
        let message = err.to_string();
        let (code, exit) = match &err {
            StateError::InvalidName(_) => ("invalid-name", exit::USAGE),
            StateError::InvalidValue => ("invalid-value", exit::USAGE),
            StateError::ClockJump { .. } => ("clock-jump", exit::REFUSED),
            StateError::NotContinuous(_) => ("not-continuous", exit::REFUSED),
            StateError::IdReused { .. } => ("id-reused", exit::REFUSED),
            StateError::Claimed(_) => ("claimed", exit::REFUSED),
            StateError::NoThread(_) => ("not-found", exit::NOT_FOUND),
            StateError::ResourceHeld { .. } => ("resource-held", exit::REFUSED),
            StateError::Format(_) => ("unreadable", exit::UNREADABLE),
        };
        let mut error = Self::new(code, exit, message);
        if code == "unreadable" {
            error = error.with(json!({"hint": "run acyclic doctor"}));
        }
        error
    }
}

impl From<WriteError> for CliError {
    fn from(err: WriteError) -> Self {
        let message = err.to_string();
        match err {
            WriteError::State(err) => err.into(),
            WriteError::InvalidName(_) => Self::new("invalid-name", exit::USAGE, message),
            WriteError::NoItem(_) => Self::new("not-found", exit::NOT_FOUND, message),
            WriteError::Deleted(_) => Self::refused("deleted", message),
            WriteError::ParentCycle(_) => Self::refused("parent-cycle", message),
            WriteError::LinkCycle(path) => {
                Self::refused("link-cycle", message).with(json!({"items": path}))
            },
            WriteError::Checked { .. } => Self::refused("checked-field", message),
            WriteError::UserOnly { .. } => Self::refused("user-only", message),
            WriteError::SelfLink(_) => Self::refused("self-link", message),
            WriteError::FollowsRetired => Self::refused("follows-retired", message),
        }
    }
}

impl From<StoreError> for CliError {
    fn from(err: StoreError) -> Self {
        let message = err.to_string();
        match err {
            StoreError::State(err) => err.into(),
            StoreError::Git(_) => Self::new("git", exit::STORE, message),
            StoreError::Conflict => Self::new("conflict", exit::STORE, message),
            StoreError::Locked { .. } => Self::new("locked", exit::STORE, message),
            StoreError::Timeout => Self::new("timeout", exit::STORE, message),
            StoreError::Config(_) => Self::new("no-replica", exit::CONFIG, message),
            StoreError::RemoteMoved => Self::new("remote-moved", exit::STORE, message),
            StoreError::Duplicate(_) => Self::refused("duplicate-replica", message),
            StoreError::Rejected { .. } => Self::new("rejected", exit::STORE, message),
        }
    }
}

impl From<BatchError<CliError>> for CliError {
    fn from(err: BatchError<CliError>) -> Self {
        match err {
            BatchError::Store(err) => err.into(),
            BatchError::Apply(err) => err,
        }
    }
}

impl From<ScopeError> for CliError {
    fn from(err: ScopeError) -> Self {
        let message = err.to_string();
        match err {
            ScopeError::Write(err) => err.into(),
            ScopeError::NotTheUser(_) => Self::refused("user-only", message),
            ScopeError::NoProposal(_) => Self::new("not-found", exit::NOT_FOUND, message),
            ScopeError::Decided(_) => Self::refused("decided", message),
        }
    }
}

impl From<DeleteError> for CliError {
    fn from(err: DeleteError) -> Self {
        let message = err.to_string();
        match err {
            DeleteError::Write(err) => err.into(),
            DeleteError::NotAllowed { .. } => Self::refused("user-only", message),
            DeleteError::InUse { item, reasons } => {
                Self::refused("in-use", message).with(json!({"item": item, "reasons": reasons}))
            },
        }
    }
}

impl From<RenumberError> for CliError {
    fn from(err: RenumberError) -> Self {
        let message = err.to_string();
        match err {
            RenumberError::Write(err) => err.into(),
            RenumberError::NotTheUser(_) => Self::refused("user-only", message),
            RenumberError::NoThread(_) => Self::new("not-found", exit::NOT_FOUND, message),
            RenumberError::InUse { thread, reasons } => {
                Self::refused("in-use", message).with(json!({"thread": thread, "reasons": reasons}))
            },
        }
    }
}

impl From<ArchiveError> for CliError {
    fn from(err: ArchiveError) -> Self {
        let message = err.to_string();
        match err {
            ArchiveError::Write(err) => err.into(),
            ArchiveError::Ineligible { item, reasons } => {
                Self::refused("ineligible", message).with(json!({"item": item, "reasons": reasons}))
            },
            ArchiveError::NotDone(_) => Self::refused("not-done", message),
            ArchiveError::NotAllowed(_) => Self::refused("not-allowed", message),
        }
    }
}

impl From<ParseError> for CliError {
    fn from(err: ParseError) -> Self {
        Self::refused("invalid-import", err.to_string())
    }
}

impl From<ApplyError> for CliError {
    fn from(err: ApplyError) -> Self {
        let message = err.to_string();
        match err {
            ApplyError::Write(err) => err.into(),
            ApplyError::Invalid(errors) => {
                let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
                Self::refused("invalid-import", message).with(json!({"errors": errors}))
            },
            ApplyError::UserOnly(_) => Self::refused("user-only", message),
            ApplyError::Cycles(cycles) => {
                Self::refused("import-cycle", message).with(json!({"cycles": cycles}))
            },
        }
    }
}

impl From<LocateError> for CliError {
    fn from(err: LocateError) -> Self {
        Self::new("no-tracker", exit::CONFIG, err.to_string())
    }
}
