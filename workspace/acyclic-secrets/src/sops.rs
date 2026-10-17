//! Decrypting the secrets file with the `sops` program.

use std::path::Path;

use zeroize::Zeroizing;

use crate::{Error, Runner, Secrets, dotenv};

/// Decrypts the dotenv secrets file at `path`, keeping the plaintext only in memory.
///
/// # Errors
///
/// [`Error::Spawn`] when `sops` can't run, [`Error::Failed`] when it can't decrypt the file (for example without the
/// age identity), and [`Error::Parse`] when the plaintext isn't dotenv.
pub fn decrypt(runner: &impl Runner, path: &Path) -> Result<Secrets, Error> {
    let path = path.to_string_lossy();
    let output = runner.run(
        "sops",
        &["decrypt", "--input-type", "dotenv", "--output-type", "dotenv", &path],
        None,
    )?;
    if !output.success {
        return Err(Error::Failed { command: "sops decrypt".into(), stderr: output.stderr });
    }
    let text = Zeroizing::new(String::from_utf8_lossy(&output.stdout).into_owned());
    dotenv::parse(&text).map_err(Error::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;

    #[test]
    fn decrypts_with_dotenv_types_and_parses_the_output() {
        let runner = FakeRunner::ok("TOKEN=value\n");
        let secrets = decrypt(&runner, Path::new("/config/secrets.sops.env")).unwrap();
        assert_eq!(secrets.get("TOKEN"), Some("value"));
        let calls = runner.calls.borrow();
        assert_eq!(calls[0].program, "sops");
        assert_eq!(
            calls[0].args,
            [
                "decrypt",
                "--input-type",
                "dotenv",
                "--output-type",
                "dotenv",
                "/config/secrets.sops.env"
            ]
        );
    }

    #[test]
    fn a_failed_decrypt_reports_sops_error() {
        let runner = FakeRunner::failing("no identity matched any of the recipients\n");
        let error = decrypt(&runner, Path::new("/x")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "`sops decrypt` failed: no identity matched any of the recipients"
        );
    }
}
