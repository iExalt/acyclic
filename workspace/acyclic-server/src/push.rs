//! Web Push: telling the user's devices when something new needs them.
//!
//! The server keeps, under `~/.config/acyclic/push`, its VAPID key, the devices subscribed, and the needs it has
//! already told them of, so a restart or a sync tells them nothing twice. After agents' writes, syncs, and every
//! minute besides (the CLI writes to the clone without the server), it reads the needs-you list and pushes one message
//! naming what is new. What the user's own writes through the app bring about is taken as seen: revoking an approval
//! from the phone shouldn't buzz the phone. Sends are coalesced over a short window, never hold up a write, and back
//! off when the push services fail.

pub mod crypto;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use acyclic_cli::Env;
use acyclic_cli::args::Command;
use axum::body::Bytes;
use axum::http::{Request, StatusCode, Uri};
use base64ct::{Base64UrlUnpadded, Encoding as _};
use http_body_util::Full;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use p256::SecretKey;
use p256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// How pushes leave the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Over HTTPS only, to push services anywhere: the server's.
    #[default]
    Https,
    /// Over plain HTTP to this machine, as well as HTTPS: for tests' stand-in push service.
    LoopbackHttp,
}

/// How the server pushes.
#[derive(Debug, Clone)]
pub struct Settings {
    /// How pushes leave the server.
    pub transport: Transport,
    /// Who push services may contact about the server's pushes, `https://…` or `mailto:…`; some services require one.
    /// The Tailscale origin under `serve`, else `~/.config/acyclic/push/subject` if it exists.
    pub subject: Option<String>,
    /// How long to wait for more news before pushing.
    pub window: Duration,
    /// How often to look for news the server wasn't told of, such as the CLI's writes.
    pub every: Duration,
    /// The first wait before trying again when no device could be reached; it doubles up to [`BACKOFF_MAX`].
    pub backoff: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            transport: Transport::Https,
            subject: None,
            window: WINDOW,
            every: EVERY,
            backoff: BACKOFF,
        }
    }
}

/// How long news is gathered before one push tells of it all.
const WINDOW: Duration = Duration::from_secs(3);
/// How often the server looks for news without being told of any.
const EVERY: Duration = Duration::from_secs(60);
const BACKOFF: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(600);
/// How many devices may subscribe; a new one past this replaces the oldest.
const DEVICES: usize = 10;
/// How many sends in a row a push service may refuse before its device is dropped. Sends that fail on the way, or that
/// the service fails or defers, don't count: being offline for a while must not cost the user their devices.
const FAILURES: u32 = 5;
/// How long a push service keeps a message for a device that is away: a day.
const TTL: u32 = 86_400;
/// The longest endpoint taken.
const ENDPOINT_MAX: usize = 1024;

/// A device's subscription, as the browser's `PushSubscription` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// Where the device's push service takes its messages.
    pub endpoint: String,
    /// The keys its messages are encrypted to.
    pub keys: Keys,
    /// Sends in a row its push service refused.
    #[serde(default)]
    pub failures: u32,
}

/// A subscription's keys, base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    /// The device's P-256 public key.
    pub p256dh: String,
    /// The secret shared with the device.
    pub auth: String,
}

/// Why a subscription or a key couldn't be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

/// What makes the server look at the needs-you list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// The user wrote through the app: what is new now is the user's doing, and taken as seen.
    Seen,
    /// Something else may have changed: push what is new.
    Look,
    /// A device subscribed: a reason to try at once, even while backing off.
    Device,
}

/// The server's pushing: its settings, its files, and the task that pushes, started on first use.
pub struct Push {
    inner: Arc<Inner>,
    trigger: OnceLock<mpsc::UnboundedSender<Trigger>>,
}

struct Inner {
    tracker: PathBuf,
    env: Env,
    settings: Settings,
    /// `~/.config/acyclic/push`, or none without a home.
    dir: Option<PathBuf>,
    /// Held while the files are read and written.
    files: Mutex<()>,
    client: OnceLock<Option<Sender>>,
    /// Each push service's VAPID header, and until when it holds.
    signed: Mutex<BTreeMap<String, (String, u64)>>,
}

