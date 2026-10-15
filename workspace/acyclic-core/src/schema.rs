//! Names of the entity kinds and fields the model gives meaning to. The state layer stores any name; these are the
//! ones later steps read and write.

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
    /// An actor: a person, an agent or a run.
    pub const ACTOR: &str = "actor";
    /// A code repository record.
    pub const REPOSITORY: &str = "repository";
    /// A proposed edit awaiting a decision.
    pub const PROPOSAL: &str = "proposal";
    /// Settings; the global ones have the id [`super::GLOBAL`].
    pub const SETTINGS: &str = "settings";
}

/// The id of the global settings entity.
pub const GLOBAL: &str = "global";

/// Thread fields set only by the user's batches.
pub mod thread {
    /// Whether the thread's agent is a project-management agent.
    pub const PM_FLAG: &str = "pm-flag";
    /// The thread's autonomy level, overriding the global default.
    pub const AUTONOMY: &str = "autonomy";
    /// The thread's launch choice: provider, credential and model.
    pub const LAUNCH: &str = "launch";
    /// Where the thread runs: the cluster or the Mac.
    pub const PLACEMENT: &str = "placement";
}

/// Global settings fields.
pub mod settings {
    /// The default autonomy level.
    pub const AUTONOMY: &str = "autonomy";
}
