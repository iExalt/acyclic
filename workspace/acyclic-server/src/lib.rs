#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acyclic_cli::args::{
    Command, ItemCommand, LinkCommand, ProposalCommand, RepoCommand, Serve, ThreadCommand, Up,
};
use acyclic_cli::error::{CliError, exit};
use acyclic_cli::{Applied, Env, Operation, Start};
use acyclic_core::state::{Context, Millis};
use acyclic_guard::DenyList;
use acyclic_operator::token::{Claims, Verifier};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{
    AUTHORIZATION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE as COOKIE_HEADER, HOST, LOCATION,
    ORIGIN, SET_COOKIE, WWW_AUTHENTICATE, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tower::ServiceExt as _;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::SizeAbove;

pub mod app;
pub mod code;
pub mod push;
pub mod session;

use session::Token;

/// The header Tailscale sets to the login of the user behind a request.
pub const IDENTITY: &str = "Tailscale-User-Login";

/// What the server serves, whichever listener a request arrives on.
#[derive(Debug, Clone)]
pub struct Config {
    /// The tracker clone every command runs on.
    pub tracker: PathBuf,
    /// The environment every command runs in.
    pub env: Env,
    /// The directory of the app's files, if any, in place of the copy built into the binary.
    pub static_dir: Option<PathBuf>,
    /// How the server pushes to the user's devices.
    pub push: push::Settings,
}

/// Who a listener lets in, and how it tells.
#[derive(Debug, Clone)]
pub enum Access {
    /// Requests through Tailscale, which sets [`IDENTITY`] from its own lookup. Whatever reaches the listener without
    /// passing through Tailscale can forge the header, so only Tailscale may be able to reach it.
    Tailscale {
        /// The user's login, which [`IDENTITY`] must equal.
        login: String,
        /// The name `Host` must carry, compared without regard to case, with its port if it has one.
        host: String,
        /// The origin a browser's request must come from, such as `https://<host>`.
        origin: String,
    },
    /// Requests on 127.0.0.1 presenting the session `acyclic up` issued. Every process on the machine, and every
    /// container, can reach a loopback port, so the address proves nothing and the session is what counts.
    Session {
        /// The port the listener is bound to, which `Host` must carry.
        port: u16,
        /// The session.
        token: Token,
    },
    /// Requests from runs, each presenting its run token as `Authorization: Bearer <token>`. `Host` isn't checked:
    /// a run reaches the server by whatever name its container or pod has for it, and the check guards a browser's
    /// ambient cookie against a page whose name rebinds to the server, which a token in a header can't be.
    Run {
        /// What checks run tokens: the operator's public key.
        verifier: Verifier,
        /// The terms no run may write, which every write is scanned for, with Gitleaks besides.
        deny: DenyList,
        /// The code repositories runs reach through the server, if it serves them.
        code: Option<Arc<code::Code>>,
    },
}

impl Access {
    /// Access through Tailscale for `login` at `host`, from `origin`, by default `https://<host>`.
    ///
    /// # Errors
    ///
    /// A usage error when the login or the host is empty.
    pub fn tailscale(login: &str, host: &str, origin: Option<&str>) -> Result<Self, CliError> {
        if login.is_empty() || host.is_empty() {
            return Err(CliError::new("usage", exit::USAGE, "--login and --host may not be empty"));
        }
        Ok(Self::Tailscale {
            login: login.to_owned(),
            host: host.to_owned(),
            origin: origin.map_or_else(|| format!("https://{host}"), str::to_owned),
        })
    }

    /// The names `Host` may carry on a session listener: the loopback address and `localhost`, with the listener's
    /// port, so that a page from another name that resolves to 127.0.0.1 can't reach it.
    fn local_hosts(port: u16) -> [String; 3] {
        [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
    }

    /// Checks that a request is for this server: on a session listener, that it names the listener's own address.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn host(&self, headers: &HeaderMap) -> Result<(), Refusal> {
        let sent = single(headers, HOST);
        let ok = match self {
            Self::Tailscale { host, .. } => {
                sent.is_some_and(|sent| sent.eq_ignore_ascii_case(host.as_bytes()))
            },
            Self::Session { port, .. } => sent.is_some_and(|sent| {
                Self::local_hosts(*port)
                    .iter()
                    .any(|name| sent.eq_ignore_ascii_case(name.as_bytes()))
            }),
            Self::Run { .. } => true,
        };
        if ok { Ok(()) } else { Err(Refusal::WrongHost) }
    }

    /// Checks that a request comes from the user, or on a runners' listener from a run, by the listener's own means,
    /// for this server, at `now`, and says how.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn check(&self, headers: &HeaderMap, now: Millis) -> Result<Credential, Refusal> {
        match self {
            Self::Tailscale { login, .. } => {
                match single(headers, IDENTITY) {
                    None => return Err(Refusal::NoIdentity),
                    Some(sent) if sent != login.as_bytes() => return Err(Refusal::WrongIdentity),
                    Some(_) => {},
                }
                self.host(headers)?;
                Ok(Credential::Identity)
            },
            Self::Session { port, token } => {
                self.host(headers)?;
                if headers.contains_key(AUTHORIZATION) {
                    return match bearer(headers) {
                        Some(presented) if token.matches(presented) => Ok(Credential::Bearer),
                        _ => Err(Refusal::NoSession),
                    };
                }
                let browser = token.browser();
                if cookies(headers, *port).any(|value| session::equal(value, browser.as_bytes())) {
                    return Ok(Credential::Cookie);
                }
                Err(Refusal::NoSession)
            },
            Self::Run { verifier, .. } => {
                let token = bearer(headers).and_then(|token| std::str::from_utf8(token).ok());
                let claims = token.and_then(|token| verifier.verify(token, now).ok());
                claims.map(Credential::Run).ok_or(Refusal::NoToken)
            },
        }
    }

    /// Checks that a command's request is JSON and, when a browser sent it, that it comes from the server's own
    /// origin, so that no other site can make the user's browser send one. A request the cookie let in must say
    /// where it comes from: the cookie goes to every port on the machine, so any local page could otherwise send one.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn same_origin(&self, headers: &HeaderMap, credential: &Credential) -> Result<(), Refusal> {
        let json = single(headers, CONTENT_TYPE).is_some_and(|value| {
            let media = value.split(|byte| *byte == b';').next().unwrap_or_default();
            media.trim_ascii().eq_ignore_ascii_case(b"application/json")
        });
        if !json {
            return Err(Refusal::NotJson);
        }
        if *credential == Credential::Cookie {
            let site = single(headers, "sec-fetch-site");
            if headers.contains_key("sec-fetch-site") && site != Some(b"same-origin".as_slice()) {
                return Err(Refusal::WrongOrigin);
            }
            let own = single(headers, HOST).map(|host| [b"http://".as_slice(), host].concat());
            return match (single(headers, ORIGIN), own) {
                (Some(origin), Some(own)) if origin.eq_ignore_ascii_case(&own) => Ok(()),
                _ => Err(Refusal::WrongOrigin),
            };
        }
        if headers.get(ORIGIN).is_none() {
            return Ok(());
        }
        let allowed = |origin: &[u8]| match self {
            Self::Tailscale { origin: own, .. } => origin.eq_ignore_ascii_case(own.as_bytes()),
            Self::Session { port, .. } => Self::local_hosts(*port)
                .iter()
                .any(|name| origin.eq_ignore_ascii_case(format!("http://{name}").as_bytes())),
            // No browser speaks for a run.
            Self::Run { .. } => false,
        };
        match single(headers, ORIGIN) {
            Some(origin) if allowed(origin) => Ok(()),
            _ => Err(Refusal::WrongOrigin),
        }
    }
}

