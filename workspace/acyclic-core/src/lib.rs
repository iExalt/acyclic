#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// The version shared by every acyclic crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Declares a write that bypasses the write judge: public with the `fixtures` feature, which only tests enable, so
/// they can build any state, and crate-private otherwise, so the shipped binaries write records through the judge.
macro_rules! fixture {
    ($(#[$meta:meta])* fn $($rest:tt)*) => {
        #[cfg(feature = "fixtures")]
        $(#[$meta])*
        pub fn $($rest)*

        #[cfg(not(feature = "fixtures"))]
        #[allow(dead_code, reason = "only tests and the judge's neighbours write past the judge")]
        $(#[$meta])*
        pub(crate) fn $($rest)*
    };
}

pub mod claim;
pub mod delete;
pub mod export;
pub mod graph;
pub mod ids;
pub mod import;
pub mod locate;
pub mod merged;
pub mod model;
pub mod natural;
pub mod needs;
pub mod place;
pub mod private;
#[doc(hidden)]
pub mod probe;
pub mod ready;
mod say;
pub mod schema;
pub mod scope;
pub mod state;
pub mod store;
pub mod time;
pub mod write;
