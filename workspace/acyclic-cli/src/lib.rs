#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

pub mod error;

pub mod args;
mod commands;
mod context;
mod edit;
mod hints;
mod page;
mod pretty;
mod view;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use acyclic_core::time::rfc3339;

use crate::args::{Cli, Command, Format, Serve, Up};
pub use crate::commands::Operation;
pub use crate::context::now;
use crate::error::{CliError, exit};

/// Starts the server for `acyclic serve` or `acyclic up` on a tracker clone, and returns when it stops.
pub type ServeHook = fn(Start<'_>, PathBuf, &Env) -> Result<(), CliError>;

/// The command that starts a server.
#[derive(Debug, Clone, Copy)]
pub enum Start<'a> {
    /// `acyclic serve`, behind Tailscale.
    Serve(&'a Serve),
    /// `acyclic up`, on this machine.
    Up(&'a Up),
}

/// What a command sees of its process: environment variables and the working directory.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Environment variables.
    pub vars: BTreeMap<String, String>,
    /// The working directory.
    pub cwd: PathBuf,
    /// The terminal's width in columns when standard output is one, which `-o pretty` fits its tables to.
    pub terminal: Option<u16>,
}

impl Env {
    /// The current process's environment.
    #[must_use]
    pub fn from_process() -> Self {
        use std::io::IsTerminal as _;
        let stdout = std::io::stdout();
        let terminal = stdout
            .is_terminal()
            .then(|| terminal_size::terminal_size_of(&stdout).map(|(width, _)| width.0))
            .flatten();
        Self {
            vars: std::env::vars().collect(),
            cwd: std::env::current_dir().unwrap_or_default(),
            terminal,
        }
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str).filter(|value| !value.is_empty())
    }
}

/// Runs the CLI with the process's arguments, the first being the program name, and prints its output.
pub fn run(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    run_with(args, None)
}

/// Runs the CLI as [`run`] does, with the server that `acyclic serve` starts, if this binary has one.
pub fn run_with(args: impl IntoIterator<Item = OsString>, serve: Option<ServeHook>) -> ExitCode {
    let outcome = execute_with(args, &Env::from_process(), serve);
    if let Err(err) = emit(&mut std::io::stdout().lock(), &outcome.stdout) {
        // A reader that has gone, as `| head` does, isn't a failure: the command has already run.
        if err.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("acyclic: cannot write output: {err}");
        }
    }
    // Standard error is unbuffered and this is only a message, so a closed one is nothing to report.
    let _ = emit(&mut std::io::stderr().lock(), &outcome.stderr);
    acyclic_core::probe::mark("print output");
    ExitCode::from(outcome.code)
}

/// Writes `text` and flushes, returning errors instead of panicking as `print!` does.
fn emit(out: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    out.write_all(text.as_bytes())?;
    out.flush()
}

/// What a command line produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The exit code.
    pub code: u8,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

impl Outcome {
    fn out(code: u8, stdout: String) -> Self {
        Self { code, stdout, stderr: String::new() }
    }
}

/// Runs a command line. Every command prints one JSON document to standard output, `{"ok": true, "data": …}` or
/// `{"ok": false, "error": {"code", "message", "details"}}`, with times as RFC 3339 in UTC. Pretty output, the default
/// at a terminal, prints the same data as text for people instead, and an error as a line on standard error with the
/// same exit code. Help and
/// version text, `prime`, `completions`, `graph` without `--file` and `import --schema` print text instead, and a
/// command line that doesn't parse prints its error and usage as text to standard error, with exit code 2, as other
/// command-line tools do.
pub fn execute(args: impl IntoIterator<Item = OsString>, env: &Env) -> Outcome {
    execute_with(args, env, None)
}

