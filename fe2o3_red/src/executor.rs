//! Command execution backend for the agent's shell tool.
//!
//! `Executor` is a pluggable boundary (plan D4): the `Local` variant
//! runs commands directly under the Red process's user, in a given
//! working directory, with a timeout.  It is a *run location*, not a
//! security sandbox — appropriate for the trusted, self-hosted
//! environment (plan D0).  A future `Remote` variant can offload
//! execution to another host without changing callers.

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;
use std::time::Duration;


/// Where and how the agent runs shell commands.
#[derive(Clone, Debug)]
pub enum Executor {
    /// Run locally under the Red process's user, capped by `timeout`.
    Local { timeout: Duration },
}

/// The captured result of a command.
#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub stdout:    String,
    pub stderr:    String,
    pub exit_code: i32,
    pub timed_out: bool,
}

impl Executor {

    /// A local executor with a sensible default timeout.
    pub fn local_default() -> Self {
        Self::Local { timeout: Duration::from_secs(120) }
    }

    /// Run `command` via `sh -c` in `cwd`, capturing stdout/stderr.
    ///
    /// On timeout the child is killed (via `kill_on_drop`) and a
    /// `timed_out` result is returned rather than an error.
    pub async fn run(&self, command: &str, cwd: &Path) -> Outcome<CommandOutput> {
        match self {
            Self::Local { timeout } => {
                use tokio::process::Command;
                let mut cmd = Command::new("sh");
                cmd.arg("-c").arg(command)
                    .current_dir(cwd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
                let child = res!(cmd.spawn()
                    .map_err(|e| err!(e, "Executor: failed to spawn command."; IO, Init)));
                match tokio::time::timeout(*timeout, child.wait_with_output()).await {
                    Ok(Ok(out)) => Ok(CommandOutput {
                        stdout:    String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr:    String::from_utf8_lossy(&out.stderr).to_string(),
                        exit_code: out.status.code().unwrap_or(-1),
                        timed_out: false,
                    }),
                    Ok(Err(e)) => Err(err!(e, "Executor: waiting on command failed."; IO)),
                    Err(_) => Ok(CommandOutput {
                        stdout:    String::new(),
                        stderr:    fmt!("Command timed out after {} seconds.", timeout.as_secs()),
                        exit_code: -1,
                        timed_out: true,
                    }),
                }
            }
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_echo() {
        let ex = Executor::local_default();
        let out = ex.run("echo hello", Path::new("/tmp")).await.expect("run");
        assert_eq!(out.stdout.trim(), "hello");
        assert_eq!(out.exit_code, 0);
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn test_run_exit_code() {
        let ex = Executor::local_default();
        let out = ex.run("exit 3", Path::new("/tmp")).await.expect("run");
        assert_eq!(out.exit_code, 3);
    }

    #[tokio::test]
    async fn test_run_timeout() {
        let ex = Executor::Local { timeout: Duration::from_millis(200) };
        let out = ex.run("sleep 5", Path::new("/tmp")).await.expect("run");
        assert!(out.timed_out);
    }
}
