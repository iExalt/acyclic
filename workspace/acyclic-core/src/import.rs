//! The import format: a whole plan as one JSON document with symbolic keys.
//!
//! The format is versioned and kept apart from the model, so the model can change without breaking documents
//! written against an earlier version. `acyclic import` maps a validated document into the model.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

mod apply;

pub use apply::{ApplyError, Report, apply};

/// The value of the document's `format` field.
pub const FORMAT: &str = "acyclic-import";

/// The newest version this build reads. Version 2 names link kinds `waits-for` and `relates-to`; version 1, still
/// read, names them `blocks` and `relates` (and takes the new names too). Both versions refuse the retired soft link,
/// `follows` in version 2 and `after` in version 1.
pub const VERSION: u32 = 2;

/// An annotated version 2 document, as `acyclic import --schema` prints it: lines starting with `//` are notes to
/// remove, since JSON has no comments.
pub const SCHEMA: &str = r#"// An acyclic import document: a whole plan, its records named by keys you choose.
// Importing adds and updates what it names and never deletes, so importing it again changes nothing.
// JSON has no comments: remove the lines that start with //.
// Version 1 documents are still read: they name the link kinds blocks and relates.
{
  "format": "acyclic-import",
  "version": 2,
  // A code repository that items can name; all four fields are required.
  "repositories": [
    {"key": "code", "owner": "me", "name": "garden", "public": false, "default_branch": "main"}
  ],
  "items": [
    // Required: key and title. "state" is backlog, parked, in-progress or done, and defaults to backlog.
    // Optional: parent (an item's key), notes, gate, size (xs, s, m, l, xl), areas, resources, repositories,
    // archived, and evidence on done items.
    {"key": "beds", "title": "Build the beds", "size": "m", "repositories": ["code"]},
    {"key": "plant", "title": "Plant the seeds", "areas": ["garden"]}
  ],
  // "from" waits-for or relates-to "to": plant can't start until beds is done.
  "links": [
    {"from": "plant", "kind": "waits-for", "to": "beds"}
  ],
  // A thread groups items for one continuing agent conversation; "items" lists its members' keys. Optional: brief,
  // repositories (keys, beyond those of its items) and approved (only the user may import approved threads).
  "threads": [
    {"key": "spring", "title": "Spring planting", "items": ["beds", "plant"]}
  ]
}
"#;

/// A plan to import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    /// Always [`FORMAT`].
    pub format: String,
    /// The format version, at most [`VERSION`].
    pub version: u32,
    /// Code repositories that items name.
    #[serde(default)]
    pub repositories: Vec<Repository>,
    /// Items, in any order.
    #[serde(default)]
    pub items: Vec<Item>,
    /// Links between items.
    #[serde(default)]
    pub links: Vec<Link>,
    /// Threads grouping items.
    #[serde(default)]
    pub threads: Vec<Thread>,
}

/// A code repository record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    /// The symbolic key items use.
    pub key: String,
    /// The owner, such as an organization.
    pub owner: String,
    /// The repository's name.
    pub name: String,
    /// Whether the repository is public.
    pub public: bool,
    /// The default branch.
    pub default_branch: String,
}

/// An item's state. "In review" and readiness are derived on read, never imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Not started.
    #[default]
    Backlog,
    /// Set aside on purpose.
    Parked,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
}

/// A rough size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Size {
    /// Extra small.
    Xs,
    /// Small.
    S,
    /// Medium.
    M,
    /// Large.
    L,
    /// Extra large.
    Xl,
}

/// What shows a done item was done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The commit that completed the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The repository holding the commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// A free-form note, such as where a record lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A work item at any depth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// The symbolic key links, parents and threads use.
    pub key: String,
    /// A short title.
    pub title: String,
    /// Design notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The exit check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// The item's state; backlog when left out.
    #[serde(default)]
    pub state: State,
    /// The parent item's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Area tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub areas: Vec<String>,
    /// Keys of the repositories the item touches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    /// Named resources the item holds while worked on, so items sharing one never run at once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    /// A rough size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Whether this done subtree's root is archived.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// Evidence of completion, for done items only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// A link's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    /// A hard dependency: `from` waits for `to`.
    WaitsFor,
    /// A retired soft dependency, read only so validation can refuse it with the words to use instead.
    Follows,
    /// Context only.
    RelatesTo,
}

