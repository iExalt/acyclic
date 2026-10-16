use axum::body::{Body, to_bytes};
use axum::http::Method;
use tower::ServiceExt as _;

use serde_json::json;

use super::*;

const LOGIN: &str = "user@example.com";
const HOST_NAME: &str = "acyclic.example.ts.net";

fn config(static_dir: Option<PathBuf>) -> Config {
    Config {
        login: LOGIN.into(),
        host: HOST_NAME.into(),
        origin: format!("https://{HOST_NAME}"),
        tracker: PathBuf::from("/nonexistent/tracker"),
        env: Env::default(),
        static_dir,
    }
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
    let config = config(None);
    let check = |pairs: &[(&str, &str)]| identity(&headers(pairs), &config);
    let user = (IDENTITY, LOGIN);
    let host = ("host", HOST_NAME);
    assert_eq!(check(&[user, host]), Ok(()));
    assert_eq!(check(&[user, ("host", "ACYCLIC.example.ts.net")]), Ok(()));
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
    let ported = Config { host: format!("{HOST_NAME}:8080"), ..config.clone() };
    assert_eq!(
        identity(&headers(&[user, ("host", "acyclic.example.ts.net:8080")]), &ported),
        Ok(())
    );
    assert_eq!(identity(&headers(&[user, host]), &ported), Err(Refusal::WrongHost));
}

#[test]
fn commands_come_as_json_from_no_other_origin() {
    let config = config(None);
    let check = |pairs: &[(&str, &str)]| same_origin(&headers(pairs), &config);
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
fn every_route_is_behind_the_identity_check() {
    let dir = tempfile::tempdir().unwrap();
    let www = dir.path().join("www");
    std::fs::create_dir(&www).unwrap();
    std::fs::write(www.join("index.html"), "the app").unwrap();
    std::fs::write(dir.path().join("secret"), "outside").unwrap();
    let router = router(config(Some(www)));
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
}

#[test]
fn the_server_runs_only_the_commands_it_allows() {
    let router = router(config(None));
    let pass = [("host", HOST_NAME), (IDENTITY, LOGIN), ("content-type", "application/json")];
    let post = |pairs: &[(&str, &str)], body: &str| {
        let (status, body) = send(&router, Method::POST, "/api/command", pairs, body);
        (status, code(&body))
    };
    // A read passes every check and runs; the tracker clone is missing, so it fails as the CLI's would.
    let (status, code) = post(&pass, r#"{"show": {"items": ["a"]}}"#);
    assert!(status.is_server_error() && !code.is_empty(), "{status} {code}");
    // So does a write.
    let (status, code) = post(&pass, r#"{"item": {"add": {"id": "a", "title": "A"}}}"#);
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
    ] {
        assert_eq!(classify(read.clone()), Use::Read, "{read}");
    }
    for write in [
        json!({"claim": {"id": "a"}}),
        json!({"release": {"id": "a"}}),
        json!({"done": {"id": "a", "note": "n"}}),
        json!({"level": {"level": "blockers"}}),
        json!({"item": {"add": {"id": "a", "title": "A"}}}),
        json!({"item": {"archive": {"id": "a"}}}),
        json!({"link": {"add": {"from": "a", "kind": "blocks", "to": "b"}}}),
        json!({"thread": {"approve": {"thread": "t"}}}),
        json!({"repo": {"attach": {"repository": "r", "item": "a"}}}),
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
        json!({"graph": {"output": "/tmp/graph.dot"}}),
    ] {
        assert_eq!(classify(refused.clone()), Use::Refused, "{refused}");
    }
}

#[test]
fn a_write_that_cant_get_its_turn_is_told_the_server_is_busy() {
    let server = Arc::new(Server { turn: Duration::from_millis(50), ..Server::new(config(None)) });
    let router = routes(server.clone());
    let pass = [("host", HOST_NAME), (IDENTITY, LOGIN), ("content-type", "application/json")];
    // A sync holds the turn throughout.
    let _sync = server.turns.clone().try_lock_owned().unwrap();
    let (status, body) = send(
        &router,
        Method::POST,
        "/api/command",
        &pass,
        r#"{"item": {"add": {"id": "a", "title": "A"}}}"#,
    );
    assert_eq!((status, code(&body)), (StatusCode::SERVICE_UNAVAILABLE, "busy".into()));
    // Reads don't wait for it.
    let (status, body) = send(&router, Method::POST, "/api/command", &pass, r#""list""#);
    assert_ne!(code(&body), "busy", "{status}");
}
