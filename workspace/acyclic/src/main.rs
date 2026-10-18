//! The `acyclic` command.

use std::process::ExitCode;

fn main() -> ExitCode {
    // The mirrors' hook, which git runs during a run's push; not a command anyone types.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == acyclic_server::code::hook::COMMAND) {
        return acyclic_server::code::hook::main();
    }
    acyclic_cli::run_with(std::env::args_os(), Some(acyclic_server::start))
}
