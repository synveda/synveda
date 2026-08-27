//! The key plane: wrapped data keys, and the ring that materialises them
//! (TEN-4, ADR-0064).
//!
//! Two scopes, two tables, one ring. `tenant_keys` is what "per-tenant
//! encryption keys" names; `deployment_keys` exists because
//! `console_sessions` structurally cannot select a per-tenant key — see
//! migration 0038's header and ADR-0064 decision 5.
//!
//! Nothing here ever holds a key in the database. A row carries the data key
//! *wrapped* by the KEK, and materialising a [`SealingKey`] is an unwrap
//! through the [`Kms`] seam. Unwrapped keys are cached for a bounded time and
//! wiped when the last holder drops them.
//!
//! **The cache is per process.** A rotation on one replica is invisible to
//! another until its entry expires, which is the same staleness shape the
//! PDP's entity fragments have and lands in the same place: OPS-7 owns
//! cross-process invalidation, and this cache is named in it rather than
//! growing a second transport of its own. The bound is [`KeyRing::ttl`], and until OPS-7 the
//! chart pins one gateway replica anyway.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use sqlx::{PgExecutor, PgPool};
use synveda_crypto::{DataKey, KeyManagement, KeyScope, KeyVersion, Kms, SealingKey};
use synveda_types::{Error, Result, TenantId};

use crate::rls::begin_tenant_tx;

/// Counter: data keys unwrapped through the KMS, labelled `scope`. A rate
/// that tracks request rate rather than sitting near zero means the cache is
/// not doing its job — which is the signal ADR-0064's KMS-latency reversal
/// trigger reads.
pub const KEY_UNWRAPS_TOTAL: &str = "synveda_key_unwraps_total";

/// Counter: key-ring lookups, labelled `scope` and `outcome` = `hit` | `miss`.
pub const KEY_CACHE_LOOKUPS_TOTAL: &str = "synveda_key_cache_lookups_total";

/// Counter: data keys minted, labelled `scope` and `reason` = `provision` |
/// `rotate`.
pub const KEYS_MINTED_TOTAL: &str = "synveda_keys_minted_total";

/// Counter: sealed payloads that did not open, labelled `scope` and
/// `purpose`.
///
/// Under ADR-0064 decision 4 this is what a cross-tenant transplant looks
/// like, and it is also what corruption and a missing key look like. Anything
/// but zero is worth an alert: nothing in normal operation fails to open.
/// Emitted by the callers that hold the plaintext's meaning, because only
/// they know whether a failure is a 401 or a 500.
pub const KEY_OPEN_FAILURES_TOTAL: &str = "synveda_key_open_failures_total";

/// The algorithm recorded beside a wrapped key. Advisory — the envelope
/// header is authoritative — and kept in step with migration 0038's check
/// constraint.
const ALGORITHM: &str = "xchacha20-poly1305";

/// How long an unwrapped key stays materialised before it is dropped and
/// re-unwrapped. Short enough that a rotation propagates without an operator
/// waiting on it, long enough that a KMS is not on the request path.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation: two writers minted a generation for the
        // same scope. The loser retries and finds the winner's key.
        if db.code().as_deref() == Some("23505") {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// One stored generation of one scope's data key, as it sits in the table.
#[derive(Debug, Clone)]
pub struct StoredKey {
    /// Which generation this is.
    pub version: KeyVersion,
    /// The data key, sealed by the KEK named in [`Self::kek_ref`].
    pub wrapped_dek: Vec<u8>,
    /// Which KEK wrapped it.
    pub kek_ref: String,
    /// Whether a newer generation has superseded it. A retired key still
    /// opens everything sealed under it — that is the whole of why rotation
    /// can be lazy.
    pub retired: bool,
}

// ── Row access ──────────────────────────────────────────────────────────────
//
// Two tables rather than one with a nullable `tenant_id`, so the RLS
// completeness guard's structural discovery keeps working: a table with a
// `tenant_id` gets a tenant predicate, and a nullable one would fail that
// predicate on exactly the deployment row that must be readable before any
// tenant exists.

/// The deployment's current (un-retired) key, if it has been provisioned.
pub async fn deployment_current(executor: impl PgExecutor<'_>) -> Result<Option<StoredKey>> {
    let row = sqlx::query!(
        r#"
        select version, wrapped_dek, kek_ref, retired_at
        from deployment_keys
        where retired_at is null
        "#
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StoredKey {
            version: KeyVersion::from_i32(row.version)?,
            wrapped_dek: row.wrapped_dek,
            kek_ref: row.kek_ref,
            retired: row.retired_at.is_some(),
        })
    })
    .transpose()
}

