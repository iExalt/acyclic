//! What a command line that doesn't parse should have said, when it used what this build has replaced.

use std::ffi::OsString;

/// The words that name the command: the arguments that aren't options or their values. Only the options that every
/// command takes, `--tracker` and `--actor`, are known to take a value, so words after a command's own value-taking
/// option may be misread; a hint is a courtesy, not a parse.
fn words(args: &[OsString]) -> Vec<String> {
    let mut words = Vec::new();
    let mut skip = false;
    for arg in args.iter().skip(1) {
        let arg = arg.to_string_lossy();
        if std::mem::take(&mut skip) {
            continue;
        }
        if matches!(&*arg, "--tracker" | "--actor") {
            skip = true;
        } else if !arg.starts_with('-') {
            words.push(arg.into_owned());
        }
    }
    words
}

/// The link kinds the command line used to name, with their replacements.
const OLD_LINK_WORDS: [(&str, &str); 3] =
    [("blocks", "waits-for"), ("after", "follows"), ("relates", "relates-to")];

/// A message for a command line that failed to parse because it used a replaced name, if it did.
#[must_use]
pub fn hint(args: &[OsString]) -> Option<String> {
    link_word(&words(args))
}

fn link_word(words: &[String]) -> Option<String> {
    let [group, verb, from, kind, to, ..] = words else { return None };
    if group != "link" || !["add", "remove"].contains(&verb.as_str()) {
        return None;
    }
    let (_, new) = OLD_LINK_WORDS.iter().find(|(old, _)| old == kind)?;
    Some(format!("`{kind}` is now `{new}`: use `acyclic link {verb} {from} {new} {to}`"))
}
