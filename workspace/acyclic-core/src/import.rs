//! The import format: a whole plan as one JSON document, every record named by its `UUIDv7` id.
//!
//! The format is versioned and kept apart from the model, so the model can change without breaking documents
//! written against an earlier version. `acyclic import` maps a validated document into the model. Since records are
//! named by their ids, importing a document again updates the records it made, and a document may refer to records
//! already in the tracker as well as its own.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids;

mod apply;

pub use apply::{ApplyError, Report, apply, preview};

/// The value of the document's `format` field.
pub const FORMAT: &str = "acyclic-import";

/// The one version this build reads.
pub const VERSION: u32 = 3;

/// An annotated version 3 document, as `acyclic import --schema` prints it: lines starting with `//` are notes to
/// remove, since JSON has no comments.
pub const SCHEMA: &str = r#"// An acyclic import document: a whole plan, every record named by its own UUIDv7 id.
// `acyclic id 20` prints twenty new ids to name records with. Importing the document again updates the same records.
// A reference (parent, thread, waits_for, evidence) is an id, of a record in the document or already
// in the tracker. A field left out keeps what the record holds; a new record takes the default.
// JSON has no comments: remove the lines that start with //.
{
  "format": "acyclic-import",
  "version": 3,
  // A code repository that evidence can name, as "owner/name"; all three fields are required.
  "repositories": [
    {"id": "01900000-0000-7000-8000-000000000001", "name": "me/garden", "default_branch": "main"}
  ],
  "items": [
    // Required: id and title. "state" is open, parked, done or canceled, open for a new item. Canceled skips the check
    // for another holder's claim that `item edit --state canceled` makes, as done work skips the one finishing makes.
    // Optional: parent, thread, notes, waits_for (the items it can't start before they are done),
    // and evidence on done items: {"commit": "<repository id>@<sha>", "text": "…"}, either part or both. A list
    // given replaces the record's: [] clears it.
    {"id": "01900000-0000-7000-8000-000000000003", "title": "Build the beds",
     "thread": "01900000-0000-7000-8000-000000000002"},
    {"id": "01900000-0000-7000-8000-000000000004", "title": "Plant the seeds",
     "thread": "01900000-0000-7000-8000-000000000002", "waits_for": ["01900000-0000-7000-8000-000000000003"]}
  ],
  // A thread groups items for one continuing agent conversation; an item joins one with its "thread". Optional:
  // notes ("brief", as older documents say, reads as notes).
  "threads": [
    {"id": "01900000-0000-7000-8000-000000000002", "title": "Spring planting"}
  ]
}
// On the command line, quote a title in single quotes ('Plant the seeds'), so the shell leaves its quotes and
// backticks alone, or name a record by the last 6 or more characters of its id.
"#;

/// A plan to import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    /// Always [`FORMAT`].
    pub format: String,
    /// The format version, [`VERSION`].
    pub version: u32,
    /// Code repositories that evidence names.
    #[serde(default)]
    pub repositories: Vec<Repository>,
    /// Items, in any order.
    #[serde(default)]
    pub items: Vec<Item>,
    /// Threads grouping items.
    #[serde(default)]
    pub threads: Vec<Thread>,
}

/// A code repository record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    /// Its id.
    pub id: String,
    /// Its `owner/name`.
    pub name: String,
    /// The default branch.
    pub default_branch: String,
}

/// The item's state. Stage and readiness are derived on read, never imported.
///
/// This compatibility re-export keeps import documents and the model on the same state type.
pub use crate::model::ItemState as State;

