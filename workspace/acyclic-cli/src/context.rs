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

/// How to print: `--output`, else pretty for a person at a terminal and JSON for whatever reads a pipe, as an agent's
/// tool does.
#[must_use]
pub fn output(cli: &Cli, env: &Env) -> Format {
    cli.output.unwrap_or(if env.terminal.is_some() { Format::Pretty } else { Format::Json })
}

/// The actor, a label on what it writes: `--actor`, else `ACYCLIC_ACTOR`, else the user.
pub fn actor(cli: &Cli, env: &Env) -> Result<String, CliError> {
    match cli.actor.as_deref().or_else(|| env.var("ACYCLIC_ACTOR")) {
        Some(actor) => checked_actor(actor),
        None => Ok(USER.to_owned()),
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

/// The machine's code clones, by repository `owner/name`, from `~/.config/acyclic/clones.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clones {
    #[serde(default)]
    clones: BTreeMap<String, String>,
}

/// Where each repository record's clone is on this machine: the one `clones.toml` maps its `owner/name` to, in any
/// case. A key that isn't an `owner/name` is refused, rather than leaving a merge unchecked.
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
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(error) => {
                    return Err(CliError::new(
                        "config",
                        exit::CONFIG,
                        format!("~/.config/acyclic/clones.toml: {error}"),
                    ));
                },
            }
        },
        None => BTreeMap::new(),
    };
    if let Some(key) = mapping.keys().find(|key| acyclic_core::import::owner_name(key).is_none()) {
        return Err(CliError::new(
            "config",
            exit::CONFIG,
            format!(
                "~/.config/acyclic/clones.toml: {key:?} isn't a repository's owner/name; key each clone by owner/name, \
                 such as \"me/garden\" = \"~/code/garden\""
            ),
        ));
    }
    let mapping: BTreeMap<String, String> =
        mapping.into_iter().map(|(key, path)| (key.to_ascii_lowercase(), path)).collect();
    Ok(move |record: &Repository| {
        let name = crate::resolve::repository_name(record)?.to_ascii_lowercase();
        mapping.get(&name).map(|path| match (path.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => Path::new(path).to_path_buf(),
        })
    })
}
