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
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use acyclic_core::time::rfc3339;

use crate::args::{Cli, Command, Format, Serve, Up};
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