/// What shows a done item was done: a commit, words, or both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The commit that completed the item, as `<owner>/<name>@<sha>`, naming a repository in the document or the
    /// tracker, or as `<repository id>@<sha>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Evidence in words, such as where a record lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A work item at any depth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Its id.
    pub id: String,
    /// A short title.
    pub title: String,
    /// Design notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The item's state: `open`, `parked`, `done` or `canceled`; left out, an existing item keeps its own and a new
    /// one is open. `backlog`, as older documents say, reads as open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    /// The parent item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The thread it is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The items it waits for, which it can't start until they are done: the whole list, replacing the item's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_for: Option<Vec<String>>,
    /// Evidence of completion, for done items only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// A thread: items for one continuing agent conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thread {
    /// Its id.
    pub id: String,
    /// A short title.
    pub title: String,
    /// Its notes; `brief`, as older documents say, reads as notes.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "brief")]
    pub notes: Option<String>,
}

/// Why a document couldn't be read.
#[derive(Debug)]
pub enum ParseError {
    /// Not JSON, or not shaped like an import.
    Json(serde_json::Error),
    /// The `format` field is missing or wrong.
    NotAnImport,
    /// Any version but [`VERSION`].
    UnsupportedVersion {
        /// The document's version.
        found: u64,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(err) => write!(
                f,
                "not a valid import document: {err}; `acyclic import --schema` shows the format"
            ),
            Self::NotAnImport => write!(
                f,
                "not an import document: `format` must be {FORMAT:?}; `acyclic import --schema` shows an example"
            ),
            Self::UnsupportedVersion { found } => {
                write!(
                    f,
                    "import version {found} is not supported; this build reads version {VERSION}"
                )
            },
        }
    }
}

impl std::error::Error for ParseError {}

/// The kind of record an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyKind {
    /// A repository.
    Repository,
    /// An item.
    Item,
    /// A thread.
    Thread,
}

impl fmt::Display for KeyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Repository => "repository",
            Self::Item => "item",
            Self::Thread => "thread",
        })
    }
}

/// A rule a document breaks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationError {
    /// An id or reference that isn't a `UUIDv7`.
    InvalidId {
        /// Where it is, such as `item …, parent`.
        at: String,
        /// What it holds.
        id: String,
    },
    /// Two records of one kind share an id.
    DuplicateId(KeyKind, String),
    /// A reference names no record of its kind, in the document or the tracker.
    UnknownReference {
        /// Where the reference is, such as `item …, parent`.
        at: String,
        /// The kind it should name.
        kind: KeyKind,
        /// The missing id.
        id: String,
    },
    /// An item waits for itself.
    SelfLink(String),
    /// An item waits for one above or below it.
    LineageLink {
        /// The waiting item.
        item: String,
        /// The item it waits for.
        waits_for: String,
    },
    /// An item names the same item twice in its waits-for.
    DuplicateLink {
        /// The waiting item.
        item: String,
        /// The item it names twice.
        waits_for: String,
    },
    /// Evidence on an item that isn't done.
    EvidenceNotDone(String),
    /// Done on an item with children, which reads done once they are.
    DoneParent(String),
    /// Canceled on an item that is done, or a parent that reads done: it is reopened first.
    CanceledDone(String),
    /// A new item with no thread whose siblings are in different threads, or some in none: the document says which.
    ThreadUnclear {
        /// The new item.
        item: String,
        /// The parent whose children they are.
        parent: String,
    },
    /// A repository's name that isn't `owner/name`.
    InvalidRepositoryName {
        /// The repository.
        repository: String,
        /// The name.
        name: String,
    },
    /// Two repositories would share an `owner/name`, in any case: one the document names, and another in it or in the
    /// tracker.
    RepositoryNameTaken {
        /// The document's repository.
        repository: String,
        /// The name, as the document gives it.
        name: String,
        /// The other repository that has it.
        other: String,
    },
    /// An evidence commit that isn't `<owner>/<name>@<sha>` or `<repository id>@<sha>`.
    InvalidCommit {
        /// The item.
        item: String,
        /// The commit.
        commit: String,
    },
    /// Evidence naming a repository record that lacks fields it needs to be verified.
    IncompleteRepository {
        /// The item.
        item: String,
        /// The repository record.
        repository: String,
        /// The fields it lacks.
        missing: Vec<String>,
    },
}

