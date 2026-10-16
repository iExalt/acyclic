//! The command line.

use std::path::PathBuf;

use acyclic_core::model::{ItemState, Level, LinkKind, Size};
use std::fmt::Write as _;

use clap::{CommandFactory, Parser, Subcommand};

pub use crate::page::Renderer;
use serde::de::DeserializeOwned;

/// The acyclic tracker. Every command that runs prints one JSON document.
#[derive(Debug, Parser)]
#[command(name = "acyclic", version, disable_help_subcommand = true, arg_required_else_help = true)]
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

/// The top level's help sections, in order: what every session uses, the groups, then the rest.
pub const SECTIONS: [(&str, &[&str]); 3] = [
    ("Common Commands", &["ready", "list", "show", "claim", "release", "done", "needs"]),
    ("Command Groups", &["item", "link", "thread", "repo", "proposal"]),
    ("Other Commands", &["init", "sync", "prime", "import", "graph", "level", "doctor"]),
];

/// The command line, with its top-level help in [`SECTIONS`] rather than one list.
pub fn command() -> clap::Command {
    let command = Cli::command();
    let width = command.get_subcommands().map(|c| c.get_name().len()).max().unwrap_or(0);
    let mut template = String::from("{about-with-newline}\n{usage-heading} {usage}\n");
    for (heading, names) in SECTIONS {
        write!(template, "\n{heading}:\n").expect("a String takes writes");
        for name in names {
            let about = command
                .find_subcommand(name)
                .and_then(clap::Command::get_about)
                .map(ToString::to_string)
                .unwrap_or_default();
            writeln!(template, "  {name:width$}  {about}").expect("a String takes writes");
        }
    }
    template.push_str("\nOptions:\n{options}\n\nRun 'acyclic <command> --help' for more information on a command.\n");
    command.help_template(template)
}

/// Parses a kebab-case name into one of the model's enums.
fn kebab<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| format!("unknown value {text:?}"))
}

/// The commands: the ones every session uses at the top, the rest grouped by what they act on.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sets up a tracker clone as a new replica, creating the repository if there is none.
    Init {
        /// Where the tracker clone is or goes.
        path: PathBuf,
        /// The new replica's id, such as `mac`.
        #[arg(long)]
        replica: String,
        /// The shared tracker repository to sync with, set as the clone's `origin`.
        #[arg(long)]
        remote: Option<String>,
    },
    /// Fetches every replica's state from `origin`, joins it into this one, and pushes this replica's refs.
    Sync,
    /// Prints conventions for agents, matched to this version, as text.
    Prime,
    /// Lists ready leaves, best first.
    Ready {
        /// Judge archived work as if it weren't archived.
        #[arg(long)]
        include_archived: bool,
    },
    /// Lists every item, by id.
    List,
    /// Shows items with their role, readiness and claims, as an array in the order given.
    Show {
        /// The items.
        #[arg(required = true)]
        items: Vec<String>,
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
    /// Releases the actor's claim on an item, leaving anyone else's.
    Release {
        /// The item.
        id: String,
    },
    /// Marks an item done, with evidence, and releases the actor's claim on it.
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
        /// Finish it although it isn't ready or someone else holds it.
        #[arg(long)]
        force: bool,
    },
    /// Lists what needs the user.
    Needs,
    /// Sets the autonomy level, globally or for a thread (the user's).
    Level {
        /// `everything`, `scope-and-irreversible` or `blockers`.
        #[arg(value_parser = kebab::<Level>)]
        level: Level,
        /// The thread; none sets the global level.
        #[arg(long)]
        thread: Option<String>,
    },
    /// Imports a plan: adds and updates what the document names, and never deletes.
    Import {
        /// The import document.
        file: PathBuf,
        /// Report what would change without writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Lists entities that can't be read.
    Doctor,
    /// Draws the graph with threads as clusters: DOT or Mermaid, or an HTML page to open in a browser.
    Graph {
        /// The document's format: DOT by default, HTML with `--open`. HTML needs `--output` or `--open`.
        #[arg(long, value_enum)]
        format: Option<GraphFormat>,
        /// What lays out the HTML page: Graphviz by default.
        #[arg(long, value_enum)]
        renderer: Option<Renderer>,
        /// Write the HTML page into the clone's acyclic directory and open it in the default browser.
        #[arg(long)]
        open: bool,
        /// Only this item and its descendants.
        #[arg(long)]
        root: Option<String>,
        /// Only this thread's members; with `--root`, the items in both.
        #[arg(long)]
        thread: Option<String>,
        /// Draw archived items too.
        #[arg(long)]
        include_archived: bool,
        /// Write the document here and print a JSON summary instead.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Adds, edits, moves, reopens or archives items, or sets their state.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Adds or removes links between items.
    #[command(subcommand)]
    Link(LinkCommand),
    /// Creates or edits threads, and puts items in them; agents' edits to approved threads wait as proposals.
    #[command(subcommand)]
    Thread(ThreadCommand),
    /// Creates or edits repository records, and attaches them to items or threads.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Keeps or drops proposals (the user's).
    #[command(subcommand)]
    Proposal(ProposalCommand),
}

/// A graph document's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphFormat {
    /// Graphviz DOT.
    Dot,
    /// A Mermaid flowchart.
    Mermaid,
    /// A self-contained HTML page that draws it.
    Html,
}

