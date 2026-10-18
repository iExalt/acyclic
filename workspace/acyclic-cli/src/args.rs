//! The command line.

use std::path::PathBuf;

use acyclic_core::model::{ItemState, Level, LinkKind, Size};
use std::fmt::Write as _;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{CommandFactory, Parser, Subcommand};

pub use crate::page::Renderer;
use serde::Deserialize;

use crate::error::{CliError, exit};
use serde::Serialize;

/// The heading the options that every command takes sit under in its help.
const GLOBAL_OPTIONS: &str = "Global options";

/// The acyclic tracker. Every command prints one JSON document when piped and text for people at a terminal (`-o`
/// chooses), except `prime`, `completions`, `graph` without `--file`, `import --schema` and help and version text.
#[derive(Debug, Parser)]
#[command(name = "acyclic", version, arg_required_else_help = true)]
pub struct Cli {
    /// The tracker clone; otherwise `ACYCLIC_TRACKER`, else the clone you are in, else the mapping in
    /// ~/.config/acyclic/trackers.toml.
    #[arg(long, global = true, help_heading = GLOBAL_OPTIONS)]
    pub tracker: Option<PathBuf>,
    /// Who acts; otherwise `ACYCLIC_ACTOR`, else `agent/undeclared`. The user must say so on the command.
    #[arg(long, global = true, help_heading = GLOBAL_OPTIONS)]
    pub actor: Option<String>,
    /// Act as the user.
    #[arg(long, global = true, conflicts_with = "actor", help_heading = GLOBAL_OPTIONS)]
    pub as_user: bool,
    /// How to print: the JSON envelope, or tables and summaries for people, with errors as text on standard error;
    /// otherwise `ACYCLIC_OUTPUT`, else pretty on a terminal and json when piped.
    #[arg(
        short = 'o',
        long,
        global = true,
        value_parser = values(&Format::ALL),
        help_heading = GLOBAL_OPTIONS
    )]
    pub output: Option<Format>,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// How a command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// One JSON document.
    Json,
    /// Tables and summaries, fitted to the terminal's width, with no borders when not printing to one.
    Pretty,
}

impl Format {
    /// Every format, in the order help lists them.
    pub const ALL: [Self; 2] = [Self::Json, Self::Pretty];
}

/// The top level's help sections, in order: what every session uses, the groups, then the rest.
pub const SECTIONS: [(&str, &[&str]); 3] = [
    ("Common Commands", &["ready", "list", "show", "claim", "release", "done", "needs"]),
    ("Command Groups", &["item", "link", "thread", "repo", "proposal"]),
    (
        "Other Commands",
        &[
            "init",
            "sync",
            "ops",
            "prime",
            "import",
            "graph",
            "level",
            "doctor",
            "serve",
            "up",
            "completions",
        ],
    ),
];

/// The command line, with its top-level help in [`SECTIONS`] rather than one list.
pub fn command() -> clap::Command {
    let mut command = Cli::command();
    command.build();
    let width = command.get_subcommands().map(|c| c.get_name().len()).max().unwrap_or(0);
    let mut template = String::from("{about-with-newline}\nUsage: acyclic [OPTIONS] <COMMAND>\n");
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
    template.push_str(
        "\n{all-args}\n\nRun 'acyclic help <command>' for more information on a command.\n",
    );
    // The sections list the commands, so clap's own list is left out.
    command.mut_subcommands(|sub| sub.hide(true)).help_template(template)
}

/// The name serde gives a value of one of the model's enums: kebab-case.
fn name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        other => unreachable!("an enum serializes as its name, not {other:?}"),
    }
}

/// A parser for one of the model's enums that takes each of `all` by its kebab-case name, lists them in help, and
/// names them when a value isn't one.
fn values<T>(all: &'static [T]) -> impl TypedValueParser<Value = T>
where
    T: Serialize + Clone + Send + Sync + 'static,
{
    let names: Vec<String> = all.iter().map(name).collect();
    PossibleValuesParser::new(names.clone()).map(move |chosen| {
        let at = names.iter().position(|name| *name == chosen).expect("the parser checked it");
        all[at].clone()
    })
}

/// The longest lease, in minutes.
pub const LEASE_MAX_MINUTES: u32 = 480;

/// Reads a lease in minutes, refusing one outside 1 to the longest in plain words.
fn lease_minutes(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|minutes| (1..=LEASE_MAX_MINUTES).contains(minutes))
        .ok_or_else(|| format!("--lease takes 1 to {LEASE_MAX_MINUTES} minutes"))
}

