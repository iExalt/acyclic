//! Remote mode, for runs: with `ACYCLIC_REMOTE=http://<host>:<port>` and the run's token in `ACYCLIC_TOKEN`, each
//! command goes to the server's runners' listener, which runs it as the run, instead of to a tracker clone.
//!
//! A write the run may send carries its number in the run's sequence, which the server applies once. The number is
//! the server's own latest applied, plus one, asked for just before sending, so the client keeps nothing: an answer
//! lost on the way is found out by the next write, whose number then follows the lost one's if it was applied and
//! takes its place if not. Writes from one machine take turns under a lock, so two can't take the same number. A
//! failure that may pass, such as the server being unreachable or busy, is tried again a few times with the same
//! number, which the server answers `duplicate` if it had applied it after all.

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value as Json;

use crate::Env;
use crate::args::{Cli, Command};
use crate::error::{CliError, exit};

/// The variable naming the server's runners' listener.
pub const REMOTE: &str = "ACYCLIC_REMOTE";
/// The variable holding the run's token.
pub const TOKEN: &str = "ACYCLIC_TOKEN";

/// How long to wait to connect, and for an answer: past the server's wait for a write's turn, with time to apply it.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The pauses before each further try of a failure that may pass.
const BACKOFF: [Duration; 3] =
    [Duration::from_millis(500), Duration::from_secs(1), Duration::from_secs(2)];

/// What a command sent to the server came to: its data, or its exit code and error envelope.
pub type Answer = Result<Json, (u8, Json)>;

/// Runs `cli`'s command on the server at `url`, as the run whose token `env` holds.
///
/// # Errors
///
/// The exit code and error envelope of a command the server refused or couldn't be reached for, or that remote mode
/// doesn't run.
pub fn run(cli: &Cli, env: &Env, url: &str) -> Answer {
    let local = |err: CliError| Err((err.exit, err.envelope()));
    if cli.actor.is_some() || cli.as_user {
        return local(CliError::new(
            "usage",
            exit::USAGE,
            "a run's token says who it is: remote mode takes no --actor or --as-user",
        ));
    }
    if matches!(
        cli.command,
        Command::Init { .. }
            | Command::Sync { .. }
            | Command::Import { .. }
            | Command::Graph { .. }
            | Command::Doctor
            | Command::Serve(_)
            | Command::Up(_)
    ) {
        return local(CliError::new(
            "not-remote",
            exit::USAGE,
            "remote mode reads and writes the tracker through the server alone; this command needs a tracker clone",
        ));
    }
    let server = match Server::new(url, env) {
        Ok(server) => server,
        Err(err) => return local(err),
    };
    let body = match serde_json::to_string(&cli.command) {
        Ok(body) => body,
        Err(err) => return local(CliError::new("usage", exit::USAGE, format!("{err}"))),
    };
    if !cli.command.run_may_send() {
        return server.send("POST", "/api/command", &body, None);
    }
    let _turn = match turn(env) {
        Ok(turn) => turn,
        Err(err) => return local(err),
    };
    let applied = server.send("GET", "/api/run", "", None)?;
    let seq = applied["applied"].as_u64().unwrap_or(0) + 1;
    server.send("POST", "/api/command", &body, Some(seq))
}

/// The server at `url`, with the run's token.
struct Server {
    /// `host:port`.
    address: String,
    token: String,
}

impl Server {
    fn new(url: &str, env: &Env) -> Result<Self, CliError> {
        let config = |message: String| CliError::new("config", exit::CONFIG, message);
        let address = url
            .strip_prefix("http://")
            .map(|rest| rest.trim_end_matches('/'))
            .filter(|address| !address.is_empty() && !address.contains('/'))
            .ok_or_else(|| config(format!("{REMOTE} must be http://<host>:<port>, not {url}")))?;
        let token = env
            .vars
            .get(TOKEN)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| config(format!("remote mode needs the run's token in {TOKEN}")))?;
        Ok(Self { address: address.to_owned(), token: token.clone() })
    }

    /// Sends one request, trying a failure that may pass again with the same request, and returns the answer's data.
    fn send(&self, method: &str, path: &str, body: &str, seq: Option<u64>) -> Answer {
        let mut tries = BACKOFF.iter();
        loop {
            match self.exchange(method, path, body, seq) {
                Err(failure) if failure.passes() => match tries.next() {
                    Some(pause) => std::thread::sleep(*pause),
                    None => return Err(failure.answer()),
                },
                Err(failure) => return Err(failure.answer()),
                Ok(data) => return Ok(data),
            }
        }
    }

    fn exchange(
        &self,
        method: &str,
        path: &str,
        body: &str,
        seq: Option<u64>,
    ) -> Result<Json, Failure> {
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n",
            self.address, self.token
        );
        if method == "POST" {
            let _ = write!(
                request,
                "Content-Type: application/json
Content-Length: {}
",
                body.len()
            );
        }
        if let Some(seq) = seq {
            let _ = write!(request, "Acyclic-Operation: {seq}\r\n");
        }
        request.push_str("\r\n");
        request.push_str(body);
        let unreachable = |err: std::io::Error| Failure::Unreachable(err.to_string());
        let addresses =
            std::net::ToSocketAddrs::to_socket_addrs(&self.address).map_err(unreachable)?;
        let mut last = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, TIMEOUT) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                },
                Err(err) => last = Some(err),
            }
        }
        let mut stream = stream.ok_or_else(|| {
            unreachable(last.unwrap_or_else(|| std::io::ErrorKind::NotFound.into()))
        })?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(unreachable)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(unreachable)?;
        stream.write_all(request.as_bytes()).map_err(unreachable)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).map_err(unreachable)?;
        let (status, envelope) = parse(&response).ok_or(Failure::Garbled)?;
        match envelope.get("ok").and_then(Json::as_bool) {
            Some(true) => Ok(envelope.get("data").cloned().unwrap_or(Json::Null)),
            _ => Err(Failure::Refused(status, envelope)),
        }
    }
}