/// The HTTP client a [`Transport`] sends with.
enum Sender {
    Https(Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>),
    Http(Client<HttpConnector, Full<Bytes>>),
}

impl Push {
    /// Pushing for the tracker clone at `tracker`, keeping its files under the home directory `env` names.
    #[must_use]
    pub fn new(tracker: PathBuf, env: Env, mut settings: Settings) -> Self {
        let dir = env
            .vars
            .get("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".config/acyclic/push"));
        if settings.subject.is_none() {
            settings.subject = dir
                .as_ref()
                .and_then(|dir| std::fs::read_to_string(dir.join("subject")).ok())
                .map(|subject| subject.trim().to_owned())
                .filter(|subject| !subject.is_empty());
        }
        let inner = Inner {
            tracker,
            env,
            settings,
            dir,
            files: Mutex::default(),
            client: OnceLock::new(),
            signed: Mutex::default(),
        };
        Self { inner: Arc::new(inner), trigger: OnceLock::new() }
    }

    /// The server's VAPID public key, base64url, made on first use.
    ///
    /// # Errors
    ///
    /// The key can't be read or written.
    pub fn public_key(&self) -> Result<String, Refused> {
        let _files =
            self.inner.files.lock().map_err(|_| Refused("the push files are in use".into()))?;
        let key = self.inner.key()?;
        Ok(Base64UrlUnpadded::encode_string(&key.verifying_key().to_sec1_bytes()))
    }

    /// Takes a device's subscription, replacing one with the same endpoint, and looks for news to tell it of.
    ///
    /// # Errors
    ///
    /// The endpoint or the keys are unfit, or the files can't be written.
    pub fn subscribe(&self, mut subscription: Subscription) -> Result<(), Refused> {
        self.inner.check(&subscription)?;
        subscription.failures = 0;
        {
            let _files =
                self.inner.files.lock().map_err(|_| Refused("the push files are in use".into()))?;
            let mut devices = self.inner.devices();
            devices.retain(|one| one.endpoint != subscription.endpoint);
            devices.push(subscription);
            let excess = devices.len().saturating_sub(DEVICES);
            devices.drain(..excess);
            self.inner.save("subscriptions.json", &devices)?;
        }
        self.send(Trigger::Device);
        Ok(())
    }

    /// Forgets a device.
    ///
    /// # Errors
    ///
    /// The files can't be written.
    pub fn unsubscribe(&self, endpoint: &str) -> Result<(), Refused> {
        let _files =
            self.inner.files.lock().map_err(|_| Refused("the push files are in use".into()))?;
        let mut devices = self.inner.devices();
        devices.retain(|one| one.endpoint != endpoint);
        self.inner.save("subscriptions.json", &devices)
    }

    /// The user wrote through the app: what that brings about needs no push.
    pub fn seen(&self) {
        self.send(Trigger::Seen);
    }

    /// Something besides the user may have changed the tracker: push what is new.
    pub fn look(&self) {
        self.send(Trigger::Look);
    }

    fn send(&self, trigger: Trigger) {
        if self.inner.dir.is_none() {
            return;
        }
        let sender = self.trigger.get_or_init(|| {
            let (sender, receiver) = mpsc::unbounded_channel();
            tokio::spawn(work(Arc::clone(&self.inner), receiver, sender.downgrade()));
            sender
        });
        let _ = sender.send(trigger);
    }
}

/// The pushing task: gathers triggers over the window, then reads the needs-you list once and acts on it. It ends
/// once its [`Push`] is gone.
async fn work(
    inner: Arc<Inner>,
    mut triggers: mpsc::UnboundedReceiver<Trigger>,
    again: mpsc::WeakUnboundedSender<Trigger>,
) {
    let mut ticks = tokio::time::interval(inner.settings.every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    let mut backoff = inner.settings.backoff;
    // While backing off, looks wait for the retry, so an unreachable push service isn't sent to more often.
    let mut resume = tokio::time::Instant::now();
    loop {
        let first = tokio::select! {
            trigger = triggers.recv() => match trigger { Some(trigger) => trigger, None => return },
            _ = ticks.tick() => Trigger::Look,
        };
        if first == Trigger::Device {
            resume = tokio::time::Instant::now();
            backoff = inner.settings.backoff;
        }
        if first == Trigger::Look && tokio::time::Instant::now() < resume {
            continue;
        }
        let mut seen = first == Trigger::Seen;
        if !seen {
            tokio::time::sleep(inner.settings.window).await;
        }
        while let Ok(trigger) = triggers.try_recv() {
            seen |= trigger == Trigger::Seen;
        }
        let round = Arc::clone(&inner);
        let Ok(Some(plan)) = tokio::task::spawn_blocking(move || round.plan(seen)).await else {
            continue;
        };
        let reached = inner.deliver(&plan).await;
        let told = reached.values().any(StatusCode::is_success);
        let done = Arc::clone(&inner);
        let _ = tokio::task::spawn_blocking(move || done.settle(&plan, &reached)).await;
        if told {
            backoff = inner.settings.backoff;
        } else {
            // No device took it: the news stays untold, and the next look tries again.
            let again = again.clone();
            let wait = backoff;
            resume = tokio::time::Instant::now() + wait;
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                if let Some(again) = again.upgrade() {
                    let _ = again.send(Trigger::Look);
                }
            });
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }
}

/// What one look found to push, and to whom.
struct Plan {
    devices: Vec<Subscription>,
    /// The keys of everything on the list that notifies, now.
    now: BTreeSet<String>,
    /// The message, if anything new is to be told.
    message: Option<Value>,
}

impl Inner {
    /// Reads the list and decides; with `seen`, takes what is new as seen and plans nothing.
    fn plan(&self, seen: bool) -> Option<Plan> {
        // The files are held only to read and write them, not while the list is read, so a device can subscribe
        // meanwhile.
        let devices = {
            let _files = self.files.lock().ok()?;
            self.devices()
        };
        if devices.is_empty() {
            // With no device, nothing is read, and what is on the list stays untold: the first device to subscribe
            // hears of it.
            return None;
        }
        let (code, envelope) =
            acyclic_cli::call(Command::Needs, self.tracker.clone(), true, &self.env);
        if code != 0 {
            eprintln!("acyclic: the needs-you list couldn't be read for pushing: {envelope}");
            return None;
        }
        let entries: Vec<&Value> = envelope["data"]["needs"]
            .as_array()
            .map(|all| all.iter().filter(|entry| entry["notify"] == true).collect())
            .unwrap_or_default();
        let now: BTreeSet<String> = entries.iter().map(|entry| key(entry)).collect();
        let _files = self.files.lock().ok()?;
        let told: BTreeSet<String> = self.load::<BTreeSet<String>>("sent.json").unwrap_or_default();
        if seen {
            let _ = self.save("sent.json", &now);
            return None;
        }
        let new: Vec<&&Value> =
            entries.iter().filter(|entry| !told.contains(&key(entry))).collect();
        let message = new.first().map(|first| {
            let more = new.len() - 1;
            let body =
                if more == 0 { line(first) } else { format!("{} (and {more} more)", line(first)) };
            json!({"title": "Needs you", "body": body, "url": "/?needs", "count": now.len()})
        });
        if message.is_none() {
            // Nothing new; what left the list is forgotten, so it is told again if it comes back.
            let kept: BTreeSet<String> = told.intersection(&now).cloned().collect();
            if kept != told {
                let _ = self.save("sent.json", &kept);
            }
            return None;
        }
        Some(Plan { devices, now, message })
    }