/// The commands: the ones every session uses at the top, the rest grouped by what they act on.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
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
    Sync {
        /// Fetch every writer's usage records too, as a server does for cost totals.
        #[arg(long)]
        #[serde(default)]
        with_usage: bool,
    },
    /// Prints conventions for agents, matched to this version, as text.
    Prime,
    /// Lists threads with ready work, the actor's own first, each with its ready items.
    Ready {
        /// Judge archived work as if it weren't archived.
        #[arg(long)]
        #[serde(default)]
        include_archived: bool,
        /// Only items in threads the user approved, with the item among what the approval saw.
        #[arg(short = 'a', long)]
        #[serde(default)]
        approved: bool,
    },
    /// Lists every item, by id.
    List,
    /// Shows items with their role, readiness and claims, as an array in the order given.
    Show {
        /// The items.
        #[arg(required = true, value_name = "ITEM")]
        #[serde(default)]
        items: Vec<String>,
    },
    /// Claims a thread with ready work, or renews the actor's claim on it.
    Claim {
        /// The thread.
        thread: String,
        /// The lease in minutes: 60 by default, 1 to 480. A run's claim has none and lives while the run does.
        #[arg(short = 'l', long, value_parser = lease_minutes)]
        lease: Option<u32>,
        /// Claim even when the thread has no ready work.
        #[arg(short = 'f', long)]
        #[serde(default)]
        force: bool,
    },
    /// Releases the actor's claim on a thread, leaving anyone else's.
    Release {
        /// The thread.
        thread: String,
    },
    /// Marks an item done, with evidence.
    Done {
        /// The item.
        item: String,
        /// The repository record holding the commit.
        #[arg(short = 'r', long, requires = "commit")]
        repository: Option<String>,
        /// The commit that completed it.
        #[arg(short = 'c', long, requires = "repository")]
        commit: Option<String>,
        /// A note, such as where a record lives.
        #[arg(short = 'n', long)]
        note: Option<String>,
        /// Finish it although it isn't ready or someone else holds it.
        #[arg(short = 'f', long)]
        #[serde(default)]
        force: bool,
    },
    /// Lists what needs the user.
    Needs,
    /// Prints the autonomy levels; with a level, sets the global one (user only).
    Level {
        /// The global level to set; none prints it and every thread's own.
        #[arg(value_parser = values(&Level::ALL))]
        level: Option<Level>,
    },
    /// Imports a plan, adding and updating what it names and never deleting; `--schema` prints an example.
    Import {
        /// The import document.
        #[arg(required_unless_present = "schema")]
        file: Option<PathBuf>,
        /// Report what would change without writing it.
        #[arg(short = 'n', long, conflicts_with = "schema")]
        #[serde(default)]
        dry_run: bool,
        /// Print an annotated example of the document's current version as text, and import nothing.
        #[arg(long, conflicts_with = "file")]
        #[serde(default)]
        schema: bool,
    },
    /// Lists entities that can't be read.
    Doctor,
    /// Draws the graph with threads as clusters: DOT or Mermaid, or an HTML page to open in a browser.
    Graph {
        /// The document's format: DOT by default, HTML with `--open`. HTML needs `--file` or `--open`.
        #[arg(long, value_enum)]
        format: Option<GraphFormat>,
        /// What lays out the HTML page: Graphviz by default.
        #[arg(long, value_enum)]
        renderer: Option<Renderer>,
        /// Write the HTML page into the clone's acyclic directory and open it in the default browser.
        #[arg(long)]
        #[serde(default)]
        open: bool,
        /// Only this item and its descendants.
        #[arg(long)]
        root: Option<String>,
        /// Only this thread's members; with `--root`, the items in both.
        #[arg(long)]
        thread: Option<String>,
        /// Draw archived items too.
        #[arg(long)]
        #[serde(default)]
        include_archived: bool,
        /// Write the document here and print a JSON summary instead.
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Adds, edits, reopens, archives and deletes items.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Adds or removes links between items.
    #[command(subcommand)]
    Link(LinkCommand),
    /// Adds, edits, lists, shows, approves and revokes threads; agents' edits to approved scope wait as proposals.
    #[command(subcommand)]
    Thread(ThreadCommand),
    /// Adds, edits, lists and shows repository records.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Lists and shows proposals, and keeps or drops them (user only).
    #[command(subcommand)]
    Proposal(ProposalCommand),
    /// Pushes a run's queued operations, or drops the ones the tracker hasn't spent.
    ///
    /// `git config acyclic.operations <run>` sets a clone to queue run <run>'s writes as operations instead of making
    /// them, and to sync without pushing.
    #[command(subcommand)]
    #[serde(skip)]
    Ops(OpsCommand),
    /// Serves the JSON API and the app, refusing any request without the user's Tailscale identity.
    #[serde(skip)]
    Serve(Serve),
    /// Serves the JSON API and the app on this machine, to whoever holds its session.
    ///
    /// The session lives in ~/.config/acyclic/up/session, readable only by the user, and a request presents it as
    /// `Authorization: Bearer <session>`.
    #[serde(skip)]
    Up(Up),
    /// Prints a completion script for a shell, as text.
    ///
    /// For zsh, write it to a directory on `fpath`: `acyclic completions zsh > ~/.zfunc/_acyclic`.
    #[serde(skip)]
    Completions {
        /// The shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

/// The server's settings.
#[derive(Debug, Clone, clap::Args)]
pub struct Serve {
    /// Where to listen. Whatever reaches this address directly can forge the identity header, localhost included, which
    /// every process and container on the machine can reach: keep it reachable only from the proxy that sets the
    /// header, such as in a pod of its own. On a Mac, use `acyclic up`.
    #[arg(long, default_value = "127.0.0.1:7477")]
    pub listen: std::net::SocketAddr,
    /// The user's Tailscale login, which every request's `Tailscale-User-Login` header must equal.
    #[arg(long)]
    pub login: String,
    /// The name every request's `Host` header must carry, such as the server's `.ts.net` name, with a port only if
    /// requests carry one.
    #[arg(long)]
    pub host: String,
    /// The origin a browser's request must come from; `https://<host>` by default.
    #[arg(long)]
    pub origin: Option<String>,
    /// A directory of static files to serve, such as the built app. Symbolic links in it are followed, so it should
    /// hold only what the server's own build put there.
    #[arg(long = "static")]
    pub static_dir: Option<PathBuf>,
    /// Seconds between syncs with the clone's `origin`; 0 never syncs. Writes also push when the clone's
    /// `acyclic.pushAfterWrite` says so.
    #[arg(long, default_value_t = 30)]
    pub sync_every: u64,
}

/// The settings of `acyclic up`.
#[derive(Debug, Clone, clap::Args)]
pub struct Up {
    /// The port to listen on, on 127.0.0.1 only; 0 picks a free one.
    #[arg(long, default_value_t = 7478)]
    pub port: u16,
    /// A directory of static files to serve, such as the built app. Symbolic links in it are followed, so it should
    /// hold only what acyclic's own build put there.
    #[arg(long = "static")]
    pub static_dir: Option<PathBuf>,
    /// Seconds between syncs with the clone's `origin`; 0, the default, never syncs, so the clone syncs only when
    /// told to, as it does without the server.
    #[arg(long, default_value_t = 0)]
    pub sync_every: u64,
    /// Replace the session with a new one before serving, so whatever held the old one, paired browsers included, is
    /// shut out. A server already running keeps the old session until it restarts.
    #[arg(long)]
    pub rotate: bool,
    /// Print a link that pairs a browser with the running server, valid once for five minutes, and serve nothing.
    /// Give the same --port as the server, if it has one.
    #[arg(long, conflicts_with_all = ["rotate", "static_dir", "sync_every", "tailscale"])]
    pub link: bool,
    /// Also serve through `tailscale serve`, pointed at a Unix socket that only the user can open, to requests carrying
    /// the user's Tailscale identity. Run `tailscale serve --bg unix:<socket>` as the output says.
    #[arg(long, requires_all = ["login", "host"])]
    pub tailscale: bool,
    /// The user's Tailscale login, which every request through the socket must carry in `Tailscale-User-Login`.
    #[arg(long, requires = "tailscale")]
    pub login: Option<String>,
    /// The machine's `.ts.net` name, which every request through the socket must carry in `Host`.
    #[arg(long, requires = "tailscale")]
    pub host: Option<String>,
    /// The origin a browser's request through the socket must come from; `https://<host>` by default.
    #[arg(long, requires = "tailscale")]
    pub origin: Option<String>,
    /// The socket; ~/.config/acyclic/up/tailscale.sock by default. Its directory should be open to the user alone.
    #[arg(long, requires = "tailscale")]
    pub socket: Option<PathBuf>,
}

/// A usage error.
fn usage(message: impl Into<String>) -> CliError {
    CliError::new("usage", exit::USAGE, message)
}

/// Refuses a flag and the one that clears it, given together.
fn clears(flag: &str, set: bool, clear: bool) -> Result<(), CliError> {
    if set && clear {
        return Err(usage(format!("--{flag} and --no-{flag} contradict each other: pass one")));
    }
    Ok(())
}

/// Refuses a set-valued flag and the one that removes from it, naming the same element.
fn overlaps(flag: &str, add: &[String], remove: &[String]) -> Result<(), CliError> {
    match add.iter().find(|name| remove.contains(name)) {
        Some(name) => Err(usage(format!("--{flag} and --no-{flag} both name {name}"))),
        None => Ok(()),
    }
}

/// What a run does with its queue of operations.
#[derive(Debug, Clone, Copy, Subcommand)]
pub enum OpsCommand {
    /// Pushes the queue for the operator to apply, and says how far the tracker got.
    ///
    /// A push the operator refused in part leaves the operations it applied spent. Push again after a failure that
    /// may pass, such as a busy tracker: what was applied is found applied, and the rest is sent again.
    Push,
    /// Drops the queued operations after the last one the tracker spent, to abandon them.
    ///
    /// For operations the run won't send as they are, such as one the operator refused to forward; not for a failure
    /// that may pass, which a push retries. Refused while a push's outcome is unknown: push again first.
    Reset,
}

impl Command {
    /// Whether a run may send this command as an operation: the writes of its own work, and nothing that approves,
    /// configures, reaches outside the tracker or reshapes threads. With no catch-all arm, a new command fails to
    /// compile until it is placed.
    #[must_use]
    pub fn run_may_send(&self) -> bool {
        match self {
            Self::Claim { force: false, .. }
            | Self::Release { .. }
            | Self::Done { force: false, .. }
            | Self::Item(
                ItemCommand::Add { .. } | ItemCommand::Edit(_) | ItemCommand::Reopen { .. },
            )
            | Self::Link(LinkCommand::Add { .. } | LinkCommand::Remove { .. }) => true,
            // Forcing a claim past readiness, or a finish past its checks, is the user's to do.
            Self::Claim { force: true, .. }
            | Self::Done { force: true, .. }
            | Self::Init { .. }
            | Self::Sync { .. }
            | Self::Prime
            | Self::Ready { .. }
            | Self::List
            | Self::Show { .. }
            | Self::Needs
            | Self::Level { .. }
            | Self::Import { .. }
            | Self::Graph { .. }
            | Self::Doctor
            | Self::Item(ItemCommand::Archive { .. } | ItemCommand::Delete { .. })
            | Self::Thread(_)
            | Self::Repo(_)
            | Self::Proposal(_)
            | Self::Ops(_)
            | Self::Serve(_)
            | Self::Up(_)
            | Self::Completions { .. } => false,
        }
    }

    /// Checks what the command line's parser checks but deserializing doesn't: the arguments it requires, and the ones
    /// that require or exclude each other, and the flags that clear what another sets.
    ///
    /// # Errors
    ///
    /// A usage error naming the missing or clashing arguments.
    pub fn validate(&self) -> Result<(), CliError> {
        match self {
            Self::Show { items } if items.is_empty() => Err(usage("show needs at least one item")),
            Self::Thread(ThreadCommand::Show { threads }) if threads.is_empty() => {
                Err(usage("thread show needs at least one thread"))
            },
            Self::Repo(RepoCommand::Show { repositories }) if repositories.is_empty() => {
                Err(usage("repo show needs at least one repository"))
            },
            Self::Proposal(ProposalCommand::Show { proposals }) if proposals.is_empty() => {
                Err(usage("proposal show needs at least one proposal"))
            },
            Self::Claim { lease: Some(lease), .. } if !(1..=LEASE_MAX_MINUTES).contains(lease) => {
                Err(usage(format!("--lease takes 1 to {LEASE_MAX_MINUTES} minutes")))
            },
            Self::Done { repository, commit, .. } if repository.is_some() != commit.is_some() => {
                Err(usage("done takes --repository and --commit together, or --note alone"))
            },
            Self::Import { file: None, schema: false, .. } => {
                Err(usage("import needs a file, or --schema for an example of one"))
            },
            Self::Import { file: Some(_), schema: true, .. }
            | Self::Import { dry_run: true, schema: true, .. } => {
                Err(usage("--schema prints an example: it takes no file and no --dry-run"))
            },
            Self::Item(ItemCommand::Edit(edit)) => edit.validate(),
            Self::Thread(ThreadCommand::Edit(edit)) => edit.validate(),
            Self::Repo(RepoCommand::Add { public, private, .. }) if public == private => {
                Err(usage("a repository is --public or --private: pass one"))
            },
            Self::Repo(RepoCommand::Edit { public: true, private: true, .. }) => {
                Err(usage("--public and --private contradict each other"))
            },
            _ => Ok(()),
        }
    }
}

/// A graph document's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphFormat {
    /// Graphviz DOT.
    Dot,
    /// A Mermaid flowchart.
    Mermaid,
    /// A self-contained HTML page that draws it.
    Html,
}

/// The states a command may set: not `done`, which `acyclic done` sets with its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateWord {
    /// Not started.
    Backlog,
    /// Set aside on purpose.
    Parked,
    /// Being worked on.
    InProgress,
}

