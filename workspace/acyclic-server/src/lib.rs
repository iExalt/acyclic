#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use acyclic_cli::args::{Class, Command, Serve, Up};
use acyclic_cli::error::{CliError, exit};
use acyclic_cli::{Env, Start};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, ORIGIN, X_FRAME_OPTIONS};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::sync::OwnedMutexGuard;
use tower::ServiceExt as _;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::SizeAbove;

pub mod app;
pub mod push;

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
    /// passing through Tailscale can forge the header, so only Tailscale and local processes, which are trusted
    /// anyway, may be able to reach it.
    Tailscale {
        /// The user's login, which [`IDENTITY`] must equal.
        login: String,
        /// The name `Host` must carry, compared without regard to case, with its port if it has one.
        host: String,
        /// The origin a browser's request must come from, such as `https://<host>`.
        origin: String,
    },
    /// Requests on 127.0.0.1 from anything on the machine: every local process, agents included, acts as the user.
    /// Only the checks on `Host` and `Origin` keep out web pages the user's browser opens.
    Local {
        /// The port the listener is bound to, which `Host` must carry.
        port: u16,
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

    /// The names `Host` may carry on a local listener: the loopback address and `localhost`, with the listener's
    /// port, so that a page from another name that resolves to 127.0.0.1 can't reach it.
    fn local_hosts(port: u16) -> [String; 3] {
        [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
    }

    /// Checks that a request is for this server: on a local listener, that it names the listener's own address.
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
            Self::Local { port } => sent.is_some_and(|sent| {
                Self::local_hosts(*port)
                    .iter()
                    .any(|name| sent.eq_ignore_ascii_case(name.as_bytes()))
            }),
        };
        if ok { Ok(()) } else { Err(Refusal::WrongHost) }
    }

    /// Checks that a request comes from the user, by the listener's own means, for this server.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn check(&self, headers: &HeaderMap) -> Result<(), Refusal> {
        if let Self::Tailscale { login, .. } = self {
            match single(headers, IDENTITY) {
                None => return Err(Refusal::NoIdentity),
                Some(sent) if sent != login.as_bytes() => return Err(Refusal::WrongIdentity),
                Some(_) => {},
            }
        }
        self.host(headers)
    }

    /// Checks that a command's request is JSON and, when a browser sent it, that it comes from the server's own
    /// origin, so that no other site can make the user's browser send one.
    ///
    /// # Errors
    ///
    /// Why the request is refused.
    pub fn same_origin(&self, headers: &HeaderMap) -> Result<(), Refusal> {
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
        let allowed = |origin: &[u8]| match self {
            Self::Tailscale { origin: own, .. } => origin.eq_ignore_ascii_case(own.as_bytes()),
            Self::Local { port } => Self::local_hosts(*port)
                .iter()
                .any(|name| origin.eq_ignore_ascii_case(format!("http://{name}").as_bytes())),
        };
        match single(headers, ORIGIN) {
            Some(origin) if allowed(origin) => Ok(()),
            _ => Err(Refusal::WrongOrigin),
        }
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
        let envelope = json!({"ok": false, "error": {"code": self.code(), "message": self.message(), "details": null}});
        match self {
            Self::NotJson => (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(envelope)).into_response(),
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
            acyclic_cli::call(command, server.config.tracker.clone(), &server.config.env)
        });
        run.await.unwrap_or_else(|_| {
            let err =
                CliError::new("internal", exit::UNREADABLE, "the command failed unexpectedly");
            (err.exit, err.envelope())
        })
    }
}

/// A listener's state: the server and who the listener lets in.
struct Listener {
    server: Arc<Server>,
    access: Access,
}

async fn guard(State(listener): State<Arc<Listener>>, request: Request, next: Next) -> Response {
    match listener.access.check(request.headers()) {
        Ok(()) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    }
}