    /// Sends the plan's message to each device, giving each endpoint's HTTP status, or none when it couldn't be sent.
    async fn deliver(&self, plan: &Plan) -> BTreeMap<String, StatusCode> {
        let mut reached = BTreeMap::new();
        let Some(message) = &plan.message else { return reached };
        let text = message.to_string();
        for device in &plan.devices {
            match self.post(device, text.as_bytes()).await {
                Ok(status) => {
                    reached.insert(device.endpoint.clone(), status);
                },
                Err(err) => eprintln!("acyclic: a push to a device failed: {err}"),
            }
        }
        reached
    }

    /// Records what the sends came to: the news told, and the devices that are gone, or whose service keeps refusing
    /// them, dropped.
    fn settle(&self, plan: &Plan, reached: &BTreeMap<String, StatusCode>) {
        let Ok(_files) = self.files.lock() else { return };
        let mut devices = self.devices();
        for device in &mut devices {
            match reached.get(&device.endpoint) {
                Some(status) if status.is_success() => device.failures = 0,
                Some(&StatusCode::NOT_FOUND | &StatusCode::GONE) => device.failures = FAILURES,
                // Refused as malformed, unauthorised or too large: likely to stay so.
                Some(status)
                    if status.is_client_error() && *status != StatusCode::TOO_MANY_REQUESTS =>
                {
                    device.failures += 1;
                },
                // Not sent, failed or deferred by the service: tried again later, and never counted.
                _ => {},
            }
        }
        devices.retain(|device| device.failures < FAILURES);
        let _ = self.save("subscriptions.json", &devices);
        if reached.values().any(StatusCode::is_success) {
            let _ = self.save("sent.json", &plan.now);
        }
    }