impl StateWord {
    /// Every state a command may set.
    pub const ALL: [Self; 3] = [Self::Backlog, Self::Parked, Self::InProgress];

    /// The model's state for the word.
    #[must_use]
    pub fn state(self) -> ItemState {
        match self {
            Self::Backlog => ItemState::Backlog,
            Self::Parked => ItemState::Parked,
            Self::InProgress => ItemState::InProgress,
        }
    }
}

/// The item commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum ItemCommand {
    /// Adds an item; adding it again with the same fields changes nothing, and with other fields is refused.
    Add {
        /// The new item.
        item: String,
        /// Its title.
        #[arg(short = 't', long)]
        title: String,
        /// Its parent.
        #[arg(short = 'p', long)]
        parent: Option<String>,
        /// Its thread.
        #[arg(long)]
        thread: Option<String>,
        /// Its state; backlog by default.
        #[arg(short = 's', long, value_parser = values(&StateWord::ALL))]
        state: Option<StateWord>,
        /// Its notes.
        #[arg(long)]
        notes: Option<String>,
        /// Its gate.
        #[arg(long)]
        gate: Option<String>,
        /// Its size.
        #[arg(long, value_parser = values(&Size::ALL))]
        size: Option<Size>,
        /// An area it is in; repeat for more.
        #[arg(long, value_name = "AREA")]
        #[serde(default)]
        area: Vec<String>,
        /// A named resource it holds while worked on; repeat for more.
        #[arg(long, value_name = "RESOURCE")]
        #[serde(default)]
        resource: Vec<String>,
        /// A repository it touches; repeat for more.
        #[arg(long, value_name = "REPOSITORY")]
        #[serde(default)]
        repository: Vec<String>,
    },
    /// Edits an item: `--field` sets a field or adds to a set, `--no-field` clears it or removes from a set.
    Edit(ItemEdit),
    /// Reopens a done item.
    Reopen {
        /// The item.
        item: String,
    },
    /// Archives a done subtree (user or project-management run only).
    Archive {
        /// The subtree's root.
        item: String,
    },
    /// Deletes an item nothing refers to: no children, links, evidence or live claim; in an approved thread, user only.
    Delete {
        /// The item.
        item: String,
    },
}

