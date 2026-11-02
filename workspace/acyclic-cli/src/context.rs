//! Where a command runs: its tracker, actor, clock and code clones.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use acyclic_core::locate::{Settings, Trackers};
use acyclic_core::model::Repository;
use acyclic_core::scope::USER;
use acyclic_core::state::Millis;
use serde::Deserialize;

use crate::Env;
use crate::args::{Cli, Format};
use crate::error::{CliError, exit};

/// The actor when none is declared: an agent, never the user.
pub const UNDECLARED: &str = "agent/undeclared";

/// How to print: `--output`, else `ACYCLIC_OUTPUT`, else pretty for a person at a terminal and JSON for whatever
/// reads a pipe, as an agent's tool does.
pub fn output(cli: &Cli, env: &Env) -> Result<Format, CliError> {
    if let Some(format) = cli.output {
        return Ok(format);
    }
    match env.var("ACYCLIC_OUTPUT") {
        Some("json") => Ok(Format::Json),
        Some("pretty") => Ok(Format::Pretty),
        Some(other) => Err(CliError::new(
            "usage",
            exit::USAGE,
            format!("ACYCLIC_OUTPUT is {other}: set it to json or pretty, or unset it"),
        )),
        None if env.terminal.is_some() => Ok(Format::Pretty),
        None => Ok(Format::Json),
    }
}

/// The actor: `--as-user` or `--actor`, else `ACYCLIC_ACTOR`, else [`UNDECLARED`]. The environment may not name the
/// user: a shell profile setting it would make every agent started from that shell the user.
pub fn actor(cli: &Cli, env: &Env) -> Result<String, CliError> {
    if cli.as_user {
        return Ok(USER.to_owned());
    }
    if let Some(actor) = &cli.actor {
        return checked_actor(actor);
    }
    match env.var("ACYCLIC_ACTOR") {
        Some(USER) => Err(CliError::new(
            "user-from-environment",
            exit::USAGE,
            "ACYCLIC_ACTOR may not name the user, or every agent started from this shell would act as the user; pass \
             --as-user on the command instead",
        )),
        Some(actor) => checked_actor(actor),
        None => Ok(UNDECLARED.to_owned()),
    }
}

fn checked_actor(actor: &str) -> Result<String, CliError> {
    let valid = actor.split_once('/').is_some_and(|(kind, id)| !kind.is_empty() && !id.is_empty())
        && !actor.chars().any(char::is_whitespace);
    if valid {
        Ok(actor.to_owned())
    } else {
        Err(CliError::new(
            "invalid-actor",
            exit::USAGE,
            format!("{actor:?} isn't a valid actor: use a name such as session/mac"),
        ))
    }
}

/// The clock. Debug builds honour `ACYCLIC_NOW` (milliseconds) so tests are repeatable; release builds never do,
/// since a caller could otherwise stamp leases far into the future.
#[must_use]
pub fn now(env: &Env) -> Millis {
    #[cfg(debug_assertions)]
    if let Some(now) = env.var("ACYCLIC_NOW").and_then(|now| now.parse().ok()) {
        return now;
    }
    let _ = env;
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| Millis::try_from(d.as_millis()).unwrap_or(Millis::MAX))
}

/// Opens a file in the default application without waiting for it. Debug builds honour `ACYCLIC_OPENER`, a program
/// run in its place, so tests never open a browser; release builds never do.
pub fn open(env: &Env, path: &Path) -> std::io::Result<()> {
    #[cfg(debug_assertions)]
    if let Some(opener) = env.var("ACYCLIC_OPENER") {
        return spawn(opener, path);
    }
    let _ = env;
    spawn(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }, path)
}

fn spawn(program: &str, path: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

fn home(env: &Env) -> Option<PathBuf> {
    env.var("HOME").map(PathBuf::from)
}

/// The tracker clone: `--tracker`, else `ACYCLIC_TRACKER`, else the working directory's repository when it is a tracker
/// clone, else the mapping for it.
pub fn tracker(cli: &Cli, env: &Env) -> Result<PathBuf, CliError> {
    if let Some(path) = &cli.tracker {
        return Ok(env.cwd.join(path));
    }
    if let Some(path) = env.var("ACYCLIC_TRACKER") {
        return Ok(env.cwd.join(path));
    }
    // A tracker clone needs neither HOME nor the mapping.
    let settings = Settings::read(&env.cwd)?;
    if let Some(clone) = settings.own_clone()? {
        return Ok(clone);
    }
    let home = home(env).ok_or_else(|| {
        CliError::new("no-tracker", exit::CONFIG, "HOME isn't set, so ~/.config/acyclic/trackers.toml can't be found; pass --tracker or set ACYCLIC_TRACKER")
    })?;
    Ok(Trackers::load(&home)?.for_remotes(&settings.remotes, &home)?)
}

/// The machine's code clones, by repository record id, from `~/.config/acyclic/clones.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clones {
    #[serde(default)]
    clones: BTreeMap<String, String>,
}

/// Where each repository record's clone is on this machine: the one `clones.toml` maps.
pub fn clones(env: &Env) -> Result<impl Fn(&Repository) -> Option<PathBuf> + use<>, CliError> {
    let home = home(env);
    let mapping = match &home {
        Some(home) => {
            let path = home.join(".config/acyclic/clones.toml");
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    toml::from_str::<Clones>(&text)
                        .map_err(|e| {
                            CliError::new(
                                "config",
                                exit::CONFIG,
                                format!("~/.config/acyclic/clones.toml: {}", e.message()),
                            )
                        })?
                        .clones
                },
                Err(_) => BTreeMap::new(),
            }
        },
        None => BTreeMap::new(),
    };
    Ok(move |record: &Repository| {
        mapping.get(&record.id).map(|path| match (path.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => Path::new(path).to_path_buf(),
        })
    })
}
