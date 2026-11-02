//! Naming a record: every argument that names one item, thread or repository record goes through [`Records::item`],
//! [`Records::thread`] or [`Records::repository`], which look only among records of that kind.
//!
//! A reference is, in order:
//!
//! 1. a full UUID, in any case: that record;
//! 2. an item's or thread's exact title, open, done or archived alike, or a repository's `owner/name` in any case:
//!    the one record that has it, refused as `ambiguous` when several do;
//! 3. the end of exactly one record's id, at least [`MIN_ENDING`] characters, in any case.
//!
//! Every rule is judged before deciding, so a reference that is one record's title and another's id ending is refused
//! as `ambiguous`, naming both; one that is the title and an ending of the same record names it. A title matches as
//! typed: only the comparison of an id ending is lowercased, never the reference as a whole. A deleted record is never
//! a candidate, except a thread items still name: like a thread with no record, it stays one so that `thread show`
//! finds its members and pretty output can name it, and a write that would move an item into it refuses it as missing.
//!
//! A removal (`--no-waits-for`, `--no-repository`) first looks among the ids the record holds, by exact id or an
//! ending, with [`held`], so it lets go of an element whose record is gone; only when none matches do these rules
//! apply.
//!
//! Pretty output names items and threads by [`endings`] over the same candidates, lengthened past any other record's
//! title, so every ending it prints names its record, and repositories by their `owner/name`. The exception is an id
//! shorter than [`MIN_ENDING`], which no command makes: it prints whole, and names nothing.

use std::collections::BTreeMap;

use acyclic_core::ids;
use acyclic_core::model::Model;
use serde_json::{Value as Json, json};

use crate::error::{CliError, exit};

/// The fewest characters of an id that name a record.
pub(crate) const MIN_ENDING: usize = 6;

/// A record a reference may name: its id, and the names it may be called by.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate<'a> {
    id: &'a str,
    /// Its titles, one unless replicas disagree, or its `owner/name`.
    names: Vec<String>,
}

/// The kind of record an argument names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Item,
    Thread,
    Repository,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Thread => "thread",
            Self::Repository => "repository",
        }
    }

    /// The key a candidate's name sits under in an error's details.
    fn name_key(self) -> &'static str {
        if self == Self::Repository { "name" } else { "title" }
    }
}

/// The records of a model that references may name.
pub(crate) struct Records<'a> {
    items: Vec<Candidate<'a>>,
    threads: Vec<Candidate<'a>>,
    repositories: Vec<Candidate<'a>>,
}

impl<'a> Records<'a> {
    /// Every item, thread and repository record in `model`. A thread an item names but that has no record of its own
    /// is a thread too, with no title.
    pub(crate) fn of(model: &'a Model) -> Self {
        let items = model
            .items
            .values()
            .map(|item| Candidate {
                id: &item.id,
                names: item.title.all().into_iter().cloned().collect(),
            })
            .collect();
        let mut threads: BTreeMap<&str, Vec<String>> = model
            .threads
            .values()
            .map(|thread| (thread.id.as_str(), thread.title.all().into_iter().cloned().collect()))
            .collect();
        for item in model.items.values() {
            for thread in item.thread.all() {
                threads.entry(thread.as_str()).or_default();
            }
        }
        let repositories = model
            .repositories
            .values()
            .map(|record| Candidate {
                id: &record.id,
                names: repository_name(record).into_iter().collect(),
            })
            .collect();
        Self {
            items,
            threads: threads.into_iter().map(|(id, names)| Candidate { id, names }).collect(),
            repositories,
        }
    }

    /// The item `reference` names.
    pub(crate) fn item(&self, reference: &str) -> Result<String, CliError> {
        resolve(Kind::Item, reference, &self.items)
    }

    /// The thread `reference` names.
    pub(crate) fn thread(&self, reference: &str) -> Result<String, CliError> {
        resolve(Kind::Thread, reference, &self.threads)
    }