fn execute_with(
    args: impl IntoIterator<Item = OsString>,
    env: &Env,
    serve: Option<ServeHook>,
) -> Outcome {
    use clap::FromArgMatches;
    use clap::error::ErrorKind;
    acyclic_core::probe::mark("start");
    let args: Vec<OsString> = args.into_iter().collect();
    let parsed = args::command()
        .try_get_matches_from(&args)
        .and_then(|matches| args::Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        // A bare command or group prints its help, as `--help` would.
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) =>
        {
            return Outcome::out(exit::OK, err.to_string());
        },
        Err(err) => {
            let stderr = match hints::hint(&args) {
                Some(hint) => format!("error: {hint}\n\nFor more information, try '--help'.\n"),
                None => err.to_string(),
            };
            return Outcome { code: exit::USAGE, stdout: String::new(), stderr };
        },
    };
    // An unreadable `ACYCLIC_OUTPUT` is reported as JSON, which every reader can take.
    let output = match context::output(&cli, env) {
        Ok(output) => output,
        Err(err) => return Outcome::out(err.exit, render(&err.envelope())),
    };
    let failed = |err: CliError| match output {
        Format::Json => Outcome::out(err.exit, render(&err.envelope())),
        Format::Pretty => Outcome {
            code: err.exit,
            stdout: String::new(),
            stderr: pretty::error(&err.envelope()),
        },
    };
    if let Err(err) = cli.command.validate() {
        return failed(err);
    }
    let start = match &cli.command {
        Command::Serve(options) => Some(Start::Serve(options)),
        Command::Up(options) => Some(Start::Up(options)),
        _ => None,
    };
    if let (Some(start), Some(serve)) = (start, serve) {
        return match serve_with(&cli, start, env, serve) {
            Ok(()) => Outcome::out(exit::OK, String::new()),
            Err(err) => failed(err),
        };
    }
    let result = commands::run(&cli, env);
    acyclic_core::probe::mark("command");
    match result {
        Ok(commands::Output::Json(mut data)) => {
            let output = match output {
                Format::Json => render(&json!({"ok": true, "data": data})),
                Format::Pretty => {
                    timestamps(&mut data);
                    pretty::render(&cli.command, &data, env.terminal)
                },
            };
            acyclic_core::probe::mark("render output");
            Outcome::out(exit::OK, output)
        },
        Ok(commands::Output::Text(text)) => Outcome::out(exit::OK, text),
        Err(err) => failed(err),
    }
}

fn serve_with(cli: &Cli, start: Start<'_>, env: &Env, serve: ServeHook) -> Result<(), CliError> {
    if cli.actor.is_some() || cli.as_user {
        let name = match start {
            Start::Serve(_) => "serve",
            Start::Up(_) => "up",
        };
        return Err(CliError::new(
            "usage",
            exit::USAGE,
            format!(
                "{name} takes no --actor or --as-user: the server acts as the user only for requests carrying their \
                 identity"
            ),
        ));
    }
    serve(start, context::tracker(cli, env)?, env)
}

/// Runs one command for another front end, such as the server: on the tracker clone at `tracker`, as the user or
/// else as `env`'s actor, on `env`'s clock. Returns the exit code and the envelope, with times as RFC 3339 in UTC; a
/// command that prints text returns it as the envelope's data.
#[must_use]
pub fn call(command: Command, tracker: PathBuf, as_user: bool, env: &Env) -> (u8, Value) {
    let cli =
        Cli { tracker: Some(tracker), actor: None, as_user, output: Some(Format::Json), command };
    answer(&cli, env)
}

/// Runs one command as [`call`] does, as `actor`.
#[must_use]
pub fn call_as(command: Command, tracker: PathBuf, actor: &str, env: &Env) -> (u8, Value) {
    let cli = Cli {
        tracker: Some(tracker),
        actor: Some(actor.to_owned()),
        as_user: false,
        output: Some(Format::Json),
        command,
    };
    answer(&cli, env)
}

fn answer(cli: &Cli, env: &Env) -> (u8, Value) {
    let result = cli.command.validate().and_then(|()| commands::run(cli, env));
    let (code, mut envelope) = match result {
        Ok(commands::Output::Json(data)) => (exit::OK, json!({"ok": true, "data": data})),
        Ok(commands::Output::Text(text)) => (exit::OK, json!({"ok": true, "data": text})),
        Err(err) => (err.exit, err.envelope()),
    };
    timestamps(&mut envelope);
    (code, envelope)
}

/// What applying a run's operation did.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    /// The command ran, or the tracker refused it: either way its number is spent. The exit code and the envelope.
    Ran(u8, Value),
    /// The number was applied already, so nothing ran: the outcome recorded for it, if it was the run's latest.
    Duplicate(Option<Value>),
}

