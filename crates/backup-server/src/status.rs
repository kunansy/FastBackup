use backup_core::Error;
use tonic::Status;

/// Map a domain error to a gRPC status and log it at a matching level. The log keeps
/// the full error; the client gets details only for errors it can act on.
pub fn to_status(context: &str, err: Error) -> Status {
    let msg = format!("{context}: {err}");
    match err {
        Error::NotFound(_) => {
            log::info!("{msg}");
            Status::not_found(msg)
        }
        Error::InvalidArgument(_) => {
            log::info!("{msg}");
            Status::invalid_argument(msg)
        }
        Error::FailedPrecondition(_) => {
            log::info!("{msg}");
            Status::failed_precondition(msg)
        }
        Error::Busy(_) => {
            log::info!("{msg}");
            Status::aborted(msg)
        }
        Error::Timeout { .. } => {
            log::error!("{msg}");
            Status::unavailable(msg)
        }
        Error::Unavailable(_) => {
            log::error!("{msg}");
            Status::unavailable(format!("{context}: storage unavailable, retry later"))
        }
        Error::Storage(_) | Error::Config(_) | Error::Process { .. } | Error::Io { .. } => {
            log::error!("{msg}");
            Status::internal(format!("{context}: internal error, see server logs"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use backup_core::Error;
    use tonic::Code;

    use super::to_status;

    #[test]
    fn test_codes() {
        let cases = [
            (Error::NotFound("x".into()), Code::NotFound),
            (
                Error::FailedPrecondition("x".into()),
                Code::FailedPrecondition,
            ),
            (Error::Busy("x".into()), Code::Aborted),
            (Error::InvalidArgument("x".into()), Code::InvalidArgument),
            (Error::Unavailable("x".into()), Code::Unavailable),
            (Error::Storage("x".into()), Code::Internal),
            (Error::Config("x".into()), Code::Internal),
            (Error::io("x", std::io::Error::other("y")), Code::Internal),
            (
                Error::Timeout {
                    operation: "x".into(),
                    limit: Duration::from_secs(1),
                },
                Code::Unavailable,
            ),
            (
                Error::Process {
                    program: "pg_dump".into(),
                    status: "1".into(),
                    stderr: "boom".into(),
                },
                Code::Internal,
            ),
        ];

        for (err, code) in cases {
            assert_eq!(to_status("ctx", err).code(), code);
        }
    }

    #[test]
    fn test_message_has_context() {
        let status = to_status("Dump failed", Error::NotFound("db".into()));

        assert_eq!(status.message(), "Dump failed: not found: db");
    }

    #[test]
    fn test_internal_details_are_not_sent_to_client() {
        let errors = [
            Error::Process {
                program: "pg_dump".into(),
                status: "1".into(),
                stderr: "connection to server at \"db-host\" failed".into(),
            },
            Error::io(
                "Could not create /app/data/backup_x",
                std::io::Error::other("disk full"),
            ),
            Error::Storage("HTTP 401: token for sa@project expired".into()),
            Error::Unavailable("HTTP 503: backend at 10.0.0.1".into()),
        ];

        for err in errors {
            let status = to_status("Backup failed", err);

            for secret in ["db-host", "/app/data", "sa@project", "10.0.0.1"] {
                assert!(!status.message().contains(secret), "{}", status.message());
            }
            assert!(status.message().starts_with("Backup failed: "));
        }
    }
}
