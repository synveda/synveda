//! The read path's record queries (CTX-1, ADR-0024; CTX-2, ADR-0025):
//! the dense ANN leg, the fused candidates' verify-and-hydrate, the
//! change scan the search indexer tails, and the composition engine's
//! candidate read.
//!
//! Every query here filters on `tenant_id` explicitly — tenant
//! correctness never rides on the RLS backstop alone, which the
//! dev-compose superuser bypasses (ADR-0009) — and the dense leg and
//! hydration additionally take the caller's allowed scopes and
//! sensitivities: there is no unfiltered variant to call (ADR-0024
//! decision 2). Orchestration (Tantivy, fusion, the PDP-derived scope
//! set) lives in `synveda-retrieval`; this module owns only the SQL.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, RecordId, Result, ScopeId, Sensitivity, TenantId};
use uuid::Uuid;

use crate::records::{RecordRow, RecordVersion, storage_error};

/// The ANN dimensions with a matching partial HNSW index and
/// compile-checked query (migration 0016, ADR-0024 decision 5): the
/// deterministic hash embedder and BGE-M3 dense.
pub const SUPPORTED_ANN_DIMS: [usize; 2] = [16, 1024];

/// One change-scan result: every record id whose bitemporal state moved
/// after `since`, with the stamps the indexer advances its watermark by.
#[derive(Debug)]
pub struct ChangeScan {
    /// Distinct ids to re-read (their current version decides upsert vs
    /// delete).
    pub ids: Vec<RecordId>,
    /// The greatest transaction-time stamp seen, if any rows changed.
    pub max_stamp: Option<DateTime<Utc>>,
    /// The database's `now()` at scan time — the watermark's idle-time
    /// advance is computed from server clock, never the client's
    /// (ADR-0024 decision 4).
    pub db_now: DateTime<Utc>,
}

/// A record's indexable projection: exactly the fields the sidecar
/// index stores or filters on. Content is the persisted (post-MEM-2
/// redacted) text.
#[derive(Debug)]
pub struct IndexableRecord {
    /// The record id — the index's document key.
    pub id: RecordId,
    /// The scope the record attaches to — a pushdown term.
    pub scope_id: ScopeId,
    /// The record's sensitivity — a pushdown term.
    pub sensitivity: Sensitivity,
    /// The indexed text.
    pub content: String,
}

/// One dense-leg hit: a candidate id and its cosine distance, nearest
/// first.
#[derive(Debug)]
pub struct DenseHit {
    /// The candidate record.
    pub record_id: RecordId,
    /// Cosine distance to the query vector (smaller is nearer).
    pub distance: f64,
}

/// Every record id whose current version changed or closed after
/// `since` — inserts and updates via `records.tx_from`, updates' closed
/// predecessors and temporal deletes via `records_history.tx_to`
/// (ADR-0006's pair is the change feed; ADR-0024 decision 4).
#[tracing::instrument(
    name = "store.search.changes_since",
    skip_all,
    fields(tenant.id = %tenant_id, since = %since),
    err(Display)
)]
pub async fn changes_since(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    since: DateTime<Utc>,
) -> Result<ChangeScan> {
    let db_now = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&mut *conn)
        .await
        .map_err(storage_error)?;
    let rows = sqlx::query!(
        r#"
        select id as "id!", stamp as "stamp!"
        from (
            select id, tx_from as stamp
            from records
            where tenant_id = $1 and tx_from > $2
            union all
            select id, tx_to as stamp
            from records_history
            where tenant_id = $1 and tx_to > $2
        ) as changed
        "#,
        tenant_id.as_uuid(),
        since,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    let max_stamp = rows.iter().map(|row| row.stamp).max();
    let mut ids: Vec<RecordId> = rows
        .into_iter()
        .map(|row| RecordId::from_uuid(row.id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ChangeScan {
        ids,
        max_stamp,
        db_now,
    })
}

/// The current indexable projection of each id in `ids`. Ids absent
/// from the result no longer have a current version — the indexer
/// deletes their documents.
#[tracing::instrument(
    name = "store.search.for_index",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn for_index(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
) -> Result<Vec<IndexableRecord>> {
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query!(
        r#"
        select id as "id!", scope_id as "scope_id!",
               sensitivity as "sensitivity!", content as "content!"
        from records
        where tenant_id = $1 and id = any($2)
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(IndexableRecord {
                id: RecordId::from_uuid(row.id),
                scope_id: ScopeId::from_uuid(row.scope_id),
                sensitivity: row.sensitivity.parse().map_err(|err| Error::Internal {
                    message: format!("stored value outside vocabulary: {err}"),
                })?,
                content: row.content,
            })
        })
        .collect()
}

