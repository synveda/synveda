//! Exact request-local policy over current database sources (ADR-0145).

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgConnection;
use synveda_store::policy_packs;
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;

static SNAPSHOT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn snapshot_slot(
    slots: &tokio::sync::Semaphore,
    wait: Duration,
) -> Result<tokio::sync::SemaphorePermit<'_>> {
    // Brief contention is expected with concurrent readers. Bound the wait
    // without widening construction concurrency or bypassing current policy.
    tokio::time::timeout(wait, slots.acquire())
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())
}

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
    let slot = snapshot_slot(&SNAPSHOT_SLOTS, Duration::from_secs(5)).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn contending_readers_wait_for_an_existing_slot() {
        let slots = tokio::sync::Semaphore::new(2);
        let occupied = slots.acquire_many(2).await.unwrap();
        let waiting = snapshot_slot(&slots, Duration::from_secs(5));
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            _ = &mut waiting => panic!("an occupied slot must wait"),
            () = tokio::task::yield_now() => {}
        }
        drop(occupied);
        let admitted = waiting.await.unwrap();
        assert_eq!(slots.available_permits(), 1);
        drop(admitted);
        assert_eq!(slots.available_permits(), 2);
    }

    #[tokio::test]
    async fn saturated_snapshot_wait_is_bounded_and_content_free() {
        let slots = tokio::sync::Semaphore::new(2);
        let _occupied = slots.acquire_many(2).await.unwrap();
        let error = snapshot_slot(&slots, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Conflict { .. }));
        assert_eq!(error.to_string(), unavailable().to_string());
        assert_eq!(slots.available_permits(), 0);
    }

    #[tokio::test]
    async fn cancelled_wait_does_not_consume_or_release_another_readers_slot() {
        let slots = tokio::sync::Semaphore::new(2);
        let occupied = slots.acquire_many(2).await.unwrap();
        {
            let waiting = snapshot_slot(&slots, Duration::from_secs(5));
            tokio::pin!(waiting);
            tokio::select! {
                biased;
                _ = &mut waiting => panic!("an occupied slot must wait"),
                () = tokio::task::yield_now() => {}
            }
        }
        assert_eq!(slots.available_permits(), 0);
        drop(occupied);
        let _both = slots.try_acquire_many(2).unwrap();
    }
}
