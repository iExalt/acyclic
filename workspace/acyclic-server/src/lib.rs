#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use acyclic_cli::Env;
use acyclic_cli::args::{
    Command, ItemCommand, LinkCommand, ProposalCommand, RepoCommand, Serve, ThreadCommand,
};
use acyclic_cli::error::{CliError, exit};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{CONTENT_TYPE, HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tower_http::services::ServeDir;

/// The header Tailscale sets to the login of the user behind a request.
pub const IDENTITY: &str = "Tailscale-User-Login";

/// What the server checks requests against, and what it serves.
#[derive(Debug, Clone)]
pub struct Config {
    /// The user's login, which [`IDENTITY`] must equal.
    pub login: String,
    /// The name `Host` must carry, compared without regard to case, with its port if it has one.
    pub host: String,
    /// The origin a browser's request must come from, such as `https://<host>`.
    pub origin: String,
    /// The tracker clone every command runs on.
    pub tracker: PathBuf,
    /// The environment every command runs in.
    pub env: Env,
    /// The directory of static files, if any.
    pub static_dir: Option<PathBuf>,
}

impl Config {
    /// The configuration `acyclic serve` was given.
    ///
    /// # Errors
    ///
    /// A usage error when the login or the host is empty.
    pub fn new(options: &Serve, tracker: PathBuf, env: &Env) -> Result<Self, CliError> {
        if options.login.is_empty() || options.host.is_empty() {
            return Err(CliError::new("usage", exit::USAGE, "--login and --host may not be empty"));
        }
        Ok(Self {
            login: options.login.clone(),
            host: options.host.clone(),
            origin: options.origin.clone().unwrap_or_else(|| format!("https://{}", options.host)),
            tracker,
            env: env.clone(),
            static_dir: options.static_dir.clone(),
        })
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No identity header, or more than one.
    NoIdentity,
    /// Another login.
    WrongIdentity,
    /// No `Host`, more than one, or another name.
    WrongHost,
    /// A browser's request from another origin.
    WrongOrigin,
    /// A body that isn't JSON.
    NotJson,
    /// A command the server doesn't run.
    NotAllowed,
}

impl Refusal {
    fn code(self) -> &'static str {
        match self {
            Self::NoIdentity => "no-identity",
            Self::WrongIdentity => "wrong-identity",
            Self::WrongHost => "wrong-host",
            Self::WrongOrigin => "wrong-origin",
            Self::NotJson => "not-json",
            Self::NotAllowed => "not-allowed",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NoIdentity => "the request carries no single Tailscale identity",
            Self::WrongIdentity => "the request's Tailscale identity isn't the user's",
            Self::WrongHost => "the request is for another host",
            Self::WrongOrigin => "the request comes from another origin",
            Self::NotJson => "the request's body must be application/json",
            Self::NotAllowed => "the server doesn't run this command",
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let status = if self == Self::NotJson {
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        } else {
            StatusCode::FORBIDDEN
        };
        let envelope = json!({"ok": false, "error": {"code": self.code(), "message": self.message(), "details": null}});
        (status, Json(envelope)).into_response()
    }
}

/// The header's one value, or `None` when it is missing or repeated.
fn single(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<&[u8]> {
    let mut values = headers.get_all(name).into_iter();
    match (values.next(), values.next()) {
        (Some(value), None) => Some(value.as_bytes()),
        _ => None,
    }
}

/// Checks that a request comes from the user, through Tailscale, for this server.
///
/// # Errors
///
/// Why the request is refused.
pub fn identity(headers: &HeaderMap, config: &Config) -> Result<(), Refusal> {
    match single(headers, IDENTITY) {
        None => return Err(Refusal::NoIdentity),
        Some(login) if login != config.login.as_bytes() => return Err(Refusal::WrongIdentity),
        Some(_) => {},
    }
    match single(headers, HOST) {
        Some(host) if host.eq_ignore_ascii_case(config.host.as_bytes()) => Ok(()),
        _ => Err(Refusal::WrongHost),
    }
}

/// Checks that a command's request is JSON and, when a browser sent it, that it comes from the server's own origin, so
/// that no other site can make the user's browser send one.
///
/// # Errors
///
/// Why the request is refused.
pub fn same_origin(headers: &HeaderMap, config: &Config) -> Result<(), Refusal> {
    let json = single(headers, CONTENT_TYPE).is_some_and(|value| {
        let media = value.split(|byte| *byte == b';').next().unwrap_or_default();
        media.trim_ascii().eq_ignore_ascii_case(b"application/json")
    });
    if !json {
        return Err(Refusal::NotJson);
    }
    if headers.get(ORIGIN).is_none() {
        return Ok(());
    }
    match single(headers, ORIGIN) {
        Some(origin) if origin.eq_ignore_ascii_case(config.origin.as_bytes()) => Ok(()),
        _ => Err(Refusal::WrongOrigin),
    }
}

/// What a command does, for the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    /// It only reads, so it runs beside anything.
    Read,
    /// It may write, so it waits for the other writes and the sync loop.
    Write,
    /// The server doesn't run it for a request: it sets up or syncs the clone, reads or writes files on the server, or
    /// starts another server.
    Refused,
}

/// What a command does. With no catch-all arm, a new command or subcommand fails to compile until it is placed here.
fn classify(command: &Command) -> Use {
    match command {
        Command::Init { .. }
        | Command::Sync { .. }
        | Command::Prime
        | Command::Import { .. }
        | Command::Doctor
        | Command::Graph { .. }
        | Command::Serve(_) => Use::Refused,
        Command::Ready { .. }
        | Command::List
        | Command::Show { .. }
        | Command::Needs
        | Command::Level { level: None }
        | Command::Thread(ThreadCommand::List | ThreadCommand::Show { .. })
        | Command::Repo(RepoCommand::List | RepoCommand::Show { .. })
        | Command::Proposal(ProposalCommand::Show { .. }) => Use::Read,
        Command::Claim { .. }
        | Command::Release { .. }
        | Command::Done { .. }
        | Command::Level { level: Some(_) }
        | Command::Item(
            ItemCommand::Add { .. }
            | ItemCommand::Edit(_)
            | ItemCommand::Reopen { .. }
            | ItemCommand::Archive { .. }
            | ItemCommand::Delete { .. },
        )
        | Command::Link(LinkCommand::Add { .. } | LinkCommand::Remove { .. })
        | Command::Thread(
            ThreadCommand::Add { .. }
            | ThreadCommand::Edit(_)
            | ThreadCommand::Approve { .. }
            | ThreadCommand::Revoke { .. },
        )
        | Command::Repo(RepoCommand::Add { .. } | RepoCommand::Edit { .. })
        | Command::Proposal(ProposalCommand::Keep { .. } | ProposalCommand::Drop { .. }) => {
            Use::Write
        },
    }
}

/// The HTTP status for a command's exit code.
fn status(code: u8) -> StatusCode {
    match code {
        exit::OK => StatusCode::OK,
        exit::REFUSED => StatusCode::CONFLICT,
        exit::USAGE => StatusCode::BAD_REQUEST,
        exit::NOT_FOUND => StatusCode::NOT_FOUND,
        exit::STORE => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// How long a write waits for its turn before the server answers that it is busy.
const TURN: Duration = Duration::from_secs(15);

/// A running server's state.
struct Server {
    config: Config,
    /// Held by each write and each sync until it finishes, so that their pushes never race each other.
    turns: Arc<tokio::sync::Mutex<()>>,
    /// How long a write waits for its turn.
    turn: Duration,
}

impl Server {
    fn new(config: Config) -> Self {
        Self { config, turns: Arc::default(), turn: TURN }
    }

    /// Waits up to [`Self::turn`] for a write's turn.
    async fn turn(&self) -> Result<OwnedMutexGuard<()>, CliError> {
        tokio::time::timeout(self.turn, self.turns.clone().lock_owned()).await.map_err(|_| {
            CliError::new(
                "busy",
                exit::STORE,
                "the server is still syncing or writing; try again shortly",
            )
        })
    }

    /// Runs a command as the user, off the async threads, and returns its exit code and envelope. A turn, if given,
    /// is kept until the command finishes, even if the request is dropped meanwhile.
    async fn call(
        self: &Arc<Self>,
        command: Command,
        turn: Option<OwnedMutexGuard<()>>,
    ) -> (u8, Value) {
        let server = Arc::clone(self);
        let run = tokio::task::spawn_blocking(move || {
            let _turn = turn;
            acyclic_cli::call(command, server.config.tracker.clone(), true, &server.config.env)
        });
        run.await.unwrap_or_else(|_| {
            let err =
                CliError::new("internal", exit::UNREADABLE, "the command failed unexpectedly");
            (err.exit, err.envelope())
        })
    }
}

async fn guard(State(server): State<Arc<Server>>, request: Request, next: Next) -> Response {
    match identity(request.headers(), &server.config) {
        Ok(()) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    }
}

async fn command(State(server): State<Arc<Server>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(refusal) = same_origin(&headers, &server.config) {
        return refusal.into_response();
    }
    let command: Command = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(err) => {
            let err = CliError::new("usage", exit::USAGE, format!("not a command: {err}"));
            return (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response();
        },
    };
    let (code, envelope) = match classify(&command) {
        Use::Refused => return Refusal::NotAllowed.into_response(),
        Use::Read => server.call(command, None).await,
        Use::Write => match server.turn().await {
            Ok(turn) => server.call(command, Some(turn)).await,
            Err(err) => (err.exit, err.envelope()),
        },
    };
    (status(code), Json(envelope)).into_response()
}

/// The server's routes, every one behind [`identity`].
pub fn router(config: Config) -> Router {
    routes(Arc::new(Server::new(config)))
}

fn routes(server: Arc<Server>) -> Router {
    let mut router = Router::new().route("/api/command", post(command));
    if let Some(dir) = &server.config.static_dir {
        router = router.fallback_service(ServeDir::new(dir));
    }
    router.layer(middleware::from_fn_with_state(server.clone(), guard)).with_state(server)
}

/// Serves the tracker clone at `tracker` until interrupted: `acyclic serve`. Prints one JSON line with the address
/// it listens on once it does, and syncs with the clone's `origin` as the settings say.
///
/// # Errors
///
/// When the settings are wrong or the address can't be bound.
pub fn serve(options: &Serve, tracker: PathBuf, env: &Env) -> Result<(), CliError> {
    let config = Config::new(options, tracker, env)?;
    if !options.listen.ip().is_loopback() {
        eprintln!(
            "acyclic: {} isn't a loopback address: whatever reaches it without passing through Tailscale can forge \
             the identity header",
            options.listen
        );
    }
    let failed = |what: &str, err: std::io::Error| {
        CliError::new("serve", exit::CONFIG, format!("{what}: {err}"))
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| failed("starting the runtime", err))?;
    let server = Arc::new(Server::new(config));
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(options.listen)
            .await
            .map_err(|err| failed(&format!("listening on {}", options.listen), err))?;
        let address = listener.local_addr().map_err(|err| failed("reading the address", err))?;
        if options.sync_every > 0 {
            tokio::spawn(sync(server.clone(), Duration::from_secs(options.sync_every)));
        }
        let mut stdout = std::io::stdout();
        writeln!(stdout, "{}", json!({"ok": true, "data": {"listening": address.to_string()}}))
            .and_then(|()| stdout.flush())
            .map_err(|err| failed("printing the address", err))?;
        axum::serve(listener, routes(server))
            .with_graceful_shutdown(stopped())
            .await
            .map_err(|err| failed("serving", err))
    })
}

/// Syncs the clone with its `origin` every `every`, fetching every writer's usage records too, each sync after the last
/// has finished, until the server stops. A failed sync prints its envelope to standard error and the next one tries
/// again; a clone without `origin` stops the loop after saying so.
async fn sync(server: Arc<Server>, every: Duration) {
    let mut ticks = tokio::time::interval(every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        // A sync waits as long as it must: it is the server's own, and the next tick would only queue behind it.
        let turn = server.turns.clone().lock_owned().await;
        let (code, envelope) = server.call(Command::Sync { with_usage: true }, Some(turn)).await;
        if code == exit::OK {
            continue;
        }
        if envelope["error"]["code"] == "no-remote" {
            eprintln!("acyclic: the tracker clone has no origin, so the server doesn't sync");
            return;
        }
        eprintln!("{envelope}");
    }
}

/// Waits for an interrupt or, on Unix, a termination signal.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate =
            signal(SignalKind::terminate()).expect("the runtime has signal handling");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests;
