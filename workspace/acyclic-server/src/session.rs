//! The session `acyclic up` issues: a random token in a file only the user can read, which a request presents as
//! `Authorization: Bearer <token>`.
//!
//! A browser can't be given a header, so it pairs instead: it opens a link holding a code made from the session,
//! valid for a few minutes and only once, and gets a cookie in exchange. The cookie holds a credential derived from the
//! session rather than the session itself, so rotating the session shuts the browser out too, and a link left in the
//! browser's history is spent. Browsers send a cookie for `localhost` to every port on it, and treat every such port
//! as the same site, so a page from any other local server carries the cookie too: a write that the cookie
//! authenticates must therefore come from the listener's own origin, and one without an `Origin` is refused; and no
//! page may frame the app, or it could be clicked into writes that its own origin sends.
//!
//! The file outlives the server, so a browser paired once stays paired across restarts; `acyclic up --rotate`
//! replaces it, and a server already running keeps the old token until it restarts. Whatever can read the file holds
//! the session, so it must never be mounted into a container.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use acyclic_cli::error::{CliError, exit};
use acyclic_core::private;
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

/// The session's own refusal, with how to recover from a file that others could open.
fn refused(err: &private::Error) -> CliError {
    match err {
        private::Error::Open { .. } => {
            failed(format!("{err}, then rotate the session with acyclic up --rotate"))
        },
        _ => failed(err.to_string()),
    }
}

/// The session in the home directory `home`, created if there is none and replaced if `rotate` says so, with the
/// file's path. The directory is created private and the file is written private, and either refused if it isn't.
///
/// # Errors
///
/// When the directory or the file is open to other users, isn't what it should be, or holds no token.
pub fn open(home: &Path, rotate: bool) -> Result<(Token, PathBuf), CliError> {
    let dir = home.join(DIR);
    private::dir(&dir).map_err(|err| refused(&err))?;
    let path = dir.join("session");
    if !rotate && private::check(&path, false).map_err(|err| refused(&err))? {
        return read(&path).map(|token| (token, path));
    }
    // If another server got there first, its token is the session.
    let token = Token::random()?;
    let placed = private::place(&path, format!("{}\n", token.0).as_bytes(), rotate)
        .map_err(|err| refused(&err))?;
    if placed { Ok((token, path)) } else { read(&path).map(|token| (token, path)) }
}

/// The session that already exists in the home directory `home`, with the file's path.
///
/// # Errors
///
/// When there is none, or it would be refused as [`open`] refuses it.
pub fn existing(home: &Path) -> Result<(Token, PathBuf), CliError> {
    let dir = home.join(DIR);
    let path = dir.join("session");
    let exists = |path: &Path, dir| private::check(path, dir).map_err(|err| refused(&err));
    if !exists(&dir, true)? || !exists(&path, false)? {
        return Err(CliError::new(
            "no-session",
            exit::CONFIG,
            "there is no session yet: start acyclic up first",
        ));
    }
    read(&path).map(|token| (token, path))
}

fn read(path: &Path) -> Result<Token, CliError> {
    let text = private::read(path).map_err(|err| refused(&err))?;
    Token::parse(&text).ok_or_else(|| {
        failed(format!(
            "{} holds no session token: replace it with acyclic up --rotate",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests;
