//! Podman secrets that carry one run's credentials into its container.

use crate::{Error, Runner, Secrets};

/// The label marking a secret with the run it belongs to, so secrets a crash left behind can be found.
pub const RUN_LABEL: &str = "acyclic.run";

/// One value to hand a run's container: the name in the secrets file and the environment variable it becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant<'a> {
    /// The value's name in the secrets file.
    pub name: &'a str,
    /// The environment variable it appears as in the container.
    pub target: &'a str,
}

/// The Podman secrets created for one run, named `acyclic-<run>-<target>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSecrets {
    run: String,
    created: Vec<(String, String)>,
}

impl RunSecrets {
    /// Creates a Podman secret for each grant, writing its value on standard input.
    ///
    /// Every value is looked up before any secret is created, so a missing name creates nothing. If Podman fails
    /// partway, the secrets already created are removed before the error returns.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] for a name the file lacks, or the error from running Podman.
    pub fn create(
        runner: &impl Runner,
        run: &str,
        secrets: &Secrets,
        grants: &[Grant],
    ) -> Result<Self, Error> {
        let values = grants
            .iter()
            .map(|grant| {
                secrets.get(grant.name).ok_or_else(|| Error::Missing { name: grant.name.into() })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut this = Self { run: run.into(), created: Vec::new() };
        let label = format!("{RUN_LABEL}={run}");
        for (grant, value) in grants.iter().zip(values) {
            let secret = format!("acyclic-{run}-{}", grant.target.to_ascii_lowercase());
            let created = podman(
                runner,
                &["secret", "create", "--label", &label, &secret, "-"],
                Some(value.as_bytes()),
            );
            if let Err(error) = created {
                // The original error matters more than a failure to clean up after it.
                let _ = this.remove(runner);
                return Err(error);
            }
            this.created.push((secret, grant.target.into()));
        }
        Ok(this)
    }

    /// The `--secret` arguments for `podman run` or `podman create`, one per grant.
    #[must_use]
    pub fn run_args(&self) -> Vec<String> {
        self.created
            .iter()
            .flat_map(|(secret, target)| {
                ["--secret".into(), format!("{secret},type=env,target={target}")]
            })
            .collect()
    }

    /// The Podman secrets created, each with the environment variable it becomes.
    pub fn names(&self) -> impl Iterator<Item = (&str, &str)> {
        self.created.iter().map(|(secret, target)| (secret.as_str(), target.as_str()))
    }

    /// The run these secrets belong to.
    #[must_use]
    pub fn run(&self) -> &str {
        &self.run
    }

    /// Removes the run's secrets from Podman.
    ///
    /// # Errors
    ///
    /// The error from running Podman; the secrets it couldn't remove still carry the run's label.
    pub fn remove(&mut self, runner: &impl Runner) -> Result<(), Error> {
        if self.created.is_empty() {
            return Ok(());
        }
        let mut args = vec!["secret", "rm"];
        args.extend(self.created.iter().map(|(secret, _)| secret.as_str()));
        podman(runner, &args, None)?;
        self.created.clear();
        Ok(())
    }
}

fn podman(runner: &impl Runner, args: &[&str], stdin: Option<&[u8]>) -> Result<(), Error> {
    let output = runner.run("podman", args, stdin)?;
    if output.success {
        Ok(())
    } else {
        Err(Error::Failed {
            command: format!("podman {} {}", args[0], args[1]),
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dotenv::parse;
    use crate::runner::fake::FakeRunner;

    const GRANTS: [Grant; 2] = [
        Grant { name: "INFERENCE_KEY", target: "ANTHROPIC_AUTH_TOKEN" },
        Grant { name: "INFERENCE_ENDPOINT", target: "ANTHROPIC_BASE_URL" },
    ];

    fn secrets() -> Secrets {
        parse("INFERENCE_KEY=sk-value\nINFERENCE_ENDPOINT=https://endpoint.test\n").unwrap()
    }

    #[test]
    fn values_go_on_stdin_byte_for_byte_and_never_in_arguments() {
        let runner = FakeRunner::ok("");
        let created = RunSecrets::create(&runner, "r1", &secrets(), &GRANTS).unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].args,
            [
                "secret",
                "create",
                "--label",
                "acyclic.run=r1",
                "acyclic-r1-anthropic_auth_token",
                "-"
            ]
        );
        assert_eq!(calls[0].stdin.as_deref(), Some(&b"sk-value"[..]));
        assert_eq!(calls[1].stdin.as_deref(), Some(&b"https://endpoint.test"[..]));
        for call in calls.iter() {
            assert!(
                call.args
                    .iter()
                    .all(|arg| !arg.contains("sk-value") && !arg.contains("endpoint.test"))
            );
        }
        assert_eq!(
            created.run_args(),
            [
                "--secret",
                "acyclic-r1-anthropic_auth_token,type=env,target=ANTHROPIC_AUTH_TOKEN",
                "--secret",
                "acyclic-r1-anthropic_base_url,type=env,target=ANTHROPIC_BASE_URL",
            ]
        );
    }

    #[test]
    fn a_missing_name_creates_nothing() {
        let runner = FakeRunner::ok("");
        let grants = [GRANTS[0], Grant { name: "ABSENT", target: "X" }];
        let error = RunSecrets::create(&runner, "r1", &secrets(), &grants).unwrap_err();
        assert_eq!(error.to_string(), "the secrets file has no `ABSENT`");
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn remove_deletes_every_secret_in_one_call_and_then_nothing() {
        let runner = FakeRunner::ok("");
        let mut created = RunSecrets::create(&runner, "r1", &secrets(), &GRANTS).unwrap();
        created.remove(&runner).unwrap();
        created.remove(&runner).unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[2].args,
            ["secret", "rm", "acyclic-r1-anthropic_auth_token", "acyclic-r1-anthropic_base_url"]
        );
    }

    #[test]
    fn a_failed_create_removes_the_secrets_already_made_and_returns_its_own_error() {
        let runner = FakeRunner::answering(&[
            (true, "", ""),
            (false, "", "Error: no space\n"),
            (true, "", ""),
        ]);
        let error = RunSecrets::create(&runner, "r1", &secrets(), &GRANTS).unwrap_err();
        assert_eq!(error.to_string(), "`podman secret create` failed: Error: no space");
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].args, ["secret", "rm", "acyclic-r1-anthropic_auth_token"]);
    }

    #[test]
    fn a_podman_failure_names_the_command() {
        let runner = FakeRunner::failing("Error: secret name in use\n");
        let error = RunSecrets::create(&runner, "r1", &secrets(), &GRANTS).unwrap_err();
        assert_eq!(error.to_string(), "`podman secret create` failed: Error: secret name in use");
    }
}