/// Why a request failed.
#[derive(Debug)]
enum Failure {
    /// The server couldn't be reached, or the connection broke.
    Unreachable(String),
    /// The answer wasn't HTTP with a JSON body.
    Garbled,
    /// The server answered with an error: its status and envelope.
    Refused(u16, Json),
}

impl Failure {
    /// Whether trying again may get past it.
    fn passes(&self) -> bool {
        match self {
            Self::Unreachable(_) => true,
            Self::Garbled => false,
            Self::Refused(status, _) => matches!(status, 502..=504),
        }
    }

    fn answer(self) -> (u8, Json) {
        match self {
            Self::Unreachable(reason) => {
                let err = CliError::new(
                    "unreachable",
                    exit::STORE,
                    format!("the server can't be reached: {reason}; try again shortly"),
                );
                (err.exit, err.envelope())
            },
            Self::Garbled => {
                let err =
                    CliError::new("garbled", exit::STORE, "the server's answer can't be read");
                (err.exit, err.envelope())
            },
            Self::Refused(status, envelope) => (exit_of(status), envelope),
        }
    }
}

/// The exit code for an error answer's status, as the server chose the status from the exit code.
fn exit_of(status: u16) -> u8 {
    match status {
        400 | 415 => exit::USAGE,
        404 => exit::NOT_FOUND,
        500 => exit::UNREADABLE,
        501..=599 => exit::STORE,
        _ => exit::REFUSED,
    }
}

/// An HTTP/1.1 response's status and JSON body, sent whole or in chunks.
fn parse(response: &[u8]) -> Option<(u16, Json)> {
    let split = response.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&response[..split]).ok()?;
    let mut body = response[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    });
    if chunked {
        body = dechunk(&body)?;
    }
    Some((status, serde_json::from_slice(&body).ok()?))
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut whole = Vec::new();
    loop {
        let end = body.windows(2).position(|window| window == b"\r\n")?;
        let size = std::str::from_utf8(&body[..end]).ok()?.split(';').next()?.trim();
        let size = usize::from_str_radix(size, 16).ok()?;
        body = &body[end + 2..];
        if size == 0 {
            return Some(whole);
        }
        whole.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

/// Waits for this machine's turn to write, under a lock in the user's state directory, held until dropped.
fn turn(env: &Env) -> Result<std::fs::File, CliError> {
    let failed = |err: std::io::Error| {
        CliError::new("config", exit::CONFIG, format!("can't take the remote writes' lock: {err}"))
    };
    let state = env
        .vars
        .get("XDG_STATE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env.vars
                .get("HOME")
                .filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .ok_or_else(|| {
            CliError::new(
                "config",
                exit::CONFIG,
                "remote mode needs HOME or XDG_STATE_HOME for its lock",
            )
        })?;
    let dir = state.join("acyclic");
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("remote.lock"))
        .map_err(failed)?;
    file.lock().map_err(failed)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn refusal(code: &str) -> Json {
        json!({"ok": false, "error": {"code": code, "message": "m", "details": null}})
    }

    #[test]
    fn answers_are_read_whole_or_in_chunks() {
        let whole = b"HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"ok\":true}";
        assert_eq!(parse(whole), Some((409, json!({"ok": true}))));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"ok\"\r\n6\r\n:true}\r\n0\r\n\r\n";
        assert_eq!(parse(chunked), Some((200, json!({"ok": true}))));
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\n\r\nnot json"), None);
        assert_eq!(parse(b"garbage"), None);
    }

    #[test]
    fn only_failures_that_may_pass_are_tried_again() {
        assert!(Failure::Unreachable("refused".into()).passes());
        assert!(Failure::Refused(503, refusal("busy")).passes());
        for status in [400, 401, 403, 409, 500] {
            assert!(!Failure::Refused(status, refusal("x")).passes(), "{status}");
        }
        assert_eq!(Failure::Refused(409, refusal("not-in-thread")).answer().0, exit::REFUSED);
        assert_eq!(Failure::Refused(503, refusal("busy")).answer().0, exit::STORE);
    }

    #[test]
    fn the_remote_is_a_plain_http_address() {
        let env = Env { vars: [(TOKEN.to_owned(), "t".to_owned())].into(), ..Env::default() };
        assert_eq!(Server::new("http://127.0.0.1:7479/", &env).unwrap().address, "127.0.0.1:7479");
        for url in ["https://host:1", "host:1", "http://host:1/api", "http://"] {
            assert!(Server::new(url, &env).is_err(), "{url}");
        }
        assert!(Server::new("http://host:1", &Env::default()).is_err(), "no token");
    }
}
