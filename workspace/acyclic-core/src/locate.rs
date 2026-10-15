//! Finding a code repository's tracker: a private mapping from remote URL prefixes to tracker clones, kept in
//! `~/.config/acyclic/trackers.toml` so no acyclic files go into code repositories.
//!
//! ```toml
//! # Used when no prefix matches.
//! default = "~/trackers/personal"
//!
//! [prefixes]
//! # A host, an owner or a single repository; the longest matching prefix wins.
//! "github.com/some-org" = "~/trackers/other"
//! "github.com/some-org/one-repo" = "~/trackers/personal"
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// The mapping's file, under the home directory.
pub const CONFIG_PATH: &str = ".config/acyclic/trackers.toml";

/// Why a tracker couldn't be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateError {
    /// The mapping exists but can't be read or parsed.
    Config(String),
    /// The repository has no remote to look up.
    NoRemote,
    /// No prefix matches any of the repository's remotes and there is no default.
    NotMapped(Vec<String>),
}

impl fmt::Display for LocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "~/{CONFIG_PATH}: {message}"),
            Self::NoRemote => write!(f, "the repository has no remote to find its tracker by"),
            Self::NotMapped(remotes) => write!(
                f,
                "no tracker in ~/{CONFIG_PATH} for {}; add a prefix or a default",
                remotes.join(", ")
            ),
        }
    }
}

impl std::error::Error for LocateError {}

/// The mapping from remote URL prefixes to tracker clones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trackers {
    /// The tracker used when no prefix matches.
    #[serde(default)]
    pub default: Option<String>,
    /// Tracker clones by normalized remote URL prefix.
    #[serde(default)]
    pub prefixes: BTreeMap<String, String>,
}

impl Trackers {
    /// Reads the mapping under `home`; a missing file is an empty mapping.
    ///
    /// # Errors
    ///
    /// Fails when the file exists but can't be read or isn't a valid mapping.
    pub fn load(home: &Path) -> Result<Self, LocateError> {
        let path = home.join(CONFIG_PATH);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(LocateError::Config(err.to_string())),
        };
        toml::from_str(&text).map_err(|err| LocateError::Config(err.message().to_owned()))
    }

    /// The tracker clone for a remote URL: the longest prefix that matches whole path components, else the default.
    /// A leading `~/` expands to `home`.
    #[must_use]
    pub fn lookup(&self, remote: &str, home: &Path) -> Option<PathBuf> {
        let remote = normalize(remote);
        self.prefixes
            .iter()
            .filter_map(|(prefix, tracker)| {
                let prefix = normalize(prefix);
                let matches = remote == prefix
                    || remote.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with('/'));
                matches.then_some((prefix.len(), tracker))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, tracker)| tracker)
            .or(self.default.as_ref())
            .map(|tracker| expand(tracker, home))
    }

    /// The tracker for the code repository at `repo`, by its remotes: `origin` first, then the others by name.
    ///
    /// # Errors
    ///
    /// Fails when the repository has no remote, or none is mapped and there is no default.
    pub fn for_repo(&self, repo: &Path, home: &Path) -> Result<PathBuf, LocateError> {
        let remotes = remote_urls(repo);
        if remotes.is_empty() {
            return self.default.as_ref().map(|t| expand(t, home)).ok_or(LocateError::NoRemote);
        }
        remotes.iter().find_map(|url| self.lookup(url, home)).ok_or_else(|| {
            LocateError::NotMapped(remotes.iter().map(|url| normalize(url)).collect())
        })
    }
}

/// A remote URL reduced to `host/path`, lowercase, without scheme, user, port, `.git` or trailing slashes, so the
/// HTTPS, SSH and scp-like forms of one repository compare equal.
#[must_use]
pub fn normalize(url: &str) -> String {
    let url = url.trim();
    let (rest, scheme) = match url.split_once("://") {
        Some((_, rest)) => (rest, true),
        None => (url, false),
    };
    let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    let (host, path) = match rest.split_once('/') {
        // scp-like `host:path`, or `host:port/path` after a scheme.
        Some((host, path)) => (host, path),
        None => (rest, ""),
    };
    let (host, path) = match host.split_once(':') {
        Some((host, port)) if scheme && port.chars().all(|c| c.is_ascii_digit()) => {
            (host, path.to_owned())
        },
        Some((host, first)) => (host, format!("{first}/{path}")),
        None => (host, path.to_owned()),
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let joined = if path.is_empty() { host.to_owned() } else { format!("{host}/{path}") };
    joined.trim_end_matches('/').to_ascii_lowercase()
}

fn expand(tracker: &str, home: &Path) -> PathBuf {
    match tracker.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(tracker),
    }
}

/// The repository's remote URLs, `origin` first.
fn remote_urls(repo: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .env("LC_ALL", "C")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let mut remotes: Vec<(String, String)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some((name.to_owned(), url.to_owned()))
        })
        .collect();
    remotes.sort_by_key(|(name, _)| (name != "origin", name.clone()));
    remotes.into_iter().map(|(_, url)| url).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_of_a_remote_normalizes_alike() {
        for url in [
            "https://github.com/Some-Org/Repo.git",
            "https://user@github.com/some-org/repo/",
            "ssh://git@github.com:22/some-org/repo.git",
            "git@github.com:some-org/repo.git",
            "github.com:some-org/repo",
        ] {
            assert_eq!(normalize(url), "github.com/some-org/repo", "{url}");
        }
        assert_eq!(normalize("ssh://git@example.test/a"), "example.test/a");
    }

    fn trackers(text: &str) -> Trackers {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn the_longest_whole_component_prefix_wins() {
        let home = Path::new("/home/u");
        let map = trackers(
            r#"
            [prefixes]
            "github.com/some-org" = "/t/org"
            "github.com/some-org/one" = "~/t/one"
            "https://Example.test" = "/t/host"
            "#,
        );
        let lookup = |url| map.lookup(url, home);
        assert_eq!(lookup("git@github.com:some-org/one.git"), Some("/home/u/t/one".into()));
        assert_eq!(lookup("https://github.com/some-org/two"), Some("/t/org".into()));
        // `some-org` doesn't match `some-organ`.
        assert_eq!(lookup("https://github.com/some-organ/x"), None);
        assert_eq!(lookup("https://example.test/a/b"), Some("/t/host".into()));
    }

    #[test]
    fn the_default_catches_unmapped_remotes() {
        let map = trackers("default = \"~/t/personal\"\n[prefixes]\n\"github.com/x\" = \"/t/x\"\n");
        assert_eq!(
            map.lookup("https://gitlab.test/y", Path::new("/h")),
            Some("/h/t/personal".into())
        );
    }

    #[test]
    fn a_missing_mapping_is_empty_and_a_bad_one_fails() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Trackers::load(home.path()), Ok(Trackers::default()));
        let path = home.path().join(CONFIG_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "defualt = \"/t\"\n").unwrap();
        assert!(matches!(Trackers::load(home.path()), Err(LocateError::Config(_))));
    }
}