/// What `item edit` changes.
#[derive(Debug, clap::Args, Serialize, Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "each field has a flag that clears it")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ItemEdit {
    /// The item.
    pub item: String,
    /// A new title.
    #[arg(short = 't', long)]
    pub title: Option<String>,
    /// Put it under this parent.
    #[arg(short = 'p', long)]
    pub parent: Option<String>,
    /// Put it at the top level.
    #[arg(long)]
    #[serde(default)]
    pub no_parent: bool,
    /// Put it in this thread, taking it out of any other.
    #[arg(long)]
    pub thread: Option<String>,
    /// Take it out of its thread.
    #[arg(long)]
    #[serde(default)]
    pub no_thread: bool,
    /// Set its state: not `done`, which `acyclic done` sets.
    #[arg(short = 's', long, value_parser = values(&StateWord::ALL))]
    pub state: Option<StateWord>,
    /// New notes.
    #[arg(long)]
    pub notes: Option<String>,
    /// Clear its notes.
    #[arg(long)]
    #[serde(default)]
    pub no_notes: bool,
    /// A new gate.
    #[arg(long)]
    pub gate: Option<String>,
    /// Clear its gate.
    #[arg(long)]
    #[serde(default)]
    pub no_gate: bool,
    /// A new size.
    #[arg(long, value_parser = values(&Size::ALL))]
    pub size: Option<Size>,
    /// Clear its size.
    #[arg(long)]
    #[serde(default)]
    pub no_size: bool,
    /// Add an area; repeat for more.
    #[arg(long, value_name = "AREA")]
    #[serde(default)]
    pub area: Vec<String>,
    /// Remove an area; repeat for more.
    #[arg(long, value_name = "AREA")]
    #[serde(default)]
    pub no_area: Vec<String>,
    /// Add a named resource; repeat for more.
    #[arg(long, value_name = "RESOURCE")]
    #[serde(default)]
    pub resource: Vec<String>,
    /// Remove a named resource; repeat for more.
    #[arg(long, value_name = "RESOURCE")]
    #[serde(default)]
    pub no_resource: Vec<String>,
    /// Add a repository it touches; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub repository: Vec<String>,
    /// Remove a repository it touches; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub no_repository: Vec<String>,
}

