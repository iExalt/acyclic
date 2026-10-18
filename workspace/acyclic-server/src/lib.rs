#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use acyclic_cli::args::{
    Command, ItemCommand, LinkCommand, ProposalCommand, RepoCommand, Serve, ThreadCommand, Up,
};
use acyclic_cli::error::{CliError, exit};
use acyclic_cli::{Env, Start};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{
    AUTHORIZATION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE as COOKIE_HEADER, HOST, LOCATION,
    ORIGIN, SET_COOKIE, WWW_AUTHENTICATE, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tower_http::services::ServeDir;

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
    /// The directory of static files, if any.
    pub static_dir: Option<PathBuf>,
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
        };
        if ok { Ok(()) } else { Err(Refusal::WrongHost) }
    }

    /// Checks that a request comes from the user, by the listener's own means, for this server, and says how.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn check(&self, headers: &HeaderMap) -> Result<Credential, Refusal> {
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
            Self::Session { token, .. } => {
                self.host(headers)?;
                if headers.contains_key(AUTHORIZATION) {
                    return match bearer(headers) {
                        Some(presented) if token.matches(presented) => Ok(Credential::Bearer),
                        _ => Err(Refusal::NoSession),
                    };
                }
                let browser = token.browser();
                if cookies(headers).any(|value| session::equal(value, browser.as_bytes())) {
                    return Ok(Credential::Cookie);
                }
                Err(Refusal::NoSession)
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
    pub fn same_origin(&self, headers: &HeaderMap, credential: Credential) -> Result<(), Refusal> {
        let json = single(headers, CONTENT_TYPE).is_some_and(|value| {
            let media = value.split(|byte| *byte == b';').next().unwrap_or_default();
            media.trim_ascii().eq_ignore_ascii_case(b"application/json")
        });
        if !json {
            return Err(Refusal::NotJson);
        }
        if credential == Credential::Cookie {
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
        };
        match single(headers, ORIGIN) {
            Some(origin) if allowed(origin) => Ok(()),
            _ => Err(Refusal::WrongOrigin),
        }
    }
}

/// How a request showed it comes from the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    /// Tailscale's identity header.
    Identity,
    /// The session, in `Authorization`.
    Bearer,
    /// A paired browser's cookie.
    Cookie,
}

/// The cookie a paired browser holds.
const COOKIE: &str = "acyclic-up";

