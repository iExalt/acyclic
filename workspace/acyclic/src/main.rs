//! The `acyclic` command.

use std::process::ExitCode;

fn main() -> ExitCode {
    // A repack that a command started and left behind runs here, apart from the command that wanted it.
    if acyclic_core::store::run_maintenance_child() {
        return ExitCode::SUCCESS;
    }
    acyclic_core::store::set_maintenance(acyclic_core::store::Maintenance::Process);
    acyclic_cli::run_with(std::env::args_os(), Some(acyclic_server::start))
}