/// How a request showed it comes from the user, or from a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// Tailscale's identity header.
    Identity,
    /// The session, in `Authorization`.
    Bearer,
    /// A paired browser's cookie.
    Cookie,
    /// A run's token, in `Authorization`, and what it says.
    Run(Claims),
}

/// The cookie a paired browser holds, named for the listener's port: browsers scope cookies by host and not by port, so
/// servers on other ports of the same host would otherwise replace each other's.
fn cookie(port: u16) -> String {
    format!("acyclic-up-{port}")
}

/// The name every server's cookie had before it carried the port. It is still read, so a browser paired before then
/// stays paired; another server's cookie of this name can only fail to match, so it does no harm.
const SHARED_COOKIE: &str = "acyclic-up";

/// The values of every cookie in the request named for `port` or [`SHARED_COOKIE`]; each is tried.
fn cookies(headers: &HeaderMap, port: u16) -> impl Iterator<Item = &[u8]> {
    let own = cookie(port);
    headers.get_all(COOKIE_HEADER).into_iter().flat_map(move |value| {
        let own = own.clone();
        value.as_bytes().split(|byte| *byte == b';').filter_map(move |pair| {
            let (name, value) = pair
                .trim_ascii()
                .split_at(pair.trim_ascii().iter().position(|byte| *byte == b'=')?);
            (name == own.as_bytes() || name == SHARED_COOKIE.as_bytes()).then_some(&value[1..])
        })
    })
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No identity header, or more than one.
    NoIdentity,
    /// No session, another one, or more than one.
    NoSession,
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
    /// No valid run token.
    NoToken,
    /// The token's run isn't under way.
    Inactive,
    /// A command a run may not send.
    NotForRuns,
    /// A run's write without its operation's number.
    NoOperation,
    /// A run's write holding a deny-listed term.
    DenyListed,
    /// A run's write holding a secret.
    Leak,
    /// A run's write that couldn't be scanned.
    ScanFailed,
}

