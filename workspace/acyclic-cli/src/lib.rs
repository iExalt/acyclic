#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

pub mod error;

mod args;
mod commands;
mod context;
mod page;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use acyclic_core::time::{TIMESTAMP_KEYS, rfc3339};

use crate::error::exit;

/// What a command sees of its process: environment variables and the working directory.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Environment variables.
    pub vars: BTreeMap<String, String>,
    /// The working directory.
    pub cwd: PathBuf,
}

impl Env {
    /// The current process's environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self { vars: std::env::vars().collect(), cwd: std::env::current_dir().unwrap_or_default() }
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str).filter(|value| !value.is_empty())
    }
}

/// Runs the CLI with the process's arguments, the first being the program name, and prints its output.
pub fn run(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let outcome = execute(args, &Env::from_process());
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    acyclic_core::probe::mark("print output");
    ExitCode::from(outcome.code)
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
/// `{"ok": false, "error": {"code", "message", "details"}}`, with times as RFC 3339 in UTC. Help, `--version` and
/// `prime` print text instead, and a command line that doesn't parse prints its error and usage as text to standard
/// error, with exit code 2, as other command-line tools do.
pub fn execute(args: impl IntoIterator<Item = OsString>, env: &Env) -> Outcome {
    use clap::FromArgMatches;
    use clap::error::ErrorKind;
    acyclic_core::probe::mark("start");
    let parsed = args::command()
        .try_get_matches_from(args)
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
    let result = commands::run(&cli, env);
    acyclic_core::probe::mark("command");
    match result {
        Ok(commands::Output::Json(data)) => {
            let output = render(&json!({"ok": true, "data": data}));
            acyclic_core::probe::mark("render output");
            Outcome::out(exit::OK, output)
        },
        Ok(commands::Output::Text(text)) => Outcome::out(exit::OK, text),
        Err(err) => Outcome::out(err.exit, render(&err.envelope())),
    }
}

fn render(value: &serde_json::Value) -> String {
    let mut value = value.clone();
    timestamps(&mut value);
    let mut text = serde_json::to_string_pretty(&value).expect("JSON values always serialize");
    text.push('\n');
    text
}

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
