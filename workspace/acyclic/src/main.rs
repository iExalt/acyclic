//! The `acyclic` command.

use std::process::ExitCode;

fn main() -> ExitCode {
    // The operator's inbox hook, which git runs during a run's push; not a command anyone types.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "operator-hook") {
        return acyclic_operator::hook::main();
    }
    acyclic_cli::run_with(std::env::args_os(), Some(acyclic_server::start))
}
