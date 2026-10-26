//! The pure Capture retry schedule (ADR-0158).
//!
//! A delay is a function of the durable batch id and attempt alone, so every
//! worker that fails the same attempt computes the same deadline. The store
//! adds it to database time; nothing here reads a clock except the
//! conversion of an HTTP-date `Retry-After` into a relative hint.

use std::time::{Duration, SystemTime};

use synveda_types::CaptureBatchId;

/// The largest provider `Retry-After` hint accepted before policy capping.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// Bounded exponential backoff for retryable Capture failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    base: Duration,
    max_delay: Duration,
    max_age: Duration,
}

/// Why a retry policy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicyError {
    /// A bound was zero.
    Zero,
    /// The base delay exceeded the maximum delay, or the maximum delay the
    /// retry age.
    Unordered,
    /// The retry age exceeded one day.
    TooLong,
}

impl std::fmt::Display for RetryPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Zero => "capture retry bounds must be positive",
            Self::Unordered => {
                "capture retry base must not exceed its maximum delay, nor the delay its maximum age"
            }
            Self::TooLong => "capture retry age must not exceed one day",
        })
    }
}

impl std::error::Error for RetryPolicyError {}

const FACTOR: u32 = 4;
const MAX_POLICY_AGE: Duration = Duration::from_secs(24 * 60 * 60);

impl RetryPolicy {
    /// The provisional schedule: 30 s base, factor 4, 30 min cap, 2 h cycle.
    pub const DEFAULT: Self = Self {
        base: Duration::from_secs(30),
        max_delay: Duration::from_secs(30 * 60),
        max_age: Duration::from_secs(2 * 60 * 60),
    };

    /// Builds a policy with `base <= max_delay <= max_age <= 1 day`.
    pub fn new(
        base: Duration,
        max_delay: Duration,
        max_age: Duration,
    ) -> Result<Self, RetryPolicyError> {
        if base.is_zero() || max_delay.is_zero() || max_age.is_zero() {
            return Err(RetryPolicyError::Zero);
        }
        if base > max_delay || max_delay > max_age {
            return Err(RetryPolicyError::Unordered);
        }
        if max_age > MAX_POLICY_AGE {
            return Err(RetryPolicyError::TooLong);
        }
        Ok(Self {
            base,
            max_delay,
            max_age,
        })
    }

    /// Bound on one retry cycle, measured from its first claim.
    #[must_use]
    pub const fn max_age(&self) -> Duration {
        self.max_age
    }

    /// Bound on any single delay, including a provider hint.
    #[must_use]
    pub const fn max_delay(&self) -> Duration {
        self.max_delay
    }

    /// Delay before the attempt after `attempt` (1-based within the batch's
    /// attempt fence). Jitter spans `[0.5, 1.5)` of the exponential step and is
    /// stable for the batch and attempt. A provider hint can lengthen, never
    /// shorten, the delay; the result never exceeds the maximum delay.
    #[must_use]
    pub fn delay(
        &self,
        batch: CaptureBatchId,
        attempt: i32,
        retry_after: Option<Duration>,
    ) -> Duration {
        let exponent = u32::try_from(attempt.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
        let step = self
            .base
            .checked_mul(FACTOR.saturating_pow(exponent))
            .unwrap_or(self.max_delay)
            .min(self.max_delay);
        let jittered = step.mul_f64(0.5 + jitter_fraction(batch, attempt));
        jittered
            .max(retry_after.unwrap_or(Duration::ZERO))
            .min(self.max_delay)
    }
}

/// A deterministic fraction in `[0, 1)` derived from the batch and attempt.
fn jitter_fraction(batch: CaptureBatchId, attempt: i32) -> f64 {
    let mut hash = blake3::Hasher::new();
    hash.update(b"synveda.capture.retry-jitter.v1\0");
    hash.update(batch.as_uuid().as_bytes());
    hash.update(&attempt.to_be_bytes());
    let digest = hash.finalize();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest.as_bytes()[..8]);
    // 53 bits fill an f64 mantissa exactly, keeping the result below one.
    (u64::from_be_bytes(prefix) >> 11) as f64 / (1_u64 << 53) as f64
}

