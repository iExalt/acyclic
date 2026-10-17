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
        if let Some(bytes) = stdin {
            // Taking the pipe closes it when the block ends, so the program sees the end of its input.
            let mut pipe = child.stdin.take().expect("standard input is piped");
            pipe.write_all(bytes).map_err(spawn_error)?;
        }
        let output = child.wait_with_output().map_err(spawn_error)?;
        Ok(Output {
            success: output.status.success(),
            stdout: Zeroizing::new(output.stdout),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
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

    /// Answers every call with the same output and records the calls.
    pub struct FakeRunner {
        pub calls: RefCell<Vec<Call>>,
        success: bool,
        stdout: Vec<u8>,
        stderr: String,
    }

    impl FakeRunner {
        pub fn ok(stdout: &str) -> Self {
            Self {
                calls: RefCell::default(),
                success: true,
                stdout: stdout.into(),
                stderr: String::new(),
            }
        }

        pub fn failing(stderr: &str) -> Self {
            Self {
                calls: RefCell::default(),
                success: false,
                stdout: Vec::new(),
                stderr: stderr.into(),
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
            Ok(Output {
                success: self.success,
                stdout: Zeroizing::new(self.stdout.clone()),
                stderr: self.stderr.clone(),
            })
        }
    }
}
