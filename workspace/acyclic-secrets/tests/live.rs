//! Hands real secrets to a real container. Ignored by default: run it with `mise run check:secrets-live`, with
//! `ACYCLIC_SECRETS_FILE` naming a secrets file holding `INFERENCE_KEY` and `INFERENCE_ENDPOINT`, a running Podman
//! machine, and SOPS able to find the age identity. It prints hashes and lengths, never values.

use std::path::PathBuf;
use std::process::Command;

use acyclic_secrets::podman::{Grant, RunSecrets};
use acyclic_secrets::{SystemRunner, sops};
use sha2::{Digest, Sha256};

const IMAGE: &str = "docker.io/curlimages/curl:8.17.0";
const GRANTS: [Grant; 2] = [
    Grant { name: "INFERENCE_KEY", target: "ANTHROPIC_AUTH_TOKEN" },
    Grant { name: "INFERENCE_ENDPOINT", target: "ANTHROPIC_BASE_URL" },
];

fn podman(args: &[&str]) -> String {
    let output = Command::new("podman").args(args).output().expect("podman runs");
    assert!(
        output.status.success(),
        "podman {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("podman prints UTF-8")
}

/// Removes the run's container and secrets however the test ends, so a failure never leaves real values behind.
struct Cleanup {
    run: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let quiet = |args: &[&str]| Command::new("podman").args(args).output().ok();
        quiet(&["rm", "--force", "--ignore", &format!("acyclic-{}", self.run)]);
        let label = format!("label=acyclic.run={}", self.run);
        if let Some(listed) = quiet(&["secret", "ls", "--filter", &label, "--quiet"]) {
            for id in String::from_utf8_lossy(&listed.stdout).split_whitespace() {
                quiet(&["secret", "rm", id]);
            }
        }
    }
}

fn fingerprint(value: &str) -> String {
    let hex: String =
        Sha256::digest(value.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex} {}", value.len())
}

#[test]
#[ignore = "needs a Podman machine and the secrets file; run with mise run check:secrets-live"]
fn a_container_sees_each_value_exactly_and_inspect_shows_none() {
    let file =
        PathBuf::from(std::env::var("ACYCLIC_SECRETS_FILE").expect("ACYCLIC_SECRETS_FILE is set"));
    let secrets = sops::decrypt(&SystemRunner, &file).expect("the secrets file decrypts");
    let run = format!("live-{}", std::process::id());
    let _cleanup = Cleanup { run: run.clone() };
    let mut created =
        RunSecrets::create(&SystemRunner, &run, &secrets, &GRANTS).expect("secrets are created");

    let mut create = vec!["create".to_owned(), "--name".into(), format!("acyclic-{run}")];
    create.extend(created.run_args());
    create.extend([IMAGE.into(), "sh".into(), "-c".into()]);
    // Each variable's SHA-256 and byte length, in the same form as `fingerprint`.
    create.push(
        r#"for v in "$ANTHROPIC_AUTH_TOKEN" "$ANTHROPIC_BASE_URL"; do
             printf '%s %s\n' "$(printf %s "$v" | sha256sum | cut -d' ' -f1)" "$(printf %s "$v" | wc -c | tr -d ' ')"
           done"#
            .into(),
    );
    let container = podman(&create.iter().map(String::as_str).collect::<Vec<_>>());
    let container = container.trim();

    let mut exposed = vec![podman(&["inspect", container])];
    for (secret, _) in created.names() {
        exposed.push(podman(&["secret", "inspect", secret]));
    }
    let seen = podman(&["start", "--attach", container]);
    exposed.push(podman(&["inspect", container]));
    podman(&["rm", container]);
    created.remove(&SystemRunner).expect("secrets are removed");

    for grant in GRANTS {
        let value = secrets.get(grant.name).expect("the file has the value");
        for text in &exposed {
            assert!(!text.contains(value), "{} appears in inspect output", grant.name);
        }
    }
    let expected: Vec<_> =
        GRANTS.iter().map(|g| fingerprint(secrets.get(g.name).unwrap())).collect();
    assert_eq!(seen.lines().collect::<Vec<_>>(), expected);
    let left =
        podman(&["secret", "ls", "--filter", &format!("label=acyclic.run={run}"), "--quiet"]);
    assert_eq!(left.trim(), "", "the run's secrets are gone");
}
