//! Exact request-local policy over current database sources (ADR-0145).

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgConnection;
use synveda_store::policy_packs;
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;

static SNAPSHOT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// A snapshot either refuses without saying why or fails for an operator-side
/// reason that keeps its own classification.
enum SnapshotError {
    Refused,
    Failed(Error),
}

impl SnapshotError {
    fn into_error(self, refusal: impl FnOnce() -> Error) -> Error {
        match self {
            SnapshotError::Refused => refusal(),
            SnapshotError::Failed(error) => error,
        }
    }
}

async fn snapshot_slot(
    slots: &tokio::sync::Semaphore,
    wait: Duration,
) -> std::result::Result<tokio::sync::SemaphorePermit<'_>, SnapshotError> {
    // Brief contention is expected with concurrent readers. Bound the wait
    // without widening construction concurrency or bypassing current policy.
    tokio::time::timeout(wait, slots.acquire())
        .await
        .map_err(|_| SnapshotError::Refused)?
        .map_err(|_| SnapshotError::Refused)
}

/// Request state whose engine is exactly the tenant's current stored policy.
pub(crate) async fn current(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<AppState> {
    let snapshot = current_pdp(&state.pdp, tx, tenant).await?;
    Ok(with_pdp(state, snapshot))
}

/// [`current`] for saved-delivery replay, whose refusal keeps the original
/// receipt and names the replay remedy (ADR-0141).
pub(super) async fn for_replay(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<AppState> {
    let snapshot = snapshot(&state.pdp, tx, tenant)
        .await
        .map_err(|error| error.into_error(unavailable))?;
    Ok(with_pdp(state, snapshot))
}

/// The same bounded frozen engine for HTTP admission and durable workers.
pub(crate) async fn current_pdp(
    process: &Arc<synveda_policy::Pdp>,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> Result<Arc<synveda_policy::Pdp>> {
    snapshot(process, tx, tenant)
        .await
        .map_err(|error| error.into_error(policy_unavailable))
}

fn with_pdp(state: &AppState, pdp: Arc<synveda_policy::Pdp>) -> AppState {
    let mut frozen = state.clone();
    frozen.pdp = pdp;
    frozen
}

async fn snapshot(
    process: &Arc<synveda_policy::Pdp>,
    tx: &mut PgConnection,
    tenant: TenantId,
) -> std::result::Result<Arc<synveda_policy::Pdp>, SnapshotError> {
    let slot = snapshot_slot(&SNAPSHOT_SLOTS, Duration::from_secs(5)).await?;
    let sources =
        policy_packs::for_snapshot(&mut *tx, tenant)
            .await
            .map_err(|error| match error {
                Error::Invalid { .. } => SnapshotError::Refused,
                other => SnapshotError::Failed(other),
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
    .map_err(|_| SnapshotError::Refused)?
    .map_err(|_| SnapshotError::Refused)?;
    Ok(Arc::new(snapshot))
}

/// Ordinary admission without valid current tenant policy. It names neither
/// a pack nor the cause, and a retry under a new key repairs nothing.
fn policy_unavailable() -> Error {
    Error::Conflict {
        message: "Current tenant policy cannot be evaluated, so the request was refused; if this persists, an operator must repair the tenant's stored policy".to_owned(),
    }
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
        let Ok(admitted) = waiting.await else {
            panic!("a released slot must admit the waiting reader");
        };
        assert_eq!(slots.available_permits(), 1);
        drop(admitted);
        assert_eq!(slots.available_permits(), 2);
    }

    #[tokio::test]
    async fn saturated_snapshot_wait_is_bounded_and_content_free() {
        let slots = tokio::sync::Semaphore::new(2);
        let _occupied = slots.acquire_many(2).await.unwrap();
        let Err(error) = snapshot_slot(&slots, Duration::from_millis(1)).await else {
            panic!("a saturated slot must refuse after its bound");
        };
        assert!(matches!(error, SnapshotError::Refused));
        assert_eq!(slots.available_permits(), 0);
    }

    #[test]
    fn ordinary_and_replay_refusals_keep_their_own_remedies() {
        let ordinary = SnapshotError::Refused.into_error(policy_unavailable);
        let replay = SnapshotError::Refused.into_error(unavailable);
        assert!(matches!(ordinary, Error::Conflict { .. }));
        assert!(matches!(replay, Error::Conflict { .. }));
        let ordinary = ordinary.to_string();
        for misleading in ["idempotency", "saved", "Saved", "compose"] {
            assert!(!ordinary.contains(misleading), "{ordinary}");
        }
        assert_eq!(replay.to_string(), unavailable().to_string());
        let storage = Error::Storage {
            message: "connection reset".to_owned(),
        };
        assert!(matches!(
            SnapshotError::Failed(storage).into_error(policy_unavailable),
            Error::Storage { .. }
        ));
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
