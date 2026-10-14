//! The `acyclic` command.

use std::process::ExitCode;

fn main() -> ExitCode {
    acyclic_cli::run(std::env::args_os())
}
