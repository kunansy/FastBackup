use std::future::Future;
use std::time::Duration;

use backup_core::Error;
use serde_json::Value;

const RATE_LIMIT_REASONS: [&str; 2] = ["rateLimitExceeded", "userRateLimitExceeded"];

#[derive(Debug)]
pub(crate) enum CallError {
    /// Worth retrying: network, timeout, rate limit, 5xx.
    Transient(String),
    /// HTTP 400: a client error only when the client supplied the bad value.
    BadRequest(String),
    Permanent(Error),
}

impl CallError {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Transient(msg) => Error::Unavailable(msg),
            Self::BadRequest(msg) => Error::Storage(msg),
            Self::Permanent(err) => err,
        }
    }
}

fn classify_status(op: &str, status: u16, reason: Option<&str>, detail: &str) -> CallError {
    let msg = format!("{op}: HTTP {status}: {detail}");
    match status {
        400 => CallError::BadRequest(msg),
        404 => CallError::Permanent(Error::NotFound(msg)),
        429 | 500..=599 => CallError::Transient(msg),
        403 if reason.is_some_and(|r| RATE_LIMIT_REASONS.contains(&r)) => CallError::Transient(msg),
        _ => CallError::Permanent(Error::Storage(msg)),
    }
}

fn classify_json(op: &str, body: &Value) -> CallError {
    let error = &body["error"];
    let Some(status) = error["code"].as_u64().and_then(|c| u16::try_from(c).ok()) else {
        return CallError::Permanent(Error::Storage(format!("{op}: {body}")));
    };
    let reason = error["errors"][0]["reason"].as_str();
    let detail = error["message"].as_str().unwrap_or_default();
    classify_status(op, status, reason, detail)
}

pub(crate) fn classify(op: &str, err: google_drive3::Error) -> CallError {
    use google_drive3::Error as E;

    match err {
        E::HttpError(e) => CallError::Transient(format!("{op}: transport error: {e}")),
        E::Io(e) => CallError::Transient(format!("{op}: io error: {e}")),
        E::MissingToken(e) => CallError::Transient(format!("{op}: token error: {e}")),
        E::BadRequest(body) => classify_json(op, &body),
        E::Failure(resp) => classify_status(op, resp.status().as_u16(), None, "no details"),
        other => CallError::Permanent(Error::Storage(format!("{op}: {other}"))),
    }
}

/// Bound a Drive call in time and classify its failure.
pub(crate) async fn timed<T, F>(op: &str, limit: Duration, fut: F) -> Result<T, CallError>
where
    F: Future<Output = google_drive3::Result<T>>,
{
    match tokio::time::timeout(limit, fut).await {
        Err(_) => Err(CallError::Transient(format!(
            "{op} timed out after {limit:?}"
        ))),
        Ok(res) => res.map_err(|e| classify(op, e)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use backup_core::Error;
    use serde_json::json;

    use super::{CallError, classify, timed};

    fn bad_request(code: u64, reason: &str) -> google_drive3::Error {
        google_drive3::Error::BadRequest(json!({
            "error": {"code": code, "message": "boom", "errors": [{"reason": reason}]}
        }))
    }

    #[test]
    fn test_not_found_is_permanent() {
        match classify("Get", bad_request(404, "notFound")) {
            CallError::Permanent(Error::NotFound(msg)) => assert!(msg.contains("404"), "{msg}"),
            other => panic!("expected NotFound, {other:?} found"),
        }
    }

    #[test]
    fn test_server_errors_and_rate_limits_are_transient() {
        for (code, reason) in [
            (500, "backendError"),
            (503, "x"),
            (429, "x"),
            (403, "userRateLimitExceeded"),
        ] {
            assert!(
                matches!(
                    classify("List", bad_request(code, reason)),
                    CallError::Transient(_)
                ),
                "{code} {reason}"
            );
        }
    }

    #[test]
    fn test_bad_request_is_a_storage_error_by_default() {
        let err = classify("List", bad_request(400, "invalid"));

        assert!(matches!(err, CallError::BadRequest(_)), "{err:?}");
        assert!(matches!(err.into_error(), Error::Storage(_)));
    }

    #[test]
    fn test_unauthorized_is_permanent() {
        assert!(matches!(
            classify("List", bad_request(401, "authError")),
            CallError::Permanent(Error::Storage(_))
        ));
    }

    #[test]
    fn test_transport_and_token_errors_are_transient() {
        let errors = [
            google_drive3::Error::Io(std::io::Error::other("reset")),
            google_drive3::Error::MissingToken("oauth endpoint down".into()),
        ];
        for err in errors {
            assert!(matches!(classify("List", err), CallError::Transient(_)));
        }
    }

    #[test]
    fn test_exhausted_transient_error_is_unavailable() {
        let err = CallError::Transient("down".into()).into_error();

        assert!(matches!(err, Error::Unavailable(msg) if msg == "down"));
    }

    #[test]
    fn test_forbidden_is_permanent() {
        assert!(matches!(
            classify("List", bad_request(403, "insufficientFilePermissions")),
            CallError::Permanent(Error::Storage(_))
        ));
    }

    #[test]
    fn test_unparsable_body_is_permanent() {
        let err = google_drive3::Error::BadRequest(json!({"weird": true}));

        assert!(matches!(classify("List", err), CallError::Permanent(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn test_timed_turns_hang_into_transient_error() {
        let res: Result<(), _> = timed(
            "Hanging op",
            Duration::from_secs(10),
            std::future::pending(),
        )
        .await;

        match res {
            Err(CallError::Transient(msg)) => assert!(msg.starts_with("Hanging op timed out")),
            other => panic!("expected Transient, {other:?} found"),
        }
    }

    #[tokio::test]
    async fn test_timed_passes_result_through() {
        let res = timed("Fast op", Duration::from_secs(1), async { Ok(42) }).await;

        assert_eq!(res.unwrap(), 42);
    }
}
