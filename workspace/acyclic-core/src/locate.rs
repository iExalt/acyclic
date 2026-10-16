//! Finding a code repository's tracker: a private mapping from remote URL prefixes to tracker clones, kept in
//! `~/.config/acyclic/trackers.toml` so no acyclic files go into code repositories. A tracker clone is its own
//! tracker, so it needs no entry.
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

use crate::store::REPLICA_CONFIG;

/// The mapping's file, under the home directory.
pub const CONFIG_PATH: &str = ".config/acyclic/trackers.toml";

/// Why a tracker couldn't be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateError {
    /// The mapping exists but can't be read or parsed.
    Config(String),
    /// The repository has no remote to look up.
    NoRemote,
    /// Git couldn't read the repository, which may not be one.
    Git(String),
    /// No prefix matches any of the repository's remotes and there is no default.
    NotMapped(Vec<String>),
}

impl fmt::Display for LocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "~/{CONFIG_PATH}: {message}"),
            Self::NoRemote => write!(f, "the repository has no remote to find its tracker by"),
            Self::Git(message) => write!(f, "can't read the repository's remotes: {message}"),
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
        self.match_prefix(remote, home).or_else(|| self.default.as_ref().map(|t| expand(t, home)))
    }

    /// The tracker clone for a remote URL's longest matching prefix, ignoring the default.
    fn match_prefix(&self, remote: &str, home: &Path) -> Option<PathBuf> {
        let remote = normalize(remote);
        self.prefixes
            .iter()
            .filter_map(|(prefix, tracker)| {
                let prefix = normalize_prefix(prefix);
                let matches = remote == prefix
                    || remote.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with('/'));
                matches.then_some((prefix.len(), tracker))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, tracker)| expand(tracker, home))
    }

    /// The tracker for the repository at `repo`: the repository itself when it is a tracker clone, else the first
    /// of its remotes, `origin` and then the others by name, that a prefix matches, else the default.
    ///
    /// # Errors
    ///
    /// Fails when `repo` isn't a repository, or no remote is mapped and there is no default.
    pub fn for_repo(&self, repo: &Path, home: &Path) -> Result<PathBuf, LocateError> {
        let settings = Settings::read(repo)?;
        match settings.own_clone()? {
            Some(clone) => Ok(clone),
            None => self.for_remotes(&settings.remotes, home),
        }
    }

    /// The tracker for a repository's remotes, `origin` first: the first that a prefix matches, else the default.
    ///
    /// # Errors
    ///
    /// Fails when no remote is mapped and there is no default.
    pub fn for_remotes(&self, remotes: &[String], home: &Path) -> Result<PathBuf, LocateError> {
        if let Some(tracker) = remotes.iter().find_map(|url| self.match_prefix(url, home)) {
            return Ok(tracker);
        }
        match (&self.default, remotes.is_empty()) {
            (Some(tracker), _) => Ok(expand(tracker, home)),
            (None, true) => Err(LocateError::NoRemote),
            (None, false) => {
                Err(LocateError::NotMapped(remotes.iter().map(|url| normalize(url)).collect()))
            },
        }
    }
}

/// A remote URL reduced to `host/path`, without scheme, user, port, `.git` or trailing slashes, so the HTTPS, SSH
/// and scp-like forms of one repository compare equal. Host URLs are lowercased, since hosting services ignore case;
/// local paths (`/srv/x.git`, `file:///srv/x`, `../x`) keep theirs.
#[must_use]
pub fn normalize(url: &str) -> String {
    let url = url.trim();
    let tidy = |path: &str| {
        let path = path.trim_end_matches('/');
        path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/').to_owned()
    };
    let (authority, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return tidy(rest);
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        // Drop a port.
        (host.split_once(':').map_or(host, |(host, _)| host), path)
    } else {
        // Like git: scp-like `[user@]host:path` only when a colon comes before any slash; otherwise a local path.
        match url.split_once(':') {
            Some((authority, path)) if !authority.contains('/') && !authority.is_empty() => {
                (authority.rsplit_once('@').map_or(authority, |(_, host)| host), path)
            },
            _ => return tidy(url),
        }
    };
    let path = tidy(path.trim_start_matches('/'));
    let joined = if path.is_empty() { authority.to_owned() } else { format!("{authority}/{path}") };
    joined.to_ascii_lowercase()
}

/// A prefix as [`normalize`] reduces a remote. A prefix may also be a bare `host/path`, which `normalize` would read
/// as a local path; anything not shaped like a local path names a host, so it is lowercased like a host URL.
fn normalize_prefix(prefix: &str) -> String {
    let normalized = normalize(prefix);
    let local = prefix.trim().starts_with(['/', '.', '~']) || prefix.trim().starts_with("file://");
    if local { normalized } else { normalized.to_ascii_lowercase() }
}

fn expand(tracker: &str, home: &Path) -> PathBuf {
    match tracker.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(tracker),
    }
}

/// What a repository's own configuration says about finding its tracker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Where it was read.
    pub repo: PathBuf,
    /// Its remotes' URLs, `origin` first and then the others by name.
    pub remotes: Vec<String>,
    /// Whether it is a tracker clone: setup gave it a replica id.
    pub tracker: bool,
}

impl Settings {
    /// Reads the settings of the repository at `repo`, which may be any directory in its working tree.
    ///
    /// # Errors
    ///
    /// Fails when `repo` isn't a repository or git can't run.
    pub fn read(repo: &Path) -> Result<Self, LocateError> {
        let pattern = format!(r"^(remote\..*\.url|{})$", REPLICA_CONFIG.replace('.', r"\."));
        let output = git(repo, &["config", "--local", "--get-regexp", &pattern])?;
        // Exit status 1 means no key matched; anything else, such as not being a repository, is a failure.
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(LocateError::Git(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let mut tracker = false;
        let mut remotes: Vec<(String, String)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ').unwrap_or((line, ""));
                tracker |= key == REPLICA_CONFIG;
                let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect();
        remotes.sort_by_key(|(name, _)| (name != "origin", name.clone()));
        let remotes = remotes.into_iter().map(|(_, url)| url).collect();
        Ok(Self { repo: repo.to_owned(), remotes, tracker })
    }

    /// The working tree's root when the repository is a tracker clone, so a command run anywhere inside one opens
    /// it, with no mapping.
    ///
    /// # Errors
    ///
    /// Fails when git can't find the working tree's root.
    pub fn own_clone(&self) -> Result<Option<PathBuf>, LocateError> {
        if !self.tracker {
            return Ok(None);
        }
        let output = git(&self.repo, &["rev-parse", "--show-toplevel"])?;
        if !output.status.success() {
            return Err(LocateError::Git(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(Some(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim_end())))
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output, LocateError> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|err| LocateError::Git(format!("git can't run: {err}")))
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
        // An @ in the path isn't a user.
        assert_eq!(normalize("https://host.test/org/repo@v2"), "host.test/org/repo@v2");
        assert_eq!(normalize("git@host.test:org/repo@v2.git"), "host.test/org/repo@v2");
        // Local paths keep their case.
        assert_eq!(normalize("/srv/Repos/X.git"), "/srv/Repos/X");
        assert_eq!(normalize("file:///srv/Repos/X/"), "/srv/Repos/X");
        assert_eq!(normalize("../Up/X"), "../Up/X");
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
            "github.com/Some-Org/One" = "~/t/one"
            "https://Example.test" = "/t/host"
            "#,
        );
        let lookup = |url| map.lookup(url, home);
        // A bare `host/path` prefix ignores case like the host URL it stands for.
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