/// The dense ANN leg: nearest current records to `query_vector` among
/// the allowed scopes and sensitivities, for vectors written by `model`.
/// Dispatches to the compile-checked query for the vector's dimension;
/// an unsupported dimension is [`Error::Invalid`] naming
/// [`SUPPORTED_ANN_DIMS`] (ADR-0024 decision 5).
///
/// Must run inside a tenant transaction: the HNSW GUCs are set
/// transaction-locally so the iterative scan keeps yielding candidates
/// past post-filtering without leaking tuning into the pool.
#[tracing::instrument(
    name = "store.search.dense",
    skip_all,
    fields(tenant.id = %tenant_id, model, dim = query_vector.len(), limit),
    err(Display)
)]
pub async fn dense_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    model: &str,
    query_vector: &[f32],
    scopes: &[ScopeId],
    sensitivities: &[Sensitivity],
    limit: i64,
) -> Result<Vec<DenseHit>> {
    let scopes: Vec<Uuid> = scopes.iter().map(ScopeId::as_uuid).collect();
    let sensitivities: Vec<String> = sensitivities
        .iter()
        .map(|level| level.as_str().to_owned())
        .collect();
    // Iterative scanning (pgvector ≥0.8) is what makes predicate
    // pushdown real for HNSW: without it, a selective scope filter
    // starves the LIMIT after ef_search candidates (ADR-0024 decision 5).
    sqlx::query!(
        r#"
        select set_config('hnsw.iterative_scan', 'relaxed_order', true) as "a!",
               set_config('hnsw.ef_search', '100', true) as "b!"
        "#
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let rows = match query_vector.len() {
        16 => {
            sqlx::query_as!(
                DenseHitRow,
                r#"
                select e.record_id as "record_id!",
                       (e.embedding::vector(16) <=> $2::real[]::vector(16))::float8
                           as "distance!"
                from record_embeddings e
                join records r on r.id = e.record_id
                where e.tenant_id = $1
                  and e.dim = 16
                  and e.model = $3
                  and r.tenant_id = $1
                  and r.scope_id = any($4)
                  and r.sensitivity = any($5)
                order by e.embedding::vector(16) <=> $2::real[]::vector(16)
                limit $6
                "#,
                tenant_id.as_uuid(),
                query_vector,
                model,
                &scopes,
                &sensitivities,
                limit,
            )
            .fetch_all(&mut *conn)
            .await
        }
        1024 => {
            sqlx::query_as!(
                DenseHitRow,
                r#"
                select e.record_id as "record_id!",
                       (e.embedding::vector(1024) <=> $2::real[]::vector(1024))::float8
                           as "distance!"
                from record_embeddings e
                join records r on r.id = e.record_id
                where e.tenant_id = $1
                  and e.dim = 1024
                  and e.model = $3
                  and r.tenant_id = $1
                  and r.scope_id = any($4)
                  and r.sensitivity = any($5)
                order by e.embedding::vector(1024) <=> $2::real[]::vector(1024)
                limit $6
                "#,
                tenant_id.as_uuid(),
                query_vector,
                model,
                &scopes,
                &sensitivities,
                limit,
            )
            .fetch_all(&mut *conn)
            .await
        }
        unsupported => {
            return Err(Error::Invalid {
                message: format!(
                    "no ANN index for {unsupported}-dimension vectors; supported: \
                     {SUPPORTED_ANN_DIMS:?} (ADR-0024 decision 5)"
                ),
            });
        }
    };
    Ok(rows
        .map_err(storage_error)?
        .into_iter()
        .map(|row| DenseHit {
            record_id: RecordId::from_uuid(row.record_id),
            distance: row.distance,
        })
        .collect())
}

struct DenseHitRow {
    record_id: Uuid,
    distance: f64,
}

/// The composition engine's candidate read (CTX-2, ADR-0025
/// decision 5): every current record in the allowed scopes and
/// sensitivities whose valid-time window covers `at`, capped per
/// `(scope, kind)` so a flood of derived records cannot crowd pinned
/// material out of the fetch (nor one scope's records another's). The
/// cap selects deterministically — newest `valid_from`, then newest
/// `tx_from`, then id — and the caller owns final ordering (SQL does
/// not know chain positions). `at` is the caller's explicit instant:
/// no clock is read here (the determinism AC).
#[tracing::instrument(
    name = "store.search.compose_candidates",
    skip_all,
    fields(tenant.id = %tenant_id, scopes.count = scopes.len(), at = %at),
    err(Display)
)]
pub async fn compose_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scopes: &[ScopeId],
    sensitivities: &[Sensitivity],
    at: DateTime<Utc>,
    per_scope_kind_limit: i64,
) -> Result<Vec<RecordVersion>> {
    let scopes: Vec<Uuid> = scopes.iter().map(ScopeId::as_uuid).collect();
    let sensitivities: Vec<String> = sensitivities
        .iter()
        .map(|level| level.as_str().to_owned())
        .collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from (
            select id, tenant_id, scope_id, owner_id, kind, class, content,
                   sensitivity, provenance, valid_from, valid_to, tx_from, tx_to,
                   row_number() over (
                       partition by scope_id, kind
                       order by valid_from desc, tx_from desc, id
                   ) as position
            from records
            where tenant_id = $1 and scope_id = any($2)
              and sensitivity = any($3)
              and valid_from <= $4 and (valid_to is null or valid_to > $4)
        ) ranked
        where position <= $5
        "#,
        tenant_id.as_uuid(),
        &scopes,
        &sensitivities,
        at,
        per_scope_kind_limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The composition engine's published-channel read (FLOW-2, ADR-0031
/// decision 9): the current version of each id a scope's published tree
/// names, through the same scope, sensitivity, and valid-time predicate
/// the derived sweep applies.
///
/// Fetched by id rather than swept, and uncapped, because a published set
/// is bounded by `MAX_CHANNEL_MEMBERS` and must not compete with derived
/// records for the per-`(scope, kind)` cap — a promoted extraction is
/// still `kind = derived`, so the capped sweep could crowd a scope's own
/// published material out of its own fetch.
///
/// An id the predicate rejects — deleted, re-scoped, re-classified above
/// the ceiling, or outside its valid window — simply does not come back.
/// A published set can therefore only go stale by *missing* material,
/// never by resurfacing it: current Postgres truth decides, as it does
/// for the sidecar index (ADR-0024 decision 6).
#[tracing::instrument(
    name = "store.search.compose_members",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len(), at = %at),
    err(Display)
)]
pub async fn compose_members(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
    scopes: &[ScopeId],
    sensitivities: &[Sensitivity],
    at: DateTime<Utc>,
) -> Result<Vec<RecordVersion>> {
    if ids.is_empty() || scopes.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let scopes: Vec<Uuid> = scopes.iter().map(ScopeId::as_uuid).collect();
    let sensitivities: Vec<String> = sensitivities
        .iter()
        .map(|level| level.as_str().to_owned())
        .collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where tenant_id = $1 and id = any($2)
          and scope_id = any($3) and sensitivity = any($4)
          and valid_from <= $5 and (valid_to is null or valid_to > $5)
        "#,
        tenant_id.as_uuid(),
        &ids,
        &scopes,
        &sensitivities,
        at,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Verify-and-hydrate for the fused candidate set: re-reads `ids`
/// against current truth with the scope and sensitivity predicate
/// re-applied in SQL, so a lagging sidecar index can only miss — never
/// resurface a deleted, re-scoped, or re-classified record (ADR-0024
/// decision 6). Row order is unspecified; the caller restores fused
/// rank order.
#[tracing::instrument(
    name = "store.search.hydrate",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn hydrate_verified(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
    scopes: &[ScopeId],
    sensitivities: &[Sensitivity],
) -> Result<Vec<RecordVersion>> {
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let scopes: Vec<Uuid> = scopes.iter().map(ScopeId::as_uuid).collect();
    let sensitivities: Vec<String> = sensitivities
        .iter()
        .map(|level| level.as_str().to_owned())
        .collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where tenant_id = $1 and id = any($2)
          and scope_id = any($3) and sensitivity = any($4)
        "#,
        tenant_id.as_uuid(),
        &ids,
        &scopes,
        &sensitivities,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}
