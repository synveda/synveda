//! Deployment-key sealed PostgreSQL login ledger (OPS-7, ADR-0126).

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use synveda_crypto::{KeyScope, Purpose, RowKey};
use synveda_identity::{ConsumedHandoff, ConsumedPending, LoginLedger};
use synveda_store::{keys::KeyRing, one_time_login};
use synveda_types::Result;

/// Production, cross-process one-time state for OIDC and CLI handoff.
pub struct PostgresLoginLedger {
    pool: PgPool,
    keys: Arc<KeyRing>,
}

impl PostgresLoginLedger {
    /// Uses the ordinary gateway role and the deployment key plane.
    pub fn new(pool: PgPool, keys: Arc<KeyRing>) -> Self {
        Self { pool, keys }
    }

    async fn seal(&self, purpose: Purpose, hash: &[u8; 32], payload: &[u8]) -> Result<Vec<u8>> {
        self.keys
            .sealing_key(&self.pool, KeyScope::Deployment)
            .await?
            .seal(purpose, RowKey::Hash(hash), payload)
    }

    async fn open(&self, purpose: Purpose, hash: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>> {
        let opened = self
            .keys
            .opening_key(&self.pool, KeyScope::Deployment, sealed)
            .await?
            .open(purpose, RowKey::Hash(hash), sealed)
            .inspect_err(|error| {
                metrics::counter!(
                    synveda_store::keys::KEY_OPEN_FAILURES_TOTAL,
                    "scope" => "deployment",
                    "purpose" => purpose.as_str(),
                )
                .increment(1);
                tracing::warn!(%error, purpose = purpose.as_str(), "one-time login payload did not open");
            })?;
        Ok(opened.to_vec())
    }
}

#[async_trait]
impl LoginLedger for PostgresLoginLedger {
    async fn park_pending(
        &self,
        state_hash: [u8; 32],
        correlation_hash: Option<[u8; 32]>,
        payload: Vec<u8>,
    ) -> Result<()> {
        let sealed = self
            .seal(Purpose::PendingLogin, &state_hash, &payload)
            .await?;
        one_time_login::park_pending(&self.pool, &state_hash, correlation_hash.as_ref(), &sealed)
            .await
    }

    async fn peek_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>> {
        match one_time_login::peek_pending(
            &self.pool,
            &state_hash,
            presented_correlation_hash.as_ref(),
        )
        .await?
        {
            Some(sealed) => self
                .open(Purpose::PendingLogin, &state_hash, &sealed)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn consume_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<Option<ConsumedPending>> {
        match one_time_login::consume_pending(
            &self.pool,
            &state_hash,
            presented_correlation_hash.as_ref(),
        )
        .await?
        {
            Some(row) => Ok(Some(ConsumedPending {
                payload: self
                    .open(Purpose::PendingLogin, &state_hash, &row.payload_sealed)
                    .await?,
                expired: row.expired,
            })),
            None => Ok(None),
        }
    }

    async fn abandon_pending(
        &self,
        state_hash: [u8; 32],
        presented_correlation_hash: Option<[u8; 32]>,
    ) -> Result<bool> {
        one_time_login::abandon_pending(
            &self.pool,
            &state_hash,
            presented_correlation_hash.as_ref(),
        )
        .await
    }

    async fn park_handoff(
        &self,
        code_hash: [u8; 32],
        state_hash: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<()> {
        let sealed = self.seal(Purpose::CliHandoff, &code_hash, &payload).await?;
        one_time_login::park_handoff(&self.pool, &code_hash, &state_hash, &sealed).await
    }

    async fn consume_handoff(&self, code_hash: [u8; 32]) -> Result<Option<ConsumedHandoff>> {
        match one_time_login::consume_handoff(&self.pool, &code_hash).await? {
            Some(row) => Ok(Some(ConsumedHandoff {
                state_hash: row.state_hash,
                payload: self
                    .open(Purpose::CliHandoff, &code_hash, &row.payload_sealed)
                    .await?,
                expired: row.expired,
            })),
            None => Ok(None),
        }
    }
}
