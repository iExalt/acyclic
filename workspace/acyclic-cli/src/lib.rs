#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::ffi::OsString;
use std::process::ExitCode;

/// Runs the CLI with the process's arguments, the first being the program name.
pub fn run(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let args: Vec<OsString> = args.into_iter().skip(1).collect();
    if let Some("--version" | "-V") = args.first().and_then(|arg| arg.to_str()) {
        println!("acyclic {}", acyclic_core::VERSION);
        ExitCode::SUCCESS
    } else {
        eprintln!("usage: acyclic --version");
        ExitCode::from(2)
    }
}
