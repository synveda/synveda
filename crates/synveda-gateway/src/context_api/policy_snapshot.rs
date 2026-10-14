//! Exact request-local policy over current database sources (ADR-0145).

use std::sync::Arc;

use sqlx::PgConnection;
use synveda_store::policy_packs;
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;

static SNAPSHOT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(crate) async fn current(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<AppState> {
    let snapshot = current_pdp(&state.pdp, tx, tenant).await?;
    let mut frozen = state.clone();
    frozen.pdp = snapshot;
    Ok(frozen)
}

/// The same bounded frozen engine for HTTP admission and durable workers.
pub(crate) async fn current_pdp(
    process: &Arc<synveda_policy::Pdp>,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<Arc<synveda_policy::Pdp>> {
    let slot = SNAPSHOT_SLOTS.try_acquire().map_err(|_| unavailable())?;
    let sources =
        policy_packs::for_snapshot(&mut *tx, tenant)
            .await
            .map_err(|error| match error {
                Error::Invalid { .. } => unavailable(),
                other => other,
            })?;
    let process = Arc::clone(process);
    // Local bounded Cedar compilation can outlive a cancelled request. Keep
    // its concurrency permit in that CPU task until the work actually ends.
    let snapshot = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        process.frozen_snapshot(
            tenant,
            sources.iter().map(|pack| {
                (
                    pack.name.as_str(),
                    pack.version,
                    pack.source.as_str(),
                    &pack.config,
                )
            }),
        )
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    Ok(Arc::new(snapshot))
}

pub(super) fn unavailable() -> Error {
    Error::Conflict {
        message: "Saved context delivery is unavailable under current policy or source state; use a new idempotency key to compose current context".to_owned(),
    }
}
