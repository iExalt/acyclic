//! The command line.

use std::path::PathBuf;

use acyclic_core::model::{ItemState, Size};
use std::fmt::Write as _;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{CommandFactory, Parser, Subcommand};

use serde::Deserialize;

use crate::error::{CliError, exit};
use serde::Serialize;

/// The heading the options that every command takes sit under in its help.
const GLOBAL_OPTIONS: &str = "Global options";

/// The acyclic tracker. Every command prints one JSON document when piped and text for people at a terminal (`-o`
/// chooses), except `prime`, `completions`, `import --schema` and help and version text. A command line that doesn't parse prints its error as text on standard error, whatever `-o` says.
#[derive(Debug, Parser)]
#[command(name = "acyclic", version, arg_required_else_help = true)]
pub struct Cli {
    /// The tracker clone; otherwise `ACYCLIC_TRACKER`, else the clone you are in, else, in a Git repository, the mapping
    /// in ~/.config/acyclic/trackers.toml, whose default applies when no prefix matches.
    #[arg(long, global = true, help_heading = GLOBAL_OPTIONS)]
    pub tracker: Option<PathBuf>,
    /// Who acts, as `kind/id` such as `session/mac`; otherwise `ACYCLIC_ACTOR`, else the user, `actor/user`.
    #[arg(long, global = true, help_heading = GLOBAL_OPTIONS)]
    pub actor: Option<String>,
    /// How to print: the JSON envelope, or tables and summaries for people, with errors as text on standard error;
    /// otherwise pretty on a terminal and json when piped.
    #[arg(
        short = 'o',
        long,
        global = true,
        value_enum,
        help_heading = GLOBAL_OPTIONS
    )]
    pub output: Option<Format>,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// How a command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[expect(
    missing_docs,
    reason = "clap prints a value's doc as its help, which switches help to its long layout"
)]
pub enum Format {
    // One JSON document.
    Json,
    // Tables and summaries, fitted to the terminal's width, with no borders when not printing to one.
    Pretty,
}

/// The top level's help sections, in order: the groups, then the commands that act on the tracker or the tool.
pub const SECTIONS: [(&str, &[&str]); 2] = [
    ("Command Groups", &["item", "thread", "repository"]),
    (
        "Other Commands",
        &["needs", "init", "sync", "prime", "id", "import", "doctor", "serve", "up", "completions"],
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

/// The most ids `acyclic id` prints at once.
pub const ID_MAX: u32 = 1000;

/// Reads how many ids to print, refusing a count outside 1 to [`ID_MAX`] in plain words.
fn id_count(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|count| (1..=ID_MAX).contains(count))
        .ok_or_else(|| format!("acyclic id prints 1 to {ID_MAX} ids"))
}

fn one() -> u32 {
    1
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

/// The commands: one group per kind of record, and at the top the ones that act on the tracker or the tool.
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
    Sync,
    /// Prints conventions for agents, matched to this version, as text.
    Prime,
    /// Prints new `UUIDv7` ids, one per line, as text, to name records with in `--id` or an import document.
    Id {
        /// How many: 1 to 1000.
        #[arg(default_value_t = 1, value_parser = id_count)]
        #[serde(default = "one")]
        count: u32,
    },
    /// Lists what needs the user.
    Needs,
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
    /// Adds, edits, shows, lists, finishes, reopens, archives and deletes items.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Adds, edits, shows and lists threads, and finds, claims and releases ready work.
    #[command(subcommand)]
    Thread(ThreadCommand),
    /// Adds, edits, shows and lists repository records.
    #[command(subcommand)]
    Repository(RepositoryCommand),
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
    /// A directory holding the built app, to serve in place of the copy built into the binary; it must have an
    /// `index.html`. Symbolic links in it are followed, so it should hold only what the server's own build put there.
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
    /// A directory holding the built app, to serve in place of the copy built into the binary; it must have an
    /// `index.html`. Symbolic links in it are followed, so it should hold only what acyclic's own build put there.
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
    /// Also serve through `tailscale serve`, to requests carrying the user's Tailscale identity: on a port of 127.0.0.1,
    /// under a secret first path segment derived from the session, or on a Unix socket with --socket. Run the
    /// `tailscale serve --bg` command the output gives. Tailscale's Mac apps can't open a Unix socket, and every process
    /// and container on the machine can reach the port, so the port answers only paths under the secret, which
    /// `tailscale serve` adds; --rotate changes it, and the command must be run again.
    #[arg(long, requires_all = ["login", "host"])]
    pub tailscale: bool,
    /// The user's Tailscale login, which every request through Tailscale must carry in `Tailscale-User-Login`.
    #[arg(long, requires = "tailscale")]
    pub login: Option<String>,
    /// The machine's `.ts.net` name, without a port, which every request through Tailscale must carry in `Host`.
    #[arg(long, requires = "tailscale")]
    pub host: Option<String>,
    /// The origin a browser's request through Tailscale must come from; `https://<host>` by default.
    #[arg(long, requires = "tailscale")]
    pub origin: Option<String>,
    /// The port of 127.0.0.1 that `tailscale serve` sends requests to; 0 picks a free one. Kept from run to run, since
    /// `tailscale serve --bg` remembers it.
    #[arg(long, requires = "tailscale", default_value_t = 7479)]
    pub tailscale_port: u16,
    /// A Unix socket for `tailscale serve` in place of the port, for the open-source tailscaled, which can open one.
    /// Its directory should be open to the user alone; only the user's processes can then reach it, so it needs no
    /// secret.
    #[arg(long, requires = "tailscale", conflicts_with = "tailscale_port")]
    pub socket: Option<PathBuf>,
}

/// What a command needs and does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Needs no tracker: `init`, `prime`, `id`, `completions` and `import --schema`.
    Standalone,
    /// Starts a server: `serve` and `up`.
    Server,
    /// Works on the clone as a whole: `sync`, `import` and `doctor`.
    Clone,
    /// Only reads the tracker.
    Read,
    /// May write the tracker.
    Write,
}