/// The item commands.
#[derive(Debug, Subcommand)]
pub enum ItemCommand {
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
    Move {
        /// The item.
        id: String,
        /// The new parent; none moves it to the top level.
        parent: Option<String>,
    },
    /// Sets an item's state; use `acyclic done` to finish it.
    State {
        /// The item.
        id: String,
        /// `backlog`, `parked` or `in-progress`.
        #[arg(value_parser = kebab::<ItemState>)]
        state: ItemState,
    },
    /// Reopens a done item.
    Reopen {
        /// The item.
        id: String,
    },
    /// Archives a done subtree (the user or a project-management run).
    Archive {
        /// The subtree's root.
        id: String,
    },
}

/// The link commands.
#[derive(Debug, Subcommand)]
pub enum LinkCommand {
    /// Links two items: `from` blocks on, follows (`after`) or relates to `to`.
    Add {
        /// The dependent item.
        from: String,
        /// `blocks`, `after` or `relates`.
        #[arg(value_parser = kebab::<LinkKind>)]
        kind: LinkKind,
        /// The item it depends on.
        to: String,
    },
    /// Removes a link.
    Remove {
        /// The dependent item.
        from: String,
        /// `blocks`, `after` or `relates`.
        #[arg(value_parser = kebab::<LinkKind>)]
        kind: LinkKind,
        /// The item it depends on.
        to: String,
    },
}

/// The thread commands.
#[derive(Debug, Subcommand)]
pub enum ThreadCommand {
    /// Creates or edits a thread.
    Set {
        /// The thread's id.
        id: String,
        /// Its title.
        #[arg(long)]
        title: Option<String>,
        /// Its brief.
        #[arg(long)]
        brief: Option<String>,
    },
    /// Puts an item in a thread, taking it out of any other.
    Add {
        /// The item.
        item: String,
        /// The thread.
        thread: String,
    },
    /// Takes an item out of its thread.
    Remove {
        /// The item.
        item: String,
    },
    /// Approves a thread's scope (the user's).
    Approve {
        /// The thread.
        thread: String,
    },
    /// Revokes a thread's approval (the user's).
    Revoke {
        /// The thread.
        thread: String,
    },
}

/// The repository commands.
#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// Creates or edits a repository record.
    Set {
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
    /// Attaches a repository record to an item or a thread.
    Attach(Attachment),
    /// Detaches a repository record from an item or a thread.
    Detach(Attachment),
}

/// A repository record and the item or thread it attaches to.
#[derive(Debug, clap::Args)]
pub struct Attachment {
    /// The repository record.
    pub repository: String,
    /// The item.
    #[arg(long, conflicts_with = "thread", required_unless_present = "thread")]
    pub item: Option<String>,
    /// The thread.
    #[arg(long)]
    pub thread: Option<String>,
}

/// The proposal commands (the user's).
#[derive(Debug, Subcommand)]
pub enum ProposalCommand {
    /// Keeps a proposal.
    Keep {
        /// The proposal.
        proposal: String,
    },
    /// Drops a proposal.
    Drop {
        /// The proposal.
        proposal: String,
    },
}