impl ItemEdit {
    fn validate(&self) -> Result<(), CliError> {
        clears("parent", self.parent.is_some(), self.no_parent)?;
        clears("thread", self.thread.is_some(), self.no_thread)?;
        clears("notes", self.notes.is_some(), self.no_notes)?;
        clears("gate", self.gate.is_some(), self.no_gate)?;
        clears("size", self.size.is_some(), self.no_size)?;
        overlaps("area", &self.area, &self.no_area)?;
        overlaps("resource", &self.resource, &self.no_resource)?;
        overlaps("repository", &self.repository, &self.no_repository)
    }
}

/// A link's kind as the command line names it. The stored names are the model's [`LinkKind`], which are kept as they
/// are on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkWord {
    /// `<FROM>` can't start until `<TO>` is done.
    WaitsFor,
    /// Context only: neither orders the other.
    RelatesTo,
}

impl LinkWord {
    /// Every word.
    pub const ALL: [Self; 2] = [Self::WaitsFor, Self::RelatesTo];

    /// The model's kind for the word.
    #[must_use]
    pub fn kind(self) -> LinkKind {
        match self {
            Self::WaitsFor => LinkKind::Blocks,
            Self::RelatesTo => LinkKind::Relates,
        }
    }
}

/// What the link commands' help shows beside their arguments.
const LINK_EXAMPLE: &str = concat!(
    "Kinds:\n",
    "  waits-for   <FROM> can't start until <TO> is done\n",
    "  relates-to  context only: neither orders the other\n",
    "\n",
    "Example: acyclic link add plant waits-for beds (plant can't start until beds is done)"
);

