#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// The version shared by every acyclic crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
