use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use backup_core::{DbTarget, Dumper, Error, Restorer, Result};

use crate::args::{dump_command, query_command, restore_command};
use crate::{CommandRunner, PgCommand, ZstdLevel};

const STDERR_LIMIT: usize = 4096;

/// Runs PostgreSQL client binaries against one database.
pub struct PgTools {
    target: DbTarget,
    level: ZstdLevel,
    runner: Arc<dyn CommandRunner>,
}

impl PgTools {
    pub fn new(target: DbTarget, level: ZstdLevel, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            target,
            level,
            runner,
        }
    }

    pub fn db_name(&self) -> &str {
        &self.target.name
    }

    /// Run one SQL statement through `psql`, return its unaligned stdout.
    pub async fn query(&self, sql: &str) -> Result<String> {
        self.execute(&query_command(&self.target, sql)).await
    }

    pub async fn ping(&self) -> Result<()> {
        let out = self.query("SELECT 1").await?;
        match out.trim() {
            "1" => Ok(()),
            other => Err(Error::FailedPrecondition(format!(
                "unexpected healthcheck result: '{other}'"
            ))),
        }
    }

    async fn execute(&self, cmd: &PgCommand) -> Result<String> {
        let out = self.runner.run(cmd).await?;
        if !out.success {
            return Err(Error::Process {
                program: cmd.program.clone(),
                status: out.status,
                stderr: tail(&out.stderr, STDERR_LIMIT).to_string(),
            });
        }
        if !out.stderr.trim().is_empty() {
            log::warn!(
                "`{}` stderr: {}",
                cmd.program,
                tail(&out.stderr, STDERR_LIMIT)
            );
        }
        Ok(out.stdout)
    }
}

#[async_trait::async_trait]
impl Dumper for PgTools {
    async fn dump(&self, dst: &Path) -> Result<()> {
        log::info!("Dumping database '{}' to {dst:?}", self.target.name);
        let start = Instant::now();

        self.execute(&dump_command(&self.target, self.level, dst))
            .await?;

        log::info!(
            "Database '{}' dumped for {:?}",
            self.target.name,
            start.elapsed()
        );
        Ok(())
    }
}

#[async_trait::async_trait]
impl Restorer for PgTools {
    async fn restore(&self, src: &Path) -> Result<()> {
        log::info!("Restoring database '{}' from {src:?}", self.target.name);
        let start = Instant::now();

        self.execute(&restore_command(&self.target, src)).await?;

        log::info!(
            "Database '{}' restored for {:?}",
            self.target.name,
            start.elapsed()
        );
        Ok(())
    }
}

/// Last `limit` bytes of `s`, cut on a char boundary: pg tools print the cause last.
fn tail(s: &str, limit: usize) -> &str {
    let s = s.trim_end();
    let mut start = s.len().saturating_sub(limit);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use backup_core::{DbTarget, Dumper, Error, Restorer, Result, Secret};

    use super::{PgTools, tail};
    use crate::{CommandOutput, CommandRunner, PgCommand, ZstdLevel};

    struct FakeRunner {
        output: CommandOutput,
        calls: Mutex<Vec<PgCommand>>,
    }

    impl FakeRunner {
        fn new(success: bool, stdout: &str, stderr: &str) -> Arc<Self> {
            Arc::new(Self {
                output: CommandOutput {
                    success,
                    status: if success {
                        "exit status: 0"
                    } else {
                        "exit status: 1"
                    }
                    .into(),
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                },
                calls: Mutex::new(Vec::new()),
            })
        }

        fn programs(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.program.clone())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, cmd: &PgCommand) -> Result<CommandOutput> {
            self.calls.lock().unwrap().push(cmd.clone());
            Ok(self.output.clone())
        }
    }

    fn tools(runner: Arc<FakeRunner>) -> PgTools {
        let target = DbTarget {
            host: "db".into(),
            port: 5432,
            user: "u".into(),
            password: Secret::new("p"),
            name: "tracker".into(),
        };
        PgTools::new(target, ZstdLevel::default(), runner)
    }

    #[tokio::test]
    async fn test_dump_runs_pg_dump() {
        let runner = FakeRunner::new(true, "", "");

        tools(runner.clone())
            .dump(Path::new("/tmp/x"))
            .await
            .unwrap();

        assert_eq!(runner.programs(), ["pg_dump"]);
    }

    #[tokio::test]
    async fn test_restore_runs_pg_restore() {
        let runner = FakeRunner::new(true, "", "");

        tools(runner.clone())
            .restore(Path::new("/tmp/x"))
            .await
            .unwrap();

        assert_eq!(runner.programs(), ["pg_restore"]);
    }

    #[tokio::test]
    async fn test_failed_process_returns_stderr() {
        let runner = FakeRunner::new(false, "", "pg_dump: error: connection refused\n");

        let err = tools(runner).dump(Path::new("/tmp/x")).await.unwrap_err();

        match err {
            Error::Process {
                program, stderr, ..
            } => {
                assert_eq!(program, "pg_dump");
                assert_eq!(stderr, "pg_dump: error: connection refused");
            }
            other => panic!("expected Process error, {other:?} found"),
        }
    }

    #[tokio::test]
    async fn test_ping_ok() {
        let runner = FakeRunner::new(true, "1\n", "");

        assert!(tools(runner).ping().await.is_ok());
    }

    #[tokio::test]
    async fn test_ping_rejects_unexpected_output() {
        let runner = FakeRunner::new(true, "2\n", "");

        assert!(tools(runner).ping().await.is_err());
    }

    #[test]
    fn test_tail_keeps_end_on_char_boundary() {
        assert_eq!(tail("abcdef\n", 3), "def");
        assert_eq!(tail("ab", 10), "ab");
        assert_eq!(tail("ыыы", 3), "ы");
    }
}