/// The link commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum LinkCommand {
    /// Links two items: `<FROM>` waits for or relates to `<TO>`.
    #[command(after_help = LINK_EXAMPLE)]
    Add {
        /// The item that waits or relates.
        from: String,
        /// How `<FROM>` stands to `<TO>`.
        #[arg(value_parser = values(&LinkWord::ALL))]
        kind: LinkWord,
        /// The item it stands to.
        to: String,
    },
    /// Removes a link: `<FROM>` no longer waits for or relates to `<TO>`.
    #[command(after_help = LINK_EXAMPLE)]
    Remove {
        /// The item that waits or relates.
        from: String,
        /// How `<FROM>` stands to `<TO>`.
        #[arg(value_parser = values(&LinkWord::ALL))]
        kind: LinkWord,
        /// The item it stands to.
        to: String,
    },
}

/// The thread commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum ThreadCommand {
    /// Lists every thread, by id, with its approval, its members and its heads: the open leaves nothing in it orders.
    List,
    /// Shows threads with their heads and each member's title, state and readiness, as an array in the order given.
    Show {
        /// The threads.
        #[arg(required = true, value_name = "THREAD")]
        #[serde(default)]
        threads: Vec<String>,
    },
    /// Adds a thread; adding it again with the same fields changes nothing, and with other fields is refused.
    Add {
        /// The new thread.
        thread: String,
        /// Its title.
        #[arg(short = 't', long)]
        title: String,
        /// Its brief.
        #[arg(short = 'b', long)]
        brief: Option<String>,
        /// A repository it touches beyond its items'; repeat for more.
        #[arg(long, value_name = "REPOSITORY")]
        #[serde(default)]
        repository: Vec<String>,
        /// Its autonomy level (user only).
        #[arg(long, value_parser = values(&Level::ALL))]
        level: Option<Level>,
    },
    /// Edits a thread: `--field` sets a field or adds to a set, `--no-field` clears it or removes from a set.
    Edit(ThreadEdit),
    /// Approves a thread's scope (user only).
    Approve {
        /// The thread.
        thread: String,
    },
    /// Revokes a thread's approval (user only).
    Revoke {
        /// The thread.
        thread: String,
    },
}