/// The names version 1 gave link kinds, with the ones that replaced them.
pub const RENAMED_LINK_KINDS: [(&str, &str); 3] =
    [("blocks", "waits-for"), ("after", "follows"), ("relates", "relates-to")];

/// A link between two items: `from` depends on or relates to `to`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// The dependent item's key.
    pub from: String,
    /// The key of the item it depends on.
    pub to: String,
    /// The link's kind.
    pub kind: LinkKind,
}

/// A thread: items for one continuing agent conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thread {
    /// The thread's symbolic key.
    pub key: String,
    /// A short title.
    pub title: String,
    /// The brief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Member items' keys.
    pub items: Vec<String>,
    /// Repositories added explicitly, beyond those of its items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    /// Whether the thread's scope is approved. Whether an import may set it depends on who imports.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approved: bool,
}

/// Why a document couldn't be read.
#[derive(Debug)]
pub enum ParseError {
    /// Not JSON, or not shaped like an import.
    Json(serde_json::Error),
    /// The `format` field is missing or wrong.
    NotAnImport,
    /// Version 0, or written by a newer version than this build reads.
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
                    "import version {found} is not supported; this build reads up to {VERSION}"
                )
            },
        }
    }
}

impl std::error::Error for ParseError {}

/// The kind of thing a key names.
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

