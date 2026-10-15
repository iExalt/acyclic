#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

pub mod error;

mod args;
mod commands;
mod context;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use acyclic_core::time::{TIMESTAMP_KEYS, rfc3339};

use crate::error::{CliError, exit};

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
    let (code, output) = execute(args, &Env::from_process());
    print!("{output}");
    acyclic_core::probe::mark("print output");
    ExitCode::from(code)
}

/// Runs a command line and returns its exit code and standard output. Every command prints one JSON document,
/// `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message", "details"}}`; only help, `--version`
/// and `prime` print text, and times print as RFC 3339 in UTC.
pub fn execute(args: impl IntoIterator<Item = OsString>, env: &Env) -> (u8, String) {
    acyclic_core::probe::mark("start");
    let cli = match args::Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            use clap::error::ErrorKind;
            // A bare command or group prints its help, as `--help` would.
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                return (exit::OK, err.to_string());
            }
            let message = err.kind().as_str().map_or_else(|| err.to_string(), str::to_owned);
            let usage = CliError::new("usage", exit::USAGE, message)
                .with(json!({"help": err.render().to_string()}));
            return (usage.exit, render(&usage.envelope()));
        },
    };
    let result = commands::run(&cli, env);
    acyclic_core::probe::mark("command");
    match result {
        Ok(commands::Output::Json(data)) => {
            let output = render(&json!({"ok": true, "data": data}));
            acyclic_core::probe::mark("render output");
            (exit::OK, output)
        },
        Ok(commands::Output::Text(text)) => (exit::OK, text),
        Err(err) => (err.exit, render(&err.envelope())),
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
