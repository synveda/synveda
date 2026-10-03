//! One-time, pre-tenant login state. Production persistence is supplied by
//! the gateway/store tier; the in-memory implementation exists for tests only.

use async_trait::async_trait;
use synveda_types::Result;

/// A pending login removed atomically from the ledger. Expiry is determined
/// by the ledger's clock, not a gateway process clock.
pub struct ConsumedPending {
    /// Opaque protocol state, sealed in production storage.
    pub payload: Vec<u8>,
    /// Whether the database deadline had passed when consumed.
    pub expired: bool,
}

/// A CLI handoff removed atomically on its first redemption attempt.
pub struct ConsumedHandoff {
    /// Hash of the CLI's independently minted state.
    pub state_hash: [u8; 32],
    /// Opaque completed session, sealed in production storage.
    pub payload: Vec<u8>,
    /// Whether the database deadline had passed when consumed.
    pub expired: bool,
}

/// The narrow storage contract for AUTH-1 and ADPT-1 one-time state.
/// Selectors are hashes of independent 32-byte random secrets. A console
/// correlation mismatch must leave its pending row untouched.
#[async_trait]
pub trait LoginLedger: Send + Sync {
    /// Park one pending OIDC flow, enforcing the global count bound.
    async fn park_pending(
        &self,
        state_hash: [u8; 32],
        correlation_hash: Option<[u8; 32]>,
        payload: Vec<u8>,
    ) -> Result<()>;
    /// Inspect a pending flow without consuming it, including an expired row.
    async fn peek_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>>;
    /// Consume one flow only when its browser correlation matches.
    async fn consume_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<Option<ConsumedPending>>;
    /// Discard a failed flow only when its browser correlation matches.
    async fn abandon_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<bool>;
    /// Park a completed CLI session under an independently random code.
    async fn park_handoff(
        &self,
        code_hash: [u8; 32],
        state_hash: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<()>;
    /// Consume a CLI code on its first redemption attempt.
    async fn consume_handoff(&self, code_hash: [u8; 32]) -> Result<Option<ConsumedHandoff>>;
}

#[cfg(any(test, feature = "test-support"))]
mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use synveda_types::{Error, Result};

    use super::{ConsumedHandoff, ConsumedPending, LoginLedger};

    const PENDING_TTL: Duration = Duration::from_secs(600);
    const HANDOFF_TTL: Duration = Duration::from_secs(60);
    const CAP: usize = 10_000;

    struct Pending {
        correlation_hash: Option<[u8; 32]>,
        payload: Vec<u8>,
        expires_at: Instant,
    }

    struct Handoff {
        state_hash: [u8; 32],
        payload: Vec<u8>,
        expires_at: Instant,
    }

    /// Test-only ledger for protocol unit tests and mock-IdP route tests.
    #[derive(Default)]
    pub struct MemoryLoginLedger {
        pending: Mutex<HashMap<[u8; 32], Pending>>,
        handoffs: Mutex<HashMap<[u8; 32], Handoff>>,
    }

    impl MemoryLoginLedger {
        /// Constructs an empty test ledger.
        pub fn new() -> Self {
            Self::default()
        }

        #[cfg(test)]
        pub(crate) fn expire_pending(&self, state_hash: [u8; 32]) {
            if let Ok(mut rows) = self.pending.lock()
                && let Some(row) = rows.get_mut(&state_hash)
            {
                row.expires_at = Instant::now() - Duration::from_secs(1);
            }
        }

        #[cfg(test)]
        pub(crate) fn expire_handoff(&self, code_hash: [u8; 32]) {
            if let Ok(mut rows) = self.handoffs.lock()
                && let Some(row) = rows.get_mut(&code_hash)
            {
                row.expires_at = Instant::now() - Duration::from_secs(1);
            }
        }
    }

    fn lock_error() -> Error {
        Error::Internal {
            message: "test login ledger lock was poisoned".to_owned(),
        }
    }

    fn matches_binding(stored: Option<[u8; 32]>, presented: Option<[u8; 32]>) -> bool {
        stored.is_none() || stored == presented
    }

    #[async_trait]
    impl LoginLedger for MemoryLoginLedger {
        async fn park_pending(
            &self,
            state_hash: [u8; 32],
            correlation_hash: Option<[u8; 32]>,
            payload: Vec<u8>,
        ) -> Result<()> {
            let mut rows = self.pending.lock().map_err(|_| lock_error())?;
            let now = Instant::now();
            rows.retain(|_, row| row.expires_at > now);
            if rows.len() >= CAP {
                return Err(Error::RateLimited {
                    message: "too many logins in flight; retry shortly".to_owned(),
                });
            }
            rows.insert(
                state_hash,
                Pending {
                    correlation_hash,
                    payload,
                    expires_at: now + PENDING_TTL,
                },
            );
            Ok(())
        }

        async fn peek_pending(
            &self,
            state_hash: [u8; 32],
            presented_correlation_hash: Option<[u8; 32]>,
        ) -> Result<Option<Vec<u8>>> {
            let rows = self.pending.lock().map_err(|_| lock_error())?;
            Ok(rows
                .get(&state_hash)
                .filter(|row| matches_binding(row.correlation_hash, presented_correlation_hash))
                .map(|row| row.payload.clone()))
        }

        async fn consume_pending(
            &self,
            state_hash: [u8; 32],
            presented_correlation_hash: Option<[u8; 32]>,
        ) -> Result<Option<ConsumedPending>> {
            let mut rows = self.pending.lock().map_err(|_| lock_error())?;
            if !rows.get(&state_hash).is_some_and(|row| {
                matches_binding(row.correlation_hash, presented_correlation_hash)
            }) {
                return Ok(None);
            }
            Ok(rows.remove(&state_hash).map(|row| ConsumedPending {
                payload: row.payload,
                expired: row.expires_at <= Instant::now(),
            }))
        }

        async fn abandon_pending(
            &self,
            state_hash: [u8; 32],
            presented_correlation_hash: Option<[u8; 32]>,
        ) -> Result<bool> {
            let mut rows = self.pending.lock().map_err(|_| lock_error())?;
            if !rows.get(&state_hash).is_some_and(|row| {
                matches_binding(row.correlation_hash, presented_correlation_hash)
            }) {
                return Ok(false);
            }
            Ok(rows.remove(&state_hash).is_some())
        }

        async fn park_handoff(
            &self,
            code_hash: [u8; 32],
            state_hash: [u8; 32],
            payload: Vec<u8>,
        ) -> Result<()> {
            let mut rows = self.handoffs.lock().map_err(|_| lock_error())?;
            let now = Instant::now();
            rows.retain(|_, row| row.expires_at > now);
            if rows.len() >= CAP {
                return Err(Error::RateLimited {
                    message: "too many logins in flight; retry shortly".to_owned(),
                });
            }
            rows.insert(
                code_hash,
                Handoff {
                    state_hash,
                    payload,
                    expires_at: now + HANDOFF_TTL,
                },
            );
            Ok(())
        }

        async fn consume_handoff(&self, code_hash: [u8; 32]) -> Result<Option<ConsumedHandoff>> {
            let mut rows = self.handoffs.lock().map_err(|_| lock_error())?;
            Ok(rows.remove(&code_hash).map(|row| ConsumedHandoff {
                state_hash: row.state_hash,
                payload: row.payload,
                expired: row.expires_at <= Instant::now(),
            }))
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use memory::MemoryLoginLedger;