impl ValidationError {
    /// Whether the error is an id that isn't a `UUIDv7`.
    #[must_use]
    pub fn is_invalid_id(&self) -> bool {
        matches!(self, Self::InvalidId { .. })
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { at, id } => {
                write!(f, "{at}: {id:?} isn't a UUIDv7; `acyclic id` prints new ones")
            },
            Self::DuplicateId(kind, id) => write!(f, "{kind} id {id} is used twice"),
            Self::UnknownReference { at, kind, id } => {
                write!(f, "{at} names unknown {kind} {id}")
            },
            Self::SelfLink(item) => write!(f, "item {item} waits for itself"),
            Self::LineageLink { item, waits_for } => write!(
                f,
                "item {item} waits for {waits_for}, which is above or below it, so neither could ever be done"
            ),
            Self::DuplicateLink { item, waits_for } => {
                write!(f, "item {item} names {waits_for} twice in waits_for")
            },
            Self::EvidenceNotDone(item) => write!(f, "item {item} has evidence but isn't done"),
            Self::DoneParent(item) => {
                write!(
                    f,
                    "item {item} has children, so it reads done once they are: leave its state out"
                )
            },
            Self::CanceledDone(item) => {
                write!(f, "item {item} is done, so it can't be canceled until it is reopened")
            },
            Self::ThreadUnclear { item, parent } => write!(
                f,
                "item {item}: the other children of {parent} are in different threads, or some in none, so give it a \
                 thread"
            ),
            Self::InvalidRepositoryName { repository, name } => {
                write!(f, "repository {repository}: {name:?} isn't owner/name")
            },
            Self::RepositoryNameTaken { repository, name, other } => {
                write!(f, "repository {repository}: {name} is repository {other}'s name too")
            },
            Self::InvalidCommit { item, commit } => {
                write!(f, "item {item}: evidence commit {commit:?} isn't <repository id>@<sha>")
            },
            Self::IncompleteRepository { item, repository, missing } => write!(
                f,
                "item {item}: evidence names repository {repository}, which is missing {}",
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Splits a repository's `owner/name` into its two parts, when it is one: both non-empty, with one `/` and no space.
#[must_use]
pub fn owner_name(text: &str) -> Option<(&str, &str)> {
    let (owner, name) = text.split_once('/')?;
    let part = |p: &str| !p.is_empty() && !p.contains('/') && !p.chars().any(char::is_whitespace);
    (part(owner) && part(name)).then_some((owner, name))
}

/// Whether `sha` looks like a Git object id: 7 to 64 hexadecimal characters.
#[must_use]
pub fn is_object_id(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Import {
    /// An empty document of the current version.
    #[must_use]
    pub fn new() -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            repositories: Vec::new(),
            items: Vec::new(),
            threads: Vec::new(),
        }
    }

    /// Reads a document, checking its format and version before its shape, and lowercases every id in it, as stored
    /// ids are.
    ///
    /// # Errors
    ///
    /// Fails when the text isn't JSON, isn't an import, is another version, or has unknown or missing fields.
    pub fn from_json(text: &str) -> Result<Self, ParseError> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(ParseError::Json)?;
        if value.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
            return Err(ParseError::NotAnImport);
        }
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(found) if found != u64::from(VERSION) => {
                return Err(ParseError::UnsupportedVersion { found });
            },
            _ => {},
        }
        let mut doc: Self = serde_json::from_value(value).map_err(ParseError::Json)?;
        doc.lowercase();
        Ok(doc)
    }

    /// Lowercases every id and reference: a UUID in any case names the same record.
    fn lowercase(&mut self) {
        let low = |id: &mut String| id.make_ascii_lowercase();
        let all = |ids: &mut Option<Vec<String>>| ids.iter_mut().flatten().for_each(low);
        for record in &mut self.repositories {
            low(&mut record.id);
        }
        for record in &mut self.threads {
            low(&mut record.id);
        }
        for item in &mut self.items {
            low(&mut item.id);
            item.parent.iter_mut().for_each(low);
            item.thread.iter_mut().for_each(low);
            all(&mut item.waits_for);
            if let Some(commit) = item.evidence.as_mut().and_then(|e| e.commit.as_mut())
                && let Some((repository, sha)) = commit.rsplit_once('@')
            {
                *commit = format!("{}@{sha}", repository.to_ascii_lowercase());
            }
        }
    }

    /// Writes the document as pretty-printed JSON.
    ///
    /// # Panics
    ///
    /// Never: the document holds only strings, numbers and booleans.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("an import always serializes")
    }

    /// Checks the rules the document can break on its own and returns each broken one, sorted. When any id isn't a
    /// `UUIDv7`, those are all it returns, since nothing else can be judged. Whether references name records, and
    /// whether evidence sits on done work, depend on the tracker too: [`apply`] checks them.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = self.invalid_ids();
        if errors.is_empty() {
            self.check_unique(&mut errors);
            self.check_links(&mut errors);
            self.check_shapes(&mut errors);
        }
        errors.sort();
        errors.dedup();
        errors
    }

    /// Every id and reference that isn't a `UUIDv7`.
    fn invalid_ids(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let mut check = |at: String, id: &str| {
            if ids::v7(id).is_none() {
                errors.push(ValidationError::InvalidId { at, id: id.to_owned() });
            }
        };
        for record in &self.repositories {
            check("repository id".to_owned(), &record.id);
        }
        for record in &self.threads {
            check("thread id".to_owned(), &record.id);
        }
        for item in &self.items {
            check("item id".to_owned(), &item.id);
            let at = |field: &str| format!("item {}, {field}", item.id);
            for (field, id) in [("parent", &item.parent), ("thread", &item.thread)] {
                if let Some(id) = id {
                    check(at(field), id);
                }
            }
            for id in item.waits_for.iter().flatten() {
                check(at("waits_for"), id);
            }
            let commit = item.evidence.as_ref().and_then(|e| e.commit.as_ref());
            if let Some((repository, _)) = commit.and_then(|commit| commit.rsplit_once('@'))
                && owner_name(repository).is_none()
            {
                check(at("evidence"), repository);
            }
        }
        errors
    }

    fn check_unique(&self, errors: &mut Vec<ValidationError>) {
        let mut once = |kind: KeyKind, ids: Vec<&String>| {
            let mut seen = HashSet::new();
            for id in ids {
                if !seen.insert(id) {
                    errors.push(ValidationError::DuplicateId(kind, id.clone()));
                }
            }
        };
        once(KeyKind::Repository, self.repositories.iter().map(|r| &r.id).collect());
        once(KeyKind::Item, self.items.iter().map(|i| &i.id).collect());
        once(KeyKind::Thread, self.threads.iter().map(|t| &t.id).collect());
    }

    /// Cycles, among parents or waits-for, are left to the import, which judges them with what the tracker holds.
    fn check_links(&self, errors: &mut Vec<ValidationError>) {
        for item in &self.items {
            let mut seen = HashSet::new();
            for to in item.waits_for.iter().flatten() {
                if !seen.insert(to) {
                    errors.push(ValidationError::DuplicateLink {
                        item: item.id.clone(),
                        waits_for: to.clone(),
                    });
                }
                if *to == item.id {
                    errors.push(ValidationError::SelfLink(item.id.clone()));
                }
            }
        }
    }

    fn check_shapes(&self, errors: &mut Vec<ValidationError>) {
        for record in &self.repositories {
            if owner_name(&record.name).is_none() {
                errors.push(ValidationError::InvalidRepositoryName {
                    repository: record.id.clone(),
                    name: record.name.clone(),
                });
            }
        }
        for item in &self.items {
            let Some(commit) = item.evidence.as_ref().and_then(|e| e.commit.as_ref()) else {
                continue;
            };
            if !commit.rsplit_once('@').is_some_and(|(_, sha)| is_object_id(sha)) {
                errors.push(ValidationError::InvalidCommit {
                    item: item.id.clone(),
                    commit: commit.clone(),
                });
            }
        }
    }
}

impl Default for Import {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
