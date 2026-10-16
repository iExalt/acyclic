#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use acyclic_cli::Env;
use acyclic_cli::args::{Command, Serve, ThreadCommand};
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

/// Whether the server runs a command: reads, for now.
fn allowed(command: &Command) -> bool {
    matches!(
        command,
        Command::List
            | Command::Show { .. }
            | Command::Ready { .. }
            | Command::Needs
            | Command::Thread(ThreadCommand::List | ThreadCommand::Show { .. })
    )
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

async fn guard(State(config): State<Arc<Config>>, request: Request, next: Next) -> Response {
    match identity(request.headers(), &config) {
        Ok(()) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    }
}

async fn command(State(config): State<Arc<Config>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(refusal) = same_origin(&headers, &config) {
        return refusal.into_response();
    }
    let command: Command = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(err) => {
            let err = CliError::new("usage", exit::USAGE, format!("not a command: {err}"));
            return (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response();
        },
    };
    if !allowed(&command) {
        return Refusal::NotAllowed.into_response();
    }
    let run = tokio::task::spawn_blocking(move || {
        acyclic_cli::call(command, config.tracker.clone(), false, &config.env)
    });
    let Ok((code, envelope)) = run.await else {
        let err = CliError::new("internal", exit::UNREADABLE, "the command failed unexpectedly");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(err.envelope())).into_response();
    };
    (status(code), Json::<Value>(envelope)).into_response()
}

/// The server's routes, every one behind [`identity`].
pub fn router(config: Config) -> Router {
    let config = Arc::new(config);
    let mut router = Router::new().route("/api/command", post(command));
    if let Some(dir) = &config.static_dir {
        router = router.fallback_service(ServeDir::new(dir));
    }
    router.layer(middleware::from_fn_with_state(config.clone(), guard)).with_state(config)
}

/// Serves the tracker clone at `tracker` until interrupted: `acyclic serve`. Prints one JSON line with the address
/// it listens on once it does.
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
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(options.listen)
            .await
            .map_err(|err| failed(&format!("listening on {}", options.listen), err))?;
        let address = listener.local_addr().map_err(|err| failed("reading the address", err))?;
        let mut stdout = std::io::stdout();
        writeln!(stdout, "{}", json!({"ok": true, "data": {"listening": address.to_string()}}))
            .and_then(|()| stdout.flush())
            .map_err(|err| failed("printing the address", err))?;
        axum::serve(listener, router(config))
            .with_graceful_shutdown(stopped())
            .await
            .map_err(|err| failed("serving", err))
    })
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
