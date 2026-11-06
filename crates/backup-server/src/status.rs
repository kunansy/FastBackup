use backup_core::Error;
use tonic::Status;

/// Map a domain error to a gRPC status and log it at a matching level.
pub fn to_status(context: &str, err: Error) -> Status {
    let msg = format!("{context}: {err}");
    match err {
        Error::NotFound(_) => {
            log::info!("{msg}");
            Status::not_found(msg)
        }
        Error::FailedPrecondition(_) => {
            log::info!("{msg}");
            Status::failed_precondition(msg)
        }
        Error::Busy(_) => {
            log::info!("{msg}");
            Status::aborted(msg)
        }
        Error::Storage(_) | Error::Timeout { .. } => {
            log::error!("{msg}");
            Status::unavailable(msg)
        }
        Error::Config(_) | Error::Process { .. } | Error::Io { .. } => {
            log::error!("{msg}");
            Status::internal(msg)
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
            (Error::Storage("x".into()), Code::Unavailable),
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
}
