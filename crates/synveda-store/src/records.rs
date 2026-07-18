//! Bitemporal storage for memory records (FND-4, ADR-0006).
//!
//! A record is updated by writing a complete new version: the trigger layer
//! archives the previous version into `records_history` with its transaction
//! period closed, and stamps the new one. [`delete`] is a *temporal* delete —
//! the record stops being current but every closed version stays queryable
//! through [`as_of`]. Nothing in this module reads or writes `tx_from`/`tx_to`
//! directly; those columns belong to the triggers.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    Error, IdentityId, RecordClass, RecordId, RecordKind, Result, ScopeId, Sensitivity, TenantId,
};
use uuid::Uuid;

/// The mutable portion of a record: everything except its identity
/// (`id`, `tenant_id`) and the trigger-owned transaction time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordState {
    /// Hierarchy node the record attaches to (seed §4.1). Mutable: a mover's
    /// records may re-scope (AUTH-4).
    pub scope_id: ScopeId,
    /// User or service identity that owns the record.
    pub owner_id: IdentityId,
    /// Derived or pinned (seed §4.2).
    pub kind: RecordKind,
    /// What the record asserts (fact, decision, ...).
    pub class: RecordClass,
    /// The summarised content.
    pub content: String,
    /// Sensitivity classification; drives policy.
    pub sensitivity: Sensitivity,
    /// Source session, extraction method, model version, confidence — shape
    /// is owned by the extraction pipeline (MEM-3).
    pub provenance: serde_json::Value,
    /// When the fact started holding in the world (valid time).
    pub valid_from: DateTime<Utc>,
    /// When it stopped holding; `None` = no known end.
    pub valid_to: Option<DateTime<Utc>>,
}

/// One version of a record as stored: its state plus identity and the
/// transaction period during which the database held this version as truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordVersion {
    /// Record identifier; stable across versions.
    pub id: RecordId,
    /// Owning tenant; immutable for the life of the record.
    pub tenant_id: TenantId,
    /// The version's state.
    pub state: RecordState,
    /// When the database started holding this version (trigger-stamped).
    pub tx_from: DateTime<Utc>,
    /// When it stopped; `None` = this is the current version.
    pub tx_to: Option<DateTime<Utc>>,
}

/// Raw row shared by every query in this module; converted with `TryFrom`
/// so vocabulary columns decode through the `synveda-types` enums.
struct RecordRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    owner_id: Uuid,
    kind: String,
    class: String,
    content: String,
    sensitivity: String,
    provenance: serde_json::Value,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    tx_from: DateTime<Utc>,
    tx_to: Option<DateTime<Utc>>,
}

impl TryFrom<RecordRow> for RecordVersion {
    type Error = Error;