/// One generation of the deployment key, retired or not.
pub async fn deployment_at(
    executor: impl PgExecutor<'_>,
    version: KeyVersion,
) -> Result<Option<StoredKey>> {
    let row = sqlx::query!(
        r#"
        select version, wrapped_dek, kek_ref, retired_at
        from deployment_keys
        where version = $1
        "#,
        version.as_i32(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StoredKey {
            version: KeyVersion::from_i32(row.version)?,
            wrapped_dek: row.wrapped_dek,
            kek_ref: row.kek_ref,
            retired: row.retired_at.is_some(),
        })
    })
    .transpose()
}

/// A tenant's current (un-retired) key. Runs inside the caller's tenant
/// transaction, so RLS applies.
pub async fn tenant_current(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<StoredKey>> {
    let row = sqlx::query!(
        r#"
        select version, wrapped_dek, kek_ref, retired_at
        from tenant_keys
        where tenant_id = $1 and retired_at is null
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StoredKey {
            version: KeyVersion::from_i32(row.version)?,
            wrapped_dek: row.wrapped_dek,
            kek_ref: row.kek_ref,
            retired: row.retired_at.is_some(),
        })
    })
    .transpose()
}

/// One generation of a tenant's key, retired or not.
pub async fn tenant_at(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    version: KeyVersion,
) -> Result<Option<StoredKey>> {
    let row = sqlx::query!(
        r#"
        select version, wrapped_dek, kek_ref, retired_at
        from tenant_keys
        where tenant_id = $1 and version = $2
        "#,
        tenant_id.as_uuid(),
        version.as_i32(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StoredKey {
            version: KeyVersion::from_i32(row.version)?,
            wrapped_dek: row.wrapped_dek,
            kek_ref: row.kek_ref,
            retired: row.retired_at.is_some(),
        })
    })
    .transpose()
}

