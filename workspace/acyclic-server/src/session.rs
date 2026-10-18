//! The session `acyclic up` issues: a random token in a file only the user can read, which a request presents as
//! `Authorization: Bearer <token>`.
//!
//! A browser can't be given a header, so it pairs instead: it opens a link holding a code made from the session,
//! valid for a few minutes and only once, and gets a cookie in exchange. The cookie holds a credential derived from the
//! session rather than the session itself, so rotating the session shuts the browser out too, and a link left in the
//! browser's history is spent. Browsers send a cookie for `localhost` to every port on it, and treat every such port
//! as the same site, so a page from any other local server carries the cookie too: a write that the cookie
//! authenticates must therefore come from the listener's own origin, and one without an `Origin` is refused.
//!
//! The file outlives the server, so a browser paired once stays paired across restarts; `acyclic up --rotate`
//! replaces it, and a server already running keeps the old token until it restarts. Whatever can read the file holds
//! the session, so it must never be mounted into a container.

use std::fmt::{self, Write as _};
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use acyclic_cli::error::{CliError, exit};
use sha2::{Digest as _, Sha256};

/// The directory under the home directory that holds the session.
const DIR: &str = ".config/acyclic/up";

/// The token's length in bytes; the file holds it as twice as many lowercase hex digits.
const BYTES: usize = 32;

/// A session token, as the hex text a request presents.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    /// A new random token.
    fn random() -> Result<Self, CliError> {
        let mut bytes = [0; BYTES];
        getrandom::fill(&mut bytes).map_err(|err| {
            CliError::new("session", exit::CONFIG, format!("no randomness for the session: {err}"))
        })?;
        Ok(Self(hex(&bytes)))
    }

    /// The token in `text`, which must be exactly the hex digits, with at most a final newline.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let hex = |byte: &u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte);
        (text.len() == 2 * BYTES && text.as_bytes().iter().all(hex)).then(|| Self(text.to_owned()))
    }

    /// The credential a paired browser's cookie holds.
    pub(crate) fn browser(&self) -> String {
        hex(&hmac(self.0.as_bytes(), b"browser"))
    }

    /// A pairing code made at `now`, in milliseconds, with the random `nonce`.
    pub(crate) fn code(&self, now: i64, nonce: [u8; 8]) -> String {
        let stamped = format!("{now}.{}", hex(&nonce));
        format!("{stamped}.{}", hex(&hmac(self.0.as_bytes(), format!("pair:{stamped}").as_bytes())))
    }

    /// Whether `code` is one this session made, between [`CODE_LIFE`] before `now` and [`CODE_SKEW`] after it,
    /// returning when it was made.
    pub(crate) fn verify(&self, code: &str, now: i64) -> Option<i64> {
        let (stamped, mac) = code.rsplit_once('.')?;
        let (made, nonce) = stamped.split_once('.')?;
        let made: i64 = made.parse().ok()?;
        if nonce.len() != 16
            || made > now.saturating_add(CODE_SKEW)
            || made < now.saturating_sub(CODE_LIFE)
        {
            return None;
        }
        let expected = hex(&hmac(self.0.as_bytes(), format!("pair:{stamped}").as_bytes()));
        equal(expected.as_bytes(), mac.as_bytes()).then_some(made)
    }

    /// Whether `presented` is this token, taking the same time whatever it holds of the right length.
    pub(crate) fn matches(&self, presented: &[u8]) -> bool {
        equal(self.0.as_bytes(), presented)
    }
}

/// How long a pairing code stays valid, in milliseconds.
pub(crate) const CODE_LIFE: i64 = 5 * 60 * 1000;

/// How far ahead of the server's clock a pairing code may be dated, in milliseconds.
pub(crate) const CODE_SKEW: i64 = 30 * 1000;

/// A new random nonce for a pairing code.
///
/// # Errors
///
/// When the system has no randomness to give.
pub fn nonce() -> Result<[u8; 8], CliError> {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).map_err(|err| {
        CliError::new("session", exit::CONFIG, format!("no randomness for the code: {err}"))
    })?;
    Ok(bytes)
}

