#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// The version shared by every acyclic crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod archive;
pub mod claim;
pub mod export;
pub mod graph;
pub mod import;
pub mod locate;
pub mod model;
pub mod needs;
pub mod place;
#[doc(hidden)]
pub mod probe;
pub mod ready;
pub mod schema;
pub mod scope;
pub mod state;
pub mod store;
pub mod time;
pub mod usage;
