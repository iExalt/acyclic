use axum::body::{Body, to_bytes};
use axum::http::Method;
use tower::ServiceExt as _;

use serde_json::json;

use super::session::{CODE_LIFE, CODE_SKEW};
use super::*;

const LOGIN: &str = "user@example.com";
const HOST_NAME: &str = "acyclic.example.ts.net";

const PORT: u16 = 7478;
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn config(static_dir: Option<PathBuf>) -> Config {
    Config { tracker: PathBuf::from("/nonexistent/tracker"), env: Env::default(), static_dir }
}

fn tailscale() -> Access {
    Access::Tailscale {
        login: LOGIN.into(),
        host: HOST_NAME.into(),
        origin: format!("https://{HOST_NAME}"),
    }
}

fn session() -> Access {
    Access::Session { port: PORT, token: Token::parse(TOKEN).unwrap() }
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    map
}

#[test]
fn only_the_user_through_tailscale_for_this_host_passes() {
    let access = tailscale();
    let check = |pairs: &[(&str, &str)]| access.check(&headers(pairs), 0);
    let user = (IDENTITY, LOGIN);
    let host = ("host", HOST_NAME);
    assert_eq!(check(&[user, host]), Ok(Credential::Identity));
    assert_eq!(check(&[user, ("host", "ACYCLIC.example.ts.net")]), Ok(Credential::Identity));
    assert_eq!(check(&[host]), Err(Refusal::NoIdentity));
    assert_eq!(check(&[user, user, host]), Err(Refusal::NoIdentity));
    assert_eq!(check(&[(IDENTITY, "other@example.com"), host]), Err(Refusal::WrongIdentity));
    assert_eq!(check(&[(IDENTITY, "USER@example.com"), host]), Err(Refusal::WrongIdentity));
    assert_eq!(check(&[(IDENTITY, ""), host]), Err(Refusal::WrongIdentity));
    assert_eq!(check(&[user]), Err(Refusal::WrongHost));
    assert_eq!(check(&[user, host, host]), Err(Refusal::WrongHost));
    assert_eq!(check(&[user, ("host", "other.example.ts.net")]), Err(Refusal::WrongHost));
    // A port counts: none configured refuses one sent, and one configured must be sent.
    assert_eq!(check(&[user, ("host", "acyclic.example.ts.net:8080")]), Err(Refusal::WrongHost));
    let ported = Access::Tailscale {
        login: LOGIN.into(),
        host: format!("{HOST_NAME}:8080"),
        origin: format!("https://{HOST_NAME}"),
    };
    assert_eq!(
        ported.check(&headers(&[user, ("host", "acyclic.example.ts.net:8080")]), 0),
        Ok(Credential::Identity)
    );
    assert_eq!(ported.check(&headers(&[user, host]), 0), Err(Refusal::WrongHost));
}