/// A rule a parsed document breaks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationError {
    /// A key is empty.
    EmptyKey(KeyKind),
    /// Two records of one kind share a key.
    DuplicateKey(KeyKind, String),
    /// A reference names no record of the expected kind.
    UnknownReference {
        /// Where the reference is, such as `item a1, parent`.
        at: String,
        /// The kind it should name.
        kind: KeyKind,
        /// The missing key.
        key: String,
    },
    /// Following parents from this item leads back to it.
    ParentCycle(String),
    /// A link joins an item to itself.
    SelfLink(String),
    /// Dependencies among exactly these items form a cycle.
    DependencyCycle(Vec<String>),
    /// The same link appears twice.
    DuplicateLink(Link),
    /// A follows link, which is retired.
    FollowsRetired(Link),
    /// An item is a member of several threads.
    ItemInSeveralThreads {
        /// The item.
        item: String,
        /// The threads claiming it.
        threads: Vec<String>,
    },
    /// An archived item that isn't a wholly done subtree.
    ArchivedNotDone(String),
    /// Evidence on an item that isn't done.
    EvidenceNotDone(String),
    /// A key or name that isn't a valid id: at most 64 lowercase letters, digits, `-`, `_` or `.`, not starting
    /// with `.`.
    InvalidName {
        /// Where it is, such as `item a1, areas`.
        at: String,
        /// The name.
        name: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyKey(kind) => write!(f, "a {kind} has an empty key"),
            Self::DuplicateKey(kind, key) => write!(f, "{kind} key {key:?} is used twice"),
            Self::UnknownReference { at, kind, key } => {
                write!(f, "{at} names unknown {kind} {key:?}")
            },
            Self::ParentCycle(item) => write!(f, "item {item:?} is its own ancestor"),
            Self::SelfLink(item) => write!(f, "item {item:?} links to itself"),
            Self::DependencyCycle(items) => {
                write!(f, "dependencies form a cycle through {}", items.join(", "))
            },
            Self::ItemInSeveralThreads { item, threads } => {
                write!(f, "item {item:?} is in several threads: {}", threads.join(", "))
            },
            Self::ArchivedNotDone(item) => {
                write!(f, "item {item:?} is archived but it or a descendant isn't done")
            },
            Self::DuplicateLink(link) => {
                write!(f, "link {:?} to {:?} ({:?}) appears twice", link.from, link.to, link.kind)
            },
            Self::FollowsRetired(link) => write!(
                f,
                "link {:?} to {:?}: follows links are retired; use waits-for when {:?} can't start before {:?} is \
                 done, or relates-to for context",
                link.from, link.to, link.from, link.to
            ),
            Self::EvidenceNotDone(item) => write!(f, "item {item:?} has evidence but isn't done"),
            Self::InvalidName { at, name } => write!(
                f,
                "{at}: {name:?} isn't a valid name (at most 64 lowercase letters, digits, '-', '_' or '.')"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

impl Import {
    /// An empty document of the current version.
    #[must_use]
    pub fn new() -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            repositories: Vec::new(),
            items: Vec::new(),
            links: Vec::new(),
            threads: Vec::new(),
        }
    }

    /// Reads a document, checking its format and version before its shape.
    ///
    /// # Errors
    ///
    /// Fails when the text isn't JSON, isn't an import, is from a newer version, or has unknown or missing fields.
    pub fn from_json(text: &str) -> Result<Self, ParseError> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(ParseError::Json)?;
        if value.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
            return Err(ParseError::NotAnImport);
        }
        if let Some(found) = value.get("version").and_then(serde_json::Value::as_u64)
            && (found == 0 || found > u64::from(VERSION))
        {
            return Err(ParseError::UnsupportedVersion { found });
        }
        let mut value = value;
        if value.get("version").and_then(serde_json::Value::as_u64) == Some(1) {
            for link in value.get_mut("links").and_then(|l| l.as_array_mut()).into_iter().flatten()
            {
                let renamed = link.get("kind").and_then(|k| k.as_str()).and_then(|old| {
                    RENAMED_LINK_KINDS.iter().find(|(name, _)| *name == old).map(|(_, new)| *new)
                });
                if let Some(new) = renamed {
                    link["kind"] = new.into();
                }
            }
        }
        serde_json::from_value(value).map_err(ParseError::Json)
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

    /// Checks every rule and returns each broken one, sorted.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let repositories =
            keys(KeyKind::Repository, self.repositories.iter().map(|r| &r.key), &mut errors);
        let items = keys(KeyKind::Item, self.items.iter().map(|i| &i.key), &mut errors);
        keys(KeyKind::Thread, self.threads.iter().map(|t| &t.key), &mut errors);

        let mut check = |at: String, kind: KeyKind, key: &str, known: &HashSet<&str>| {
            if !known.contains(key) {
                errors.push(ValidationError::UnknownReference { at, kind, key: key.to_owned() });
            }
        };
        for item in &self.items {
            if let Some(parent) = &item.parent {
                check(format!("item {:?}, parent", item.key), KeyKind::Item, parent, &items);
            }
            for repository in &item.repositories {
                let at = format!("item {:?}, repositories", item.key);
                check(at, KeyKind::Repository, repository, &repositories);
            }
            if let Some(repository) = item.evidence.as_ref().and_then(|e| e.repository.as_ref()) {
                let at = format!("item {:?}, evidence", item.key);
                check(at, KeyKind::Repository, repository, &repositories);
            }
        }
        for link in &self.links {
            check(
                format!("link {:?} to {:?}", link.from, link.to),
                KeyKind::Item,
                &link.from,
                &items,
            );
            check(
                format!("link {:?} to {:?}", link.from, link.to),
                KeyKind::Item,
                &link.to,
                &items,
            );
        }
        for thread in &self.threads {
            for item in &thread.items {
                check(format!("thread {:?}, items", thread.key), KeyKind::Item, item, &items);
            }
            for repository in &thread.repositories {
                let at = format!("thread {:?}, repositories", thread.key);
                check(at, KeyKind::Repository, repository, &repositories);
            }
        }

        self.check_names(&mut errors);
        self.check_parents(&mut errors);
        self.check_links(&mut errors);
        self.check_threads(&mut errors);
        self.check_done(&mut errors);
        errors.sort();
        errors.dedup();
        errors
    }

    fn check_names(&self, errors: &mut Vec<ValidationError>) {
        let mut check = |at: String, name: &str| {
            if !name.is_empty() && crate::model::item_key(name).is_err() {
                errors.push(ValidationError::InvalidName { at, name: name.to_owned() });
            }
        };
        for repository in &self.repositories {
            check("repository key".to_owned(), &repository.key);
        }
        for item in &self.items {
            check("item key".to_owned(), &item.key);
            for (field, names) in [("areas", &item.areas), ("resources", &item.resources)] {
                for name in names {
                    check(format!("item {:?}, {field}", item.key), name);
                }
            }
        }
        for thread in &self.threads {
            check("thread key".to_owned(), &thread.key);
        }
    }

    fn check_parents(&self, errors: &mut Vec<ValidationError>) {
        let parents: HashMap<&str, &str> = self
            .items
            .iter()
            .filter_map(|item| item.parent.as_deref().map(|parent| (item.key.as_str(), parent)))
            .collect();
        for item in &self.items {
            let mut seen = HashSet::new();
            let mut current = item.key.as_str();
            while let Some(&parent) = parents.get(current) {
                if parent == item.key {
                    errors.push(ValidationError::ParentCycle(item.key.clone()));
                    break;
                }
                if !seen.insert(parent) {
                    break; // A cycle above this item, reported for its members.
                }
                current = parent;
            }
        }
    }

    fn check_links(&self, errors: &mut Vec<ValidationError>) {
        let mut edges = Vec::new();
        let mut seen = HashSet::new();
        for link in &self.links {
            if !seen.insert((&link.from, &link.to, link.kind)) {
                errors.push(ValidationError::DuplicateLink(link.clone()));
            }
            if link.kind == LinkKind::Follows {
                errors.push(ValidationError::FollowsRetired(link.clone()));
            }
            if link.from == link.to {
                errors.push(ValidationError::SelfLink(link.from.clone()));
            } else if link.kind == LinkKind::WaitsFor {
                edges.push((link.from.as_str(), link.to.as_str()));
            }
        }
        for cycle in crate::graph::cycles(&edges) {
            errors.push(ValidationError::DependencyCycle(
                cycle.into_iter().map(str::to_owned).collect(),
            ));
        }
    }

    fn check_threads(&self, errors: &mut Vec<ValidationError>) {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for thread in &self.threads {
            for item in &thread.items {
                owners.entry(item.as_str()).or_default().insert(thread.key.as_str());
            }
        }
        for (item, threads) in owners.into_iter().filter(|(_, threads)| threads.len() > 1) {
            errors.push(ValidationError::ItemInSeveralThreads {
                item: item.to_owned(),
                threads: threads.into_iter().map(str::to_owned).collect(),
            });
        }
    }

    fn check_done(&self, errors: &mut Vec<ValidationError>) {
        let mut children: HashMap<&str, Vec<&Item>> = HashMap::new();
        for item in &self.items {
            if let Some(parent) = &item.parent {
                children.entry(parent.as_str()).or_default().push(item);
            }
        }
        for item in &self.items {
            if item.evidence.is_some() && item.state != State::Done {
                errors.push(ValidationError::EvidenceNotDone(item.key.clone()));
            }
            if item.archived {
                let mut stack = vec![item];
                let mut seen = HashSet::new();
                while let Some(node) = stack.pop() {
                    if !seen.insert(node.key.as_str()) {
                        continue;
                    }
                    if node.state != State::Done {
                        errors.push(ValidationError::ArchivedNotDone(item.key.clone()));
                        break;
                    }
                    stack.extend(children.get(node.key.as_str()).into_iter().flatten());
                }
            }
        }
    }
}

