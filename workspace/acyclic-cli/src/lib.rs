#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

pub mod error;

pub mod args;
mod commands;
mod context;
mod edit;
mod keep;
mod pretty;
mod resolve;
mod view;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

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

/// Runs the CLI with the process's arguments, the first being the program name, and prints its output; `serve` is
/// the server that `acyclic serve` starts, if this binary has one.
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
/// same exit code. Help and version text, `prime`, `completions` and `import --schema` print text instead, and a
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
            return Outcome { code: exit::USAGE, stdout: String::new(), stderr: err.to_string() };
        },
    };
    let output = context::output(&cli, env);
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
    let result = match output {
        Format::Json => commands::run(&cli, env).map(|output| (output, commands::Names::new())),
        Format::Pretty => commands::run_named(&cli, env),
    };
    acyclic_core::probe::mark("command");
    match result {
        Ok((commands::Output::Json(data), names)) => {
            let output = match output {
                Format::Json => render(&json!({"ok": true, "data": data})),
                Format::Pretty => pretty::render(&cli.command, &data, env.terminal, &names),
            };
            acyclic_core::probe::mark("render output");
            Outcome::out(exit::OK, output)
        },
        Ok((commands::Output::Text(text), _)) => Outcome::out(exit::OK, text),
        Err(err) => failed(err),
    }
}

fn serve_with(cli: &Cli, start: Start<'_>, env: &Env, serve: ServeHook) -> Result<(), CliError> {
    if cli.actor.is_some() {
        let name = match start {
            Start::Serve(_) => "serve",
            Start::Up(_) => "up",
        };
        return Err(CliError::new(
            "usage",
            exit::USAGE,
            format!(
                "{name} takes no --actor: the server acts as the user only for requests carrying their identity"
            ),
        ));
    }
    serve(start, context::tracker(cli, env)?, env)
}

/// Runs one command for another front end, such as the server: on the tracker clone at `tracker`, as the user, on
/// `env`'s clock. Returns the exit code and the envelope, with times as RFC 3339 in UTC; a
/// command that prints text returns it as the envelope's data.
#[must_use]
pub fn call(command: Command, tracker: PathBuf, env: &Env) -> (u8, Value) {
    call_as(command, tracker, acyclic_core::scope::USER, env)
}

/// Runs one command as [`call`], requiring these write conditions to hold under the tracker lock.
#[must_use]
pub fn call_expected(
    command: Command,
    tracker: PathBuf,
    env: &Env,
    expected: Vec<acyclic_core::write::Expectation>,
) -> (u8, Value) {
    let cli = Cli {
        tracker: Some(tracker),
        actor: Some(acyclic_core::scope::USER.to_owned()),
        output: Some(Format::Json),
        command,
    };
    answer_with_expected(&cli, env, Some(expected))
}

/// Runs one app write, returning exact notifying needs acknowledged by its locked batch.
#[must_use]
pub fn call_observed(
    command: Command,
    tracker: PathBuf,
    env: &Env,
    expected: Vec<acyclic_core::write::Expectation>,
    observed: Vec<String>,
) -> (u8, Value, Vec<String>) {
    let cli = Cli {
        tracker: Some(tracker),
        actor: Some(acyclic_core::scope::USER.to_owned()),
        output: Some(Format::Json),
        command,
    };
    answer_with_observed(&cli, env, expected, observed)
}

/// Runs one command as [`call`] does, as `actor`.
#[must_use]
pub fn call_as(command: Command, tracker: PathBuf, actor: &str, env: &Env) -> (u8, Value) {
    let cli = Cli {
        tracker: Some(tracker),
        actor: Some(actor.to_owned()),
        output: Some(Format::Json),
        command,
    };
    answer(&cli, env)
}

fn answer(cli: &Cli, env: &Env) -> (u8, Value) {
    answer_with_expected(cli, env, None)
}

fn answer_with_expected(
    cli: &Cli,
    env: &Env,
    expected: Option<Vec<acyclic_core::write::Expectation>>,
) -> (u8, Value) {
    let result = cli.command.validate().and_then(|()| match expected {
        Some(expected) => commands::run_expected(cli, env, expected),
        None => commands::run(cli, env),
    });
    let (code, envelope) = match result {
        Ok(commands::Output::Json(data)) => (exit::OK, json!({"ok": true, "data": data})),
        Ok(commands::Output::Text(text)) => (exit::OK, json!({"ok": true, "data": text})),
        Err(err) => (err.exit, err.envelope()),
    };
    (code, envelope)
}

fn answer_with_observed(
    cli: &Cli,
    env: &Env,
    expected: Vec<acyclic_core::write::Expectation>,
    observed: Vec<String>,
) -> (u8, Value, Vec<String>) {
    let result =
        cli.command.validate().and_then(|()| commands::run_observed(cli, env, expected, observed));
    match result {
        Ok((commands::Output::Json(data), acknowledged)) => {
            (exit::OK, json!({"ok": true, "data": data}), acknowledged)
        },
        Ok((commands::Output::Text(text), acknowledged)) => {
            (exit::OK, json!({"ok": true, "data": text}), acknowledged)
        },
        Err(err) => (err.exit, err.envelope(), Vec::new()),
    }
}

fn render(value: &serde_json::Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON values always serialize");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