#[test]
fn only_the_session_for_this_port_passes_on_the_machine() {
    let access = session();
    let check = |pairs: &[(&str, &str)]| access.check(&headers(pairs), 0);
    let bearer = format!("Bearer {TOKEN}");
    let session = ("authorization", bearer.as_str());
    let host = ("host", "127.0.0.1:7478");
    assert_eq!(check(&[session, host]), Ok(Credential::Bearer));
    assert_eq!(check(&[session, ("host", "localhost:7478")]), Ok(Credential::Bearer));
    assert_eq!(check(&[session, ("host", "LOCALHOST:7478")]), Ok(Credential::Bearer));
    assert_eq!(check(&[session, ("host", "[::1]:7478")]), Ok(Credential::Bearer));
    let lower = format!("bearer {TOKEN}");
    assert_eq!(check(&[("authorization", lower.as_str()), host]), Ok(Credential::Bearer));
    // A name that merely resolves to the machine, another port, or a spelling that isn't one of the three is refused,
    // so a page from elsewhere can't rebind its name to the loopback address.
    for other in [
        "evil.example:7478",
        "localhost:7479",
        "localhost",
        "localhost.:7478",
        "127.0.0.1:7478.",
        "127.0.0.1:07478",
        "::1:7478",
    ] {
        assert_eq!(check(&[session, ("host", other)]), Err(Refusal::WrongHost), "{other}");
    }
    assert_eq!(check(&[session]), Err(Refusal::WrongHost));
    assert_eq!(check(&[session, host, host]), Err(Refusal::WrongHost));
    let short = &TOKEN[1..];
    let long = format!("{TOKEN}0");
    let wrong = format!("1{short}");
    let upper = TOKEN.to_uppercase();
    for presented in [
        String::new(),
        "Bearer".into(),
        "Bearer ".into(),
        format!("Bearer {short}"),
        format!("Bearer {long}"),
        format!("Bearer {wrong}"),
        format!("Bearer {upper}"),
        format!("Bearer  {TOKEN}"),
        format!("Bearer {TOKEN} "),
        format!("Basic {TOKEN}"),
        TOKEN.into(),
    ] {
        let sent = check(&[("authorization", presented.as_str()), host]);
        assert_eq!(sent, Err(Refusal::NoSession), "{presented:?}");
    }
    assert_eq!(check(&[host]), Err(Refusal::NoSession));
    assert_eq!(check(&[session, session, host]), Err(Refusal::NoSession));
    // Tailscale's header counts for nothing here.
    assert_eq!(check(&[(IDENTITY, LOGIN), host]), Err(Refusal::NoSession));
}

#[test]
fn commands_come_as_json_from_no_other_origin() {
    let access = tailscale();
    let check = |pairs: &[(&str, &str)]| access.same_origin(&headers(pairs), &Credential::Identity);
    let json = ("content-type", "application/json");
    assert_eq!(check(&[json]), Ok(()));
    assert_eq!(check(&[("content-type", "Application/JSON; charset=utf-8")]), Ok(()));
    assert_eq!(check(&[json, ("origin", "https://acyclic.example.ts.net")]), Ok(()));
    assert_eq!(check(&[]), Err(Refusal::NotJson));
    assert_eq!(check(&[("content-type", "text/plain")]), Err(Refusal::NotJson));
    assert_eq!(check(&[("content-type", "application/jsonx")]), Err(Refusal::NotJson));
    assert_eq!(check(&[json, ("origin", "null")]), Err(Refusal::WrongOrigin));
    assert_eq!(check(&[json, ("origin", "https://elsewhere.example")]), Err(Refusal::WrongOrigin));
    assert_eq!(
        check(&[json, ("origin", "http://acyclic.example.ts.net")]),
        Err(Refusal::WrongOrigin)
    );
    assert_eq!(
        check(&[json, ("origin", "https://acyclic.example.ts.net:444")]),
        Err(Refusal::WrongOrigin)
    );
    // On the machine, a page from the listener's own names and port.
    let access = session();
    let check = |pairs: &[(&str, &str)]| access.same_origin(&headers(pairs), &Credential::Bearer);
    assert_eq!(check(&[json]), Ok(()));
    assert_eq!(check(&[json, ("origin", "http://127.0.0.1:7478")]), Ok(()));
    assert_eq!(check(&[json, ("origin", "http://localhost:7478")]), Ok(()));
    for other in
        ["http://localhost:3000", "https://127.0.0.1:7478", "http://evil.example:7478", "null"]
    {
        assert_eq!(check(&[json, ("origin", other)]), Err(Refusal::WrongOrigin), "{other}");
    }
}

