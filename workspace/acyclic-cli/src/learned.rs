//! Code clones this machine has been seen working in. Any command run inside a clone of a github.com repository
//! records where that repository's main checkout is, in `$XDG_STATE_HOME/acyclic/clones.json` (by default
//! `~/.local/state/acyclic/clones.json`), so commit evidence is checked without a hand-kept `clones.toml`, which
//! stays as an override and is looked at first.
//!
//! The cache is a hint, never state: it is best-effort throughout. A corrupt file reads as empty, a failed write is
//! ignored, and a lost concurrent update is relearned by the next command run in that clone.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use acyclic_core::locate::Settings;
use serde::{Deserialize, Serialize};

use crate::Env;

#[derive(Debug, Default, Deserialize, Serialize)]
struct Cache {
    /// Main checkouts' working directories, by lower-case `owner/name`.
    #[serde(default)]
    clones: BTreeMap<String, String>,
}

/// Where the cache lives: under `$XDG_STATE_HOME` when that is an absolute path, else `~/.local/state`.
pub(crate) fn cache_path(env: &Env) -> Option<PathBuf> {
    state_file(env, "clones.json")
}

/// A file of acyclic's best-effort state, `name` under `$XDG_STATE_HOME/acyclic`, else `~/.local/state/acyclic`.
pub(crate) fn state_file(env: &Env, name: &str) -> Option<PathBuf> {
    let base = match env.var("XDG_STATE_HOME") {
        Some(dir) if Path::new(dir).is_absolute() => PathBuf::from(dir),
        _ => PathBuf::from(env.var("HOME")?).join(".local/state"),
    };
    Some(base.join("acyclic").join(name))
}

/// A state file's contents; empty when it is missing or corrupt.
pub(crate) fn read<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes through a temporary file renamed into place, so a reader never sees half a file.
pub(crate) fn write<T: Serialize>(path: &Path, cache: &T) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".clones.json.{}.tmp", std::process::id()));
    let result = serde_json::to_vec(cache)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&temp, bytes))
        .and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Records the main checkout of the repository the command runs in, when `origin` names a github.com repository.
/// Run once at the start of a command; it never fails one.
pub(crate) fn learn(env: &Env) {
    let _ = try_learn(env);
}

fn try_learn(env: &Env) -> Option<()> {
    let path = cache_path(env)?;
    let settings = Settings::read(&env.cwd).ok()?;
    let name = settings.github_name()?;
    // A bare repository has no checkout to record.
    let main = settings.main_checkout().ok()??.to_str()?.to_owned();
    let mut cache: Cache = read(&path);
    if cache.clones.get(&name) == Some(&main) {
        return Some(());
    }
    cache.clones.insert(name, main);
    write(&path, &cache).ok()
}

/// The learned clones a command has looked up, so that every look it takes agrees even when another command changes
/// the cache meanwhile.
#[derive(Debug, Default)]
pub(crate) struct Learned(RefCell<BTreeMap<String, Option<PathBuf>>>);

impl Learned {
    /// The learned clone of the repository named `name`, in lower case, while it still exists and its `origin` still
    /// names that repository; an entry that doesn't is dropped from the cache.
    pub(crate) fn find(&self, cache: Option<&Path>, name: &str) -> Option<PathBuf> {
        self.0.borrow_mut().entry(name.to_owned()).or_insert_with(|| lookup(cache?, name)).clone()
    }
}

fn lookup(cache: &Path, name: &str) -> Option<PathBuf> {
    let mut learned: Cache = read(cache);
    let path = PathBuf::from(learned.clones.get(name)?);
    let valid = path.is_dir()
        && Settings::read(&path).ok().and_then(|settings| settings.github_name()).as_deref()
            == Some(name);
    if valid {
        return Some(path);
    }
    learned.clones.remove(name);
    let _ = write(cache, &learned);
    None
}

#[cfg(test)]
mod tests;
