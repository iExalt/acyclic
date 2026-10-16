//! What a command line that doesn't parse should have said, when it used what this build has replaced.

use std::ffi::OsString;

/// The command words and positional arguments: what comes before the first option of the command's own, with the
/// options every command takes, `--tracker`, `--actor`, `--as-user` and `--output`, set aside. A hint is a courtesy, not a parse.
fn words(args: &[OsString]) -> Vec<String> {
    let mut words = Vec::new();
    let mut skip = false;
    for arg in args.iter().skip(1) {
        let arg = arg.to_string_lossy();
        if std::mem::take(&mut skip) {
            continue;
        }
        if matches!(&*arg, "--tracker" | "--actor" | "--output" | "-o") {
            skip = true;
        } else if arg.starts_with('-') {
            // The other options every command takes are set aside; any other is the command's own.
            let global = arg == "--as-user"
                || ["--tracker=", "--actor=", "--output="]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix));
            if !global {
                break;
            }
        } else {
            words.push(arg.into_owned());
        }
    }
    words
}

/// A command the grammar replaced, and what to use now.
struct Removed {
    /// The command's words.
    words: &'static [&'static str],
    /// An option that marks the old form, for a command that is still there, such as `level --thread`.
    flag: Option<&'static str>,
    /// At least this many words mark the old form, for a command that is still there with other arguments.
    at_least: usize,
    /// What to use instead, with each command in backticks.
    instead: &'static str,
}

const REMOVED: [Removed; 8] = [
    Removed {
        words: &["item", "move"],
        flag: None,
        at_least: 0,
        instead: "`acyclic item edit <ITEM> --parent <PARENT>`, or `--no-parent` for the top level",
    },
    Removed {
        words: &["item", "state"],
        flag: None,
        at_least: 0,
        instead: "`acyclic item edit <ITEM> --state <STATE>`",
    },
    Removed {
        words: &["thread", "set"],
        flag: None,
        at_least: 0,
        instead: "`acyclic thread add <THREAD> --title <TITLE>` to create one, or `acyclic thread edit <THREAD>` to change one",
    },
    Removed {
        words: &["thread", "add"],
        flag: None,
        at_least: 4,
        instead: "`acyclic item edit <ITEM> --thread <THREAD>` (`thread add` now creates a thread: `acyclic thread add <THREAD> --title <TITLE>`)",
    },
    Removed {
        words: &["thread", "remove"],
        flag: None,
        at_least: 0,
        instead: "`acyclic item edit <ITEM> --no-thread`",
    },
    Removed {
        words: &["repo", "set"],
        flag: None,
        at_least: 0,
        instead: "`acyclic repo add <REPOSITORY> --owner <OWNER> --name <NAME> --default-branch <BRANCH> (--public | --private)` to create one, or `acyclic repo edit <REPOSITORY>` to change one",
    },
    Removed {
        words: &["repo", "attach"],
        flag: None,
        at_least: 0,
        instead: "`acyclic item edit <ITEM> --repository <REPOSITORY>`, or `acyclic thread edit <THREAD> --repository <REPOSITORY>`",
    },
    Removed {
        words: &["repo", "detach"],
        flag: None,
        at_least: 0,
        instead: "`acyclic item edit <ITEM> --no-repository <REPOSITORY>`, or `acyclic thread edit <THREAD> --no-repository <REPOSITORY>`",
    },
];

/// `level --thread <THREAD> <LEVEL>` moved to the thread.
const LEVEL_FLAG: Removed = Removed {
    words: &["level"],
    flag: Some("--thread"),
    at_least: 0,
    instead: "`acyclic thread edit <THREAD> --level <LEVEL>`, or `--no-level`",
};

/// `graph --output <FILE>` became `--file`, since `--output` says how every command prints. A value it takes, `json`
/// or `pretty`, parses as that, so only another one reaches this hint.
const GRAPH_OUTPUT: Removed = Removed {
    words: &["graph"],
    flag: Some("--output"),
    at_least: 0,
    instead: "`acyclic graph --file <FILE>` (`--output` now says how to print: `json` or `pretty`)",
};

/// A message for a command line that failed to parse because it used a replaced name or form, if it did.
#[must_use]
pub fn hint(args: &[OsString]) -> Option<String> {
    let words = words(args);
    let removed = REMOVED.iter().chain([&LEVEL_FLAG, &GRAPH_OUTPUT]).find(|removed| {
        words.starts_with(&removed.words.iter().map(ToString::to_string).collect::<Vec<_>>())
            && words.len() >= removed.at_least
            && !(removed.words == GRAPH_OUTPUT.words && says_how_to_print(args))
            && removed.flag.is_none_or(|flag| {
                args.iter().any(|arg| {
                    let arg = arg.to_string_lossy();
                    arg == flag || arg.starts_with(&format!("{flag}="))
                })
            })
    });
    if let Some(removed) = removed {
        // `thread add` still exists, so the form that is gone is named by its arguments.
        if removed.words == ["thread", "add"] {
            return Some(format!("`thread add <ITEM> <THREAD>` is gone: use {}", removed.instead));
        }
        let old = removed.words.join(" ");
        return Some(match removed.flag {
            Some(flag) => format!("`acyclic {old} {flag}` is gone: use {}", removed.instead),
            None => format!("`acyclic {old}` is gone: use {}", removed.instead),
        });
    }
    done_state(args, &words).or_else(|| link_word(&words))
}

/// Whether `--output` names a way to print, so the command line failed for another reason than graph's old flag.
fn says_how_to_print(args: &[OsString]) -> bool {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let format = |value: &str| ["json", "pretty"].contains(&value);
    args.iter().any(|arg| arg.strip_prefix("--output=").is_some_and(format))
        || args.windows(2).any(|pair| pair[0] == "--output" && format(&pair[1]))
}

/// `--state done` finishes nothing: `acyclic done` records evidence.
fn done_state(args: &[OsString], words: &[String]) -> Option<String> {
    if !matches!(words, [item, add_or_edit, ..] if item == "item" && ["add", "edit"].contains(&add_or_edit.as_str()))
    {
        return None;
    }
    let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let done = args.iter().any(|arg| arg == "--state=done")
        || args.windows(2).any(|pair| pair[0] == "--state" && pair[1] == "done");
    done.then(|| "finish an item with `acyclic done <ITEM>`, which records its evidence".to_owned())
}

fn link_word(words: &[String]) -> Option<String> {
    let [group, verb, from, kind, to, ..] = words else { return None };
    if group != "link" || !["add", "remove"].contains(&verb.as_str()) {
        return None;
    }
    if ["follows", "after"].contains(&kind.as_str()) {
        // A stored one reads as relates-to, and goes with it.
        if verb == "remove" {
            return Some(format!(
                "`{kind}` links are retired and read as relates-to: use `acyclic link remove {from} relates-to {to}`"
            ));
        }
        return Some(format!(
            "`{kind}` links are retired: use `acyclic link {verb} {from} waits-for {to}` when {from} can't start \
             until {to} is done, or `relates-to` for context"
        ));
    }
    let (_, new) = acyclic_core::import::RENAMED_LINK_KINDS.iter().find(|(old, _)| old == kind)?;
    Some(format!("`{kind}` is now `{new}`: use `acyclic link {verb} {from} {new} {to}`"))
}