    /// The repository record `reference` names.
    pub(crate) fn repository(&self, reference: &str) -> Result<String, CliError> {
        resolve(Kind::Repository, reference, &self.repositories)
    }

    /// The items, or with `items` false the threads, each by the shortest ending of its id that names it.
    pub(crate) fn endings(&self, items: bool) -> BTreeMap<String, String> {
        let candidates = if items { &self.items } else { &self.threads };
        let mut found = endings(candidates.iter().map(|c| c.id));
        // An ending that is another record's title would resolve as ambiguous: it grows until it isn't, at the
        // latest to the whole id.
        let titled = |id: &str, ending: &str| {
            candidates.iter().any(|c| c.id != id && c.names.iter().any(|name| name == ending))
        };
        for (id, ending) in &mut found {
            while ending.len() < id.len() && titled(id, ending) {
                id[id.len() - ending.len() - 1..].clone_into(ending);
            }
        }
        found
    }
}

/// A repository record's `owner/name`, when it has one of each.
pub(crate) fn repository_name(record: &acyclic_core::model::Repository) -> Option<String> {
    Some(format!("{}/{}", record.owner.one()?, record.name.one()?))
}

/// The element of `held`, a record's own list, that `reference` names by its exact id or by an ending of at least
/// [`MIN_ENDING`] characters, so a removal finds an element whose record is gone; `None` when none or several do.
pub(crate) fn held<'h>(
    held: impl IntoIterator<Item = &'h String>,
    reference: &str,
) -> Option<String> {
    let lower = reference.to_ascii_lowercase();
    let held: Vec<&String> = held.into_iter().collect();
    if let Some(exact) = held.iter().find(|id| ***id == lower) {
        return Some((*exact).clone());
    }
    if lower.len() < MIN_ENDING {
        return None;
    }
    match held.iter().filter(|id| id.ends_with(&lower)).collect::<Vec<_>>()[..] {
        [one] => Some((*one).clone()),
        _ => None,
    }
}

