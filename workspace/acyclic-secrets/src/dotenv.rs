//! The dotenv dialect `sops decrypt --output-type dotenv` writes, and the values read from it.

use std::fmt;

use zeroize::Zeroizing;

/// Values read from a secrets file, wiped from memory when dropped.
pub struct Secrets {
    entries: Vec<(String, Zeroizing<String>)>,
}

impl Secrets {
    /// The value under `name`, if the file has one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// The names in the file, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(key, _)| key.as_str())
    }
}

/// Lists the names only, so a value never reaches a log through `{:?}`.
impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets").field("names", &self.names().collect::<Vec<_>>()).finish()
    }
}

/// A line that isn't `NAME=value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The 1-based line number.
    pub line: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {} isn't NAME=value", self.line)
    }
}

impl std::error::Error for ParseError {}

/// Reads SOPS's dotenv: one `NAME=value` per line, split at the first `=`, with the value taken verbatim except
/// that `\n` stands for a newline, as SOPS writes it. Blank lines and lines starting with `#` are skipped; quotes and
/// `#` inside a value are part of it.
///
/// # Errors
///
/// A [`ParseError`] for a line without `=` or with an empty name.
pub fn parse(text: &str) -> Result<Secrets, ParseError> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, raw)) = line.split_once('=').filter(|(name, _)| !name.is_empty()) else {
            return Err(ParseError { line: index + 1 });
        };
        entries.push((name.to_owned(), Zeroizing::new(raw.replace("\\n", "\n"))));
    }
    Ok(Secrets { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_verbatim_after_the_first_equals_sign() {
        let secrets = parse("TOKEN=a=b#c'd\"e\nURL=https://example.test/x?y=1\n").unwrap();
        assert_eq!(secrets.get("TOKEN"), Some("a=b#c'd\"e"));
        assert_eq!(secrets.get("URL"), Some("https://example.test/x?y=1"));
        assert_eq!(secrets.get("MISSING"), None);
    }

    #[test]
    fn escaped_newlines_become_newlines_and_empty_values_are_kept() {
        let secrets = parse("# a comment\n\nPEM=line1\\nline2\nEMPTY=\n").unwrap();
        assert_eq!(secrets.get("PEM"), Some("line1\nline2"));
        assert_eq!(secrets.get("EMPTY"), Some(""));
        assert_eq!(secrets.names().collect::<Vec<_>>(), ["PEM", "EMPTY"]);
    }

    #[test]
    fn a_line_without_a_name_is_an_error() {
        assert_eq!(parse("A=1\nnot a pair\n").unwrap_err(), ParseError { line: 2 });
        assert_eq!(parse("=value\n").unwrap_err(), ParseError { line: 1 });
    }

    #[test]
    fn debug_shows_names_but_not_values() {
        let shown = format!("{:?}", parse("TOKEN=sekrit-value\n").unwrap());
        assert!(shown.contains("TOKEN") && !shown.contains("sekrit-value"), "{shown}");
    }
}