impl Refusal {
    fn code(self) -> &'static str {
        match self {
            Self::NoIdentity => "no-identity",
            Self::NoSession => "no-session",
            Self::WrongIdentity => "wrong-identity",
            Self::WrongHost => "wrong-host",
            Self::WrongOrigin => "wrong-origin",
            Self::NotJson => "not-json",
            Self::NotAllowed => "not-allowed",
            Self::NoToken => "no-token",
            Self::Inactive => "inactive",
            Self::NotForRuns => "not-for-runs",
            Self::NoOperation => "no-operation",
            Self::DenyListed => "deny-listed",
            Self::Leak => "leak",
            Self::ScanFailed => "scan-failed",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NoIdentity => "the request carries no single Tailscale identity",
            Self::NoSession => "the request carries no session: present the one acyclic up issued",
            Self::WrongIdentity => "the request's Tailscale identity isn't the user's",
            Self::WrongHost => "the request is for another host",
            Self::WrongOrigin => "the request comes from another origin",
            Self::NotJson => "the request's body must be application/json",
            Self::NotAllowed => "the server doesn't run this command",
            Self::NoToken => "the request carries no valid run token",
            Self::Inactive => "the token's run isn't under way",
            Self::NotForRuns => "a run may not send this command",
            Self::NoOperation => "a run's write carries its number in Acyclic-Operation",
            Self::DenyListed => "the write holds a term the deny-list forbids",
            Self::Leak => "the write holds what Gitleaks takes for a secret",
            Self::ScanFailed => "the write couldn't be scanned; try again shortly",
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let envelope = json!({"ok": false, "error": {"code": self.code(), "message": self.message(), "details": null}});
        match self {
            Self::NotJson => (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(envelope)).into_response(),
            Self::NoOperation => (StatusCode::BAD_REQUEST, Json(envelope)).into_response(),
            Self::ScanFailed => (StatusCode::SERVICE_UNAVAILABLE, Json(envelope)).into_response(),
            Self::NoSession | Self::NoToken => {
                (StatusCode::UNAUTHORIZED, [(WWW_AUTHENTICATE, "Bearer")], Json(envelope))
                    .into_response()
            },
            _ => (StatusCode::FORBIDDEN, Json(envelope)).into_response(),
        }
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

/// The token of the request's one `Authorization` header, if it says `Bearer` and one space then the token.
fn bearer(headers: &HeaderMap) -> Option<&[u8]> {
    let value = single(headers, AUTHORIZATION)?;
    let space = value.iter().position(|byte| *byte == b' ')?;
    let (scheme, token) = value.split_at(space);
    scheme.eq_ignore_ascii_case(b"Bearer").then_some(&token[1..])
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
        | Command::Serve(_)
        | Command::Up(_)
        | Command::Completions { .. } => Use::Refused,
        Command::Ready { .. }
        | Command::List
        | Command::Show { .. }
        | Command::Needs
        | Command::Level { level: None }
        | Command::Thread(ThreadCommand::List | ThreadCommand::Show { .. })
        | Command::Repo(RepoCommand::List | RepoCommand::Show { .. })
        | Command::Proposal(ProposalCommand::List | ProposalCommand::Show { .. }) => Use::Read,
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
            | ThreadCommand::Revoke { .. }
            | ThreadCommand::Renumber { .. },
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

/// How long a write waits for its turn before the server answers that it is busy. It covers only this server's own
/// writes and syncs: once a command has its turn, it may still wait, without limit, for the clone's lock while the
/// CLI or another process writes.
const TURN: Duration = Duration::from_secs(15);

/// A running server's state.
struct Server {
    config: Config,
    /// Held by each write and each sync until it finishes, so that their pushes never race each other.
    turns: Arc<tokio::sync::Mutex<()>>,
    /// How long a write waits for its turn.
    turn: Duration,
    /// Pushes to the user's devices.
    push: push::Push,
}

impl Server {
    fn new(config: Config) -> Self {
        let push = push::Push::new(config.tracker.clone(), config.env.clone(), config.push.clone());
        Self { config, turns: Arc::default(), turn: TURN, push }
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

/// A listener's state: the server, who the listener lets in, and the pairing codes spent on it.
struct Listener {
    server: Arc<Server>,
    access: Access,
    spent: session::Spent,
    /// On a runners' listener, what the server last served each run to read: the run's writes supersede only what
    /// that held. It is the whole tracker as of the run's last read, not the parts the run looked at, so a write
    /// supersedes whatever was there then. Kept in memory: a restart forgets them, and a run's writes then keep
    /// conflicts until it reads again, which loses nothing.
    views: Mutex<BTreeMap<String, Context>>,
}

/// The path a pairing link opens.
const LOGIN: &str = "/login";

/// How long a paired browser keeps its cookie: 400 days, the most browsers allow.
const COOKIE_AGE: u32 = 400 * 24 * 60 * 60;

async fn guard(
    State(listener): State<Arc<Listener>>,
    mut request: Request,
    next: Next,
) -> Response {
    let access = &listener.access;
    if let Access::Session { port, token } = access
        && request.uri().path() == LOGIN
    {
        return match access.host(request.headers()) {
            Ok(()) => pair(&listener, *port, token, request.uri().query()),
            Err(refusal) => refusal.into_response(),
        };
    }
    match access.check(request.headers(), acyclic_cli::now(&listener.server.config.env)) {
        Ok(credential) => {
            request.extensions_mut().insert(credential);
            next.run(request).await
        },
        Err(Refusal::NoSession)
            if request.method() == Method::GET && !request.uri().path().starts_with("/api/") =>
        {
            page(StatusCode::UNAUTHORIZED, UNPAIRED)
        },
        Err(refusal) => refusal.into_response(),
    }
}

const UNPAIRED: &str = "This browser isn't paired with acyclic up. Run acyclic up --link, with the same --port as the \
                        server if it has one, and open the link it prints within five minutes.";

const SPENT: &str =
    "This pairing link has expired or was already used. Run acyclic up --link for a new one.";

/// A short page for people.
fn page(status: StatusCode, text: &'static str) -> Response {
    (status, [(CONTENT_TYPE, "text/plain; charset=utf-8")], format!("{text}\n")).into_response()
}

/// Pairs a browser that opened a link with a valid, unspent code, by giving it the cookie, and sends it home.
fn pair(listener: &Listener, port: u16, token: &Token, query: Option<&str>) -> Response {
    let now = acyclic_cli::now(&listener.server.config.env);
    let code = query.and_then(|query| query.strip_prefix("code="));
    let made = code.and_then(|code| Some((code, token.verify(code, now)?)));
    match made {
        Some((code, made)) if listener.spent.spend(code, made, now) => {
            let set = format!(
                "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={COOKIE_AGE}",
                cookie(port),
                token.browser()
            );
            (StatusCode::SEE_OTHER, [(SET_COOKIE, set), (LOCATION, "/".to_owned())]).into_response()
        },
        _ => page(StatusCode::FORBIDDEN, SPENT),
    }
}

/// A pairing link for the session listener on `port`.
fn link(port: u16, token: &Token, env: &Env) -> Result<String, CliError> {
    let code = token.code(acyclic_cli::now(env), session::nonce()?);
    Ok(format!("http://127.0.0.1:{port}{LOGIN}?code={code}"))
}

async fn command(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = listener.access.same_origin(&headers, &credential) {
        return refusal.into_response();
    }
    if let Credential::Run(claims) = credential {
        return run_command(&listener, claims, &headers, body).await;
    }
    let command: Command = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(err) => {
            let err = CliError::new("usage", exit::USAGE, format!("not a command: {err}"));
            return (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response();
        },
    };
    let server = &listener.server;
    let used = classify(&command);
    let (code, envelope) = match used {
        Use::Refused => return Refusal::NotAllowed.into_response(),
        Use::Read => server.call(command, None).await,
        Use::Write => match server.turn().await {
            Ok(turn) => server.call(command, Some(turn)).await,
            Err(err) => (err.exit, err.envelope()),
        },
    };
    // What the user's own write brings about is theirs, and needs no push.
    if code == exit::OK && used == Use::Write {
        server.push.seen();
    }
    (status(code), Json(envelope)).into_response()
}

/// Runs a command from a run: a read as the run, recording what it served, or a write the run may send, scanned and
/// applied as the run once, under the run's view. Anything else is refused.
async fn run_command(
    listener: &Arc<Listener>,
    claims: Claims,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let command: Command = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(err) => {
            let err = CliError::new("usage", exit::USAGE, format!("not a command: {err}"));
            return (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response();
        },
    };
    let write = command.run_may_send();
    if !write && classify(&command) != Use::Read {
        return Refusal::NotForRuns.into_response();
    }
    let seq = single(headers, "acyclic-operation")
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seq| *seq > 0);
    let seq = match (write, seq) {
        (true, None) => return Refusal::NoOperation.into_response(),
        (true, Some(seq)) => Some(seq),
        (false, _) => None,
    };
    let server = Arc::clone(&listener.server);
    let lister = Arc::clone(listener);
    let checked = tokio::task::spawn_blocking(move || {
        let tracker = &lister.server.config.tracker;
        if !acyclic_cli::run_active(&claims.run, &claims.thread, tracker).unwrap_or(false) {
            if let Ok(mut views) = lister.views.lock() {
                views.remove(&claims.run);
            }
            return Err(Refusal::Inactive);
        }
        if let (Some(_), Access::Run { deny, .. }) = (seq, &lister.access) {
            let text =
                serde_json::from_slice(&body).map(|value| stored(&value)).unwrap_or_default();
            match acyclic_guard::scan_text(&text, "the write", deny) {
                Ok(report) if !report.findings.is_empty() => return Err(Refusal::DenyListed),
                Ok(report) if report.leaks => return Err(Refusal::Leak),
                Ok(_) => {},
                Err(_) => return Err(Refusal::ScanFailed),
            }
        }
        Ok(claims)
    })
    .await;
    let claims = match checked {
        Ok(Ok(claims)) => claims,
        Ok(Err(refusal)) => return refusal.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(seq) = seq else { return run_read(listener, claims, command).await };
    let turn = match server.turn().await {
        Ok(turn) => turn,
        Err(err) => return (status(err.exit), Json(err.envelope())).into_response(),
    };
    let lister = Arc::clone(listener);
    let applied = tokio::task::spawn_blocking(move || {
        let _turn = turn;
        let seen = lister.views.lock().ok().and_then(|views| views.get(&claims.run).cloned());
        let operation = Operation {
            run: claims.run,
            thread: claims.thread,
            seq,
            seen: seen.unwrap_or_default(),
        };
        let config = &lister.server.config;
        acyclic_cli::apply(operation, command, config.tracker.clone(), &config.env)
    })
    .await;
    match applied {
        Ok(Ok(Applied::Ran(code, envelope))) => {
            if code == exit::OK {
                server.push.look();
            }
            (status(code), Json(envelope)).into_response()
        },
        Ok(Err((code, envelope))) => (status(code), Json(envelope)).into_response(),
        Ok(Ok(Applied::Duplicate(outcome))) => {
            let data = json!({"duplicate": true, "seq": seq, "outcome": outcome});
            (StatusCode::OK, Json(json!({"ok": true, "data": data}))).into_response()
        },
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The text a write would store: every string in the request and every key, decoded and one to a line. Scanning the
/// request as sent would miss a term written with JSON's escapes, which decoding turns back into the term.
fn stored(value: &Value) -> String {
    fn collect(value: &Value, text: &mut String) {
        match value {
            Value::String(string) => {
                text.push_str(string);
                text.push('\n');
            },
            Value::Array(values) => values.iter().for_each(|value| collect(value, text)),
            Value::Object(map) => {
                for (key, value) in map {
                    text.push_str(key);
                    text.push('\n');
                    collect(value, text);
                }
            },
            Value::Null | Value::Bool(_) | Value::Number(_) => {},
        }
    }
    let mut text = String::new();
    collect(value, &mut text);
    text
}

/// Runs a run's read as the run, and records the tracker as it stood before the read as what the run has seen: never
/// more than the read showed.
async fn run_read(listener: &Arc<Listener>, claims: Claims, command: Command) -> Response {
    let lister = Arc::clone(listener);
    let read = tokio::task::spawn_blocking(move || {
        let config = &lister.server.config;
        let opened = acyclic_core::store::Tracker::open(&config.tracker);
        let context = opened
            .ok()
            .and_then(|(tracker, _)| tracker.load().ok())
            .map(|(_, state)| state.context);
        let actor = format!("run/{}", claims.run);
        let (code, envelope) =
            acyclic_cli::call_as(command, config.tracker.clone(), &actor, &config.env);
        if code == exit::OK
            && let (Some(context), Ok(mut views)) = (context, lister.views.lock())
        {
            views.insert(claims.run, context);
        }
        (code, envelope)
    })
    .await;
    match read {
        Ok((code, envelope)) => (status(code), Json(envelope)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// What a run's client needs to carry on: its run, its thread, and the latest of its operations applied.
async fn run_status(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
) -> Response {
    let Credential::Run(claims) = credential else { return Refusal::NotAllowed.into_response() };
    let lister = Arc::clone(&listener);
    let found = tokio::task::spawn_blocking(move || {
        let tracker = &lister.server.config.tracker;
        if !acyclic_cli::run_active(&claims.run, &claims.thread, tracker).unwrap_or(false) {
            return Refusal::Inactive.into_response();
        }
        match acyclic_cli::applied(&claims.run, tracker) {
            Ok(applied) => {
                let data = json!({"run": claims.run, "thread": claims.thread, "applied": applied});
                Json(json!({"ok": true, "data": data})).into_response()
            },
            Err((code, envelope)) => (status(code), Json(envelope)).into_response(),
        }
    })
    .await;
    match found {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The server's VAPID public key, which a device subscribes with.
async fn push_key(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
) -> Response {
    if let Credential::Run(_) = credential {
        return Refusal::NotForRuns.into_response();
    }
    pushed(listener.server.push.public_key().map(|key| json!({"key": key})))
}

/// Takes a device's subscription: the browser's `PushSubscription`, as JSON.
async fn push_subscribe(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = push_allowed(&listener, &credential, &headers) {
        return refusal.into_response();
    }
    match serde_json::from_slice::<push::Subscription>(&body) {
        Ok(subscription) => {
            pushed(listener.server.push.subscribe(subscription).map(|()| Value::Null))
        },
        Err(err) => pushed(Err(push::Refused(format!("not a subscription: {err}")))),
    }
}

/// Forgets a device: `{"endpoint": …}`.
async fn push_unsubscribe(
    State(listener): State<Arc<Listener>>,
    Extension(credential): Extension<Credential>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = push_allowed(&listener, &credential, &headers) {
        return refusal.into_response();
    }
    let endpoint = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value["endpoint"].as_str().map(str::to_owned));
    match endpoint {
        Some(endpoint) => pushed(listener.server.push.unsubscribe(&endpoint).map(|()| Value::Null)),
        None => pushed(Err(push::Refused("give the endpoint to forget".into()))),
    }
}

/// Lets the user's browser change the devices pushed to, from the server's own origin; never a run.
fn push_allowed(
    listener: &Listener,
    credential: &Credential,
    headers: &HeaderMap,
) -> Result<(), Refusal> {
    if let Credential::Run(_) = credential {
        return Err(Refusal::NotForRuns);
    }
    listener.access.same_origin(headers, credential)
}

/// The envelope for a push request's outcome.
fn pushed(outcome: Result<Value, push::Refused>) -> Response {
    match outcome {
        Ok(data) => Json(json!({"ok": true, "data": data})).into_response(),
        Err(push::Refused(message)) => {
            let err = CliError::new("push", exit::USAGE, message);
            (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response()
        },
    }
}

/// Gives every response, refusals included, the app's content policy ([`app::POLICY`]), which among other things
/// forbids every page to frame it: a page from another local server is the same site as the app's, so a paired
/// browser would send its cookie into a frame, and the frame could be clicked into writes.
async fn unframed(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(app::POLICY));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

/// The server's routes for one listener, every one behind its [`Access`].
pub fn router(config: Config, access: Access) -> Router {
    routes(Arc::new(Server::new(config)), access)
}

fn routes(server: Arc<Server>, access: Access) -> Router {
    // The API's answers are compressed when they're worth it: a whole tracker's `show` is megabytes of JSON, and a
    // phone may reach the server through a relay. None of them holds a secret an attacker could guess at through
    // their compressed size: the session travels only in the request. The app's files are compressed already.
    let mut router = Router::new()
        .route("/api/command", post(command))
        .route("/api/run", get(run_status))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(push_subscribe))
        .route("/api/push/unsubscribe", post(push_unsubscribe))
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(1024)));
    // Runs have no use for the app, and get none.
    if !matches!(access, Access::Run { .. }) {
        router = router.fallback(files);
    }
    if let Access::Run { code: Some(_), .. } = &access {
        router = router.merge(code::routes());
    }
    let listener = Arc::new(Listener {
        server,
        access,
        spent: session::Spent::default(),
        views: Mutex::default(),
    });
    router
        .layer(middleware::from_fn_with_state(listener.clone(), guard))
        .layer(middleware::map_response(unframed))
        .with_state(listener)
}

/// The app's files: from `--static`'s directory if the server has one, else the copy built into the binary.
async fn files(State(listener): State<Arc<Listener>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = match &listener.server.config.static_dir {
        Some(dir) => match app::dir(dir).oneshot(request).await {
            Ok(response) => response.into_response(),
            Err(never) => match never {},
        },
        None => app::embedded(&request),
    };
    // A person who followed a stale or mistyped link is told where the app is, not shown a blank page.
    if response.status() == StatusCode::NOT_FOUND {
        return page(StatusCode::NOT_FOUND, NOWHERE);
    }
    app::cache(&path, &mut response);
    response
}

const NOWHERE: &str = "There's nothing at this address. The app is at /.";

fn failed(what: &str, err: impl std::fmt::Display) -> CliError {
    CliError::new("serve", exit::CONFIG, format!("{what}: {err}"))
}

/// Starts a server: `acyclic serve` or `acyclic up`, on the tracker clone at `tracker`, until interrupted.
///
/// # Errors
///
/// When the settings are wrong or the address can't be bound.
pub fn start(start: Start<'_>, tracker: PathBuf, env: &Env) -> Result<(), CliError> {
    match start {
        Start::Serve(options) => serve(options, tracker, env),
        Start::Up(options) => up(options, tracker, env),
    }
}

/// Serves the tracker clone at `tracker` behind Tailscale until interrupted: `acyclic serve`. Prints one JSON line
/// with the address it listens on once it does, and syncs with the clone's `origin` as the settings say.
///
/// # Errors
///
/// When the settings are wrong or the address can't be bound.
pub fn serve(options: &Serve, tracker: PathBuf, env: &Env) -> Result<(), CliError> {
    let access = Access::tailscale(&options.login, &options.host, options.origin.as_deref())?;
    if !options.listen.ip().is_loopback() {
        eprintln!(
            "acyclic: {} isn't a loopback address: whatever reaches it without passing through Tailscale can forge \
             the identity header",
            options.listen
        );
    }
    if let Some(dir) = &options.static_dir {
        app::check_dir(dir)?;
    }
    let Access::Tailscale { origin, .. } = &access else {
        unreachable!("access through Tailscale")
    };
    let push = push::Settings { subject: Some(origin.clone()), ..push::Settings::default() };
    let config = Config { tracker, env: env.clone(), static_dir: options.static_dir.clone(), push };
    run(config, options.sync_every, options.listen, Vec::new(), |address, _| {
        Ok((access, json!({"listening": address.to_string()})))
    })
}

/// Serves the tracker clone at `tracker` on 127.0.0.1 to whoever holds the session, until interrupted: `acyclic up`.
/// Prints one JSON line once it listens, with the address, the session's file and a pairing link, never the session.
/// With `--link`, prints only a fresh pairing link instead, and serves nothing.
///
/// # Errors
///
/// When the home directory or the session's file is wrong, or the port can't be bound.
pub fn up(options: &Up, tracker: PathBuf, env: &Env) -> Result<(), CliError> {
    let home = env.vars.get("HOME").filter(|home| !home.is_empty()).ok_or_else(|| {
        CliError::new("session", exit::CONFIG, "HOME isn't set, so the session has nowhere to live")
    })?;
    if options.link {
        let (token, _) = session::existing(home.as_ref())?;
        let link = link(options.port, &token, env)?;
        println!("{}", json!({"ok": true, "data": {"link": link}}));
        return Ok(());
    }
    if let Some(dir) = &options.static_dir {
        app::check_dir(dir)?;
    }
    let (token, path) = session::open(home.as_ref(), options.rotate)?;
    let tailscale = match (options.tailscale, &options.login, &options.host) {
        (true, Some(login), Some(host)) => {
            Some(Access::tailscale(login, host, options.origin.as_deref())?)
        },
        _ => None,
    };
    let secret = token.tailscale();
    let mut extra = Vec::new();
    if let Some(access) = tailscale.clone() {
        extra.push(match &options.socket {
            Some(socket) => Extra::Unix(socket.clone(), access),
            None => Extra::Tcp(
                SocketAddr::from(([127, 0, 0, 1], options.tailscale_port)),
                access,
                Some(secret.clone()),
            ),
        });
    }
    if let Some(port) = options.runners {
        extra.push(Extra::Tcp(
            SocketAddr::from(([127, 0, 0, 1], port)),
            runners(home.as_ref(), env)?,
            None,
        ));
    }
    let subject = match &tailscale {
        Some(Access::Tailscale { origin, .. }) => Some(origin.clone()),
        _ => None,
    };
    let push = push::Settings { subject, ..push::Settings::default() };
    let config = Config { tracker, env: env.clone(), static_dir: options.static_dir.clone(), push };
    let bind = SocketAddr::from(([127, 0, 0, 1], options.port));
    let link = |port| link(port, &token, env);
    let socket = options.socket.clone();
    let runners = options.runners.is_some();
    run(config, options.sync_every, bind, extra, |address, bound| {
        let mut started = json!({"listening": address.to_string(), "session": path, "link": link(address.port())?});
        let mut bound = bound.into_iter();
        if tailscale.is_some() {
            started["tailscale"] = match (&socket, bound.next().flatten()) {
                (Some(socket), _) => {
                    json!({"socket": socket, "serve": format!("tailscale serve --bg unix:{}", socket.display())})
                },
                (None, Some(at)) => json!({
                    "listening": at.to_string(),
                    "serve": format!("tailscale serve --bg http://{at}/{secret}"),
                }),
                (None, None) => unreachable!("a TCP listener has an address"),
            };
        }
        if let (true, Some(at)) = (runners, bound.next().flatten()) {
            started["runners"] = json!(at.to_string());
        }
        Ok((Access::Session { port: address.port(), token: token.clone() }, started))
    })
}

/// The runners' listener's access: the operator's public key and the deny-list, both from the home directory `home`,
/// and the mirrors of code repositories. The key is read once, so a new operator key needs the server restarted.
fn runners(home: &std::path::Path, env: &Env) -> Result<Access, CliError> {
    let verifier = Verifier::open(home).map_err(|err| {
        CliError::new(
            "no-operator",
            exit::CONFIG,
            format!("{err}: start the operator first, which writes its public key"),
        )
    })?;
    let deny = DenyList::load(&acyclic_guard::deny_list::path_under(home)).map_err(|err| {
        CliError::new(
            "no-deny-list",
            exit::CONFIG,
            format!("the runners' listener needs the deny-list: {err}"),
        )
    })?;
    let executable = std::env::current_exe()
        .map_err(|err| failed("finding the acyclic executable for the mirrors' hook", err))?;
    let settings = code::Settings {
        env: env.clone(),
        executable,
        deny: acyclic_guard::deny_list::path_under(home),
    };
    let code = code::Code::new(settings).map_err(|err| {
        CliError::new("no-mirrors", exit::CONFIG, format!("the mirrors can't be kept: {err}"))
    })?;
    Ok(Access::Run { verifier, deny, code: Some(Arc::new(code)) })
}

/// A listener besides the main one.
enum Extra {
    /// A Unix socket.
    Unix(PathBuf, Access),
    /// A TCP address, serving only under the secret first path segment if one is given.
    Tcp(SocketAddr, Access, Option<String>),
}

/// Binds `bind`, and each extra listener with its own access, gives the bound address, and each extra listener's in
/// order (none for a socket), to `access` for the main listener's access and the line to print, prints it, and serves
/// until interrupted, syncing every `sync_every` seconds unless that is 0.
fn run(
    config: Config,
    sync_every: u64,
    bind: SocketAddr,
    extra: Vec<Extra>,
    access: impl FnOnce(SocketAddr, Vec<Option<SocketAddr>>) -> Result<(Access, Value), CliError>,
) -> Result<(), CliError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| failed("starting the runtime", err))?;
    let server = Arc::new(Server::new(config));
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .map_err(|err| failed(&format!("listening on {bind}"), err))?;
        let address = listener.local_addr().map_err(|err| failed("reading the address", err))?;
        let mut serving = tokio::task::JoinSet::new();
        let mut bound = Vec::new();
        for listener in extra {
            match listener {
                Extra::Unix(path, access) => {
                    let unix = bind_socket(&path)?;
                    let routes = routes(server.clone(), access);
                    bound.push(None);
                    serving.spawn(async move {
                        axum::serve(unix, routes).with_graceful_shutdown(stopped()).await
                    });
                },
                Extra::Tcp(at, access, secret) => {
                    if let (Access::Run { code: Some(code), .. }, true) = (&access, sync_every > 0)
                    {
                        tokio::spawn(refresh(Arc::clone(code), server.config.tracker.clone()));
                    }
                    let tcp = tokio::net::TcpListener::bind(at)
                        .await
                        .map_err(|err| failed(&format!("listening on {at}"), err))?;
                    bound.push(Some(
                        tcp.local_addr().map_err(|err| failed("reading the address", err))?,
                    ));
                    let routes = routes(server.clone(), access);
                    let routes = match secret {
                        Some(secret) => under(secret, routes),
                        None => routes,
                    };
                    serving.spawn(async move {
                        axum::serve(tcp, routes).with_graceful_shutdown(stopped()).await
                    });
                },
            }
        }
        let (access, started) = access(address, bound)?;
        if sync_every > 0 {
            tokio::spawn(sync(server.clone(), Duration::from_secs(sync_every)));
        }
        // Starts the pushing, which looks for news every so often from then on.
        server.push.look();
        let mut stdout = std::io::stdout();
        writeln!(stdout, "{}", json!({"ok": true, "data": started}))
            .and_then(|()| stdout.flush())
            .map_err(|err| failed("printing the address", err))?;
        let routes = routes(server.clone(), access);
        serving.spawn(async move {
            axum::serve(listener, routes).with_graceful_shutdown(stopped()).await
        });
        while let Some(done) = serving.join_next().await {
            done.map_err(|err| failed("serving", err))?.map_err(|err| failed("serving", err))?;
        }
        Ok(())
    })
}

/// Serves `routes` only under the first path segment `secret`, which is taken off before they see the request; every
/// other path is answered 404, empty. `tailscale serve`, pointed at `http://127.0.0.1:<port>/<secret>`, prepends it to
/// every path, so browsers never see it, and only processes that can read the user's session can learn it: the Tailscale
/// port is on loopback, which every process and container on the machine can reach, and Tailscale's identity header
/// alone would let any of them act as the user.
///
/// It wraps the whole router, since a router's own layers run after it has routed the path they would rewrite.
fn under(secret: String, routes: Router) -> Router {
    Router::new().fallback_service(tower::service_fn(move |request: Request| {
        let routes = routes.clone();
        let inside = beneath(&secret, request);
        async move {
            match inside {
                Some(request) => routes.oneshot(request).await,
                None => Ok(StatusCode::NOT_FOUND.into_response()),
            }
        }
    }))
}

/// `request` with the path's first segment taken off, if that segment is `secret`, compared as sent; `/<secret>` alone
/// is `/`. Only the first segment is taken, once.
fn beneath(secret: &str, mut request: Request) -> Option<Request> {
    let rest = request.uri().path().strip_prefix('/')?;
    let (first, after) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if !session::equal(first.as_bytes(), secret.as_bytes()) {
        return None;
    }
    let after = if after.is_empty() { "/" } else { after };
    let path = match request.uri().query() {
        Some(query) => format!("{after}?{query}"),
        None => after.to_owned(),
    };
    let mut parts = request.uri().clone().into_parts();
    parts.path_and_query = Some(path.parse().ok()?);
    *request.uri_mut() = axum::http::Uri::from_parts(parts).ok()?;
    Some(request)
}

/// The longest path a Unix socket can have on macOS, the shortest limit of the systems acyclic runs on.
const SOCKET_PATH_MAX: usize = 103;

/// Binds a Unix socket at `path`, replacing one an earlier server left but refusing one a server still listens on, and
/// makes it the user's alone. Until then it has the umask's mode, which lets no one else connect under the usual
/// umask, and the default path sits in a directory only the user can enter.
fn bind_socket(path: &std::path::Path) -> Result<tokio::net::UnixListener, CliError> {
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
    let named = || path.display().to_string();
    if path.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(failed(
            &format!("listening on {}", named()),
            format!(
                "a Unix socket's path can be at most {SOCKET_PATH_MAX} bytes: give a shorter --socket"
            ),
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => {
                    let what = format!("listening on {}", named());
                    return Err(failed(&what, "another server is listening there"));
                },
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(path)
                        .map_err(|err| failed(&format!("replacing {}", named()), err))?;
                },
                Err(err) => return Err(failed(&format!("listening on {}", named()), err)),
            }
        },
        Ok(_) => {
            return Err(failed(
                &format!("listening on {}", named()),
                "something other than a socket is there",
            ));
        },
        Err(_) => {},
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|err| failed(&format!("listening on {}", named()), err))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| failed(&format!("restricting {}", named()), err))?;
    Ok(listener)
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
            server.push.look();
            continue;
        }
        if envelope["error"]["code"] == "no-remote" {
            eprintln!("acyclic: the tracker clone has no origin, so the server doesn't sync");
            return;
        }
        eprintln!("{envelope}");
    }
}

/// How often the mirrors are fetched besides runs' fetches, so readiness sees what was merged.
const REFRESH: Duration = Duration::from_secs(300);

/// Fetches the mirrors every [`REFRESH`].
async fn refresh(code: Arc<code::Code>, tracker: PathBuf) {
    let mut ticks = tokio::time::interval(REFRESH);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        code.refresh_all(&tracker).await;
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
