//! Running the `sops` and `podman` programs, behind a trait so tests can stand in for them.

use std::io::Write;
use std::process::{Command, Stdio};

use zeroize::Zeroizing;

use crate::Error;

/// What a finished program left behind.
pub struct Output {
    /// Whether it exited successfully.
    pub success: bool,
    /// Its standard output, wiped when dropped since it may hold secret values.
    pub stdout: Zeroizing<Vec<u8>>,
    /// Its standard error.
    pub stderr: String,
}

/// Runs a program with arguments and, optionally, bytes on its standard input.
///
/// Secret values travel only through `stdin` and the returned standard output, never through `args`.
pub trait Runner {
    /// Runs `program` with `args`, writing `stdin` to it when given, and waits for it to exit.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when the program can't be started or its input can't be written.
    fn run(&self, program: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, Error>;
}

/// Runs programs as child processes.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, Error> {
        let spawn_error = |source| Error::Spawn { program: program.to_owned(), source };
        let mut child = Command::new(program)
            .args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;
        // Taking the pipe closes it at the end of the block, so the program sees the end of its input. A program that
        // exits early breaks the pipe; it is still waited for, and the run counts as failed.
        let written = stdin.map_or(Ok(()), |bytes| {
            child.stdin.take().expect("standard input is piped").write_all(bytes)
        });
        let output = child.wait_with_output().map_err(spawn_error)?;
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if let Err(error) = &written {
            stderr = format!("{stderr}\n(its standard input wasn't fully written: {error})");
        }
        Ok(Output {
            success: output.status.success() && written.is_ok(),
            stdout: Zeroizing::new(output.stdout),
            stderr,
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;

    use super::{Error, Output, Runner, Zeroizing};

    /// One call the fake saw.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Call {
        pub program: String,
        pub args: Vec<String>,
        pub stdin: Option<Vec<u8>>,
    }

    /// Answers calls in turn from a list of outputs, repeating the last, and records the calls.
    pub struct FakeRunner {
        pub calls: RefCell<Vec<Call>>,
        answers: Vec<(bool, String, String)>,
    }

    impl FakeRunner {
        pub fn ok(stdout: &str) -> Self {
            Self::answering(&[(true, stdout, "")])
        }

        pub fn failing(stderr: &str) -> Self {
            Self::answering(&[(false, "", stderr)])
        }

        /// Answers with `(success, stdout, stderr)` for each call in turn.
        pub fn answering(answers: &[(bool, &str, &str)]) -> Self {
            Self {
                calls: RefCell::default(),
                answers: answers
                    .iter()
                    .map(|&(ok, out, err)| (ok, out.into(), err.into()))
                    .collect(),
            }
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, program: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, Error> {
            self.calls.borrow_mut().push(Call {
                program: program.into(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
                stdin: stdin.map(<[u8]>::to_vec),
            });
            let calls = self.calls.borrow().len();
            let (success, stdout, stderr) = &self.answers[(calls - 1).min(self.answers.len() - 1)];
            Ok(Output {
                success: *success,
                stdout: Zeroizing::new(stdout.clone().into_bytes()),
                stderr: stderr.clone(),
            })
        }
    }
}