impl Command {
    /// What the command needs and does. With no catch-all arm, a new command or subcommand fails to compile until it
    /// is placed here.
    #[must_use]
    pub fn class(&self) -> Class {
        match self {
            Self::Init { .. }
            | Self::Prime
            | Self::Id { .. }
            | Self::Completions { .. }
            | Self::Import { schema: true, .. } => Class::Standalone,
            Self::Serve(_) | Self::Up(_) => Class::Server,
            Self::Sync | Self::Import { .. } | Self::Doctor => Class::Clone,
            Self::Needs
            | Self::Item(ItemCommand::List | ItemCommand::Show { .. })
            | Self::Thread(
                ThreadCommand::List | ThreadCommand::Show { .. } | ThreadCommand::Ready { .. },
            )
            | Self::Repository(RepositoryCommand::List | RepositoryCommand::Show { .. }) => {
                Class::Read
            },
            Self::Item(
                ItemCommand::Add { .. }
                | ItemCommand::Edit(_)
                | ItemCommand::Done { .. }
                | ItemCommand::Reopen { .. }
                | ItemCommand::Archive { .. }
                | ItemCommand::Delete { .. },
            )
            | Self::Thread(
                ThreadCommand::Add { .. }
                | ThreadCommand::Edit(_)
                | ThreadCommand::Claim { .. }
                | ThreadCommand::Release { .. }
                | ThreadCommand::Delete { .. },
            )
            | Self::Repository(RepositoryCommand::Add { .. } | RepositoryCommand::Edit { .. }) => {
                Class::Write
            },
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
            Self::Item(ItemCommand::Show { items }) if items.is_empty() => {
                Err(usage("item show needs at least one item"))
            },
            Self::Thread(ThreadCommand::Show { threads }) if threads.is_empty() => {
                Err(usage("thread show needs at least one thread"))
            },
            Self::Repository(RepositoryCommand::Show { repositories })
                if repositories.is_empty() =>
            {
                Err(usage("repository show needs at least one repository"))
            },
            Self::Thread(ThreadCommand::Claim { lease: Some(lease), .. })
                if !(1..=LEASE_MAX_MINUTES).contains(lease) =>
            {
                Err(usage(format!("--lease takes 1 to {LEASE_MAX_MINUTES} minutes")))
            },
            Self::Id { count } if !(1..=ID_MAX).contains(count) => {
                Err(usage(format!("acyclic id prints 1 to {ID_MAX} ids")))
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
            _ => Ok(()),
        }
    }
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

/// The states a command may set: not `done`, which `acyclic item done` sets with its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[expect(
    missing_docs,
    reason = "clap prints a value's doc as its help, which switches help to its long layout"
)]
pub enum StateWord {
    // Not started.
    Backlog,
    // Set aside on purpose.
    Parked,
}