/// A short page for people, one paragraph of static HTML: nothing in it comes from the request, so nothing needs
/// escaping.
fn page(status: StatusCode, text: &'static str) -> Response {
    let html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\">\n<title>acyclic</title>\n<p>{text}</p>\n"
    );
    (status, [(CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

async fn command(
    State(listener): State<Arc<Listener>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = listener.access.same_origin(&headers) {
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
    // Reads run beside anything and writes wait for the other writes and the sync loop. The server runs nothing
    // else for a request: nothing that sets up or syncs the clone, reads or writes files on the server, or starts
    // another server.
    let class = command.class();
    let (code, envelope) = match class {
        Class::Read => server.call(command, None).await,
        Class::Write => match server.turn().await {
            Ok(turn) => server.call(command, Some(turn)).await,
            Err(err) => (err.exit, err.envelope()),
        },
        Class::Standalone | Class::Server | Class::Clone => {
            return Refusal::NotAllowed.into_response();
        },
    };
    // What the user's own write brings about is theirs, and needs no push.
    if code == exit::OK && class == Class::Write {
        server.push.seen();
    }
    (status(code), Json(envelope)).into_response()
}

/// The server's VAPID public key, which a device subscribes with.
async fn push_key(State(listener): State<Arc<Listener>>) -> Response {
    pushed(listener.server.push.public_key().map(|key| json!({"key": key})))
}

/// Takes a device's subscription: the browser's `PushSubscription`, as JSON.
async fn push_subscribe(
    State(listener): State<Arc<Listener>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = listener.access.same_origin(&headers) {
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
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(refusal) = listener.access.same_origin(&headers) {
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
/// forbids every page to frame it, or the frame could be clicked into writes that the app's own origin sends.
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
    // The API's answers are compressed when they're worth it: a whole tracker's `item show` is megabytes of JSON, and a
    // phone may reach the server through a relay. None of them holds a secret an attacker could guess at through
    // their compressed size. The app's files are compressed already.
    let router = Router::new()
        .route("/api/command", post(command))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(push_subscribe))
        .route("/api/push/unsubscribe", post(push_unsubscribe))
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(1024)))
        .fallback(files);
    let listener = Arc::new(Listener { server, access });
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

/// Serves the tracker clone at `tracker` on 127.0.0.1 to every local process, until interrupted: `acyclic up`.
/// Prints one JSON line with the address once it listens.
///
/// # Errors
///
/// When the settings are wrong or the port can't be bound.
pub fn up(options: &Up, tracker: PathBuf, env: &Env) -> Result<(), CliError> {
    if let Some(dir) = &options.static_dir {
        app::check_dir(dir)?;
    }
    let tailscale = match (options.tailscale, &options.login, &options.host) {
        (true, Some(login), Some(host)) => {
            Some(Access::tailscale(login, host, options.origin.as_deref())?)
        },
        _ => None,
    };
    let mut extra = Vec::new();
    if let Some(access) = tailscale.clone() {
        extra.push(match &options.socket {
            Some(socket) => Extra::Unix(socket.clone(), access),
            None => Extra::Tcp(SocketAddr::from(([127, 0, 0, 1], options.tailscale_port)), access),
        });
    }
    let subject = match &tailscale {
        Some(Access::Tailscale { origin, .. }) => Some(origin.clone()),
        _ => None,
    };
    let push = push::Settings { subject, ..push::Settings::default() };
    let config = Config { tracker, env: env.clone(), static_dir: options.static_dir.clone(), push };
    let bind = SocketAddr::from(([127, 0, 0, 1], options.port));
    let socket = options.socket.clone();
    run(config, options.sync_every, bind, extra, |address, bound| {
        let mut started = json!({"listening": address.to_string()});
        let mut bound = bound.into_iter();
        if tailscale.is_some() {
            started["tailscale"] = match (&socket, bound.next().flatten()) {
                (Some(socket), _) => {
                    json!({"socket": socket, "serve": format!("tailscale serve --bg unix:{}", socket.display())})
                },
                (None, Some(at)) => json!({
                    "listening": at.to_string(),
                    "serve": format!("tailscale serve --bg http://{at}"),
                }),
                (None, None) => unreachable!("a TCP listener has an address"),
            };
        }
        Ok((Access::Local { port: address.port() }, started))
    })
}

/// A listener besides the main one.
enum Extra {
    /// A Unix socket.
    Unix(PathBuf, Access),
    /// A TCP address.
    Tcp(SocketAddr, Access),
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
                Extra::Tcp(at, access) => {
                    let tcp = tokio::net::TcpListener::bind(at)
                        .await
                        .map_err(|err| failed(&format!("listening on {at}"), err))?;
                    bound.push(Some(
                        tcp.local_addr().map_err(|err| failed("reading the address", err))?,
                    ));
                    let routes = routes(server.clone(), access);
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

/// Syncs the clone with its `origin` every `every`, each sync after the last has finished, until the server stops. A failed sync prints its envelope to standard error and the next one tries
/// again; a clone without `origin` stops the loop after saying so.
async fn sync(server: Arc<Server>, every: Duration) {
    let mut ticks = tokio::time::interval(every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        // A sync waits as long as it must: it is the server's own, and the next tick would only queue behind it.
        let turn = server.turns.clone().lock_owned().await;
        let (code, envelope) = server.call(Command::Sync, Some(turn)).await;
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