    async fn post(&self, device: &Subscription, plain: &[u8]) -> Result<StatusCode, String> {
        let public =
            Base64UrlUnpadded::decode_vec(&device.keys.p256dh).map_err(|_| "a malformed key")?;
        let auth =
            Base64UrlUnpadded::decode_vec(&device.keys.auth).map_err(|_| "a malformed secret")?;
        let sender = fresh_key();
        let mut salt = [0_u8; 16];
        getrandom::fill(&mut salt).map_err(|err| err.to_string())?;
        let body = crypto::encrypt(
            plain,
            &crypto::Receiver { public: &public, auth: &auth },
            &sender,
            &salt,
        )
        .map_err(|err| err.0.to_owned())?;
        let uri: Uri = device.endpoint.parse().map_err(|_| "a malformed endpoint")?;
        let audience = format!(
            "{}://{}",
            uri.scheme_str().unwrap_or("https"),
            uri.authority().map(axum::http::uri::Authority::as_str).unwrap_or_default()
        );
        let authorization = {
            let _files = self.files.lock().map_err(|_| "the push files are in use")?;
            self.signed_for(&audience).map_err(|err| err.0)?
        };
        let request = Request::post(uri)
            .header("TTL", TTL)
            .header("Urgency", "normal")
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", authorization)
            .body(Full::new(Bytes::from(body)))
            .map_err(|err| err.to_string())?;
        let response = match self.client() {
            Some(Sender::Https(client)) => {
                tokio::time::timeout(Duration::from_secs(15), client.request(request)).await
            },
            Some(Sender::Http(client)) => {
                tokio::time::timeout(Duration::from_secs(15), client.request(request)).await
            },
            None => return Err("no trusted certificate authorities could be loaded".into()),
        };
        match response {
            Ok(Ok(response)) => Ok(response.status()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err("the push service didn't answer within 15 s".into()),
        }
    }

    fn client(&self) -> Option<&Sender> {
        self.client
            .get_or_init(|| match self.settings.transport {
                Transport::Https => https().map(Sender::Https),
                Transport::LoopbackHttp => Some(Sender::Http(
                    Client::builder(TokioExecutor::new()).build(HttpConnector::new()),
                )),
            })
            .as_ref()
    }

    /// The VAPID header for a push service, signed afresh when the last one is within an hour of expiring.
    fn signed_for(&self, audience: &str) -> Result<String, Refused> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let mut signed =
            self.signed.lock().map_err(|_| Refused("the VAPID cache is in use".into()))?;
        if let Some((header, until)) = signed.get(audience)
            && *until > now + 3600
        {
            return Ok(header.clone());
        }
        let until = now + 12 * 3600;
        let header = crypto::vapid(&self.key()?, audience, self.settings.subject.as_deref(), until);
        signed.insert(audience.to_owned(), (header.clone(), until));
        Ok(header)
    }