/// Applies a run's operation to the tracker clone at `tracker`: `command`, as `run/<run>`, writing under the run's
/// view and spending its number in the same batch. A command the tracker refuses spends its number too, with the
/// refusal as its outcome, so the run's later operations aren't stuck behind it.
///
/// # Errors
///
/// A failure that says nothing about the command, such as the store failing, a gap in the run's numbers, or a
/// replica that doesn't apply this run, as its exit code and envelope. The number stays unspent, so the run can send
/// the operation again.
pub fn apply(
    operation: Operation,
    command: Command,
    tracker: PathBuf,
    env: &Env,
) -> Result<Applied, (u8, Value)> {
    let cli = Cli {
        tracker: Some(tracker),
        actor: Some(format!("run/{}", operation.run)),
        as_user: false,
        output: Some(Format::Json),
        command,
    };
    let duplicate =
        |err: CliError| Applied::Duplicate(Some(err.details).filter(|details| !details.is_null()));
    let failed = |err: CliError| (err.exit, err.envelope());
    match cli.command.validate().and_then(|()| commands::apply(&cli, env, operation.clone())) {
        Ok(data) => {
            let mut envelope = json!({"ok": true, "data": data});
            timestamps(&mut envelope);
            Ok(Applied::Ran(exit::OK, envelope))
        },
        Err(err) if err.code == commands::DUPLICATE => Ok(duplicate(err)),
        Err(err) if spends(&err) => match commands::refuse(&cli, env, operation, &err) {
            Ok(()) => Ok(Applied::Ran(err.exit, err.envelope())),
            Err(again) if again.code == commands::DUPLICATE => Ok(duplicate(again)),
            Err(again) => Err(failed(again)),
        },
        Err(err) => Err(failed(err)),
    }
}

/// Starts run `run` on `thread` in the clone at `tracker`, as the operator `operator` (`operator/<id>`).
///
/// # Errors
///
/// When the thread already has a run under way, or the tracker can't be opened or written, as an exit code and
/// envelope.
pub fn start_run(
    operator: &str,
    run: &str,
    thread: &str,
    tracker: &Path,
    env: &Env,
) -> Result<Value, (u8, Value)> {
    commands::start_run(tracker, env, operator, run, thread)
        .map_err(|err| (err.exit, err.envelope()))
}

/// Moves run `run` to `state` in the clone at `tracker`, as the operator `operator`.
///
/// # Errors
///
/// When the run can't move that way, or the tracker can't be opened or written, as an exit code and envelope.
pub fn set_run_state(
    operator: &str,
    run: &str,
    state: acyclic_core::model::RunState,
    tracker: &Path,
    env: &Env,
) -> Result<Value, (u8, Value)> {
    commands::set_run_state(tracker, env, operator, run, state)
        .map_err(|err| (err.exit, err.envelope()))
}

/// Whether run `run` is under way on `thread` in the clone at `tracker`.
///
/// # Errors
///
/// When the tracker can't be opened or read, as an exit code and envelope.
pub fn run_active(run: &str, thread: &str, tracker: &Path) -> Result<bool, (u8, Value)> {
    commands::run_active(tracker, run, thread).map_err(|err| (err.exit, err.envelope()))
}

/// The latest operation of run `run` applied in the clone at `tracker`, 0 for none.
///
/// # Errors
///
/// When the tracker can't be opened or read, as an exit code and envelope.
pub fn applied(run: &str, tracker: &Path) -> Result<u64, (u8, Value)> {
    commands::applied(tracker, run).map_err(|err| (err.exit, err.envelope()))
}

/// Whether a failed operation spends its number: when the command itself was refused, as it would be again, but not
/// when the store, the setup, the clock or the run's numbering failed, which a later try may get past.
fn spends(err: &CliError) -> bool {
    match err.exit {
        exit::USAGE | exit::NOT_FOUND => true,
        exit::REFUSED => !matches!(
            err.code,
            "sequence-gap"
                | "wrong-replica"
                | "clock-jump"
                | "not-continuous"
                | "id-reused"
                | "inactive"
        ),
        _ => false,
    }
}

fn render(value: &serde_json::Value) -> String {
    let mut value = value.clone();
    timestamps(&mut value);
    let mut text = serde_json::to_string_pretty(&value).expect("JSON values always serialize");
    text.push('\n');
    text
}

/// The fields that hold a time in what commands print, each ending in `_at` but for the dates of an archive wait. A new
/// time field in output must join this list, which a test checks against the views.
const TIMESTAMP_KEYS: [&str; 4] =
    ["done_at", "eligible_at", "lease_expires_at", "lease_started_at"];

/// Rewrites every time in the output as RFC 3339 in UTC, to the second.
fn timestamps(value: &mut serde_json::Value) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                match value.as_i64() {
                    Some(millis) if TIMESTAMP_KEYS.contains(&key.as_str()) => {
                        *value = Value::String(rfc3339(millis));
                    },
                    _ => timestamps(value),
                }
            }
        },
        Value::Array(values) => values.iter_mut().for_each(timestamps),
        _ => {},
    }
}

#[cfg(test)]
mod tests;
