use std::ffi::OsString;
use std::process::Stdio;

use backup_core::{Error, Result};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgCommand {
    pub program: String,
    pub args: Vec<OsString>,
    /// Passed only to the child process; holds the password, so never logged.
    pub env: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, cmd: &PgCommand) -> Result<CommandOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for TokioCommandRunner {
    async fn run(&self, cmd: &PgCommand) -> Result<CommandOutput> {
        let output = Command::new(&cmd.program)
            .args(&cmd.args)
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A cancelled gRPC call drops the future; do not leave pg_dump running.
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| Error::io(format!("Could not run `{}`", cmd.program), e))?;

        Ok(CommandOutput {
            success: output.status.success(),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
