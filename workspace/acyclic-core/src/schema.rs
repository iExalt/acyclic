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
    /// A run of an agent on a thread.
    pub const RUN: &str = "run";
    /// An actor: a person, an agent or a run. Reserved: nothing reads or writes one yet.
    pub const ACTOR: &str = "actor";
    /// A code repository record.
    pub const REPOSITORY: &str = "repository";
    /// A proposed edit awaiting a decision.
    pub const PROPOSAL: &str = "proposal";
    /// An agent's question for the user.
    pub const QUESTION: &str = "question";
    /// Settings; the global ones have the id [`super::GLOBAL`].
    pub const SETTINGS: &str = "settings";
}

/// The id of the global settings entity.
pub const GLOBAL: &str = "global";

/// Item fields. Areas, repositories and resources are sets stored a field per element: see [`set`].
pub mod item {
    /// A short title.
    pub const TITLE: &str = "title";
    /// Design notes.
    pub const NOTES: &str = "notes";
    /// The exit check.
    pub const GATE: &str = "gate";
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

/// Proposal fields.
pub mod proposal {
    /// The proposed scope edit.
    pub const EDIT: &str = "edit";
    /// Who proposed it.
    pub const ACTOR: &str = "actor";
    /// The user's decision: keep or drop.
    pub const DECISION: &str = "decision";
}

/// Question fields.
pub mod question {
    /// The thread asking.
    pub const THREAD: &str = "thread";
    /// What kind of question it is, for the autonomy filter.
    pub const CLASS: &str = "class";
    /// The question.
    pub const TEXT: &str = "text";
    /// Whether it still awaits an answer.
    pub const OPEN: &str = "open";
}

/// Link fields. A link's id derives from its kind and ends, so two replicas adding one link make one entity.
pub mod link {
    /// The dependent item's id.
    pub const FROM: &str = "from";
    /// The id of the item it depends on.
    pub const TO: &str = "to";
    /// The link's kind.
    pub const KIND: &str = "kind";
    /// Whether the link exists; cleared to remove it, and a concurrent re-add wins.
    pub const PRESENT: &str = "present";
}

/// Repository record fields.
pub mod repository {
    /// The owner, such as an organization.
    pub const OWNER: &str = "owner";
    /// The repository's name.
    pub const NAME: &str = "name";
    /// Whether the repository is public.
    pub const PUBLIC: &str = "public";
    /// The default branch.
    pub const DEFAULT_BRANCH: &str = "default-branch";
    /// The App's installation that reaches it.
    pub const INSTALLATION: &str = "installation";
}

/// Run record fields.
pub mod run {
    /// The thread the run works on.
    pub const THREAD: &str = "thread";
    /// The run's state.
    pub const STATE: &str = "run-state";
}

/// Prefixes of the fields that hold a set, one field per element with the value `true`. A concurrent add and remove
/// keeps the add.
pub mod set {
    /// Area tags, on items.
    pub const AREA: &str = "area/";
    /// Repositories, on items and threads (those added explicitly).
    pub const REPOSITORY: &str = "repository/";
    /// Named resources, on items.
    pub const RESOURCE: &str = "resource/";
}

/// Thread fields; the flags and choices are set only by the user's batches.
pub mod thread {
    /// A short title.
    pub const TITLE: &str = "title";
    /// The brief.
    pub const BRIEF: &str = "brief";
    /// Whether the thread's scope is approved.
    pub const APPROVED: &str = "approved";
    /// The members the latest approval saw, sorted: a member that joined unseen is the user's to keep or drop.
    pub const APPROVED_MEMBERS: &str = "approved-members";
    /// Whether the thread's agent is a project-management agent.
    pub const PM_FLAG: &str = "pm-flag";
    /// The thread's autonomy level, overriding the global default.
    pub const AUTONOMY: &str = "autonomy";
    /// The thread's launch choice: provider, credential and model. Reserved: the user alone may write it, and nothing
    /// reads it yet.
    pub const LAUNCH: &str = "launch";
    /// Where the thread runs: the cluster or the Mac. Reserved like [`LAUNCH`].
    pub const PLACEMENT: &str = "placement";
    /// Set when a renumber retired the thread's id; the same field as an item's, read by `model::is_deleted`.
    pub const DELETED: &str = super::item::DELETED;
}

/// Global settings fields.
pub mod settings {
    /// The default autonomy level.
    pub const AUTONOMY: &str = "autonomy";
}