    /// Refuses an endpoint the transport can't send to, or keys that aren't a P-256 point and a 16-byte secret.
    fn check(&self, subscription: &Subscription) -> Result<(), Refused> {
        check_endpoint(&subscription.endpoint, self.settings.transport)?;
        let public = Base64UrlUnpadded::decode_vec(&subscription.keys.p256dh)
            .map_err(|_| Refused("the subscription's p256dh key isn't base64url".into()))?;
        p256::PublicKey::from_sec1_bytes(&public)
            .map_err(|_| Refused("the subscription's p256dh key isn't a P-256 point".into()))?;
        let auth = Base64UrlUnpadded::decode_vec(&subscription.keys.auth)
            .map_err(|_| Refused("the subscription's auth secret isn't base64url".into()))?;
        if auth.len() != 16 {
            return Err(Refused("the subscription's auth secret isn't 16 bytes".into()));
        }
        Ok(())
    }

    /// The server's VAPID key, made and saved on first use. The caller holds [`Inner::files`].
    fn key(&self) -> Result<SigningKey, Refused> {
        if let Some(saved) = self.load::<String>("vapid.key") {
            let bytes = Base64UrlUnpadded::decode_vec(&saved)
                .map_err(|_| Refused("vapid.key is garbled".into()))?;
            return SigningKey::from_slice(&bytes)
                .map_err(|_| Refused("vapid.key is garbled".into()));
        }
        let key = fresh_key();
        self.save("vapid.key", &Base64UrlUnpadded::encode_string(&key.to_bytes()))?;
        SigningKey::from_slice(&key.to_bytes())
            .map_err(|_| Refused("the new VAPID key is unusable".into()))
    }

    fn devices(&self) -> Vec<Subscription> {
        self.load("subscriptions.json").unwrap_or_default()
    }