/// The values of every cookie named [`COOKIE`] in the request. Another local server can set one of the same name, which
/// can only fail to match, so each is tried.
fn cookies(headers: &HeaderMap) -> impl Iterator<Item = &[u8]> {
    headers.get_all(COOKIE_HEADER).into_iter().flat_map(|value| {
        value.as_bytes().split(|byte| *byte == b';').filter_map(|pair| {
            let (name, value) = pair
                .trim_ascii()
                .split_at(pair.trim_ascii().iter().position(|byte| *byte == b'=')?);
            (name == COOKIE.as_bytes()).then_some(&value[1..])
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
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let envelope = json!({"ok": false, "error": {"code": self.code(), "message": self.message(), "details": null}});
        match self {
            Self::NotJson => (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(envelope)).into_response(),
            Self::NoSession => {
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

/// A listener's state: the server, who the listener lets in, and the pairing codes spent on it.
struct Listener {
    server: Arc<Server>,
    access: Access,
    spent: session::Spent,
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
    if let Access::Session { token, .. } = access
        && request.uri().path() == LOGIN
    {
        return match access.host(request.headers()) {
            Ok(()) => pair(&listener, token, request.uri().query()),
            Err(refusal) => refusal.into_response(),
        };
    }
    match access.check(request.headers()) {
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
fn pair(listener: &Listener, token: &Token, query: Option<&str>) -> Response {
    let now = acyclic_cli::now(&listener.server.config.env);
    let code = query.and_then(|query| query.strip_prefix("code="));
    let made = code.and_then(|code| Some((code, token.verify(code, now)?)));
    match made {
        Some((code, made)) if listener.spent.spend(code, made, now) => {
            let cookie = format!(
                "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={COOKIE_AGE}",
                token.browser()
            );
            (StatusCode::SEE_OTHER, [(SET_COOKIE, cookie), (LOCATION, "/".to_owned())])
                .into_response()
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
    if let Err(refusal) = listener.access.same_origin(&headers, credential) {
        return refusal.into_response();
    }
    let command: Command = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(err) => {
            let err = CliError::new("usage", exit::USAGE, format!("not a command: {err}"));
            return (StatusCode::BAD_REQUEST, Json(err.envelope())).into_response();
        },
    };
    let server = &listener.server;
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

/// Forbids every page to frame any response, refusals included: a page from another local server is the same site as
/// the app's, so a paired browser would send its cookie into a frame, and the frame could be clicked into writes.
async fn unframed(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("frame-ancestors 'none'"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

/// The server's routes for one listener, every one behind its [`Access`].
pub fn router(config: Config, access: Access) -> Router {
    routes(Arc::new(Server::new(config)), access)
}

fn routes(server: Arc<Server>, access: Access) -> Router {
    let mut router = Router::new().route("/api/command", post(command));
    if let Some(dir) = &server.config.static_dir {
        router = router.fallback_service(ServeDir::new(dir));
    }
    let listener = Arc::new(Listener { server, access, spent: session::Spent::default() });
    router
        .layer(middleware::from_fn_with_state(listener.clone(), guard))
        .layer(middleware::map_response(unframed))
        .with_state(listener)
}

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
    let config = Config { tracker, env: env.clone(), static_dir: options.static_dir.clone() };
    run(config, options.sync_every, options.listen, None, |address| {
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
    let (token, path) = session::open(home.as_ref(), options.rotate)?;
    let tailscale = match (options.tailscale, &options.login, &options.host) {
        (true, Some(login), Some(host)) => {
            let access = Access::tailscale(login, host, options.origin.as_deref())?;
            let socket =
                options.socket.clone().unwrap_or_else(|| path.with_file_name("tailscale.sock"));
            Some((socket, access))
        },
        _ => None,
    };
    let config = Config { tracker, env: env.clone(), static_dir: options.static_dir.clone() };
    let bind = SocketAddr::from(([127, 0, 0, 1], options.port));
    let link = |port| link(port, &token, env);
    let socket = tailscale.as_ref().map(|(socket, _)| socket.clone());
    run(config, options.sync_every, bind, tailscale, |address| {
        let mut started = json!({"listening": address.to_string(), "session": path, "link": link(address.port())?});
        if let Some(socket) = socket {
            let serve = format!("tailscale serve --bg unix:{}", socket.display());
            started["tailscale"] = json!({"socket": socket, "serve": serve});
        }
        Ok((Access::Session { port: address.port(), token: token.clone() }, started))
    })
}

/// Binds `bind`, and the Unix socket with its own access if given, gives the bound address to `access` for the
/// listener's access and the line to print, prints it, and serves until interrupted, syncing every `sync_every`
/// seconds unless that is 0.
fn run(
    config: Config,
    sync_every: u64,
    bind: SocketAddr,
    socket: Option<(PathBuf, Access)>,
    access: impl FnOnce(SocketAddr) -> Result<(Access, Value), CliError>,
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
        let unix = socket
            .map(|(path, access)| bind_socket(&path).map(|listener| (listener, access)))
            .transpose()?;
        let (access, started) = access(address)?;
        if sync_every > 0 {
            tokio::spawn(sync(server.clone(), Duration::from_secs(sync_every)));
        }
        let mut stdout = std::io::stdout();
        writeln!(stdout, "{}", json!({"ok": true, "data": started}))
            .and_then(|()| stdout.flush())
            .map_err(|err| failed("printing the address", err))?;
        let tcp =
            axum::serve(listener, routes(server.clone(), access)).with_graceful_shutdown(stopped());
        match unix {
            None => tcp.await,
            Some((unix, access)) => {
                let unix =
                    axum::serve(unix, routes(server, access)).with_graceful_shutdown(stopped());
                tokio::try_join!(tcp.into_future(), unix.into_future()).map(|_| ())
            },
        }
        .map_err(|err| failed("serving", err))
    })
}

/// The longest path a Unix socket can have on macOS, the shortest limit of the systems acyclic runs on.
const SOCKET_PATH_MAX: usize = 103;

/// Binds a Unix socket at `path`, replacing a socket left by an earlier server, and makes it the user's alone.
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
            std::fs::remove_file(path)
                .map_err(|err| failed(&format!("replacing {}", named()), err))?;
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
