//! Record ids. Every item, thread and repository record is named by a `UUIDv7`: its first 48 bits are the millisecond
//! it was made and the rest random, so two replicas adding records apart never collide, and ids sort by when they were
//! made. Stored ids are lowercase and hyphenated, as file names on a case-insensitive file system must be alike; an id
//! given in another case is lowercased before use.

use uuid::{Builder, Uuid, Variant};

/// A new `UUIDv7`, from the clock and the system's randomness.
#[must_use]
pub fn new() -> String {
    Uuid::now_v7().to_string()
}

/// The `UUIDv7` made at `millis` with these random bits, as a fixture makes one from its seed.
#[must_use]
pub fn at(millis: u64, random: [u8; 10]) -> String {
    Builder::from_unix_timestamp_millis(millis, &random).into_uuid().to_string()
}

/// Whether `text` has the shape of a full UUID: 36 characters, hex digits in groups of 8, 4, 4, 4 and 12, in any
/// case. Any version: the resolver takes it as an id, never as a title or an ending.
#[must_use]
pub fn is_full(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(at, b)| {
            if matches!(at, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
        })
}

/// `text` as a stored id, when it is a full `UUIDv7` in any case: lowercased.
#[must_use]
pub fn v7(text: &str) -> Option<String> {
    if !is_full(text) {
        return None;
    }
    let uuid = Uuid::try_parse(text).ok()?;
    (uuid.get_version_num() == 7 && uuid.get_variant() == Variant::RFC4122)
        .then(|| uuid.to_string())
}

#[cfg(test)]
mod tests;