    fn load<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        let text = std::fs::read_to_string(self.dir.as_ref()?.join(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Writes a file of the user's alone, whole or not at all.
    fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<(), Refused> {
        use std::io::Write as _;
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
        let dir = self.dir.as_ref().ok_or_else(|| {
            Refused("HOME isn't set, so push has nowhere to keep its files".into())
        })?;
        let failed = |err: std::io::Error| Refused(format!("{}: {err}", dir.join(name).display()));
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(failed)?;
        let temporary = dir.join(format!(".{name}.new"));
        let text = serde_json::to_string(value).map_err(|err| Refused(err.to_string()))?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(failed)?;
        file.write_all(text.as_bytes()).and_then(|()| file.sync_all()).map_err(failed)?;
        std::fs::rename(&temporary, dir.join(name)).map_err(failed)
    }
}

/// A fresh P-256 key from the system's random numbers.
fn fresh_key() -> SecretKey {
    loop {
        let mut bytes = [0_u8; 32];
        if getrandom::fill(&mut bytes).is_ok()
            && let Ok(key) = SecretKey::from_slice(&bytes)
        {
            return key;
        }
    }
}

/// An HTTPS client trusting the system's certificate authorities, or none, said once, when there are none.
fn https() -> Option<Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>> {
    use hyper_rustls::ConfigBuilderExt as _;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .with_native_roots();
    let tls = match tls {
        Ok(tls) => tls.with_no_client_auth(),
        Err(err) => {
            eprintln!(
                "acyclic: no trusted certificate authorities, so pushes can't be sent: {err}"
            );
            return None;
        },
    };
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_only()
        .enable_http1()
        .build();
    Some(Client::builder(TokioExecutor::new()).build(https))
}

/// Refuses an endpoint `transport` can't send to: anything but `https://` with a host, or with
/// [`Transport::LoopbackHttp`] `http://` to this machine too, or longer than [`ENDPOINT_MAX`].
///
/// # Errors
///
/// Why the endpoint is refused.
pub fn check_endpoint(endpoint: &str, transport: Transport) -> Result<(), Refused> {
    if endpoint.len() > ENDPOINT_MAX {
        return Err(Refused(format!("the endpoint is longer than {ENDPOINT_MAX} bytes")));
    }
    let uri: Uri = endpoint.parse().map_err(|_| Refused("the endpoint isn't a URL".into()))?;
    let host = uri.host().unwrap_or_default();
    match (uri.scheme_str(), transport) {
        (Some("https"), _) if !host.is_empty() => Ok(()),
        (Some("http"), Transport::LoopbackHttp) if matches!(host, "127.0.0.1" | "localhost") => {
            Ok(())
        },
        _ => Err(Refused("the endpoint must be an https:// address".into())),
    }
}

/// A need's stable key: its kind and what it names, so the same need on the next look is known as told.
#[must_use]
pub fn key(entry: &Value) -> String {
    let mut parts = vec![entry["kind"].as_str().unwrap_or_default().to_owned()];
    for field in ["id", "entity", "field", "resource"] {
        if let Some(text) = entry[field].as_str() {
            parts.push(text.to_owned());
        }
    }
    for field in ["item", "thread", "blocker"] {
        if let Some(id) = entry[field]["id"].as_str() {
            parts.push(id.to_owned());
        }
    }
    if let Some(id) = entry["proposal"]["id"].as_str() {
        parts.push(id.to_owned());
    }
    for row in entry["items"].as_array().into_iter().flatten() {
        if let Some(id) = row["id"].as_str() {
            parts.push(id.to_owned());
        }
    }
    parts.join(":")
}

/// A need in a line, for a notification.
#[must_use]
pub fn line(entry: &Value) -> String {
    let title = |row: &Value| {
        row["title"].as_str().or_else(|| row["id"].as_str()).unwrap_or("?").to_owned()
    };
    let text = match entry["kind"].as_str().unwrap_or_default() {
        "question" => format!("An agent asks: {}", entry["text"].as_str().unwrap_or_default()),
        "proposal" => format!("A proposal for {}", title(&entry["item"])),
        "unapproved" => format!(
            "{} joined {} after its approval",
            title(&entry["item"]),
            title(&entry["thread"])
        ),
        "awaiting-approval" => format!("{} waits for your approval", title(&entry["thread"])),
        "conflict" => format!(
            "{} {}'s {} was written differently on two machines",
            entry["entity"].as_str().unwrap_or_default(),
            entry["id"].as_str().unwrap_or_default(),
            entry["field"].as_str().unwrap_or_default()
        ),
        "new-blocker" => format!("{} has a new blocker", title(&entry["item"])),
        "reopened" => format!("{} was reopened", title(&entry["item"])),
        "done-early" => format!("{} is done with work in it open", title(&entry["item"])),
        "contest" => {
            format!("{} has a claim to sort out", title(&entry["thread"]))
        },
        "resource-clash" => {
            format!("Two threads need {}", entry["resource"].as_str().unwrap_or_default())
        },
        "link-cycle" | "parent-cycle" => "Items form a cycle".to_owned(),
        other => format!("{other} needs you"),
    };
    // Well within a push's 4 KB, however long a question.
    text.chars().take(140).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Transport, check_endpoint, key, line};

    #[test]
    fn an_endpoint_must_be_https_but_a_tests_transport_may_reach_this_machine() {
        assert!(check_endpoint("https://push.example/send/abc", Transport::Https).is_ok());
        for refused in ["http://push.example/x", "ftp://push.example/x", "https:///x", "not a url"]
        {
            assert!(check_endpoint(refused, Transport::Https).is_err(), "{refused}");
        }
        assert!(check_endpoint("http://127.0.0.1:9/x", Transport::Https).is_err());
        assert!(check_endpoint("http://127.0.0.1:9/x", Transport::LoopbackHttp).is_ok());
        assert!(check_endpoint("http://push.example/x", Transport::LoopbackHttp).is_err());
        assert!(
            check_endpoint(&format!("https://push.example/{}", "a".repeat(1100)), Transport::Https)
                .is_err()
        );
    }

    #[test]
    fn a_need_is_known_by_its_kind_and_what_it_names() {
        let question = |id: &str| json!({"kind": "question", "id": id, "thread": {"id": "t"}, "text": "Which?"});
        assert_eq!(key(&question("q1")), "question:q1:t");
        assert_ne!(key(&question("q1")), key(&question("q2")));
        let conflict = json!({"kind": "conflict", "entity": "item", "id": "a", "field": "title"});
        assert_eq!(key(&conflict), "conflict:a:item:title");
        assert_eq!(line(&question("q1")), "An agent asks: Which?");
        let long = json!({"kind": "question", "id": "q", "text": "x".repeat(500)});
        assert_eq!(line(&long).chars().count(), 140);
    }
}
