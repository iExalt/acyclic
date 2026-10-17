#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

pub mod dotenv;
pub mod podman;
pub mod runner;
pub mod sops;

pub use dotenv::Secrets;
pub use runner::{Runner, SystemRunner};

use std::fmt;

/// Why a secret couldn't be read or handed over.
#[derive(Debug)]
pub enum Error {
    /// A program couldn't be started at all.
    Spawn {
        /// The program.
        program: String,
        /// Why it couldn't start.
        source: std::io::Error,
    },
    /// A program ran and failed.
    Failed {
        /// The program and its subcommand, such as `sops decrypt`.
        command: String,
        /// Its standard error, which holds no secret values.
        stderr: String,
    },
    /// The decrypted file wasn't the dotenv SOPS writes.
    Parse(dotenv::ParseError),
    /// The file has no value under this name.
    Missing {
        /// The name asked for.
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } if source.kind() == std::io::ErrorKind::NotFound => {
                write!(f, "`{program}` isn't installed: run `mise install` in acyclic's repository")
            },
            Self::Spawn { program, source } => write!(f, "couldn't run `{program}`: {source}"),
            Self::Failed { command, stderr } => write!(f, "`{command}` failed: {}", stderr.trim()),
            Self::Parse(e) => write!(f, "the decrypted secrets file isn't dotenv: {e}"),
            Self::Missing { name } => write!(f, "the secrets file has no `{name}`"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}
