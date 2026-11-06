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

/// Item fields. Fields older versions wrote and no longer read, such as `gate` and the `area/`, `resource/` and
/// `repository/` sets, are ignored like any unknown field.
pub mod item {
    /// A short title.
    pub const TITLE: &str = "title";
    /// Design notes.
    pub const NOTES: &str = "notes";
    /// The item's state.
    pub const STATE: &str = "state";
    /// The parent item's id.
    pub const PARENT: &str = "parent";
    /// Evidence of completion.
    pub const EVIDENCE: &str = "evidence";
    /// The thread the item belongs to; at most one, since it is a single field.
    pub const THREAD: &str = "thread";
    /// Set when the item is deleted: a tombstone no write clears. See `crate::delete`.
    pub const DELETED: &str = "deleted";
}

/// Fields older versions wrote and no command writes now, as `(kind, field)`. Their stored values stay in the state
/// but lie dormant: a conflict in one isn't noted, since no command could settle it.
pub const RETIRED: &[(&str, &str)] = &[
    (kind::ITEM, "size"),
    (kind::ITEM, "archived"),
    (kind::ITEM, "reopened"),
    (kind::REPOSITORY, "installation"),
    (kind::ITEM, "gate"),
];

/// Prefixes of the sets older versions wrote and no command writes now, on any kind, whose elements lie dormant as
/// [`RETIRED`] fields do.
pub const RETIRED_SETS: &[&str] = &["area/", "resource/", "repository/"];

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
