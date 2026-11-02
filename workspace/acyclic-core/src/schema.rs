//! Names of the entity kinds and fields the model gives meaning to. The state layer stores any name; these are the
//! ones the model reads and writes, except those marked reserved.

/// Entity kinds.
pub mod kind {
    /// A work item.
    pub const ITEM: &str = "item";
    /// A link between two items.
    pub const LINK: &str = "link";
    /// A thread.
    pub const THREAD: &str = "thread";
    /// A code repository record.
    pub const REPOSITORY: &str = "repository";
}

/// Item fields. Repositories are a set stored a field per element: see [`set`]. Fields older versions wrote and no
/// longer read, such as `gate` and the `area/` and `resource/` sets, are ignored like any unknown field.
pub mod item {
    /// A short title.
    pub const TITLE: &str = "title";
    /// Design notes.
    pub const NOTES: &str = "notes";
    /// The item's state.
    pub const STATE: &str = "state";
    /// The parent item's id.
    pub const PARENT: &str = "parent";
    /// A rough size.
    pub const SIZE: &str = "size";
    /// Whether this done subtree's root is archived.
    pub const ARCHIVED: &str = "archived";
    /// Evidence of completion.
    pub const EVIDENCE: &str = "evidence";
    /// The thread the item belongs to; at most one, since it is a single field.
    pub const THREAD: &str = "thread";
    /// Set when the item is deleted: a tombstone no write clears. See `crate::delete`.
    pub const DELETED: &str = "deleted";
    /// Set when the item is reopened, until it is done again; flags its dependents.
    pub const REOPENED: &str = "reopened";
}

/// Link fields. A link's id derives from its kind and ends, so two replicas adding one link make one entity.
pub mod link {
    /// The dependent item's id.
    pub const FROM: &str = "from";
    /// The id of the item it depends on.
    pub const TO: &str = "to";
    /// The link's kind: always `blocks`, as `from` waits for `to`.
    pub const KIND: &str = "kind";
    /// The one kind's stored value: `from` waits for `to`. A link stored with any other kind can't be read.
    pub const BLOCKS: &str = "blocks";
    /// Whether the link exists; cleared to remove it, and a concurrent re-add wins.
    pub const PRESENT: &str = "present";
}

/// Repository record fields.
pub mod repository {
    /// The owner, such as an organization.
    pub const OWNER: &str = "owner";
    /// The repository's name.
    pub const NAME: &str = "name";
    /// The default branch.
    pub const DEFAULT_BRANCH: &str = "default-branch";
    /// The App's installation that reaches it.
    pub const INSTALLATION: &str = "installation";
}

/// Prefixes of the fields that hold a set, one field per element with the value `true`. A concurrent add and remove
/// keeps the add.
pub mod set {
    /// Repositories, on items and threads (those added explicitly).
    pub const REPOSITORY: &str = "repository/";
}

/// Thread fields.
pub mod thread {
    /// A short title.
    pub const TITLE: &str = "title";
    /// The brief.
    pub const BRIEF: &str = "brief";
    /// Set when the thread is deleted; the same field as an item's, read by `model::is_deleted`.
    pub const DELETED: &str = super::item::DELETED;
}
