//! The `acyclic` command.

use std::process::ExitCode;

fn main() -> ExitCode {
    acyclic_cli::run_with(std::env::args_os(), Some(acyclic_server::start))
}