#[test]
fn a_runners_listener_lets_in_a_valid_run_token_by_any_name_and_no_browser() {
    use acyclic_operator::token::Signer;
    const NOW: Millis = 1_792_022_400_000;
    let signer = Signer::from_seed([3; 32]);
    let access =
        Access::Run { verifier: signer.verifier(), deny: DenyList::parse("term\n"), code: None };
    let token = signer.mint("r1", "t1", NOW, 60_000).unwrap();
    let forged = Signer::from_seed([4; 32]).mint("r1", "t1", NOW, 60_000).unwrap();
    let bearer = format!("Bearer {token}");
    let check = |pairs: &[(&str, &str)]| access.check(&headers(pairs), NOW);
    // A container names the Mac as it can, and a pod names a service.
    for host in ["host.containers.internal:7479", "acyclic.runners.svc:7479", "127.0.0.1:7479"] {
        let claims = check(&[("authorization", &bearer), ("host", host)]);
        assert!(matches!(claims, Ok(Credential::Run(claims)) if claims.run == "r1"), "{host}");
    }
    let forged = format!("Bearer {forged}");
    assert_eq!(check(&[("authorization", &forged)]), Err(Refusal::NoToken));
    assert_eq!(check(&[("authorization", &format!("Bearer {TOKEN}"))]), Err(Refusal::NoToken));
    assert_eq!(check(&[]), Err(Refusal::NoToken));
    assert_eq!(
        access.check(&headers(&[("authorization", &bearer)]), NOW + 60_000),
        Err(Refusal::NoToken)
    );
    // The session is no run token, and a run token no session.
    let session = session();
    let host = ("host", "127.0.0.1:7478");
    assert_eq!(
        session.check(&headers(&[("authorization", &bearer), host]), NOW),
        Err(Refusal::NoSession)
    );
    let run = Credential::Run(signer.verifier().verify(&token, NOW).unwrap());
    let json = ("content-type", "application/json");
    assert_eq!(access.same_origin(&headers(&[json]), &run), Ok(()));
    let from = ("origin", "http://127.0.0.1:7479");
    assert_eq!(access.same_origin(&headers(&[json, from]), &run), Err(Refusal::WrongOrigin));
}

#[test]
fn a_write_is_scanned_as_it_would_be_stored() {
    let sent = r#"{"item": {"edit": {"item": "a", "title": "\u0074erm", "notes": "x\ny"}}}"#;
    let text = stored(&serde_json::from_str(sent).unwrap());
    assert!(text.lines().any(|line| line == "term"), "{text}");
    assert_eq!(DenyList::parse("term\n").matches(&text), [1]);
    assert_eq!(
        DenyList::parse("term\n").matches(sent),
        Vec::<usize>::new(),
        "as sent, the term hides"
    );
}

/// Sends one request through the router and returns its status and body.
fn send(
    router: &Router,
    method: Method,
    path: &str,
    pairs: &[(&str, &str)],
    body: &str,
) -> (StatusCode, String) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in pairs {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::from(body.to_owned())).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    })
}

fn code(body: &str) -> String {
    serde_json::from_str::<Value>(body).unwrap()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn every_route_is_behind_the_listeners_check() {
    let dir = tempfile::tempdir().unwrap();
    let www = dir.path().join("www");
    std::fs::create_dir(&www).unwrap();
    std::fs::write(www.join("index.html"), "the app").unwrap();
    std::fs::write(dir.path().join("secret"), "outside").unwrap();
    let router = router(config(Some(www.clone())), tailscale());
    let host = ("host", HOST_NAME);
    let user = (IDENTITY, LOGIN);
    for (method, path) in [
        (Method::GET, "/index.html"),
        (Method::GET, "/missing"),
        (Method::OPTIONS, "/api/command"),
        (Method::GET, "/api/command"),
        (Method::POST, "/api/command"),
    ] {
        let (status, body) = send(&router, method.clone(), path, &[host], "");
        assert_eq!(
            (status, code(&body)),
            (StatusCode::FORBIDDEN, "no-identity".into()),
            "{method} {path}"
        );
    }
    assert_eq!(
        send(&router, Method::GET, "/index.html", &[host, user], ""),
        (StatusCode::OK, "the app".into())
    );
    assert_eq!(send(&router, Method::GET, "/missing", &[host, user], "").0, StatusCode::NOT_FOUND);
    // Static files can't reach outside their directory.
    for path in ["/../secret", "/%2e%2e/secret", "/..%2fsecret"] {
        let (status, body) = send(&router, Method::GET, path, &[host, user], "");
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains("outside"), "{path}");
    }
    // On the machine, the session stands in for the identity, and a preflight without it is refused, so no other
    // page's script is ever let in.
    let router = super::router(config(Some(www)), session());
    let host = ("host", "127.0.0.1:7478");
    let bearer = format!("Bearer {TOKEN}");
    let (status, body) = send(&router, Method::GET, "/index.html", &[host, (IDENTITY, LOGIN)], "");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("acyclic up --link"), "{body}");
    for (method, path) in [
        (Method::GET, "/api/command"),
        (Method::OPTIONS, "/api/command"),
        (Method::POST, "/api/command"),
    ] {
        let (status, body) = send(&router, method.clone(), path, &[host, (IDENTITY, LOGIN)], "");
        assert_eq!(
            (status, code(&body)),
            (StatusCode::UNAUTHORIZED, "no-session".into()),
            "{method} {path}"
        );
    }
    assert_eq!(
        send(&router, Method::GET, "/index.html", &[host, ("authorization", &bearer)], ""),
        (StatusCode::OK, "the app".into())
    );
    // No page may frame the app, or a refusal.
    for pairs in [vec![host, ("authorization", bearer.as_str())], vec![host]] {
        let (_, headers) = exchange(&router, "/index.html", &pairs);
        assert_eq!(headers[CONTENT_SECURITY_POLICY], "frame-ancestors 'none'");
        assert_eq!(headers[X_FRAME_OPTIONS], "DENY");
    }
}

