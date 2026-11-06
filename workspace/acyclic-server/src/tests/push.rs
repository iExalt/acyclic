//! Pushing, against a stand-in push service on this machine that decrypts what it is sent as the device would.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Mutex as StdMutex;

use axum::http::header::AUTHORIZATION;
use base64ct::{Base64UrlUnpadded, Encoding as _};
use p256::SecretKey;

use super::*;
use crate::push::crypto::tests::{claims, decrypt};
use crate::push::{Settings, Transport};

/// What the stand-in push service was sent: each message's `Authorization` and body, and what it answered.
#[derive(Default)]
struct Service {
    sent: StdMutex<Vec<(String, Vec<u8>, StatusCode)>>,
    /// What it answers.
    answer: StdMutex<StatusCode>,
}

async fn take(State(service): State<Arc<Service>>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let authorization = headers[AUTHORIZATION].to_str().unwrap().to_owned();
    assert_eq!(headers["content-encoding"], "aes128gcm");
    assert_eq!(headers["ttl"], "86400");
    let answer = *service.answer.lock().unwrap();
    service.sent.lock().unwrap().push((authorization, body.to_vec(), answer));
    answer
}

/// Waits up to 10 seconds for `done` to give something, and gives it.
async fn eventually<T>(what: &str, mut done: impl FnMut() -> Option<T>) -> T {
    for _ in 0..1000 {
        if let Some(found) = done() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} didn't happen within 10 s");
}

