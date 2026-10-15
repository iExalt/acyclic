#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// The version shared by every acyclic crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod claim;
pub mod graph;
pub mod import;
pub mod locate;
pub mod model;
pub mod schema;
pub mod state;
pub mod store;