#[test]
fn the_server_runs_only_the_commands_it_allows() {
    let router = router(config(None), tailscale());
    let pass = [("host", HOST_NAME), (IDENTITY, LOGIN), ("content-type", "application/json")];
    let post = |pairs: &[(&str, &str)], body: &str| {
        let (status, body) = send(&router, Method::POST, "/api/command", pairs, body);
        (status, code(&body))
    };
    // A read passes every check and runs; the tracker clone is missing, so it fails as the CLI's would.
    let (status, code) = post(&pass, r#"{"show": {"items": ["a"]}}"#);
    assert!(status.is_server_error() && !code.is_empty(), "{status} {code}");
    // So does a write.
    let (status, code) = post(&pass, r#"{"item": {"add": {"item": "a", "title": "A"}}}"#);
    assert!(status.is_server_error() && !code.is_empty(), "{status} {code}");
    assert_eq!(
        post(&pass, r#"{"sync": {"with-usage": true}}"#),
        (StatusCode::FORBIDDEN, "not-allowed".into())
    );
    assert_eq!(
        post(&pass, r#"{"show": {"items": []}}"#),
        (StatusCode::BAD_REQUEST, "usage".into())
    );
    assert_eq!(post(&pass, r#"{"serve": {}}"#), (StatusCode::BAD_REQUEST, "usage".into()));
    assert_eq!(post(&pass, r#"{"up": {}}"#), (StatusCode::BAD_REQUEST, "usage".into()));
    // The commands that grammar replaced are refused, not guessed at.
    for old in [
        r#"{"thread": {"set": {"id": "t", "title": "T"}}}"#,
        r#"{"item": {"move": {"id": "a", "parent": "b"}}}"#,
        r#"{"item": {"add": {"id": "a", "title": "A"}}}"#,
    ] {
        assert_eq!(post(&pass, old), (StatusCode::BAD_REQUEST, "usage".into()), "{old}");
    }
    assert_eq!(
        post(&pass[..2], r#""list""#),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "not-json".into())
    );
    let foreign = [pass[0], pass[1], pass[2], ("origin", "https://elsewhere.example")];
    assert_eq!(post(&foreign, r#""list""#), (StatusCode::FORBIDDEN, "wrong-origin".into()));
}

#[test]
fn reads_and_writes_run_but_nothing_that_touches_the_servers_files() {
    let classify = |value: Value| classify(&serde_json::from_value(value).unwrap());
    for read in [
        json!("list"),
        json!("needs"),
        json!({"ready": {}}),
        json!({"show": {"items": ["a"]}}),
        json!({"thread": "list"}),
        json!({"thread": {"show": {"threads": ["t"]}}}),
        json!({"repo": "list"}),
        json!({"repo": {"show": {"repositories": ["r"]}}}),
        json!({"proposal": "list"}),
        json!({"proposal": {"show": {"proposals": ["p"]}}}),
        json!({"level": {}}),
    ] {
        assert_eq!(classify(read.clone()), Use::Read, "{read}");
    }
    for write in [
        json!({"claim": {"thread": "t"}}),
        json!({"release": {"thread": "t"}}),
        json!({"done": {"item": "a", "note": "n"}}),
        json!({"level": {"level": "blockers"}}),
        json!({"item": {"add": {"item": "a", "title": "A"}}}),
        json!({"item": {"edit": {"item": "a", "no-thread": true, "area": ["x"]}}}),
        json!({"item": {"archive": {"item": "a"}}}),
        json!({"item": {"delete": {"item": "a"}}}),
        json!({"link": {"add": {"from": "a", "kind": "waits-for", "to": "b"}}}),
        json!({"thread": {"approve": {"thread": "t"}}}),
        json!({"thread": {"add": {"thread": "t", "title": "T"}}}),
        json!({"thread": {"edit": {"thread": "t", "no-level": true}}}),
        json!({"repo": {"add": {"repository": "r", "owner": "o", "name": "n", "default-branch": "main", "public": true}}}),
        json!({"repo": {"edit": {"repository": "r", "private": true}}}),
        json!({"proposal": {"keep": {"proposal": "p"}}}),
    ] {
        assert_eq!(classify(write.clone()), Use::Write, "{write}");
    }
    for refused in [
        json!({"init": {"path": "/elsewhere", "replica": "x"}}),
        json!({"sync": {}}),
        json!("prime"),
        json!({"import": {"file": "/etc/passwd"}}),
        json!("doctor"),
        json!({"graph": {"file": "/tmp/graph.dot"}}),
    ] {
        assert_eq!(classify(refused.clone()), Use::Refused, "{refused}");
    }
}

#[test]
fn a_write_that_cant_get_its_turn_is_told_the_server_is_busy() {
    let server = Arc::new(Server { turn: Duration::from_millis(50), ..Server::new(config(None)) });
    let router = routes(server.clone(), tailscale());
    let pass = [("host", HOST_NAME), (IDENTITY, LOGIN), ("content-type", "application/json")];
    // A sync holds the turn throughout.
    let _sync = server.turns.clone().try_lock_owned().unwrap();
    let (status, body) = send(
        &router,
        Method::POST,
        "/api/command",
        &pass,
        r#"{"item": {"add": {"item": "a", "title": "A"}}}"#,
    );
    assert_eq!((status, code(&body)), (StatusCode::SERVICE_UNAVAILABLE, "busy".into()));
    // Reads don't wait for it.
    let (status, body) = send(&router, Method::POST, "/api/command", &pass, r#""list""#);
    assert_ne!(code(&body), "busy", "{status}");
}

/// A server for `acyclic up` whose clock reads `now`.
fn up_router(now: i64) -> Router {
    let env = Env { vars: [("ACYCLIC_NOW".to_owned(), now.to_string())].into(), ..Env::default() };
    super::router(Config { env, ..config(None) }, session())
}

/// Sends a request and returns its status and its headers.
fn exchange(router: &Router, path: &str, pairs: &[(&str, &str)]) -> (StatusCode, HeaderMap) {
    let mut request = Request::builder().uri(path);
    for (name, value) in pairs {
        request = request.header(*name, *value);
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let response = router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        (response.status(), response.headers().clone())
    })
}

#[test]
fn a_link_pairs_a_browser_once_while_it_is_fresh() {
    let now = 1_792_022_400_000;
    let router = up_router(now);
    let token = Token::parse(TOKEN).unwrap();
    let host = ("host", "127.0.0.1:7478");
    let code = token.code(now - 60_000, [7; 8]);
    let path = format!("/login?code={code}");

    let (status, headers) = exchange(&router, &path, &[("host", "evil.example:7478")]);
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(headers.get(SET_COOKIE).is_none());
    let (status, headers) = exchange(&router, &path, &[host]);
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[LOCATION], "/");
    let cookie = headers[SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("; HttpOnly; SameSite=Strict; Path=/;"), "{cookie}");
    // The cookie holds a credential of its own, never the session.
    assert!(cookie.starts_with(&format!("acyclic-up={}", token.browser())), "{cookie}");
    assert!(!cookie.contains(TOKEN));
    // A link is spent once used.
    assert_eq!(exchange(&router, &path, &[host]).0, StatusCode::FORBIDDEN);
    for stale in [token.code(now - CODE_LIFE - 1, [1; 8]), token.code(now + CODE_SKEW + 1, [2; 8])]
    {
        let path = format!("/login?code={stale}");
        assert_eq!(exchange(&router, &path, &[host]).0, StatusCode::FORBIDDEN, "{stale}");
    }
    for bad in ["/login", "/login?code=", "/login?other=1", "/login?code=1.2.3"] {
        assert_eq!(exchange(&router, bad, &[host]).0, StatusCode::FORBIDDEN, "{bad}");
    }
}

#[test]
fn a_paired_browser_writes_only_from_the_listeners_own_page() {
    let router = up_router(0);
    let browser = format!("acyclic-up={}", Token::parse(TOKEN).unwrap().browser());
    let host = ("host", "localhost:7478");
    let json = ("content-type", "application/json");
    let paired = ("cookie", browser.as_str());
    let post = |pairs: &[(&str, &str)]| {
        let (status, body) = send(&router, Method::POST, "/api/command", pairs, r#""list""#);
        (status, code(&body))
    };
    // The tracker clone is missing, so a request let in fails as the CLI would.
    let ran = |(status, _): (StatusCode, String)| status.is_server_error();
    assert!(ran(post(&[host, json, paired, ("origin", "http://localhost:7478")])));
    let injected = format!("acyclic-up=forged; {browser}");
    assert!(ran(post(&[host, json, ("cookie", &injected), ("origin", "http://localhost:7478")])));
    let same = ("sec-fetch-site", "same-origin");
    assert!(ran(post(&[host, json, paired, ("origin", "http://localhost:7478"), same])));
    // The cookie reaches every port on the machine, so a page on another one, or one that hides its origin, is refused.
    for refused in [
        vec![host, json, paired],
        vec![host, json, paired, ("origin", "http://localhost:3000")],
        vec![host, json, paired, ("origin", "http://127.0.0.1:7478")],
        vec![
            host,
            json,
            paired,
            ("origin", "http://localhost:7478"),
            ("sec-fetch-site", "same-site"),
        ],
    ] {
        assert_eq!(post(&refused), (StatusCode::FORBIDDEN, "wrong-origin".into()), "{refused:?}");
    }
    // A wrong cookie, or a wrong session beside a right cookie, lets nothing in.
    let forged = ("cookie", "acyclic-up=forged");
    let wrong = ("authorization", "Bearer forged");
    for refused in [vec![host, json, forged], vec![host, json, paired, wrong]] {
        assert_eq!(post(&refused), (StatusCode::UNAUTHORIZED, "no-session".into()), "{refused:?}");
    }
    // The session itself needs no origin: no browser sends it.
    let bearer = format!("Bearer {TOKEN}");
    assert!(ran(post(&[host, json, ("authorization", &bearer)])));
}

#[test]
fn a_socket_replaces_only_one_no_server_listens_on() {
    use std::os::unix::fs::PermissionsExt as _;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap();
    let _entered = runtime.enter();
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let long = dir.path().join("s".repeat(SOCKET_PATH_MAX));
    assert!(bind_socket(&long).unwrap_err().message.contains("at most"));
    let file = dir.path().join("file");
    std::fs::write(&file, "kept").unwrap();
    assert!(bind_socket(&file).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "kept");
    let socket = dir.path().join("s.sock");
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    let live = bind_socket(&socket).unwrap();
    assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(bind_socket(&socket).unwrap_err().message.contains("another server"));
    drop(live);
}