/// The pairing codes already spent, each kept until it would have expired anyway. A server that restarts forgets
/// them, so a code can be spent again within its few minutes after a restart.
#[derive(Debug, Default)]
pub(crate) struct Spent(std::sync::Mutex<std::collections::HashMap<String, i64>>);

impl Spent {
    /// Spends `code`, made at `made`, returning whether it was still unspent; drops the codes expired at `now`.
    pub(crate) fn spend(&self, code: &str, made: i64, now: i64) -> bool {
        let mut spent = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        spent.retain(|_, made| made.saturating_add(CODE_LIFE) >= now);
        spent.insert(code.to_owned(), made).is_none()
    }
}

/// HMAC-SHA256 (RFC 2104) of `message` under `key`.
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0; 64];
    if key.len() > block.len() {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner =
        Sha256::new().chain_update(block.map(|byte| byte ^ 0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(block.map(|byte| byte ^ 0x5c)).chain_update(inner).finalize().into()
}

/// Bytes as lowercase hex digits.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Compares two byte strings without stopping at the first difference, so the time taken says nothing of where they
/// differ. Their lengths aren't secret.
pub(crate) fn equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let difference = left.iter().zip(right).fold(0, |acc, (l, r)| acc | (l ^ r));
    std::hint::black_box(difference) == 0
}

fn failed(message: String) -> CliError {
    CliError::new("session", exit::CONFIG, message)
}

/// Refuses `path` unless it is a `kind` (not a symbolic link) that no one but its owner can read or write.
fn private(path: &Path, dir: bool) -> Result<bool, CliError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(failed(format!("can't read {}: {err}", path.display()))),
    };
    let kind = if dir { metadata.is_dir() } else { metadata.is_file() };
    if !kind {
        let what = if dir { "a directory" } else { "a regular file" };
        return Err(failed(format!(
            "{} must be {what}, not a link or anything else",
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        let mode = if dir { "700" } else { "600" };
        return Err(failed(format!(
            "{} is open to other users: run chmod {mode} on it, then rotate the session with acyclic up --rotate",
            path.display()
        )));
    }
    Ok(true)
}

/// The session in the home directory `home`, created if there is none and replaced if `rotate` says so, with the
/// file's path. The directory is created private and the file is written private, and either refused if it isn't.
///
/// # Errors
///
/// When the directory or the file is open to other users, isn't what it should be, or holds no token.
pub fn open(home: &Path, rotate: bool) -> Result<(Token, PathBuf), CliError> {
    let dir = home.join(DIR);
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| failed(format!("can't create {}: {err}", parent.display())))?;
    }
    if !private(&dir, true)? {
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {},
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(err) => return Err(failed(format!("can't create {}: {err}", dir.display()))),
        }
        private(&dir, true)?;
    }
    let path = dir.join("session");
    if !rotate && private(&path, false)? {
        return read(&path).map(|token| (token, path));
    }
    // The token is written whole to a file of its own, then put in place in one step: by a link that fails if
    // another server got there first, whose token is then the session, or by a rename when rotating.
    let token = Token::random()?;
    let staged = dir.join(format!("session.{}.new", std::process::id()));
    // One left by a crashed server with the same process id would otherwise stand in the way.
    let _ = fs::remove_file(&staged);
    let written =
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged).and_then(
            |mut file| file.write_all(format!("{}\n", token.0).as_bytes()).and(file.sync_all()),
        );
    if let Err(err) = written {
        let _ = fs::remove_file(&staged);
        return Err(failed(format!("can't write {}: {err}", staged.display())));
    }
    let placed = if rotate { fs::rename(&staged, &path) } else { fs::hard_link(&staged, &path) };
    let _ = fs::remove_file(&staged);
    match placed {
        Ok(()) => Ok((token, path)),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            private(&path, false)?;
            read(&path).map(|token| (token, path))
        },
        Err(err) => Err(failed(format!("can't write {}: {err}", path.display()))),
    }
}

fn read(path: &Path) -> Result<Token, CliError> {
    let text = fs::read_to_string(path)
        .map_err(|err| failed(format!("can't read {}: {err}", path.display())))?;
    Token::parse(&text).ok_or_else(|| {
        failed(format!(
            "{} holds no session token: replace it with acyclic up --rotate",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests;
