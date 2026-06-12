//! External-call resilience (#106): bounded retries, exponential backoff with
//! jitter, and error classification.
//!
//! Every outbound call Bifrost makes — LLM providers, ADO/GitHub REST, the
//! Importer subprocess — is flaky in the same ways (transient 5xx, throttling,
//! dropped connections). This module is the one place that policy lives, so a
//! call site only has to say *which errors are worth retrying*.
//!
//! It is deliberately generic (not LLM-specific); it lives here because every
//! crate that makes external calls already depends on `bifrost-llm`. Per-attempt
//! timeouts are applied at the call site (e.g. `reqwest`'s `.timeout()`), which
//! then surface as a retryable error through the caller's classifier.

use std::future::Future;
use std::time::Duration;

/// Whether an error is worth retrying. The caller classifies its own error type —
/// only it knows that, say, an HTTP 400 is permanent but a 503 is transient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient — retry after backoff (timeouts, connection resets, 5xx, 429).
    Retryable,
    /// Permanent — fail immediately (4xx auth/validation, parse errors).
    Permanent,
}

/// How hard to retry. Defaults: 3 attempts, 200ms base, capped at 5s.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first (so `1` means "no retries").
    pub max_attempts: u32,
    /// Backoff for the first retry; doubles each subsequent retry.
    pub base_delay: Duration,
    /// Upper bound on any single backoff.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries (one attempt) — useful in tests and air-gap
    /// paths where determinism matters more than resilience.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// Build from env: `<prefix>_RETRY_ATTEMPTS` and `<prefix>_RETRY_BASE_MS`,
    /// each falling back to the default when unset or unparseable.
    pub fn from_env(prefix: &str) -> Self {
        let d = Self::default();
        let attempts = std::env::var(format!("{prefix}_RETRY_ATTEMPTS"))
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(d.max_attempts);
        let base = std::env::var(format!("{prefix}_RETRY_BASE_MS"))
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(d.base_delay);
        Self {
            max_attempts: attempts,
            base_delay: base,
            max_delay: d.max_delay,
        }
    }

    /// Backoff before the retry that follows attempt `attempt` (1-based).
    /// Equal-jitter: `delay/2 + random(0, delay/2)`, so waits land in
    /// `[0.5, 1.0] * min(base * 2^(attempt-1), max_delay)`.
    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
        let raw = self.base_delay.saturating_mul(factor);
        let capped = raw.min(self.max_delay);
        capped.mul_f64(0.5 + 0.5 * jitter_fraction())
    }
}

/// A pseudo-random fraction in `[0.0, 1.0)` for jitter. Derived from the clock's
/// sub-second nanos — adequate to spread retries (it need not be cryptographic),
/// and avoids pulling in an RNG dependency.
fn jitter_fraction() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1_000) as f64 / 1_000.0
}

/// Run `op` under `policy`, retrying only errors `classify` calls
/// [`ErrorClass::Retryable`], with exponential backoff + jitter between attempts.
/// Returns the last error once attempts are exhausted or an error is classified
/// permanent.
pub async fn retry<T, E, F, Fut>(
    policy: RetryPolicy,
    mut classify: impl FnMut(&E) -> ErrorClass,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 1;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if attempt >= policy.max_attempts || classify(&err) == ErrorClass::Permanent {
                    return Err(err);
                }
                tokio::time::sleep(policy.backoff(attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// Classify a [`reqwest`] transport error: timeouts, connection failures, and
/// request-build/redirect errors are transient; everything else is permanent.
/// (HTTP status codes are classified by the caller — `reqwest` only yields an
/// error here for transport-level failures, not for 4xx/5xx responses.)
pub fn classify_reqwest(err: &reqwest::Error) -> ErrorClass {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        ErrorClass::Retryable
    } else {
        ErrorClass::Permanent
    }
}

/// Classify an HTTP status code: 429 and 5xx are transient; other non-success
/// codes are permanent (auth, validation, not-found).
pub fn classify_status(status: u16) -> ErrorClass {
    if status == 429 || (500..=599).contains(&status) {
        ErrorClass::Retryable
    } else {
        ErrorClass::Permanent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Debug, PartialEq)]
    enum E {
        Transient,
        Fatal,
    }
    fn classify(e: &E) -> ErrorClass {
        match e {
            E::Transient => ErrorClass::Retryable,
            E::Fatal => ErrorClass::Permanent,
        }
    }

    /// Zero base delay → retries don't actually wait, keeping tests instant.
    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn succeeds_on_first_attempt_without_retrying() {
        let calls = AtomicU32::new(0);
        let r: Result<u32, E> = retry(fast_policy(3), classify, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(7) }
        })
        .await;
        assert_eq!(r.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_transient_errors_until_success() {
        let calls = AtomicU32::new(0);
        let r: Result<&str, E> = retry(fast_policy(5), classify, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(E::Transient)
                } else {
                    Ok("ok")
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), "ok");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "two failures + one success"
        );
    }

    #[tokio::test]
    async fn permanent_errors_are_not_retried() {
        let calls = AtomicU32::new(0);
        let r: Result<(), E> = retry(fast_policy(5), classify, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(E::Fatal) }
        })
        .await;
        assert!(matches!(r, Err(E::Fatal)));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry on permanent");
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let calls = AtomicU32::new(0);
        let r: Result<(), E> = retry(fast_policy(3), classify, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(E::Transient) }
        })
        .await;
        assert!(matches!(r, Err(E::Transient)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "exactly max_attempts tries"
        );
    }

    #[test]
    fn backoff_grows_and_stays_within_equal_jitter_bounds() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
        };
        // Attempt 1: capped = 100ms → [50, 100]ms.
        for _ in 0..50 {
            let d = policy.backoff(1);
            assert!(d >= Duration::from_millis(50) && d <= Duration::from_millis(100));
        }
        // Attempt 3: capped = 400ms → [200, 400]ms.
        for _ in 0..50 {
            let d = policy.backoff(3);
            assert!(d >= Duration::from_millis(200) && d <= Duration::from_millis(400));
        }
        // Far-out attempt is clamped to max_delay → [2.5s, 5s].
        for _ in 0..50 {
            let d = policy.backoff(20);
            assert!(d >= Duration::from_millis(2500) && d <= Duration::from_secs(5));
        }
    }

    #[test]
    fn status_and_reqwest_classification() {
        assert_eq!(classify_status(503), ErrorClass::Retryable);
        assert_eq!(classify_status(429), ErrorClass::Retryable);
        assert_eq!(classify_status(404), ErrorClass::Permanent);
        assert_eq!(classify_status(401), ErrorClass::Permanent);
    }

    #[test]
    fn none_policy_makes_a_single_attempt() {
        assert_eq!(RetryPolicy::none().max_attempts, 1);
    }
}
