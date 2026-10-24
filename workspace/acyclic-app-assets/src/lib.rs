#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

/// One of the app's files, with the compressed copies the build wrote beside it.
#[derive(Debug, PartialEq, Eq)]
pub struct Asset {
    /// Its path from the app's root, such as `/index.html`.
    pub path: &'static str,
    /// Its bytes.
    pub body: &'static [u8],
    /// Its bytes compressed with Brotli, if the build wrote them.
    pub br: Option<&'static [u8]>,
    /// Its bytes compressed with gzip, if the build wrote them.
    pub gz: Option<&'static [u8]>,
    /// A hash of its bytes, for its entity tags.
    pub tag: &'static str,
}

/// The page served in place of an app that wasn't built.
pub const STUB: &[u8] = b"<!doctype html><meta charset=utf-8><title>acyclic</title>\
<p>This acyclic was built without its app. Run <code>mise run build:app</code>, then build acyclic again.\n";

include!(concat!(env!("OUT_DIR"), "/files.rs"));

/// The file at `path`, if the app has one there.
#[must_use]
pub fn get(path: &str) -> Option<&'static Asset> {
    FILES.binary_search_by(|asset| asset.path.cmp(path)).ok().map(|index| &FILES[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_sorted_and_the_root_page_is_there() {
        assert!(FILES.windows(2).all(|pair| pair[0].path < pair[1].path));
        assert!(get("/index.html").is_some());
        assert_eq!(get("/missing"), None);
    }
}