impl StateWord {
    /// The model's state for the word.
    #[must_use]
    pub fn state(self) -> ItemState {
        match self {
            Self::Backlog => ItemState::Backlog,
            Self::Parked => ItemState::Parked,
        }
    }
}

/// The item commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum ItemCommand {
    /// Adds an item; adding it again with the same fields changes nothing, and with other fields is refused. An add
    /// whose title is an open item's is refused.
    Add {
        /// The new item's id, a `UUIDv7` (`acyclic id` prints one); left out, the tracker makes one.
        #[arg(long)]
        id: Option<String>,
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
        #[arg(short = 's', long, value_enum)]
        state: Option<StateWord>,
        /// Its notes.
        #[arg(long)]
        notes: Option<String>,
        /// Its size.
        #[arg(long, value_parser = values(&Size::ALL))]
        size: Option<Size>,
        /// A repository it touches; repeat for more.
        #[arg(short = 'r', long, value_name = "REPOSITORY")]
        #[serde(default)]
        repository: Vec<String>,
        /// An item it waits for: it can't start until that one is done; repeat for more.
        #[arg(long, value_name = "ITEM")]
        #[serde(default)]
        waits_for: Vec<String>,
    },
    /// Edits an item: `--field` sets a field or adds to a set, `--no-field` clears it or removes from a set.
    Edit(ItemEdit),
    /// Shows items with their role, readiness and claims, and a parent's progress from its leaves, as an array in the
    /// order given.
    Show {
        /// The items.
        #[arg(required = true, value_name = "ITEM")]
        #[serde(default)]
        items: Vec<String>,
    },
    /// Lists every item, by id.
    List,
    /// Marks an item done, with evidence.
    Done {
        /// The item.
        item: String,
        /// The commit that completed it, after the repository that holds it, such as `owner/name@1a2b3c4`.
        #[arg(short = 'c', long, value_name = "REPOSITORY@SHA")]
        commit: Option<String>,
        /// Evidence in words, such as where a record lives.
        #[arg(short = 'e', long)]
        evidence: Option<String>,
        /// Finish it although it isn't ready or someone else holds it, or replace the evidence it was finished with.
        #[arg(short = 'f', long)]
        #[serde(default)]
        force: bool,
    },
    /// Reopens a done item.
    Reopen {
        /// The item.
        item: String,
    },
    /// Archives a done subtree.
    Archive {
        /// The subtree's root.
        item: String,
    },
    /// Deletes an item nothing refers to: no children, nothing waiting for it or that it waits for, no evidence, not
    /// done, and no one else's live claim.
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
    /// Set its state: not `done`, which `acyclic item done` sets.
    #[arg(short = 's', long, value_enum)]
    pub state: Option<StateWord>,
    /// New notes.
    #[arg(long)]
    pub notes: Option<String>,
    /// Clear its notes.
    #[arg(long)]
    #[serde(default)]
    pub no_notes: bool,
    /// A new size.
    #[arg(long, value_parser = values(&Size::ALL))]
    pub size: Option<Size>,
    /// Clear its size.
    #[arg(long)]
    #[serde(default)]
    pub no_size: bool,
    /// Add a repository it touches; repeat for more.
    #[arg(short = 'r', long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub repository: Vec<String>,
    /// Remove a repository it touches, even one whose record is gone; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub no_repository: Vec<String>,
    /// Make it wait for an item: it can't start until that one is done; repeat for more.
    #[arg(long, value_name = "ITEM")]
    #[serde(default)]
    pub waits_for: Vec<String>,
    /// Stop it waiting for an item, even one that no longer exists; repeat for more.
    #[arg(long, value_name = "ITEM")]
    #[serde(default)]
    pub no_waits_for: Vec<String>,
}

