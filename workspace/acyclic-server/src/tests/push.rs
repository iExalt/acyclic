//! Pushing, against a stand-in push service on this machine that decrypts what it is sent as the device would.

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Mutex as StdMutex;

use base64ct::{Base64UrlUnpadded, Encoding as _};
use p256::SecretKey;

use super::*;
use crate::push::crypto::tests::{claims, decrypt};
use crate::push::{Settings, Transport};

/// What the stand-in push service was sent: each message's `Authorization` and body.
#[derive(Default)]
struct Service {
    sent: StdMutex<Vec<(String, Vec<u8>)>>,
    /// What it answers.
    answer: StdMutex<StatusCode>,
}

async fn take(State(service): State<Arc<Service>>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let authorization = headers[AUTHORIZATION].to_str().unwrap().to_owned();
    assert_eq!(headers["content-encoding"], "aes128gcm");
    assert_eq!(headers["ttl"], "86400");
    service.sent.lock().unwrap().push((authorization, body.to_vec()));
    *service.answer.lock().unwrap()
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
        let (code, answer) = acyclic_cli::call(
            serde_json::from_value(command).unwrap(),
            tracker.clone(),
            true,
            &env,
        );
        assert_eq!(code, 0, "{answer}");
    };
    // An agent's edit to an approved thread's scope waits as a proposal: news for the user.
    let propose = |n: u32| {
        run(json!({"item": {"add": {"item": format!("p{n}"), "title": format!("P{n}")}}}));
        let edit = json!({"item": {"edit": {"item": format!("p{n}"), "thread": "t"}}});
        let (code, answer) = acyclic_cli::call_as(
            serde_json::from_value(edit).unwrap(),
            tracker.clone(),
            "session/agent",
            &env,
        );
        assert_eq!(code, 0, "{answer}");
    };
    run(json!({"init": {"path": tracker, "replica": "mac"}}));
    run(json!({"thread": {"add": {"thread": "t", "title": "T"}}}));
    run(json!({"thread": {"approve": {"thread": "t"}}}));
    propose(1);

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
        for _ in 0..500 {
            let sent = service.sent.lock().unwrap().clone();
            if sent.len() >= n {
                assert_eq!(sent.len(), n, "one push per look with news");
                let (authorization, body) = sent.last().unwrap().clone();
                let message: Value =
                    serde_json::from_slice(&decrypt(&body, &device, &auth)).unwrap();
                return (authorization, message);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the service wasn't sent {n} messages");
    };

    let server = Arc::new(Server::new(config.clone()));
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
    assert_eq!(message["body"], "A proposal for P1", "{message}");
    assert_eq!(message["url"], "/?needs");
    let claimed = claims(&authorization, &public);
    assert_eq!(
        (&claimed["aud"], &claimed["sub"]),
        (&json!(audience), &json!("mailto:user@example.com"))
    );

    // A look with nothing new, as after a sync that brought nothing, tells nothing: the next push names only what is
    // new since.
    server.push.look();
    propose(2);
    server.push.look();
    assert_eq!(received(2).await.1["body"], "A proposal for P2");

    // The user's own writes through the app bring news of their own doing, which is taken as seen.
    let user = |command: Value| call(&router, Method::POST, "/api/command", command.to_string());
    assert_eq!(
        user(json!({"thread": {"add": {"thread": "u", "title": "U"}}})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        user(json!({"item": {"add": {"item": "m", "title": "M", "thread": "u"}}})).await.0,
        StatusCode::OK
    );
    let sent = home.join(".config/acyclic/push/sent.json");
    for _ in 0..500 {
        if std::fs::read_to_string(&sent).unwrap().contains("awaiting-approval:u") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    propose(3);
    server.push.look();
    assert_eq!(received(3).await.1["body"], "A proposal for P3");

    // A restart on the same files tells nothing again, and a device the service says is gone is dropped.
    drop(router);
    drop(server);
    let server = Arc::new(Server::new(config.clone()));
    *service.answer.lock().unwrap() = StatusCode::GONE;
    server.push.look();
    propose(4);
    server.push.look();
    assert_eq!(received(4).await.1["body"], "A proposal for P4");
    let devices = home.join(".config/acyclic/push/subscriptions.json");
    for _ in 0..500 {
        if std::fs::read_to_string(&devices).unwrap() == "[]" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(std::fs::read_to_string(&devices).unwrap(), "[]");
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

#[test]
fn a_run_may_not_subscribe_or_read_the_key() {
    use acyclic_operator::token::Signer;
    let signer = Signer::from_seed([3; 32]);
    let access =
        Access::Run { verifier: signer.verifier(), deny: DenyList::parse("term\n"), code: None };
    let now = acyclic_cli::now(&Env::default());
    let bearer = format!("Bearer {}", signer.mint("r1", "t1", now, 600_000).unwrap());
    let router = super::router(config(None), access);
    let pass = [("authorization", bearer.as_str()), ("content-type", "application/json")];
    for (method, path) in [
        (Method::GET, "/api/push/key"),
        (Method::POST, "/api/push/subscribe"),
        (Method::POST, "/api/push/unsubscribe"),
    ] {
        let (status, body) = send(&router, method, path, &pass, "{}");
        assert_eq!((status, code(&body)), (StatusCode::FORBIDDEN, "not-for-runs".into()), "{path}");
    }
}
