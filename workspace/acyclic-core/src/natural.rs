//! Natural order for titles: runs of digits compare as numbers, so "Step 2" sorts before "Step 10", and the rest
//! compares with ASCII case folded, so "step 1" sorts beside "Step 1".

use std::cmp::Ordering;

/// One run of a string: digits, compared by value, or anything else, compared as text. Digits sort first.
enum Run<'a> {
    /// A run of ASCII digits without its leading zeros: a longer run is a larger number.
    Number {
        digits: usize,
        value: &'a str,
    },
    Text(&'a str),
}

impl Run<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number { digits, value }, Self::Number { digits: d, value: v }) => {
                (digits, value).cmp(&(d, v))
            },
            (Self::Number { .. }, Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number { .. }) => Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a
                .bytes()
                .map(|c| c.to_ascii_lowercase())
                .cmp(b.bytes().map(|c| c.to_ascii_lowercase())),
        }
    }
}

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
            Run::Text(run)
        })
    })
}

/// Compares two strings in natural order; strings that differ only in leading zeros or case, such as "02" and "2",
/// fall back to plain order, so the order is total.
#[must_use]
pub fn cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (runs(a), runs(b));
    loop {
        let order = match (x.next(), y.next()) {
            (Some(p), Some(q)) => p.cmp(&q),
            (p, q) => return p.is_some().cmp(&q.is_some()).then_with(|| a.cmp(b)),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

#[cfg(test)]
mod tests;