async fn deployment_insert(
    executor: impl PgExecutor<'_>,
    version: KeyVersion,
    wrapped_dek: &[u8],
    kek_ref: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into deployment_keys (version, wrapped_dek, kek_ref, algorithm)
        values ($1, $2, $3, $4)
        "#,
        version.as_i32(),
        wrapped_dek,
        kek_ref,
        ALGORITHM,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn tenant_insert(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    version: KeyVersion,
    wrapped_dek: &[u8],
    kek_ref: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into tenant_keys (tenant_id, version, wrapped_dek, kek_ref, algorithm)
        values ($1, $2, $3, $4, $5)
        "#,
        tenant_id.as_uuid(),
        version.as_i32(),
        wrapped_dek,
        kek_ref,
        ALGORITHM,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn deployment_retire(executor: impl PgExecutor<'_>, version: KeyVersion) -> Result<()> {
    sqlx::query!(
        "update deployment_keys set retired_at = now() where version = $1 and retired_at is null",
        version.as_i32(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn tenant_retire(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    version: KeyVersion,
) -> Result<()> {
    sqlx::query!(
        r#"
        update tenant_keys set retired_at = now()
        where tenant_id = $1 and version = $2 and retired_at is null
        "#,
        tenant_id.as_uuid(),
        version.as_i32(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

// ── The ring ────────────────────────────────────────────────────────────────

/// A materialised key and when it stops being trusted.
struct Cached {
    key: Arc<SealingKey>,
    expires_at: Instant,
}

/// Which generation is current for a scope, and when to re-ask.
struct CurrentVersion {
    version: KeyVersion,
    expires_at: Instant,
}

/// Materialises sealing keys: reads a wrapped key, unwraps it through the
/// KMS, and holds it for a bounded time.
///
/// One per process, shared by every request. A warm resolve is a read lock
/// and an `Arc` clone; a cold one is a query and one KMS call.
pub struct KeyRing {
    kms: Kms,
    ttl: Duration,
    keys: RwLock<HashMap<(KeyScope, u32), Cached>>,
    current: RwLock<HashMap<KeyScope, CurrentVersion>>,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRing")
            .field("kms", &self.kms)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl KeyRing {
    /// A ring over `kms`, caching unwrapped keys for [`DEFAULT_TTL`].
    #[must_use]
    pub fn new(kms: Kms) -> Self {
        KeyRing::with_ttl(kms, DEFAULT_TTL)
    }

    /// A ring with an explicit cache lifetime. A zero TTL means every
    /// resolve unwraps — correct, slow, and what a test that is about
    /// rotation wants.
    #[must_use]
    pub fn with_ttl(kms: Kms, ttl: Duration) -> Self {
        KeyRing {
            kms,
            ttl,
            keys: RwLock::new(HashMap::new()),
            current: RwLock::new(HashMap::new()),
        }
    }

    /// How long this ring holds an unwrapped key.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// The KMS behind this ring, for the `method` and `key_ref` an audit
    /// payload records.
    #[must_use]
    pub const fn kms(&self) -> &Kms {
        &self.kms
    }

    /// The key that seals new payloads for `scope`.
    ///
    /// Fails when the scope has no key: provisioning is an explicit act
    /// (`provision`, or `tenant create`), never something a seal does on the
    /// way past. A seal that mints its own key is a seal that silently
    /// succeeds against a key nobody recorded.
    #[tracing::instrument(
        name = "store.keys.sealing_key",
        skip_all,
        fields(key.scope = scope.label()),
        err(Display)
    )]
    pub async fn sealing_key(&self, pool: &PgPool, scope: KeyScope) -> Result<Arc<SealingKey>> {
        let version = self.current_version(pool, scope).await?;
        self.key_at(pool, scope, version).await
    }

    /// The key that opens `envelope` — the generation its header names.
    ///
    /// This is what makes rotation lazy (ADR-0064 decision 6): a payload
    /// sealed three generations ago opens under the key it names, and gets
    /// re-sealed under the current one whenever its row is next written, or
    /// never.
    ///
    /// The envelope's scope tag is checked against `scope` **here**, before a
    /// row is read or a KMS is called. `SealingKey::open` would catch the
    /// mismatch anyway — that check is the load-bearing one and it stays — but
    /// catching it first turns "unwrap a key, then fail to open" into one
    /// comparison, and gives the caller an error that names the disagreement
    /// rather than the uniform "did not open".
    #[tracing::instrument(
        name = "store.keys.opening_key",
        skip_all,
        fields(key.scope = scope.label(), key.version = tracing::field::Empty),
        err(Display)
    )]
    pub async fn opening_key(
        &self,
        pool: &PgPool,
        scope: KeyScope,
        envelope: &[u8],
    ) -> Result<Arc<SealingKey>> {
        let wants_deployment = matches!(scope, KeyScope::Deployment);
        match synveda_crypto::envelope_is_deployment_scoped(envelope) {
            Some(is_deployment) if is_deployment != wants_deployment => {
                return Err(Error::Invalid {
                    message: format!(
                        "sealed payload is {}-scoped and {scope} was asked for it",
                        if is_deployment {
                            "deployment"
                        } else {
                            "tenant"
                        }
                    ),
                });
            }
            // `None` is a scope tag this version does not define. Fall
            // through: the header parse inside `envelope_version` refuses it
            // with a message about the format, which is the more useful error
            // than one about scopes.
            _ => {}
        }
        let version = synveda_crypto::envelope_version(envelope)?;
        tracing::Span::current().record("key.version", version.get());
        self.key_at(pool, scope, version).await
    }

    /// Mints the first key for `scope`, or returns the one already there.
    ///
    /// Idempotent, because it is called from tenant admission and from an
    /// operator's backfill, and neither should care which ran first.
    #[tracing::instrument(
        name = "store.keys.provision",
        skip_all,
        fields(key.scope = scope.label()),
        err(Display)
    )]
    pub async fn provision(&self, pool: &PgPool, scope: KeyScope) -> Result<KeyVersion> {
        if let Some(existing) = self.stored_current(pool, scope).await? {
            return Ok(existing.version);
        }
        let version = KeyVersion::FIRST;
        let wrapped = self.wrap_fresh(scope).await?;
        match scope {
            KeyScope::Deployment => {
                deployment_insert(pool, version, &wrapped, self.kms.key_ref()).await
            }
            KeyScope::Tenant(tenant_id) => {
                let mut tx = begin_tenant_tx(pool, tenant_id).await?;
                tenant_insert(&mut *tx, tenant_id, version, &wrapped, self.kms.key_ref()).await?;
                tx.commit().await.map_err(storage_error)
            }
        }
        // A conflict means somebody else provisioned between the read and the
        // insert, which is the outcome this function promises anyway.
        .or_else(|err| match err {
            Error::Conflict { .. } => Ok(()),
            other => Err(other),
        })?;
        metrics::counter!(
            KEYS_MINTED_TOTAL,
            "scope" => scope.label(),
            "reason" => "provision",
        )
        .increment(1);
        Ok(version)
    }

    /// Retires the current key and mints the next generation, atomically.
    ///
    /// Nothing is re-sealed here. Payloads under the retired key keep opening
    /// under it — [`Self::opening_key`] finds it by the version in their
    /// header — and move to the new key when their rows are next written.
    /// Re-sealing everything eagerly is the stop-the-world rotation ADR-0064
    /// decision 6 exists to avoid.
    #[tracing::instrument(
        name = "store.keys.rotate",
        skip_all,
        fields(key.scope = scope.label(), key.version = tracing::field::Empty),
        err(Display)
    )]
    pub async fn rotate(&self, pool: &PgPool, scope: KeyScope) -> Result<KeyVersion> {
        let current = self.stored_current(pool, scope).await?.ok_or_else(|| {
            // Rotating a scope with no key would mint one and call it a
            // rotation, which is a different act with different audit.
            Error::NotFound {
                entity: format!("current key for {scope}"),
            }
        })?;
        let next = current.version.next();
        let wrapped = self.wrap_fresh(scope).await?;
        match scope {
            KeyScope::Deployment => {
                let mut tx = pool.begin().await.map_err(storage_error)?;
                deployment_retire(&mut *tx, current.version).await?;
                deployment_insert(&mut *tx, next, &wrapped, self.kms.key_ref()).await?;
                tx.commit().await.map_err(storage_error)?;
            }
            KeyScope::Tenant(tenant_id) => {
                let mut tx = begin_tenant_tx(pool, tenant_id).await?;
                tenant_retire(&mut *tx, tenant_id, current.version).await?;
                tenant_insert(&mut *tx, tenant_id, next, &wrapped, self.kms.key_ref()).await?;
                tx.commit().await.map_err(storage_error)?;
            }
        }
        // Drop the "which version is current" answer immediately in this
        // process. Other replicas keep the old answer until their TTL — the
        // staleness OPS-7 owns — and it is safe rather than merely tolerated:
        // they seal under a key that is retired, not gone, and every reader
        // still finds it by version.
        self.forget_current(scope);
        tracing::Span::current().record("key.version", next.get());
        metrics::counter!(
            KEYS_MINTED_TOTAL,
            "scope" => scope.label(),
            "reason" => "rotate",
        )
        .increment(1);
        Ok(next)
    }

    /// Drops every materialised key for `scope`, wiping the material when the
    /// last holder lets go. An operator's blunt instrument, and what a test
    /// about rotation uses instead of waiting out a TTL.
    pub fn invalidate(&self, scope: KeyScope) {
        self.forget_current(scope);
        let mut keys = self.keys.write().unwrap_or_else(PoisonError::into_inner);
        keys.retain(|(cached, _), _| *cached != scope);
    }

    /// Mints a data key and wraps it, without touching the database. The
    /// plaintext key exists for the length of this call and nowhere else.
    async fn wrap_fresh(&self, scope: KeyScope) -> Result<Vec<u8>> {
        let key = DataKey::generate()?;
        self.kms.wrap_key(scope, &key).await
    }

    async fn stored_current(&self, pool: &PgPool, scope: KeyScope) -> Result<Option<StoredKey>> {
        match scope {
            KeyScope::Deployment => deployment_current(pool).await,
            KeyScope::Tenant(tenant_id) => {
                let mut tx = begin_tenant_tx(pool, tenant_id).await?;
                let row = tenant_current(&mut *tx, tenant_id).await?;
                tx.commit().await.map_err(storage_error)?;
                Ok(row)
            }
        }
    }

    async fn stored_at(
        &self,
        pool: &PgPool,
        scope: KeyScope,
        version: KeyVersion,
    ) -> Result<Option<StoredKey>> {
        match scope {
            KeyScope::Deployment => deployment_at(pool, version).await,
            KeyScope::Tenant(tenant_id) => {
                let mut tx = begin_tenant_tx(pool, tenant_id).await?;
                let row = tenant_at(&mut *tx, tenant_id, version).await?;
                tx.commit().await.map_err(storage_error)?;
                Ok(row)
            }
        }
    }

    async fn current_version(&self, pool: &PgPool, scope: KeyScope) -> Result<KeyVersion> {
        if let Some(version) = self.cached_current(scope) {
            return Ok(version);
        }
        let stored = self
            .stored_current(pool, scope)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("encryption key for {scope} (provision one before sealing)"),
            })?;
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        current.insert(
            scope,
            CurrentVersion {
                version: stored.version,
                expires_at: Instant::now() + self.ttl,
            },
        );
        Ok(stored.version)
    }

    fn cached_current(&self, scope: KeyScope) -> Option<KeyVersion> {
        let current = self.current.read().unwrap_or_else(PoisonError::into_inner);
        current
            .get(&scope)
            .filter(|entry| entry.expires_at > Instant::now())
            .map(|entry| entry.version)
    }

    fn forget_current(&self, scope: KeyScope) {
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        current.remove(&scope);
    }

    async fn key_at(
        &self,
        pool: &PgPool,
        scope: KeyScope,
        version: KeyVersion,
    ) -> Result<Arc<SealingKey>> {
        if let Some(key) = self.cached_key(scope, version) {
            metrics::counter!(
                KEY_CACHE_LOOKUPS_TOTAL,
                "scope" => scope.label(),
                "outcome" => "hit",
            )
            .increment(1);
            return Ok(key);
        }
        metrics::counter!(
            KEY_CACHE_LOOKUPS_TOTAL,
            "scope" => scope.label(),
            "outcome" => "miss",
        )
        .increment(1);

        let stored =
            self.stored_at(pool, scope, version)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("key for {scope} at version {version}"),
                })?;
        let data_key = self.kms.unwrap_key(scope, &stored.wrapped_dek).await?;
        metrics::counter!(KEY_UNWRAPS_TOTAL, "scope" => scope.label()).increment(1);
        let key = Arc::new(SealingKey::new(scope, stored.version, data_key));

        let mut keys = self.keys.write().unwrap_or_else(PoisonError::into_inner);
        // Drop entries nobody will use again on the way past. This is the
        // whole eviction policy: the map is bounded by tenant count times
        // live generations, and a sweep here costs nothing a KMS call has not
        // already dwarfed.
        let now = Instant::now();
        keys.retain(|_, cached| cached.expires_at > now);
        keys.insert(
            (scope, version.get()),
            Cached {
                key: Arc::clone(&key),
                expires_at: now + self.ttl,
            },
        );
        Ok(key)
    }

    fn cached_key(&self, scope: KeyScope, version: KeyVersion) -> Option<Arc<SealingKey>> {
        let keys = self.keys.read().unwrap_or_else(PoisonError::into_inner);
        keys.get(&(scope, version.get()))
            .filter(|cached| cached.expires_at > Instant::now())
            .map(|cached| Arc::clone(&cached.key))
    }
}
