//! Files only the user can read or write, for the secrets acyclic keeps under `~/.config/acyclic/`: the session `acyclic
//! up` issues and the key the operator signs run tokens with. It lives in core, beside [`crate::locate`], which also
//! decides paths there, because the server and the operator both keep such files and both already depend on core.
//!
//! A directory or file that is open to other users, or isn't what it should be (a symbolic link, say), is refused
//! rather than repaired: whatever opened it may already have read it.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

/// Why a private file or directory was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Reading, creating or writing it failed.
    Io {
        /// The path.
        path: PathBuf,
        /// What was being done.
        doing: &'static str,
        /// The system's reason.
        reason: String,
    },
    /// It is a symbolic link or something other than a regular file or directory.
    Kind {
        /// The path.
        path: PathBuf,
        /// Whether it should be a directory.
        dir: bool,
    },
    /// Other users can read or write it.
    Open {
        /// The path.
        path: PathBuf,
        /// Whether it is a directory.
        dir: bool,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, doing, reason } => {
                write!(f, "can't {doing} {}: {reason}", path.display())
            },
            Self::Kind { path, dir } => {
                let what = if *dir { "a directory" } else { "a regular file" };
                write!(f, "{} must be {what}, not a link or anything else", path.display())
            },
            Self::Open { path, dir } => {
                let mode = if *dir { "700" } else { "600" };
                write!(f, "{} is open to other users: run chmod {mode} on it", path.display())
            },
        }
    }
}

impl std::error::Error for Error {}

fn io(path: &Path, doing: &'static str, err: &std::io::Error) -> Error {
    Error::Io { path: path.to_owned(), doing, reason: err.to_string() }
}

/// Whether `path` exists as a private directory (`dir`) or regular file; `false` when it doesn't exist.
///
/// # Errors
///
/// When it exists but is something else, or other users can read or write it.
pub fn check(path: &Path, dir: bool) -> Result<bool, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(io(path, "read", &err)),
    };
    if !(if dir { metadata.is_dir() } else { metadata.is_file() }) {
        return Err(Error::Kind { path: path.to_owned(), dir });
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::Open { path: path.to_owned(), dir });
    }
    Ok(true)
}

/// Makes `path` a private directory, creating its parents as usual and itself with no access for others.
///
/// # Errors
///
/// When it can't be created, or exists but would be refused by [`check`].
pub fn dir(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| io(parent, "create", &err))?;
    }
    if check(path, true)? {
        return Ok(());
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {},
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(err) => return Err(io(path, "create", &err)),
    }
    check(path, true).map(|_| ())
}

/// Writes `contents` to the private file `path`, whole or not at all: it is written to a file of its own first, then
/// put in place in one step. With `replace`, by a rename over any file there; without, by a link that fails if
/// another process got there first, and then returns `false`, leaving that process's file.
///
/// # Errors
///
/// When the file can't be written or put in place.
pub fn place(path: &Path, contents: &[u8], replace: bool) -> Result<bool, Error> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = path.with_file_name(format!("{name}.{}.new", std::process::id()));
    // One left by a crashed process with the same id would otherwise stand in the way.
    let _ = fs::remove_file(&staged);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .and_then(|mut file| file.write_all(contents).and(file.sync_all()));
    if let Err(err) = written {
        let _ = fs::remove_file(&staged);
        return Err(io(&staged, "write", &err));
    }
    let placed = if replace { fs::rename(&staged, path) } else { fs::hard_link(&staged, path) };
    let _ = fs::remove_file(&staged);
    match placed {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(io(path, "write", &err)),
    }
}

/// Reads the private file `path`.
///
/// # Errors
///
/// When it doesn't exist, would be refused by [`check`], or can't be read.
pub fn read(path: &Path) -> Result<String, Error> {
    if !check(path, false)? {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        return Err(io(path, "read", &err));
    }
    fs::read_to_string(path).map_err(|err| io(path, "read", &err))
}

#[cfg(test)]
mod tests;
