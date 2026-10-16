//! Sentences for refusals: the pieces that put a list of reasons on one line.

/// `items` shown in one line, separated by `separator`.
pub(crate) fn list<T: std::fmt::Display>(items: &[T], separator: &str) -> String {
    items.iter().map(ToString::to_string).collect::<Vec<_>>().join(separator)
}

/// How many of something there are, then the first three: `1 rule: a`, `5 rules: a; b; c; and 2 more`.
pub(crate) fn few<T: std::fmt::Display>(
    items: &[T],
    one: &str,
    many: &str,
    separator: &str,
) -> String {
    let noun = if items.len() == 1 { one } else { many };
    let shown = list(&items[..items.len().min(3)], separator);
    match items.len().saturating_sub(3) {
        0 => format!("{} {noun}: {shown}", items.len()),
        more => format!("{} {noun}: {shown}; and {more} more", items.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_list_shows_its_first_three() {
        assert_eq!(few(&["a"], "rule", "rules", "; "), "1 rule: a");
        assert_eq!(few(&["a", "b"], "rule", "rules", "; "), "2 rules: a; b");
        assert_eq!(
            few(&["a", "b", "c", "d", "e"], "rule", "rules", "; "),
            "5 rules: a; b; c; and 2 more"
        );
    }
}
