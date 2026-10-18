//! Run tokens: what the operator gives each run so the run's writes can be traced to it alone.
//!
//! A token is `v1.<claims>.<signature>`, both parts in lowercase hex: the claims are JSON naming the run, its thread,
//! the token's own id and its lifetime, and the signature is Ed25519 over the claims' exact bytes. The operator signs;
//! a server checks with the public key alone, so it can't mint one. The signing key lives in
//! `~/.config/acyclic/operator/key`, readable by the user alone, and is made on first use.

use std::fmt;
use std::path::{Path, PathBuf};

use acyclic_core::private;
use acyclic_core::state::Millis;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// The token format's version, its first part.
const VERSION: &str = "v1";

/// The longest a token may live, in milliseconds: a week. A run that outlives it is given a new one.
pub const MAX_LIFE: Millis = 7 * 24 * 60 * 60 * 1000;

/// How far ahead of the checker's clock a token may be issued, in milliseconds.
const SKEW: Millis = 30 * 1000;

/// The key's file, under the home directory.
const KEY_PATH: &str = ".config/acyclic/operator/key";

/// What a token says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claims {
    /// The token's own id, random.
    pub id: String,
    /// The run it was given to.
    pub run: String,
    /// The thread the run works on.
    pub thread: String,
    /// When it was issued, in milliseconds.
    pub issued: Millis,
    /// When it stops being accepted, in milliseconds.
    pub expires: Millis,
}

/// Why a token or the key was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a token of a version this build knows.
    Version,
    /// Not in a token's shape, or its claims aren't readable.
    Malformed,
    /// The signature isn't the operator's.
    Signature,
    /// Expired, or issued in the future.
    Expired,
    /// A lifetime that isn't between nothing and [`MAX_LIFE`].
    Life(Millis),
    /// A token for no run or no thread.
    Unnamed,
    /// The system gave no randomness.
    Random(String),
    /// The key file was refused or unreadable.
    Key(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version => write!(f, "the token isn't a {VERSION} run token"),
            Self::Malformed => write!(f, "the token is malformed"),
            Self::Signature => write!(f, "the token isn't signed by this operator"),
            Self::Expired => write!(f, "the token has expired or isn't valid yet"),
            Self::Life(life) => {
                write!(f, "a token lives more than nothing and at most {MAX_LIFE} ms, not {life}")
            },
            Self::Unnamed => write!(f, "a token names a run and a thread"),
            Self::Random(reason) => write!(f, "no randomness: {reason}"),
            Self::Key(reason) => write!(f, "the operator's key: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// The operator's signing key.
pub struct Signer(SigningKey);

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Signer(..)")
    }
}

impl Signer {
    /// The key kept in the home directory `home`, made if there is none, with its file's path.
    ///
    /// # Errors
    ///
    /// When the file or its directory is open to other users or isn't what it should be, holds no key, or can't be
    /// written.
    pub fn open(home: &Path) -> Result<(Self, PathBuf), Error> {
        let path = home.join(KEY_PATH);
        let key = |err: private::Error| Error::Key(err.to_string());
        if let Some(dir) = path.parent() {
            private::dir(dir).map_err(key)?;
        }
        if !private::check(&path, false).map_err(key)? {
            let seed = random::<32>()?;
            // If another process got there first, its key is the operator's.
            private::place(&path, format!("{}\n", hex(&seed)).as_bytes(), false).map_err(key)?;
        }
        let text = private::read(&path).map_err(key)?;
        let seed = unhex(text.strip_suffix('\n').unwrap_or(&text))
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| Error::Key(format!("{} holds no key", path.display())))?;
        Ok((Self(SigningKey::from_bytes(&seed)), path))
    }

    /// A key from its 32 secret bytes.
    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    /// What checks this key's tokens.
    #[must_use]
    pub fn verifier(&self) -> Verifier {
        Verifier(self.0.verifying_key())
    }

    /// A token for `run` on `thread`, issued at `now` and living `life` milliseconds.
    ///
    /// # Errors
    ///
    /// On a lifetime outside 1 to [`MAX_LIFE`], an empty run or thread, or when the system has no randomness for the
    /// token's id.
    ///
    /// # Panics
    ///
    /// Never: claims always serialize.
    pub fn mint(
        &self,
        run: &str,
        thread: &str,
        now: Millis,
        life: Millis,
    ) -> Result<String, Error> {
        if !(1..=MAX_LIFE).contains(&life) {
            return Err(Error::Life(life));
        }
        if run.is_empty() || thread.is_empty() {
            return Err(Error::Unnamed);
        }
        let claims = Claims {
            id: hex(&random::<16>()?),
            run: run.to_owned(),
            thread: thread.to_owned(),
            issued: now,
            expires: now.saturating_add(life),
        };
        let bytes = serde_json::to_vec(&claims).expect("claims always serialize");
        let signature = self.0.sign(&bytes);
        Ok(format!("{VERSION}.{}.{}", hex(&bytes), hex(&signature.to_bytes())))
    }
}

/// The operator's public key, which checks its tokens and can't mint them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verifier(VerifyingKey);

impl Verifier {
    /// The token's claims, if this operator signed it and it is valid at `now`. The version is checked before
    /// anything else, and the signature before the claims are read.
    ///
    /// # Errors
    ///
    /// Why the token is refused.
    pub fn verify(&self, token: &str, now: Millis) -> Result<Claims, Error> {
        let mut parts = token.split('.');
        if parts.next() != Some(VERSION) {
            return Err(Error::Version);
        }
        let (Some(claims), Some(signature), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(Error::Malformed);
        };
        let claims = unhex(claims).ok_or(Error::Malformed)?;
        let signature = unhex(signature)
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
            .ok_or(Error::Malformed)?;
        self.0
            .verify_strict(&claims, &Signature::from_bytes(&signature))
            .map_err(|_| Error::Signature)?;
        let claims: Claims = serde_json::from_slice(&claims).map_err(|_| Error::Malformed)?;
        if now >= claims.expires || claims.issued > now.saturating_add(SKEW) {
            return Err(Error::Expired);
        }
        Ok(claims)
    }
}

/// `N` random bytes.
fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|err| Error::Random(err.to_string()))?;
    Ok(bytes)
}

/// Bytes as lowercase hex digits.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| [DIGITS[usize::from(byte >> 4)], DIGITS[usize::from(byte & 0xf)]])
        .map(char::from)
        .collect()
}

/// Lowercase hex digits as bytes, or `None` for anything else.
fn unhex(text: &str) -> Option<Vec<u8>> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes.chunks(2).map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?)).collect()
}

#[cfg(test)]
mod tests;