#[tokio::test(flavor = "multi_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one journey through pushing, each step building on the last"
)]
async fn new_needs_are_pushed_once_to_each_device_and_a_gone_device_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let tracker = dir.path().join("tracker");
    let home = dir.path().join("home");
    let env =
        Env { vars: [("HOME".to_owned(), home.display().to_string())].into(), ..Env::default() };
    let run = |command: Value| {
        let (code, answer) =
            acyclic_cli::call(serde_json::from_value(command).unwrap(), tracker.clone(), &env);
        assert_eq!(code, 0, "{answer}");
    };
    // An agent reopening finished work that done work waits for: news for the user.
    let reopen = |n: u32| {
        let id = format!("01900000-0000-7000-8000-{n:012x}");
        let waiter = format!("01900000-0000-7000-8000-{:012x}", 0x100 + n);
        run(json!({"item": {"add": {"id": id, "title": format!("P{n}")}}}));
        run(json!({"item": {"add": {"id": waiter, "title": format!("W{n}"), "waits-for": [id]}}}));
        for item in [&id, &waiter] {
            run(json!({"item": {"done": {"item": item, "evidence": "n", "force": true}}}));
        }
        let edit = json!({"item": {"reopen": {"item": id}}});
        let (code, answer) = acyclic_cli::call_as(
            serde_json::from_value(edit).unwrap(),
            tracker.clone(),
            "session/agent",
            &env,
        );
        assert_eq!(code, 0, "{answer}");
    };
    run(json!({"init": {"path": tracker, "replica": "mac"}}));
    // An item needs a parent, and goes under the tracker's only initiative.
    run(json!({"initiative": {"add": {"title": "Default"}}}));
    reopen(1);

    let service =
        Arc::new(Service { answer: StdMutex::new(StatusCode::CREATED), ..Service::default() });
    let stub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let audience = format!("http://{}", stub.local_addr().unwrap());
    let app = Router::new().route("/push/{device}", post(take)).with_state(Arc::clone(&service));
    tokio::spawn(async move { axum::serve(stub, app).await });

    let settings = Settings {
        transport: Transport::LoopbackHttp,
        subject: Some("mailto:user@example.com".into()),
        window: Duration::from_millis(20),
        backoff: Duration::from_millis(5),
        ..Settings::default()
    };
    let config =
        Config { tracker: tracker.clone(), env: env.clone(), static_dir: None, push: settings };
    let pass = [("host", HOST_NAME), (IDENTITY, LOGIN), ("content-type", "application/json")];
    let call = async |router: &Router, method: Method, path: &str, body: String| {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in pass {
            request = request.header(name, value);
        }
        let response =
            router.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice::<Value>(&body).unwrap())
    };
    // Waits for the service to have been sent `n` messages in all, and gives the last decrypted.
    let device = SecretKey::from_slice(&[7; 32]).unwrap();
    let auth = [9_u8; 16];
    let received = async |n: usize| {
        let sent = eventually(&format!("message {n}"), || {
            let sent = service.sent.lock().unwrap().clone();
            (sent.len() >= n).then_some(sent)
        })
        .await;
        assert_eq!(sent.len(), n, "one push per look with news");
        let (authorization, body, _) = sent.last().unwrap().clone();
        let message: Value = serde_json::from_slice(&decrypt(&body, &device, &auth)).unwrap();
        (authorization, message)
    };

    let server = Arc::new(Server::new(config.clone()));
    server.push.prepare().unwrap();
    let router = routes(Arc::clone(&server), tailscale());
    let (status, key) = call(&router, Method::GET, "/api/push/key", String::new()).await;
    assert_eq!(status, StatusCode::OK, "{key}");
    let public = Base64UrlUnpadded::decode_vec(key["data"]["key"].as_str().unwrap()).unwrap();
    let subscription = json!({
        "endpoint": format!("{audience}/push/1"),
        "keys": {
            "p256dh": Base64UrlUnpadded::encode_string(&device.public_key().to_sec1_bytes()),
            "auth": Base64UrlUnpadded::encode_string(&auth),
        },
    });
    let (status, answer) =
        call(&router, Method::POST, "/api/push/subscribe", subscription.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // Subscribing tells the device what already needs the user, signed for the service by the server's key.
    let (authorization, message) = received(1).await;
    assert_eq!(message["body"], "W1 is done but waits for P1", "{message}");
    assert_eq!(message["url"], "/?needs");
    let claimed = claims(&authorization, &public);
    assert_eq!(
        (&claimed["aud"], &claimed["sub"]),
        (&json!(audience), &json!("mailto:user@example.com"))
    );

    // A look with nothing new, as after a sync that brought nothing, tells nothing: the next push names only what is
    // new since.
    server.push.look();
    reopen(2);
    server.push.look();
    assert_eq!(received(2).await.1["body"], "W2 is done but waits for P2");

    // The client sends exact identities from the Needs response it last showed.
    let observed = {
        let (code, envelope) = acyclic_cli::call(Command::Needs, tracker.clone(), &env);
        assert_eq!(code, 0, "{envelope}");
        envelope["data"]["needs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["notify"] == true)
            .map(|entry| entry["identity"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // This unrelated need arrives after the client's read, so the app write must not acknowledge it.
    reopen(3);
    let user = |command: Value, observed: Value| {
        call(
            &router,
            Method::POST,
            "/api/command",
            json!({"command": command, "expect": [], "observed": observed}).to_string(),
        )
    };
    for (index, command) in [
        json!({"item": {"add": {"id": "01900000-0000-7000-8000-0000000000c1", "title": "M"}}}),
        json!({"item": {"add": {"id": "01900000-0000-7000-8000-0000000000c2", "title": "N", "waits-for": ["01900000-0000-7000-8000-0000000000c1"]}}}),
        json!({"item": {"done": {"item": "01900000-0000-7000-8000-0000000000c1", "evidence": "n", "force": true}}}),
        json!({"item": {"done": {"item": "01900000-0000-7000-8000-0000000000c2", "evidence": "n", "force": true}}}),
        json!({"item": {"reopen": {"item": "01900000-0000-7000-8000-0000000000c1"}}}),
    ]
    .into_iter()
    .enumerate()
    {
        // The final write still commits and acknowledges its new need when optional client metadata is malformed.
        let observed = if index == 4 { json!("not a list") } else { json!(observed.clone()) };
        assert_eq!(user(command, observed).await.0, StatusCode::OK);
    }
    assert_eq!(received(3).await.1["body"], "W3 is done but waits for P3");
    let sent = home.join(".config/acyclic/push/sent.json");
    let read = |file: &std::path::Path| std::fs::read_to_string(file).unwrap();
    let identities = || serde_json::from_str::<BTreeSet<String>>(&read(&sent)).unwrap();
    let own_reopen = acyclic_core::needs::Entry::DoneWaiting {
        item: "01900000-0000-7000-8000-0000000000c2".into(),
        waits_for: "01900000-0000-7000-8000-0000000000c1".into(),
    }
    .identity();
    eventually("persisting the app's exact own-need acknowledgment", || {
        identities().contains(&own_reopen).then_some(())
    })
    .await;
    assert_eq!(service.sent.lock().unwrap().len(), 3, "the app's own need adds no push");

    // However long the service is down, the device stays, and the news arrives once it is back.
    let devices = home.join(".config/acyclic/push/subscriptions.json");
    *service.answer.lock().unwrap() = StatusCode::SERVICE_UNAVAILABLE;
    reopen(4);
    server.push.look();
    eventually("six refused sends", || (service.sent.lock().unwrap().len() >= 3 + 6).then_some(()))
        .await;
    assert!(read(&devices).contains(&audience));
    *service.answer.lock().unwrap() = StatusCode::CREATED;
    let delivered = eventually("a send once the service is back", || {
        match service.sent.lock().unwrap().last().cloned() {
            Some((_, body, StatusCode::CREATED)) => Some(body),
            _ => None,
        }
    })
    .await;
    let message: Value = serde_json::from_slice(&decrypt(&delivered, &device, &auth)).unwrap();
    assert_eq!(message["body"], "W4 is done but waits for P4");
    // The service has it, but the server records it as told only once it hears back: a restart before then would
    // tell it again.
    let p4 = acyclic_core::needs::Entry::DoneWaiting {
        item: "01900000-0000-7000-8000-000000000104".into(),
        waits_for: "01900000-0000-7000-8000-000000000004".into(),
    }
    .identity();
    eventually("recording P4 as told", || identities().contains(&p4).then_some(())).await;
    let told = service.sent.lock().unwrap().len();

    // A restart on the same files tells nothing again, and a device the service says is gone is dropped.
    drop(router);
    drop(server);
    let server = Arc::new(Server::new(config.clone()));
    *service.answer.lock().unwrap() = StatusCode::GONE;
    server.push.look();
    reopen(5);
    server.push.look();
    assert_eq!(received(told + 1).await.1["body"], "W5 is done but waits for P5");
    eventually("dropping the gone device", || (read(&devices) == "[]").then_some(())).await;
    for file in ["vapid.key", "subscriptions.json", "sent.json"] {
        let mode = std::fs::metadata(home.join(".config/acyclic/push").join(file))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{file}");
    }

    // The server's own transport sends only over HTTPS, so it refuses this service.
    let router = super::router(Config { push: Settings::default(), ..config }, tailscale());
    let (status, answer) =
        call(&router, Method::POST, "/api/push/subscribe", subscription.to_string()).await;
    assert_eq!(
        (status, &answer["error"]["code"]),
        (StatusCode::BAD_REQUEST, &json!("push")),
        "{answer}"
    );
}

#[tokio::test]
async fn getting_the_push_key_makes_none_but_preparing_does() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let env =
        Env { vars: [("HOME".to_owned(), home.display().to_string())].into(), ..Env::default() };
    let config = || Config {
        tracker: dir.path().join("tracker"),
        env: env.clone(),
        static_dir: None,
        push: Settings::default(),
    };
    let get = async |server: &Arc<Server>| {
        let request = Request::builder()
            .uri("/api/push/key")
            .header("host", HOST_NAME)
            .header(IDENTITY, LOGIN)
            .body(Body::empty())
            .unwrap();
        let response = routes(Arc::clone(server), tailscale()).oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice::<Value>(&body).unwrap())
    };

    let server = Arc::new(Server::new(config()));
    let (status, answer) = get(&server).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert!(!home.join(".config/acyclic/push/vapid.key").exists(), "a GET made the key");

    server.push.prepare().unwrap();
    let (status, first) = get(&server).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(get(&server).await.1, first);
    assert_eq!(get(&Arc::new(Server::new(config()))).await.1, first);
}
