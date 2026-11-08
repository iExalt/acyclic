//! Errors as the contract reports them: a stable kebab-case code, an exit code and a message.

use std::fmt;

use acyclic_core::import::{ApplyError, ParseError, ValidationError};
use acyclic_core::locate::LocateError;
use acyclic_core::model::WriteError;
use acyclic_core::state::{Error as StateError, FORMAT_VERSION};
use acyclic_core::store::{BatchError, StoreError};
use serde_json::{Value as Json, json};

/// What an unreadable state hints at, except from `doctor` itself.
pub(crate) const DOCTOR_HINT: &str = "run acyclic doctor";
/// What a state an earlier format version wrote hints at, from every command.
const MIGRATE_HINT: &str = "written by an earlier acyclic: read it with that version, or rebuild it with \
                            `acyclic import` from a version-4 document";
/// What a state a later format version wrote hints at.
const LATER_HINT: &str = "written by a later acyclic: run that version";
const CLOCK_JUMP_HINT: &str = "Correct the system clock and wait until it is within five minutes of this replica's \
                               saved time. If this clone has a shared remote, sync it and continue from a fresh clone \
                               with an unused replica id";

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
    /// The store failed: a lock, a lost race, or git.
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
            StateError::IdReused { .. } => ("id-reused", exit::REFUSED),
            StateError::Claimed(_) => ("claimed", exit::REFUSED),
            StateError::NoThread(_) => ("not-found", exit::NOT_FOUND),
            StateError::Format(_) | StateError::Version(_) => ("unreadable", exit::UNREADABLE),
        };
        let error = Self::new(code, exit, message);
        match err {
            StateError::Version(version) if version < u64::from(FORMAT_VERSION) => {
                error.with(json!({"hint": MIGRATE_HINT}))
            },
            StateError::Version(_) => error.with(json!({"hint": LATER_HINT})),
            StateError::ClockJump { .. } => error.with(json!({"hint": CLOCK_JUMP_HINT})),
            StateError::Format(_) => error.with(json!({"hint": DOCTOR_HINT})),
            _ => error,
        }
    }
}

impl From<WriteError> for CliError {
    fn from(err: WriteError) -> Self {
        let message = err.to_string();
        match err {
            WriteError::State(err) => err.into(),
            WriteError::InvalidName(_) => Self::new("invalid-name", exit::USAGE, message),
            WriteError::NoItem(_) | WriteError::NoThread(_) | WriteError::NoInitiative(_) => {
                Self::new("not-found", exit::NOT_FOUND, message)
            },
            WriteError::InUse { item, reasons } => {
                Self::refused("in-use", message).with(json!({"item": item, "reasons": reasons}))
            },
            WriteError::InitiativeInUse { initiative, reasons } => Self::refused("in-use", message)
                .with(json!({"initiative": initiative, "reasons": reasons})),
            WriteError::ThreadInUse { thread, reasons } => {
                Self::refused("in-use", message).with(json!({"thread": thread, "reasons": reasons}))
            },
            WriteError::Deleted(_) => Self::refused("deleted", message),
            WriteError::ParentCycle(_) => Self::refused("parent-cycle", message),
            WriteError::WaitsForCycle(path) => {
                Self::refused("waits-for-cycle", message).with(json!({"items": path}))
            },
            WriteError::SelfLink(_) => Self::refused("self-link", message),
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
            StoreError::Config(_) => Self::new("no-replica", exit::CONFIG, message),
            StoreError::RemoteMoved => Self::new("remote-moved", exit::STORE, message),
            StoreError::Duplicate(_) => Self::refused("duplicate-replica", message),
            StoreError::Rejected { .. } => Self::new("rejected", exit::STORE, message),
            StoreError::ObjectsRefused(_) => Self::new("objects-refused", exit::STORE, message),
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

impl From<ParseError> for CliError {
    fn from(err: ParseError) -> Self {
        Self::refused("invalid-import", err.to_string())
    }
}

impl From<ApplyError> for CliError {
    fn from(err: ApplyError) -> Self {
        let message = err.to_string();
        match err {
            ApplyError::Internal(violations) => Self::refused("internal", message).with(json!({
                "violations": violations.iter().map(ToString::to_string).collect::<Vec<_>>(),
            })),
            ApplyError::Write(err) => err.into(),
            // Ids are judged first, and alone: an agent branching on the code sees the real reason.
            ApplyError::Invalid(errors) if errors.iter().any(ValidationError::is_invalid_id) => {
                let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
                Self::new("invalid-id", exit::USAGE, message).with(json!({"errors": errors}))
            },
            ApplyError::Invalid(errors) => {
                let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
                Self::refused("invalid-import", message).with(json!({"errors": errors}))
            },
            ApplyError::Cycles(cycles) => {
                Self::refused("import-cycle", message).with(json!({"cycles": cycles}))
            },
        }
    }
}

impl From<acyclic_core::export::Refusal> for CliError {
    fn from(err: acyclic_core::export::Refusal) -> Self {
        let records: Vec<Json> =
            err.0.iter().map(|b| json!({"kind": b.kind, "id": b.id, "reason": b.reason})).collect();
        Self::refused("unexportable", err.to_string()).with(json!({"records": records}))
    }
}

impl From<LocateError> for CliError {
    fn from(err: LocateError) -> Self {
        Self::new("no-tracker", exit::CONFIG, err.to_string())
    }
}
