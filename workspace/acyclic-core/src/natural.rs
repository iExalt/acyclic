//! Natural order for titles: runs of digits compare as numbers, so "Step 2" sorts before "Step 10", and the rest
//! compares with ASCII case folded, so "step 1" sorts beside "Step 1".

use std::cmp::Ordering;

/// One run of a string: digits, compared by value, or anything else, compared as text. Digits sort first.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Run<'a> {
    /// A run of ASCII digits without its leading zeros: a longer run is a larger number.
    Number {
        digits: usize,
        value: &'a str,
    },
    Text(Folded<'a>),
}

/// Text compared with ASCII case folded.
#[derive(Debug)]
struct Folded<'a>(&'a str);

impl Folded<'_> {
    fn bytes(&self) -> impl Iterator<Item = u8> {
        self.0.bytes().map(|byte| byte.to_ascii_lowercase())
    }
}

impl Ord for Folded<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes().cmp(other.bytes())
    }
}

impl PartialOrd for Folded<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Folded<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Folded<'_> {}

fn runs(text: &str) -> impl Iterator<Item = Run<'_>> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let numeric = first.is_ascii_digit();
        let end = rest.find(|c: char| c.is_ascii_digit() != numeric).unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(if numeric {
            let value = run.trim_start_matches('0');
            Run::Number { digits: value.len(), value }
        } else {
            Run::Text(Folded(run))
        })
    })
}

/// Compares two strings in natural order; strings that differ only in leading zeros or case, such as "02" and "2",
/// fall back to plain order, so the order is total.
#[must_use]
pub fn cmp(a: &str, b: &str) -> Ordering {
    runs(a).cmp(runs(b)).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests;