impl Default for Import {
    fn default() -> Self {
        Self::new()
    }
}

fn keys<'a>(
    kind: KeyKind,
    keys: impl Iterator<Item = &'a String>,
    errors: &mut Vec<ValidationError>,
) -> HashSet<&'a str> {
    let mut seen = HashSet::new();
    for key in keys {
        if key.is_empty() {
            errors.push(ValidationError::EmptyKey(kind));
        } else if !seen.insert(key.as_str()) {
            errors.push(ValidationError::DuplicateKey(kind, key.clone()));
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn item(key: &str, state: State, parent: Option<&str>) -> Item {
        Item {
            key: key.to_owned(),
            title: format!("Item {key}"),
            notes: None,
            gate: None,
            state,
            parent: parent.map(str::to_owned),
            areas: Vec::new(),
            repositories: Vec::new(),
            resources: Vec::new(),
            size: None,
            archived: false,
            evidence: None,
        }
    }

    pub(super) fn link(from: &str, to: &str, kind: LinkKind) -> Link {
        Link { from: from.to_owned(), to: to.to_owned(), kind }
    }

    pub(super) fn thread(key: &str, items: &[&str]) -> Thread {
        Thread {
            key: key.to_owned(),
            title: format!("Thread {key}"),
            brief: None,
            items: items.iter().map(|&i| i.to_owned()).collect(),
            repositories: Vec::new(),
            approved: false,
        }
    }

    pub(super) fn valid() -> Import {
        let mut import = Import::new();
        import.repositories.push(Repository {
            key: "r".to_owned(),
            owner: "example".to_owned(),
            name: "code".to_owned(),
            public: false,
            default_branch: "main".to_owned(),
        });
        import.items = vec![
            item("goal", State::Backlog, None),
            item("a", State::Backlog, Some("goal")),
            item("b", State::Done, Some("goal")),
        ];
        import.items[1].repositories.push("r".to_owned());
        import.items[2].evidence = Some(Evidence {
            commit: Some("abc".to_owned()),
            repository: Some("r".to_owned()),
            note: None,
        });
        import.links =
            vec![link("a", "b", LinkKind::WaitsFor), link("b", "a", LinkKind::RelatesTo)];
        import.threads = vec![thread("t", &["a", "b"])];
        import
    }

    #[test]
    fn a_valid_document_round_trips() {
        let import = valid();
        assert_eq!(import.validate(), vec![]);
        let json = import.to_json();
        assert_eq!(Import::from_json(&json).unwrap(), import);
    }

    #[test]
    fn format_version_and_unknown_fields_are_checked() {
        assert!(matches!(Import::from_json("[]"), Err(ParseError::NotAnImport)));
        assert!(matches!(
            Import::from_json("{\"format\":\"other\",\"version\":1}"),
            Err(ParseError::NotAnImport)
        ));
        let newer = format!("{{\"format\":\"{FORMAT}\",\"version\":{}}}", VERSION + 1);
        assert!(matches!(
            Import::from_json(&newer),
            Err(ParseError::UnsupportedVersion { found: 3 })
        ));
        let zero = format!("{{\"format\":\"{FORMAT}\",\"version\":0}}");
        assert!(matches!(
            Import::from_json(&zero),
            Err(ParseError::UnsupportedVersion { found: 0 })
        ));
        let unknown = format!("{{\"format\":\"{FORMAT}\",\"version\":1,\"itmes\":[]}}");
        assert!(matches!(Import::from_json(&unknown), Err(ParseError::Json(_))));
        let minimal = format!("{{\"format\":\"{FORMAT}\",\"version\":2}}");
        assert_eq!(Import::from_json(&minimal).unwrap(), Import::new());
    }

    #[test]
    fn version_1_names_links_as_it_did_and_version_2_as_it_does() {
        let read = |version: u32, kind: &str| {
            Import::from_json(&format!(
                "{{\"format\":\"{FORMAT}\",\"version\":{version},\"items\":[{{\"key\":\"a\",\"title\":\"A\"}},\
                 {{\"key\":\"b\",\"title\":\"B\"}}],\"links\":[{{\"from\":\"a\",\"to\":\"b\",\"kind\":\"{kind}\"}}]}}"
            ))
        };
        for (old, new, expected) in [
            ("blocks", "waits-for", LinkKind::WaitsFor),
            ("after", "follows", LinkKind::Follows),
            ("relates", "relates-to", LinkKind::RelatesTo),
        ] {
            if expected == LinkKind::Follows {
                // Read, so validation can name the words to use instead.
                for (version, word) in [(1, old), (2, new)] {
                    let doc = read(version, word).unwrap();
                    assert_eq!(
                        doc.validate(),
                        vec![ValidationError::FollowsRetired(doc.links[0].clone())],
                        "{word}"
                    );
                    let message = doc.validate()[0].to_string();
                    assert!(
                        message.contains("waits-for") && message.contains("relates-to"),
                        "{message}"
                    );
                }
            }
            assert_eq!(read(1, old).unwrap().links[0].kind, expected, "{old}");
            assert_eq!(read(2, new).unwrap().links[0].kind, expected, "{new}");
            assert_eq!(read(1, new).unwrap().links[0].kind, expected, "version 1 takes {new} too");
            assert!(matches!(read(2, old), Err(ParseError::Json(_))), "{old} is version 1's");
        }
        // A left-out state is the backlog, in either version.
        assert_eq!(read(2, "waits-for").unwrap().items[0].state, State::Backlog);
    }

    #[test]
    fn the_schema_example_is_a_valid_document() {
        let json: String =
            SCHEMA.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
        let example = Import::from_json(&json).unwrap();
        assert_eq!(example.validate(), vec![]);
        assert_eq!(example.version, VERSION);
        assert_eq!(example.items[1].state, State::Backlog);
    }

    #[test]
    fn keys_must_be_unique_non_empty_and_resolve() {
        let mut import = valid();
        import.items.push(item("a", State::Backlog, Some("nowhere")));
        import.items.push(item("", State::Backlog, None));
        import.threads.push(thread("t", &["ghost"]));
        import.links.push(link("a", "missing", LinkKind::RelatesTo));
        let errors = import.validate();
        assert!(errors.contains(&ValidationError::DuplicateKey(KeyKind::Item, "a".to_owned())));
        assert!(errors.contains(&ValidationError::DuplicateKey(KeyKind::Thread, "t".to_owned())));
        assert!(errors.contains(&ValidationError::EmptyKey(KeyKind::Item)));
        let mut unknown: Vec<&str> = errors
            .iter()
            .filter_map(|e| match e {
                ValidationError::UnknownReference { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        unknown.sort_unstable();
        assert_eq!(unknown, vec!["ghost", "missing", "nowhere"]);
    }

    #[test]
    fn parent_cycles_are_refused() {
        let mut import = valid();
        import.items[0].parent = Some("a".to_owned());
        let errors = import.validate();
        assert!(errors.contains(&ValidationError::ParentCycle("goal".to_owned())));
        assert!(errors.contains(&ValidationError::ParentCycle("a".to_owned())));
        assert!(
            !errors.contains(&ValidationError::ParentCycle("b".to_owned())),
            "b is below the cycle"
        );
    }

    #[test]
    fn waits_for_links_must_be_acyclic_and_relates_to_links_never_close_a_cycle() {
        let mut import = valid();
        import.links.push(link("b", "a", LinkKind::WaitsFor));
        assert_eq!(
            import.validate(),
            vec![ValidationError::DependencyCycle(vec!["a".to_owned(), "b".to_owned()])]
        );
        // The valid document's relates-to link from b to a closes none.
        import.links.pop();
        import.links.push(link("a", "a", LinkKind::RelatesTo));
        assert_eq!(import.validate(), vec![ValidationError::SelfLink("a".to_owned())]);
    }

    #[test]
    fn a_cycle_names_only_its_members_and_each_cycle_is_reported() {
        let mut import = valid();
        import.items.push(item("c", State::Backlog, None));
        import.items.push(item("d", State::Backlog, None));
        import.items.push(item("e", State::Backlog, None));
        // a blocks on b already; b waiting for a closes a cycle that c and d only reach.
        import.links.push(link("b", "a", LinkKind::WaitsFor));
        import.links.push(link("c", "a", LinkKind::WaitsFor));
        import.links.push(link("a", "e", LinkKind::WaitsFor));
        import.links.push(link("d", "e", LinkKind::WaitsFor));
        import.links.push(link("e", "d", LinkKind::WaitsFor));
        assert_eq!(
            import.validate(),
            vec![
                ValidationError::DependencyCycle(vec!["a".to_owned(), "b".to_owned()]),
                ValidationError::DependencyCycle(vec!["d".to_owned(), "e".to_owned()]),
            ]
        );
    }

    #[test]
    fn a_repeated_link_is_refused() {
        let mut import = valid();
        import.links.push(link("b", "a", LinkKind::RelatesTo));
        assert_eq!(
            import.validate(),
            vec![ValidationError::DuplicateLink(link("b", "a", LinkKind::RelatesTo))]
        );
    }

    #[test]
    fn an_item_belongs_to_one_thread() {
        let mut import = valid();
        import.threads.push(thread("u", &["a"]));
        assert_eq!(
            import.validate(),
            vec![ValidationError::ItemInSeveralThreads {
                item: "a".to_owned(),
                threads: vec!["t".to_owned(), "u".to_owned()],
            }]
        );
    }

    #[test]
    fn archive_and_evidence_need_done() {
        let mut import = valid();
        import.items[2].archived = true;
        assert_eq!(import.validate(), vec![]);
        import.items.push(item("c", State::Backlog, Some("b")));
        assert_eq!(import.validate(), vec![ValidationError::ArchivedNotDone("b".to_owned())]);
        import.items[1].evidence =
            Some(Evidence { commit: None, repository: None, note: Some("n".to_owned()) });
        assert!(import.validate().contains(&ValidationError::EvidenceNotDone("a".to_owned())));
    }
}