/// What `thread edit` changes.
#[derive(Debug, clap::Args, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ThreadEdit {
    /// The thread.
    pub thread: String,
    /// A new title.
    #[arg(short = 't', long)]
    pub title: Option<String>,
    /// A new brief.
    #[arg(short = 'b', long)]
    pub brief: Option<String>,
    /// Clear its brief.
    #[arg(long)]
    #[serde(default)]
    pub no_brief: bool,
    /// Add a repository it touches beyond its items'; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub repository: Vec<String>,
    /// Remove a repository; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub no_repository: Vec<String>,
    /// Set its autonomy level (user only).
    #[arg(long, value_parser = values(&Level::ALL))]
    pub level: Option<Level>,
    /// Clear its autonomy level, so the global one applies (user only).
    #[arg(long)]
    #[serde(default)]
    pub no_level: bool,
}

impl ThreadEdit {
    fn validate(&self) -> Result<(), CliError> {
        clears("brief", self.brief.is_some(), self.no_brief)?;
        clears("level", self.level.is_some(), self.no_level)?;
        overlaps("repository", &self.repository, &self.no_repository)
    }
}

/// The repository commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum RepoCommand {
    /// Lists every repository record, by id.
    List,
    /// Shows repository records with the items and threads that name them, as an array in the order given.
    Show {
        /// The repositories.
        #[arg(required = true, value_name = "REPOSITORY")]
        #[serde(default)]
        repositories: Vec<String>,
    },
    /// Adds a repository record; adding it again with the same fields changes nothing, and with other fields is
    /// refused.
    Add {
        /// The new record.
        repository: String,
        /// The owner.
        #[arg(long)]
        owner: String,
        /// The repository's name.
        #[arg(long)]
        name: String,
        /// The default branch.
        #[arg(long)]
        default_branch: String,
        /// It is public: this or `--private`.
        #[arg(long)]
        #[serde(default)]
        public: bool,
        /// It is private: this or `--public`.
        #[arg(long)]
        #[serde(default)]
        private: bool,
    },
    /// Edits a repository record: any of its fields.
    Edit {
        /// The record.
        repository: String,
        /// A new owner.
        #[arg(long)]
        owner: Option<String>,
        /// A new name.
        #[arg(long)]
        name: Option<String>,
        /// A new default branch.
        #[arg(long)]
        default_branch: Option<String>,
        /// It is public.
        #[arg(long)]
        #[serde(default)]
        public: bool,
        /// It is private.
        #[arg(long)]
        #[serde(default)]
        private: bool,
    },
}

/// The proposal commands (user only).
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum ProposalCommand {
    /// Lists every proposal, open ones first and then decided ones, each by id, with who proposed it, its edit and its
    /// decision.
    List,
    /// Shows proposals with who proposed them, the edit and any decision, as an array in the order given.
    Show {
        /// The proposals.
        #[arg(required = true, value_name = "PROPOSAL")]
        #[serde(default)]
        proposals: Vec<String>,
    },
    /// Keeps a proposal (user only).
    Keep {
        /// The proposal.
        proposal: String,
    },
    /// Drops a proposal (user only).
    Drop {
        /// The proposal.
        proposal: String,
    },
}
