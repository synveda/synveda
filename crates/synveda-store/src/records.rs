//! Bitemporal storage for memory records (FND-4, ADR-0006).
//!
//! A record is updated by writing a complete new version: the trigger layer
//! archives the previous version into `records_history` with its transaction
//! period closed, and stamps the new one. [`delete`] is a *temporal* delete —
//! the record stops being current but every closed version stays queryable
//! through [`as_of`]. Nothing in this module reads or writes `tx_from`/`tx_to`
//! directly; those columns belong to the triggers.
//!
//! Embed-or-fail (MEM-4, ADR-0023): every write carries a [`RecordEmbedding`]
//! and lands the record row and its `record_embeddings` sidecar row in one
//! statement — a record without an embedding is unrepresentable through this
//! API, and migration 0015's deferred constraint trigger makes it impossible
//! to commit through any other. Vectors bind as `real[]` and cast to
//! `vector` in SQL; nothing here reads a stored vector back (CTX-1 owns the
//! read path).

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

/// The embedding that must accompany every record write (MEM-4, ADR-0023):
/// the vector computed over the state's exact `content`, and the model that
/// produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordEmbedding {
    /// Which model produced the vector, e.g. `BAAI/bge-m3` or `hash@1`.
    pub model: String,
    /// The vector itself. Dimension is whatever the model emits; it is
    /// recorded per row (`record_embeddings.dim`), never fixed in schema.
    pub vector: Vec<f32>,
}

/// The stored embedding's metadata (never the vector — CTX-1 owns reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEmbeddingMeta {
    /// The model recorded at write time.
    pub model: String,
    /// The stored vector's dimension.
    pub dim: i32,
    /// When the embedding row was last written.
    pub embedded_at: DateTime<Utc>,
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

/// Raw row shared by every record query in this crate (the read path's
/// [`crate::search`] included); converted with `TryFrom` so vocabulary
/// columns decode through the `synveda-types` enums.
pub(crate) struct RecordRow {
    pub(crate) id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) scope_id: Uuid,
    pub(crate) owner_id: Uuid,
    pub(crate) kind: String,
    pub(crate) class: String,
    pub(crate) content: String,
    pub(crate) sensitivity: String,
    pub(crate) provenance: serde_json::Value,
    pub(crate) valid_from: DateTime<Utc>,
    pub(crate) valid_to: Option<DateTime<Utc>>,
    pub(crate) tx_from: DateTime<Utc>,
    pub(crate) tx_to: Option<DateTime<Utc>>,
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
/// goes in the message). Shared with [`crate::search`] — same tables,
/// same taxonomy.
pub(crate) fn storage_error(err: sqlx::Error) -> Error {
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
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Inserts a new record and its embedding in one statement (ADR-0023
/// decision 2). Fails with [`Error::Conflict`] if `id` already has a current
/// version, and with [`Error::Invalid`] on an empty vector.
#[tracing::instrument(name = "store.records.insert", skip_all, fields(record.id = %id), err(Display))]
pub async fn insert(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    tenant_id: TenantId,
    state: &RecordState,
    embedding: &RecordEmbedding,
) -> Result<RecordVersion> {
    if embedding.vector.is_empty() {
        return Err(Error::Invalid {
            message: "record embedding vector must not be empty".to_owned(),
        });
    }
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        with new_record as (
            insert into records
                (id, tenant_id, scope_id, owner_id, kind, class, content,
                 sensitivity, provenance, valid_from, valid_to, tx_from)
            values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
            returning id, tenant_id, scope_id, owner_id, kind, class, content,
                      sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        ),
        new_embedding as (
            insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
            select id, tenant_id, $12, cardinality($13::real[]), $13::real[]::vector
            from new_record
        )
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from new_record
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
        embedding.model,
        &embedding.vector,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Replaces the current version of `id` with `state`, archiving the previous
/// version and refreshing the embedding in the same statement — rewritten
/// content never rides a stale vector (ADR-0023 decision 4). Returns `None`
/// if the record has no current version.
#[tracing::instrument(name = "store.records.update", skip_all, fields(record.id = %id), err(Display))]
pub async fn update(
    executor: impl PgExecutor<'_>,
    id: RecordId,
    state: &RecordState,
    embedding: &RecordEmbedding,
) -> Result<Option<RecordVersion>> {
    if embedding.vector.is_empty() {
        return Err(Error::Invalid {
            message: "record embedding vector must not be empty".to_owned(),
        });
    }
    let row = sqlx::query_as!(
        RecordRow,
        r#"
        with updated as (
            update records
            set scope_id = $2, owner_id = $3, kind = $4, class = $5, content = $6,
                sensitivity = $7, provenance = $8, valid_from = $9, valid_to = $10
            where id = $1
            returning id, tenant_id, scope_id, owner_id, kind, class, content,
                      sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        ),
        refreshed as (
            insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
            select id, tenant_id, $11, cardinality($12::real[]), $12::real[]::vector
            from updated
            on conflict (record_id) do update
                set model = excluded.model, dim = excluded.dim,
                    embedding = excluded.embedding, embedded_at = now()
        )
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from updated
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
        embedding.model,
        &embedding.vector,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// The stored embedding's metadata for `id`, if the record has one. Records
/// written before migration 0015 (the MEM-3 window) legitimately return
/// `None` until the re-embed workflow backfills them.
#[tracing::instrument(name = "store.records.embedding_meta", skip_all, fields(record.id = %id), err(Display))]
pub async fn embedding_meta(
    executor: impl PgExecutor<'_>,
    id: RecordId,
) -> Result<Option<RecordEmbeddingMeta>> {
    sqlx::query_as!(
        RecordEmbeddingMeta,
        "select model, dim, embedded_at from record_embeddings where record_id = $1",
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)
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

/// The current version of each id that is a record of `tenant_id` living at
/// `scope_id` — the publish path's read (FLOW-2, ADR-0031 decision 12).
///
/// Scoped rather than by id alone, because publishing is same-scope by
/// definition: a record at a child scope climbing to its parent's channel
/// needs that scope's approvers, which is FLOW-5. An id that is missing,
/// deleted, or living elsewhere simply does not come back, and the caller
/// reports the difference rather than publishing a partial set.
#[tracing::instrument(
    name = "store.records.current_at_scope",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn current_at_scope(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
    ids: &[RecordId],
) -> Result<Vec<RecordVersion>> {
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where tenant_id = $1 and scope_id = $2 and id = any($3)
        order by id
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The current versions of `ids`, wherever they live.
///
/// The scope-blind sibling of [`current_at_scope`], for a caller that
/// learns about records before it learns where they are: the FLOW-4
/// sweep folds record ids out of the audit chain and only then asks
/// which scope each belongs to, so it can group by scope and resolve one
/// effective pack per group (ADR-0033 decision 14). Records that have
/// since been deleted simply do not come back.
#[tracing::instrument(
    name = "store.records.current_many",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn current_many(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    ids: &[RecordId],
) -> Result<Vec<RecordVersion>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where tenant_id = $1 and id = any($2)
        order by id
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
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