impl ItemEdit {
    fn validate(&self) -> Result<(), CliError> {
        clears("parent", self.parent.is_some(), self.no_parent)?;
        clears("thread", self.thread.is_some(), self.no_thread)?;
        clears("notes", self.notes.is_some(), self.no_notes)?;
        clears("size", self.size.is_some(), self.no_size)?;
        overlaps("repository", &self.repository, &self.no_repository)?;
        overlaps("waits-for", &self.waits_for, &self.no_waits_for)
    }
}

/// The thread commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum ThreadCommand {
    /// Adds a thread; adding it again with the same fields changes nothing, and with other fields is refused. An add
    /// whose title is another thread's is refused.
    Add {
        /// The new thread's id, a `UUIDv7` (`acyclic id` prints one); left out, the tracker makes one.
        #[arg(long)]
        id: Option<String>,
        /// Its title.
        #[arg(short = 't', long)]
        title: String,
        /// Its brief.
        #[arg(short = 'b', long)]
        brief: Option<String>,
        /// A repository it touches beyond its items'; repeat for more.
        #[arg(short = 'r', long, value_name = "REPOSITORY")]
        #[serde(default)]
        repository: Vec<String>,
    },
    /// Edits a thread: `--field` sets a field or adds to a set, `--no-field` clears it or removes from a set.
    Edit(ThreadEdit),
    /// Shows threads with their heads and each member's title, state and readiness, as an array in the order given.
    Show {
        /// The threads.
        #[arg(required = true, value_name = "THREAD")]
        #[serde(default)]
        threads: Vec<String>,
    },
    /// Lists every thread, by id, with its members and its heads: the open leaves nothing in it orders.
    List,
    /// Lists threads with ready work, the actor's own first, each with its ready items.
    Ready {
        /// Judge archived work as if it weren't archived.
        #[arg(long)]
        #[serde(default)]
        include_archived: bool,
    },
    /// Claims a thread with ready work, or renews the actor's claim on it.
    Claim {
        /// The thread.
        thread: String,
        /// The lease in minutes: 60 by default, 1 to 480.
        #[arg(short = 'l', long, value_parser = lease_minutes)]
        lease: Option<u32>,
        /// Claim without checking the thread's readiness, as when it has no ready work. Another holder's live claim
        /// on it still refuses.
        #[arg(short = 'f', long)]
        #[serde(default)]
        force: bool,
    },
    /// Releases the actor's claim on a thread, leaving anyone else's.
    Release {
        /// The thread.
        thread: String,
    },
    /// Deletes a thread that no item is in and no one holds a live claim on.
    Delete {
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
    #[arg(short = 'r', long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub repository: Vec<String>,
    /// Remove a repository, even one whose record is gone; repeat for more.
    #[arg(long, value_name = "REPOSITORY")]
    #[serde(default)]
    pub no_repository: Vec<String>,
}

impl ThreadEdit {
    fn validate(&self) -> Result<(), CliError> {
        clears("brief", self.brief.is_some(), self.no_brief)?;
        overlaps("repository", &self.repository, &self.no_repository)
    }
}

/// The repository commands.
#[derive(Debug, Subcommand, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case", deny_unknown_fields)]
pub enum RepositoryCommand {
    /// Adds a repository record; adding it again with the same fields changes nothing, and with other fields is
    /// refused.
    Add {
        /// The new record's id, a `UUIDv7` (`acyclic id` prints one); left out, the tracker makes one.
        #[arg(long)]
        id: Option<String>,
        /// Its `owner/name`, which no other record has.
        #[arg(long, value_name = "OWNER/NAME")]
        name: String,
        /// The default branch.
        #[arg(long)]
        default_branch: String,
    },
    /// Edits a repository record: any of its fields.
    Edit {
        /// The record.
        repository: String,
        /// A new `owner/name`, which no other record has.
        #[arg(long, value_name = "OWNER/NAME")]
        name: Option<String>,
        /// A new default branch.
        #[arg(long)]
        default_branch: Option<String>,
    },
    /// Shows repository records with the items and threads that name them, as an array in the order given.
    Show {
        /// The repositories.
        #[arg(required = true, value_name = "REPOSITORY")]
        #[serde(default)]
        repositories: Vec<String>,
    },
    /// Lists every repository record, by id.
    List,
}
