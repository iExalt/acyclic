//! Files only the user can read or write, for the secrets acyclic keeps under `~/.config/acyclic/`: the session `acyclic
//! up` issues. It lives in core, beside [`crate::locate`], which also decides paths there. Like
//! every acyclic build target, it is Unix only.
//!
//! A directory is created with no access for others, and a file is written with none.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

/// Why reading, creating or writing a private file or directory failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The path.
    pub path: PathBuf,
    /// What was being done.
    pub doing: &'static str,
    /// The system's reason.
    pub reason: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "can't {} {}: {}", self.doing, self.path.display(), self.reason)
    }
}

impl std::error::Error for Error {}

fn io(path: &Path, doing: &'static str, err: &std::io::Error) -> Error {
    Error { path: path.to_owned(), doing, reason: err.to_string() }
}

/// Makes `path` a directory, creating its parents as usual and itself, when it is new, with no access for others.
///
/// # Errors
///
/// When it can't be created.
pub fn dir(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| io(parent, "create", &err))?;
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(io(path, "create", &err))
        },
        _ => Ok(()),
    }
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
/// When it doesn't exist or can't be read.
pub fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|err| io(path, "read", &err))
}

#[cfg(test)]
mod tests;