/// Parses an untrusted `Retry-After` value: delta-seconds or an HTTP-date.
/// Malformed, past and absent values give no hint; larger values are capped.
#[must_use]
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let delay = if value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse::<u64>().unwrap_or(u64::MAX))
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .ok()?
    };
    (!delay.is_zero()).then(|| delay.min(MAX_RETRY_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(n: u128) -> CaptureBatchId {
        CaptureBatchId::from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn delays_grow_by_attempt_within_jitter_and_never_exceed_the_cap() {
        let policy = RetryPolicy::DEFAULT;
        for id in 0..200 {
            let mut previous_step = Duration::ZERO;
            for attempt in 1..=15 {
                let delay = policy.delay(batch(id), attempt, None);
                let exponent = u32::try_from(attempt - 1).unwrap();
                let step = Duration::from_secs(30)
                    .checked_mul(4_u32.saturating_pow(exponent))
                    .unwrap_or(Duration::MAX)
                    .min(policy.max_delay());
                assert!(
                    delay >= step / 2,
                    "{attempt}: {delay:?} below half of {step:?}"
                );
                assert!(
                    delay <= policy.max_delay(),
                    "{attempt}: {delay:?} over the cap"
                );
                assert!(step >= previous_step);
                previous_step = step;
            }
        }
    }

    #[test]
    fn jitter_is_stable_for_a_batch_attempt_and_varies_across_batches() {
        let policy = RetryPolicy::DEFAULT;
        assert_eq!(
            policy.delay(batch(7), 2, None),
            policy.delay(batch(7), 2, None)
        );
        let distinct: std::collections::BTreeSet<_> =
            (0..64).map(|id| policy.delay(batch(id), 1, None)).collect();
        assert!(distinct.len() > 32, "jitter spreads simultaneous failures");
        for id in 0..10_000 {
            let fraction = jitter_fraction(batch(id), 1);
            assert!((0.0..1.0).contains(&fraction));
        }
    }

    #[test]
    fn provider_hints_lengthen_but_cannot_escape_the_cap() {
        let policy = RetryPolicy::new(
            Duration::from_secs(2),
            Duration::from_secs(60),
            Duration::from_secs(600),
        )
        .unwrap();
        let unhinted = policy.delay(batch(1), 1, None);
        assert!(unhinted < Duration::from_secs(3));
        assert_eq!(
            policy.delay(batch(1), 1, Some(Duration::from_secs(45))),
            Duration::from_secs(45)
        );
        assert_eq!(
            policy.delay(batch(1), 1, Some(Duration::from_secs(1))),
            unhinted
        );
        assert_eq!(
            policy.delay(batch(1), 1, Some(MAX_RETRY_AFTER)),
            Duration::from_secs(60)
        );
        let capped = policy.delay(batch(1), i32::MAX, None);
        assert!(capped >= policy.max_delay() / 2 && capped <= policy.max_delay());
        assert!(policy.delay(batch(1), i32::MIN, None) <= Duration::from_secs(3));
    }

    #[test]
    fn policies_are_ordered_positive_and_at_most_one_day() {
        let second = Duration::from_secs(1);
        assert_eq!(
            RetryPolicy::new(Duration::ZERO, second, second),
            Err(RetryPolicyError::Zero)
        );
        assert_eq!(
            RetryPolicy::new(second * 2, second, second * 3),
            Err(RetryPolicyError::Unordered)
        );
        assert_eq!(
            RetryPolicy::new(second, second * 4, second * 3),
            Err(RetryPolicyError::Unordered)
        );
        assert_eq!(
            RetryPolicy::new(second, second, Duration::from_secs(86_401)),
            Err(RetryPolicyError::TooLong)
        );
        assert!(RetryPolicy::new(second, second, second).is_ok());
        let default = RetryPolicy::DEFAULT;
        assert_eq!(
            RetryPolicy::new(
                Duration::from_secs(30),
                default.max_delay(),
                default.max_age()
            ),
            Ok(default)
        );
    }

    #[test]
    fn retry_after_accepts_seconds_and_dates_and_refuses_everything_else() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_784_000_000);
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after(" 7 ", now), Some(Duration::from_secs(7)));
        assert_eq!(
            parse_retry_after("99999999999999999999999", now),
            Some(MAX_RETRY_AFTER)
        );
        let later = httpdate::fmt_http_date(now + Duration::from_secs(90));
        assert_eq!(
            parse_retry_after(&later, now),
            Some(Duration::from_secs(90))
        );
        let earlier = httpdate::fmt_http_date(now - Duration::from_secs(90));
        for refused in [
            "",
            "0",
            "-5",
            "1.5",
            "soon",
            "120 seconds",
            earlier.as_str(),
        ] {
            assert_eq!(parse_retry_after(refused, now), None, "{refused:?}");
        }
        assert_eq!(parse_retry_after(&"9".repeat(65), now), None);
    }
}
