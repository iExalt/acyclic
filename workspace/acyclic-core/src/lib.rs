#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// The version shared by every acyclic crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod archive;
pub mod claim;
pub mod delete;
pub mod export;
pub mod graph;
pub mod ids;
pub mod import;
pub mod locate;
pub mod model;
pub mod natural;
pub mod needs;
pub mod place;
pub mod private;
#[doc(hidden)]
pub mod probe;
pub mod ready;
pub mod renumber;
mod say;
pub mod schema;
pub mod scope;
pub mod state;
pub mod store;
pub mod time;
pub mod usage;