    fn try_from(row: RecordRow) -> Result<Self> {
        // The CHECK constraints keep these columns inside the vocabulary; a
        // parse failure here means schema and code have drifted — a bug.
        let vocab = |err: Error| Error::Internal {
            message: format!("stored value outside vocabulary: {err}"),
        };
        Ok(RecordVersion {
            id: RecordId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            state: RecordState {
                scope_id: ScopeId::from_uuid(row.scope_id),
                owner_id: IdentityId::from_uuid(row.owner_id),
                kind: row.kind.parse().map_err(vocab)?,
                class: row.class.parse().map_err(vocab)?,
                content: row.content,
                sensitivity: row.sensitivity.parse().map_err(vocab)?,
                provenance: row.provenance,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
            },
            tx_from: row.tx_from,
            tx_to: row.tx_to,
        })
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy
/// (types crate rule: native errors are converted at the boundary, detail
/// goes in the message).
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation (duplicate id), 40001 serialization_failure
        // (trigger-detected transaction-time clock anomaly): both are
        // conflicts with current state, retryable by the caller.
        if matches!(db.code().as_deref(), Some("23505") | Some("40001")) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (TEN-2, ADR-0009)
        // rejected a write whose tenant does not match the transaction's
        // tenant GUC, or the role lacks a grant. Either way an application
        // defect, never the caller's fault.
        if db.code().as_deref() == Some("42501") {
            return Error::Internal {
                message: format!("row-level security or privilege violation: {db}"),
            };
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Inserts a new record. Fails with [`Error::Conflict`] if `id` already has a
/// current version.
#[tracing::instrument(name = "store.records.insert", skip_all, fields(record.id = %id), err(Display))]
pub async fn insert(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    tenant_id: TenantId,
    state: &RecordState,
) -> Result<RecordVersion> {
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        insert into records
            (id, tenant_id, scope_id, owner_id, kind, class, content,
             sensitivity, provenance, valid_from, valid_to, tx_from)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
        returning id, tenant_id, scope_id, owner_id, kind, class, content,
                  sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        state.scope_id.as_uuid(),
        state.owner_id.as_uuid(),
        state.kind.as_str(),
        state.class.as_str(),
        state.content,
        state.sensitivity.as_str(),
        state.provenance,
        state.valid_from,
        state.valid_to,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Replaces the current version of `id` with `state`, archiving the previous
/// version. Returns `None` if the record has no current version.
#[tracing::instrument(name = "store.records.update", skip_all, fields(record.id = %id), err(Display))]
pub async fn update(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    state: &RecordState,
) -> Result<Option<RecordVersion>> {
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        update records
        set scope_id = $2, owner_id = $3, kind = $4, class = $5, content = $6,
            sensitivity = $7, provenance = $8, valid_from = $9, valid_to = $10
        where id = $1
        returning id, tenant_id, scope_id, owner_id, kind, class, content,
                  sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        "#,
        id.as_uuid(),
        state.scope_id.as_uuid(),
        state.owner_id.as_uuid(),
        state.kind.as_str(),
        state.class.as_str(),
        state.content,
        state.sensitivity.as_str(),
        state.provenance,
        state.valid_from,
        state.valid_to,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Temporally deletes `id`: the current version is archived and the record
/// ceases to exist going forward, while its history stays queryable via
/// [`as_of`]. Returns `false` if there was no current version.
#[tracing::instrument(name = "store.records.delete", skip_all, fields(record.id = %id), err(Display))]
pub async fn delete(executor: impl PgExecutor<'_>, id: RecordId) -> Result<bool> {
    let result = sqlx::query!("delete from records where id = $1", id.as_uuid())
        .execute(executor)
        .await
        .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// Returns the current version of `id`, if any.
#[tracing::instrument(name = "store.records.current", skip_all, fields(record.id = %id), err(Display))]
pub async fn current(executor: impl PgExecutor<'_>, id: RecordId) -> Result<Option<RecordVersion>> {
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Transaction-time as-of: the version of `id` the database held as truth at
/// `tx_at` — "what did we know at time T". Returns `None` if the record did
/// not exist (or was temporally deleted) at that instant. Transaction periods
/// are half-open `[tx_from, tx_to)`, so a version is visible from the exact
/// instant it was written.
#[tracing::instrument(
    name = "store.records.as_of",
    skip_all,
    fields(record.id = %id, tx_at = %tx_at),
    err(Display)
)]
pub async fn as_of(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    tx_at: DateTime<Utc>,
) -> Result<Option<RecordVersion>> {
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from records_versions
        where id = $1 and tx_from <= $2 and (tx_to is null or tx_to > $2)
        "#,
        id.as_uuid(),
        tx_at,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Bitemporal as-of: the version of `id` known at `tx_at`, but only if that
/// version's valid-time window covers `valid_at` — "as known at T, did the
/// fact hold at V". Valid periods are half-open `[valid_from, valid_to)`.
#[tracing::instrument(
    name = "store.records.as_of_bitemporal",
    skip_all,
    fields(record.id = %id, tx_at = %tx_at, valid_at = %valid_at),
    err(Display)
)]
pub async fn as_of_bitemporal(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    tx_at: DateTime<Utc>,
    valid_at: DateTime<Utc>,
) -> Result<Option<RecordVersion>> {
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from records_versions
        where id = $1 and tx_from <= $2 and (tx_to is null or tx_to > $2)
          and valid_from <= $3 and (valid_to is null or valid_to > $3)
        "#,
        id.as_uuid(),
        tx_at,
        valid_at,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Every version of `id` the database has ever known, oldest first.
#[tracing::instrument(name = "store.records.versions", skip_all, fields(record.id = %id), err(Display))]
pub async fn versions(executor: impl PgExecutor<'_>, id: RecordId) -> Result<Vec<RecordVersion>> {
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from records_versions
        where id = $1
        order by tx_from
        "#,
        id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}
