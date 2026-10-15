//! The command line.

use std::path::PathBuf;

use acyclic_core::model::{ItemState, Level, LinkKind, Size};
use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;

/// The acyclic tracker. Every command prints one JSON document.
#[derive(Debug, Parser)]
#[command(name = "acyclic", version, disable_help_subcommand = true)]
pub struct Cli {
    /// The tracker clone; otherwise `ACYCLIC_TRACKER`, else the mapping in ~/.config/acyclic/trackers.toml.
    #[arg(long, global = true)]
    pub tracker: Option<PathBuf>,
    /// Who acts; otherwise `ACYCLIC_ACTOR`, else `agent/undeclared`. The user must say so on the command.
    #[arg(long, global = true)]
    pub actor: Option<String>,
    /// Act as the user.
    #[arg(long, global = true, conflicts_with = "actor")]
    pub as_user: bool,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// Parses a kebab-case name into one of the model's enums.
fn kebab<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| format!("unknown value {text:?}"))
}

/// The commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sets up a tracker clone as a new replica, creating the repository if there is none.
    Init {
        /// Where the tracker clone is or goes.
        path: PathBuf,
        /// The new replica's id, such as `mac`.
        #[arg(long)]
        replica: String,
    },
    /// Lists ready leaves, best first.
    Ready {
        /// Judge archived work as if it weren't archived.
        #[arg(long)]
        include_archived: bool,
    },
    /// Shows an item with its role, readiness and claims.
    Show {
        /// The item.
        item: String,
    },
    /// Lists what needs the user.
    Needs,
    /// Lists entities that can't be read.
    Doctor,
    /// Adds an item.
    Add {
        /// The new item's id.
        id: String,
        /// Its title.
        #[arg(long)]
        title: String,
        /// Its parent.
        #[arg(long)]
        parent: Option<String>,
    },
    /// Edits an item's plain fields and sets.
    Edit {
        /// The item.
        id: String,
        /// A new title.
        #[arg(long)]
        title: Option<String>,
        /// New notes.
        #[arg(long)]
        notes: Option<String>,
        /// A new gate.
        #[arg(long)]
        gate: Option<String>,
        /// A new size.
        #[arg(long, value_parser = kebab::<Size>)]
        size: Option<Size>,
        /// Areas to add.
        #[arg(long)]
        area: Vec<String>,
        /// Areas to remove.
        #[arg(long)]
        no_area: Vec<String>,
        /// Named resources to add.
        #[arg(long)]
        resource: Vec<String>,
        /// Named resources to remove.
        #[arg(long)]
        no_resource: Vec<String>,
    },
    /// Moves an item under a parent, or to the top level.
    Parent {
        /// The item.
        id: String,
        /// The new parent; none moves it to the top level.
        parent: Option<String>,
    },
    /// Links two items: `from` blocks on, follows (`after`) or relates to `to`.
    Link {
        /// The dependent item.
        from: String,
        /// `blocks`, `after` or `relates`.
        #[arg(value_parser = kebab::<LinkKind>)]
        kind: LinkKind,
        /// The item it depends on.
        to: String,
    },
    /// Removes a link.
    Unlink {
        /// The dependent item.
        from: String,
        /// `blocks`, `after` or `relates`.
        #[arg(value_parser = kebab::<LinkKind>)]
        kind: LinkKind,
        /// The item it depends on.
        to: String,
    },
    /// Sets an item's state; use `done` to finish it.
    State {
        /// The item.
        id: String,
        /// `backlog`, `parked` or `in-progress`.
        #[arg(value_parser = kebab::<ItemState>)]
        state: ItemState,
    },
    /// Marks an item done, with evidence.
    Done {
        /// The item.
        id: String,
        /// The repository record holding the commit.
        #[arg(long, requires = "commit")]
        repository: Option<String>,
        /// The commit that completed it.
        #[arg(long, requires = "repository")]
        commit: Option<String>,
        /// A note, such as where a record lives.
        #[arg(long)]
        note: Option<String>,
    },
    /// Reopens a done item.
    Reopen {
        /// The item.
        id: String,
    },
    /// Claims a ready leaf, or renews the actor's claim.
    Claim {
        /// The item.
        id: String,
        /// The lease in minutes: 60 by default, at most 480. A run's claim has none and lives while the run does.
        #[arg(long)]
        lease: Option<u32>,
        /// Claim even when the item isn't ready.
        #[arg(long)]
        force: bool,
    },
    /// Creates or edits a thread.
    Thread {
        /// The thread's id.
        id: String,
        /// Its title.
        #[arg(long)]
        title: Option<String>,
        /// Its brief.
        #[arg(long)]
        brief: Option<String>,
    },
    /// Puts an item in a thread, or takes it out; agents' edits to approved threads wait as proposals.
    Member {
        /// The item.
        item: String,
        /// The thread; none takes the item out.
        thread: Option<String>,
    },
    /// Adds a repository to an item or a thread, or removes it.
    Touch {
        /// The repository record.
        repository: String,
        /// The item.
        #[arg(long, conflicts_with = "thread", required_unless_present = "thread")]
        item: Option<String>,
        /// The thread.
        #[arg(long)]
        thread: Option<String>,
        /// Remove it instead.
        #[arg(long)]
        remove: bool,
    },
    /// Creates or edits a repository record.
    Repository {
        /// The record's id.
        id: String,
        /// The owner.
        #[arg(long)]
        owner: Option<String>,
        /// The repository's name.
        #[arg(long)]
        name: Option<String>,
        /// Whether it is public.
        #[arg(long)]
        public: Option<bool>,
        /// The default branch.
        #[arg(long)]
        default_branch: Option<String>,
    },
    /// Keeps a proposal (the user's).
    Keep {
        /// The proposal.
        proposal: String,
    },
    /// Drops a proposal (the user's).
    Drop {
        /// The proposal.
        proposal: String,
    },
    /// Approves a thread's scope, or revokes it (the user's).
    Approve {
        /// The thread.
        thread: String,
        /// Revoke the approval.
        #[arg(long)]
        revoke: bool,
    },
    /// Sets the autonomy level, globally or for a thread (the user's).
    Level {
        /// `everything`, `scope-and-irreversible` or `blockers`.
        #[arg(value_parser = kebab::<Level>)]
        level: Level,
        /// The thread; none sets the global level.
        #[arg(long)]
        thread: Option<String>,
    },
    /// Archives a done subtree (the user or a project-management run).
    Archive {
        /// The subtree's root.
        id: String,
    },
}