fn resolve(kind: Kind, reference: &str, candidates: &[Candidate<'_>]) -> Result<String, CliError> {
    if ids::is_full(reference) {
        let id = reference.to_ascii_lowercase();
        return match candidates.iter().find(|c| c.id == id) {
            Some(found) => Ok(found.id.to_owned()),
            None => Err(not_found(kind, reference, &[])),
        };
    }
    let named = |name: &String| match kind {
        Kind::Repository => name.eq_ignore_ascii_case(reference),
        Kind::Item | Kind::Thread => name == reference,
    };
    let by_name: Vec<&Candidate<'_>> =
        candidates.iter().filter(|c| c.names.iter().any(named)).collect();
    let lower = reference.to_ascii_lowercase();
    let ends = |c: &&Candidate<'_>| !lower.is_empty() && c.id.ends_with(&lower);
    let by_ending: Vec<&Candidate<'_>> = candidates.iter().filter(ends).collect();
    let long = reference.len() >= MIN_ENDING;
    match (&by_name[..], &by_ending[..]) {
        ([one], []) => Ok(one.id.to_owned()),
        ([one], [same]) if one.id == same.id => Ok(one.id.to_owned()),
        ([_, _, ..], _) => Err(ambiguous(
            kind,
            &format!(
                "{reference:?} is the {} of {} {}s",
                name_word(kind),
                by_name.len(),
                kind.noun()
            ),
            &by_name,
        )),
        ([one], _) if long => {
            let mut both = vec![*one];
            both.extend(by_ending.iter().filter(|c| c.id != one.id));
            Err(ambiguous(
                kind,
                &format!(
                    "{reference:?} is the {} of one {} and the end of another's id",
                    name_word(kind),
                    kind.noun()
                ),
                &both,
            ))
        },
        // Too short to be an ending, so it is that name.
        ([one], _) => Ok(one.id.to_owned()),
        ([], [one]) if long => Ok(one.id.to_owned()),
        ([], [_, _, ..]) if long => Err(ambiguous(
            kind,
            &format!("{reference:?} ends the ids of {} {}s", by_ending.len(), kind.noun()),
            &by_ending,
        )),
        ([], [_, ..]) => Err(ambiguous(
            kind,
            &format!(
                "{reference:?} is too short: an id ending takes at least {MIN_ENDING} characters"
            ),
            &by_ending,
        )),
        ([], []) => {
            // A repository named by its name alone is meant as `owner/name`.
            let meant: Vec<&Candidate<'_>> = if kind == Kind::Repository {
                candidates
                    .iter()
                    .filter(|c| {
                        c.names.iter().any(|name| {
                            name.rsplit_once('/')
                                .is_some_and(|(_, n)| n.eq_ignore_ascii_case(reference))
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Err(not_found(kind, reference, &meant))
        },
    }
}

fn name_word(kind: Kind) -> &'static str {
    if kind == Kind::Repository { "owner/name" } else { "title" }
}

fn details(kind: Kind, candidates: &[&Candidate<'_>]) -> Json {
    let listed: Vec<Json> =
        candidates.iter().map(|c| json!({"id": c.id, kind.name_key(): c.names.first()})).collect();
    json!({"candidates": listed})
}

/// Candidates as a message lists them: each id, with its name when it has one.
fn listing(candidates: &[&Candidate<'_>]) -> String {
    let each: Vec<String> = candidates
        .iter()
        .map(|c| match c.names.first() {
            Some(name) => format!("{} ({name})", c.id),
            None => c.id.to_owned(),
        })
        .collect();
    each.join(", ")
}

fn ambiguous(kind: Kind, why: &str, candidates: &[&Candidate<'_>]) -> CliError {
    CliError::new(
        "ambiguous",
        exit::USAGE,
        format!("{why}: {}; name one by its full id or a longer ending", listing(candidates)),
    )
    .with(details(kind, candidates))
}

fn not_found(kind: Kind, reference: &str, meant: &[&Candidate<'_>]) -> CliError {
    let message = match kind {
        Kind::Item => format!("no item {reference}"),
        Kind::Thread => format!(
            "no thread {reference}; `acyclic thread list` lists them, and `acyclic thread add --title <TITLE>` adds one"
        ),
        Kind::Repository if !meant.is_empty() => format!(
            "no repository {reference}: a repository is named by its owner/name, such as {}",
            listing(meant)
        ),
        Kind::Repository => format!(
            "no repository {reference}; `acyclic repository list` lists them, and `acyclic repository add --name \
             <OWNER/NAME> --default-branch <BRANCH>` adds one"
        ),
    };
    let error = CliError::new("not-found", exit::NOT_FOUND, message);
    if meant.is_empty() { error } else { error.with(details(kind, meant)) }
}

/// Each id by the shortest ending, at least [`MIN_ENDING`] characters, that no other of `ids` shares: the whole id when it is shorter.
pub(crate) fn endings<'i>(ids: impl IntoIterator<Item = &'i str>) -> BTreeMap<String, String> {
    let mut reversed: Vec<Vec<u8>> = ids.into_iter().map(|id| id.bytes().rev().collect()).collect();
    reversed.sort();
    reversed.dedup();
    let common = |a: &[u8], b: &[u8]| a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let mut found = BTreeMap::new();
    for (at, id) in reversed.iter().enumerate() {
        let before = at.checked_sub(1).map_or(0, |b| common(id, &reversed[b]));
        let after = reversed.get(at + 1).map_or(0, |next| common(id, next));
        let length = (before.max(after) + 1).max(MIN_ENDING).min(id.len());
        let id: String = id.iter().rev().map(|b| char::from(*b)).collect();
        found.insert(id.clone(), id[id.len() - length..].to_owned());
    }
    found
}

#[cfg(test)]
mod tests;
