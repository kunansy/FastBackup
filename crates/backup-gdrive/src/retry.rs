use std::future::Future;
use std::time::Duration;

use backup_core::{Error, Result};

use crate::errors::CallError;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// Retry transient failures with exponential backoff. Only for idempotent calls.
pub(crate) async fn retry<T, F, Fut>(policy: RetryPolicy, op: &str, mut call: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, CallError>>,
{
    let mut delay = policy.base_delay;
    let mut attempt = 1;
    loop {
        match call().await {
            Ok(value) => return Ok(value),
            Err(CallError::Permanent(err)) => return Err(err),
            Err(CallError::Transient(msg)) if attempt >= policy.attempts => {
                return Err(Error::Storage(msg));
            }
            Err(CallError::Transient(msg)) => {
                log::warn!("{op} failed (attempt {attempt}/{}): {msg}", policy.attempts);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(policy.max_delay);
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use backup_core::Error;

    use super::{RetryPolicy, retry};
    use crate::errors::CallError;

    const POLICY: RetryPolicy = RetryPolicy {
        attempts: 3,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(2),
    };

    #[tokio::test(start_paused = true)]
    async fn test_retries_transient_until_success() {
        let calls = AtomicU32::new(0);

        let res = retry(POLICY, "op", || async {
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 | 1 => Err(CallError::Transient("flaky".into())),
                _ => Ok("done"),
            }
        })
        .await;

        assert_eq!(res.unwrap(), "done");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn test_gives_up_after_attempts() {
        let calls = AtomicU32::new(0);

        let res: backup_core::Result<()> = retry(POLICY, "op", || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(CallError::Transient("down".into()))
        })
        .await;

        assert!(matches!(res, Err(Error::Storage(msg)) if msg == "down"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn test_permanent_error_is_not_retried() {
        let calls = AtomicU32::new(0);

        let res: backup_core::Result<()> = retry(POLICY, "op", || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(CallError::Permanent(Error::NotFound("x".into())))
        })
        .await;

        assert!(matches!(res, Err(Error::NotFound(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
