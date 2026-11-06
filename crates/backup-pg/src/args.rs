use std::ffi::OsString;
use std::path::Path;

use backup_core::DbTarget;

use crate::{PgCommand, ZstdLevel};

pub(crate) const PG_DUMP: &str = "pg_dump";
pub(crate) const PG_RESTORE: &str = "pg_restore";
pub(crate) const PSQL: &str = "psql";
const CONNECT_TIMEOUT_SECS: &str = "10";

fn connection_args(target: &DbTarget) -> Vec<OsString> {
    [
        "--host",
        &target.host,
        "--port",
        &target.port.to_string(),
        "--username",
        &target.user,
        "--dbname",
        &target.name,
        "--no-password",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn connection_env(target: &DbTarget) -> Vec<(&'static str, String)> {
    vec![
        ("PGPASSWORD", target.password.expose().to_string()),
        ("PGCONNECT_TIMEOUT", CONNECT_TIMEOUT_SECS.to_string()),
    ]
}

fn command(program: &str, flags: &[&str], target: &DbTarget, tail: Vec<OsString>) -> PgCommand {
    let mut args: Vec<OsString> = flags.iter().map(OsString::from).collect();
    args.extend(connection_args(target));
    args.extend(tail);

    PgCommand {
        program: program.to_string(),
        args,
        env: connection_env(target),
    }
}

pub(crate) fn dump_command(target: &DbTarget, level: ZstdLevel, dst: &Path) -> PgCommand {
    let compress = format!("--compress=zstd:{level}");
    let flags = [
        "--format=custom",
        &compress,
        "--no-owner",
        "--no-privileges",
    ];

    command(
        PG_DUMP,
        &flags,
        target,
        vec!["--file".into(), dst.as_os_str().to_owned()],
    )
}

pub(crate) fn restore_command(target: &DbTarget, src: &Path) -> PgCommand {
    let flags = [
        "--clean",
        "--if-exists",
        "--no-owner",
        "--no-privileges",
        "--single-transaction",
        "--exit-on-error",
    ];

    command(PG_RESTORE, &flags, target, vec![src.as_os_str().to_owned()])
}

pub(crate) fn query_command(target: &DbTarget, sql: &str) -> PgCommand {
    let flags = [
        "--no-psqlrc",
        "--tuples-only",
        "--no-align",
        "--set=ON_ERROR_STOP=1",
    ];

    command(PSQL, &flags, target, vec!["--command".into(), sql.into()])
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use backup_core::{DbTarget, Secret};

    use super::{dump_command, query_command, restore_command};
    use crate::ZstdLevel;

    fn target() -> DbTarget {
        DbTarget {
            host: "tracker-db".into(),
            port: 5433,
            user: "tracker".into(),
            password: Secret::new("hunter2"),
            name: "tracker".into(),
        }
    }

    fn args(cmd: &crate::PgCommand) -> Vec<String> {
        cmd.args
            .iter()
            .map(|a: &OsString| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_dump_command() {
        let cmd = dump_command(
            &target(),
            ZstdLevel::new(5).unwrap(),
            Path::new("/data/x.pgdump"),
        );

        assert_eq!(cmd.program, "pg_dump");
        assert_eq!(
            args(&cmd),
            [
                "--format=custom",
                "--compress=zstd:5",
                "--no-owner",
                "--no-privileges",
                "--host",
                "tracker-db",
                "--port",
                "5433",
                "--username",
                "tracker",
                "--dbname",
                "tracker",
                "--no-password",
                "--file",
                "/data/x.pgdump",
            ]
        );
    }

    #[test]
    fn test_restore_command_is_atomic_and_clean() {
        let cmd = restore_command(&target(), Path::new("/data/x.pgdump"));
        let args = args(&cmd);

        assert_eq!(cmd.program, "pg_restore");
        for flag in [
            "--clean",
            "--if-exists",
            "--single-transaction",
            "--exit-on-error",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag} missing in {args:?}");
        }
        assert_eq!(args.last().map(String::as_str), Some("/data/x.pgdump"));
    }

    #[test]
    fn test_query_command_stops_on_error() {
        let cmd = query_command(&target(), "SELECT 1");
        let args = args(&cmd);

        assert_eq!(cmd.program, "psql");
        assert!(args.iter().any(|a| a == "--set=ON_ERROR_STOP=1"));
        assert_eq!(&args[args.len() - 2..], ["--command", "SELECT 1"]);
    }

    #[test]
    fn test_password_goes_to_env_not_args() {
        for cmd in [
            dump_command(&target(), ZstdLevel::default(), Path::new("/x")),
            restore_command(&target(), Path::new("/x")),
            query_command(&target(), "SELECT 1"),
        ] {
            assert!(!args(&cmd).iter().any(|a| a.contains("hunter2")));
            assert!(cmd.env.contains(&("PGPASSWORD", "hunter2".to_string())));
        }
    }
}
