//! The read path's record queries (CTX-1, ADR-0024; CTX-2, ADR-0025):
//! the dense ANN leg, the fused candidates' verify-and-hydrate, the
//! change scan the search indexer tails, and the composition engine's
//! candidate read.
//!
//! Every query here filters on `tenant_id` explicitly — tenant
//! correctness never rides on the RLS backstop alone, which the
//! dev-compose superuser bypasses (ADR-0009) — and the dense leg and
//! hydration additionally take the caller's allowed `(scope, tier)` pairs:
//! there is no unfiltered variant to call (ADR-0024 decision 2).
//! Orchestration (Tantivy, fusion, the PDP-derived pair set) lives in
//! `synveda-retrieval`; this module owns only the SQL.
//!
//! The predicate is a **pair** since AUTHZ-5 (ADR-0038 decision 3), not a
//! scope set plus a ceiling: the PDP decides per scope and per tier, so one
//! scope may admit `confidential` while its neighbour admits only the
//! working tiers. `unnest` of two parallel arrays is how a pair set reaches
//! SQL while every query here stays compile-checked.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{
    Error, RecordClass, RecordId, Result, ScopeId, ScopeTier, Sensitivity, TenantId,
};
use uuid::Uuid;

use crate::records::{RecordRow, RecordVersion, storage_error};

/// The ANN dimensions with a matching partial HNSW index and
/// compile-checked query (migration 0016, ADR-0024 decision 5): the
/// deterministic hash embedder and BGE-M3 dense.
pub const SUPPORTED_ANN_DIMS: [usize; 2] = [16, 1024];

/// The two parallel arrays one pair set becomes on the wire.
fn pair_arrays(allowed: &[ScopeTier]) -> (Vec<Uuid>, Vec<String>) {
    allowed
        .iter()
        .map(|pair| {
            (
                pair.scope_id.as_uuid(),
                pair.sensitivity.as_str().to_owned(),
            )
        })
        .unzip()
}

/// Every scope in the tenant holding at least one record — the residence
/// half of a recall's candidate universe (CTX-5, ADR-0042 decision 2).
///
/// This is the whole of the widened universe's cost control, and it is a
/// cost control rather than a policy narrowing: [`compose_candidates`]
/// reaches derived material through a `scope_id` predicate, so a scope
/// holding nothing contributes the empty set whatever the PDP would have
/// said about it. Deciding it would change no result, only the clock.
///
/// Published material is the other half and does not come from here: a
/// published tree may name a record living below its scope (FLOW-5,
/// ADR-0034 decision 6), so residence cannot find it. The caller unions
/// this with `synveda_vedaflow::scopes_with_channel`.
///
/// Deliberately un-capped and un-paginated: the caller applies
/// ADR-0042 decision 5's cap after ordering by hierarchy distance, which
/// this query cannot do because SQL does not know the caller's chain.
#[tracing::instrument(
    name = "store.search.occupied_scopes",
    skip_all,
    fields(tenant.id = %tenant_id, scopes = tracing::field::Empty),
    err(Display)
)]
pub async fn occupied_scopes(conn: &mut PgConnection, tenant_id: TenantId) -> Result<Vec<ScopeId>> {
    let rows = sqlx::query_scalar!(
        r#"select distinct scope_id as "scope_id!"
           from records
           where tenant_id = $1
           order by scope_id"#,
        tenant_id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("scopes", rows.len());
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
}

/// The scopes `ids` live at — the residence half of the *ids* form's
/// candidate universe (CTX-5, ADR-0042 decision 2).
///
/// Reads `records_versions` rather than `records` so a named id whose
/// current version is gone still resolves to the scope it lived at, which
/// is what the as-of forms need (ADR-0042 decision 14). Naming an id the
/// caller may not read yields its scope here and a denial at the PDP;
/// nothing about the answer leaks back, because a scope that plans nothing
/// admits nothing.
#[tracing::instrument(
    name = "store.search.scopes_holding",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len(), scopes = tracing::field::Empty),
    err(Display)
)]
pub async fn scopes_holding(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
) -> Result<Vec<ScopeId>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let rows = sqlx::query_scalar!(
        r#"select distinct scope_id as "scope_id!"
           from records_versions
           where tenant_id = $1 and id = any($2)
           order by scope_id"#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("scopes", rows.len());
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
}

/// One scope's retention horizon for one record class (MEM-6, ADR-0040
/// decision 2): material of `class` at `scope_id` whose `valid_from` is at
/// or before `cutoff` is past what that scope serves.
///
/// Only classes a pack actually schedules are represented — a class it
/// keeps has no triple, never a triple at the beginning of time — so an
/// empty slice means "this plan expires nothing", which is the product
/// default and must read as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeClassCutoff {
    /// The scope whose pack set the horizon.
    pub scope_id: ScopeId,
    /// The class the horizon applies to.
    pub class: RecordClass,
    /// The instant at or before which material of that class is past it.
    pub cutoff: DateTime<Utc>,
}

/// The three parallel arrays one horizon set becomes on the wire.
fn horizon_arrays(horizons: &[ScopeClassCutoff]) -> (Vec<Uuid>, Vec<String>, Vec<DateTime<Utc>>) {
    let mut scopes = Vec::with_capacity(horizons.len());
    let mut classes = Vec::with_capacity(horizons.len());
    let mut cutoffs = Vec::with_capacity(horizons.len());
    for horizon in horizons {
        scopes.push(horizon.scope_id.as_uuid());
        classes.push(horizon.class.as_str().to_owned());
        cutoffs.push(horizon.cutoff);
    }
    (scopes, classes, cutoffs)
}

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

/// How pgvector is asked to walk the HNSW graph for one dense query.
///
/// These were constants inside the query until TEN-3 measured them
/// (ADR-0063 arm B): `relaxed_order` and an `ef_search` of 100, set
/// transaction-locally so tuning never leaks into a pooled connection.
/// They are parameters rather than environment reads because
/// configuration belongs at the edge — the gateway reads its settings and
/// passes them down (ADR-0007's shape, and what `SYNVEDA_SEARCH_POLL_MS`
/// already does for the indexer); a store that read its own environment
/// would put deployment config below the seam that owns it.
///
/// The defaults are exactly what the query hardcoded, so
/// `DenseTuning::default()` is the behaviour every caller had before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DenseTuning {
    /// Candidate-list size per HNSW layer. Raising it trades latency for
    /// recall, which is the whole of arm B.
    pub ef_search: u32,
    /// Whether the scan continues past the first batch when the filter
    /// eats it.
    pub iterative_scan: IterativeScan,
}

impl Default for DenseTuning {
    fn default() -> Self {
        Self {
            ef_search: 100,
            iterative_scan: IterativeScan::RelaxedOrder,
        }
    }
}

/// pgvector's `hnsw.iterative_scan` modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterativeScan {
    /// No iteration: one batch, then the filter. The pre-0.8 behaviour,
    /// here so a benchmark can measure what iterative scanning buys.
    Off,
    /// Keep scanning; results may not be in exact distance order.
    RelaxedOrder,
    /// Keep scanning, preserving distance order.
    StrictOrder,
}

impl IterativeScan {
    /// The GUC value. A `&'static str` bound as a parameter, never
    /// interpolated — `set_config` takes its value as an argument, so
    /// this stays inside the compile-checked-queries rule.
    #[must_use]
    pub fn as_guc(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::RelaxedOrder => "relaxed_order",
            Self::StrictOrder => "strict_order",
        }
    }
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
/// the allowed `(scope, tier)` pairs, for vectors written by `model`.
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
    allowed: &[ScopeTier],
    limit: i64,
    tuning: DenseTuning,
) -> Result<Vec<DenseHit>> {
    let (scopes, sensitivities) = pair_arrays(allowed);
    // Iterative scanning (pgvector ≥0.8) is what makes predicate
    // pushdown real for HNSW: without it, a selective scope filter
    // starves the LIMIT after ef_search candidates (ADR-0024 decision 5).
    sqlx::query!(
        r#"
        select set_config('hnsw.iterative_scan', $1, true) as "a!",
               set_config('hnsw.ef_search', $2, true) as "b!"
        "#,
        tuning.iterative_scan.as_guc(),
        tuning.ef_search.to_string(),
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
                  and (r.scope_id, r.sensitivity)
                      in (select * from unnest($4::uuid[], $5::text[]))
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
                  and (r.scope_id, r.sensitivity)
                      in (select * from unnest($4::uuid[], $5::text[]))
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
///
/// `only`, when given, restricts the read to those record ids (CTX-4,
/// ADR-0041 decision 5): the recall path names what it wants rather than
/// sweeping, and every other predicate in this query — the `(scope, tier)`
/// pairs, the valid window, the retention cut, the pinned exemption —
/// applies to it unchanged. That is the whole point of it being one query:
/// a caller cannot reach a record by naming it that a sweep would not have
/// returned.
#[tracing::instrument(
    name = "store.search.compose_candidates",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        pairs.count = allowed.len(),
        named = only.map_or(-1, |ids| i64::try_from(ids.len()).unwrap_or(i64::MAX)),
        at = %at,
    ),
    err(Display)
)]
pub async fn compose_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    allowed: &[ScopeTier],
    horizons: &[ScopeClassCutoff],
    at: DateTime<Utc>,
    per_scope_kind_limit: i64,
    only: Option<&[RecordId]>,
) -> Result<Vec<RecordVersion>> {
    let (scopes, sensitivities) = pair_arrays(allowed);
    let (horizon_scopes, horizon_classes, horizon_cutoffs) = horizon_arrays(horizons);
    let named: Option<Vec<Uuid>> = only.map(|ids| ids.iter().map(RecordId::as_uuid).collect());
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
            where tenant_id = $1
              and (scope_id, sensitivity)
                  in (select * from unnest($2::uuid[], $3::text[]))
              and valid_from <= $4 and (valid_to is null or valid_to > $4)
              -- PRMT-2 (ADR-0050 decision 8): a context pack's chunk is
              -- never admitted by `MemoryRead`. Chunk rows are `records`
              -- rows at the authoring scope, so without this they would be
              -- ordinary memory — an unpublished bundle composing into
              -- somebody's session marked `[unreviewed]`, which is the one
              -- thing "a pack reaches a session only through review" says
              -- cannot happen. The exclusion is here, in the query, rather
              -- than in a caller that could forget it.
              and not exists (
                  select 1 from context_pack_chunks c
                  where c.tenant_id = records.tenant_id and c.record_id = records.id
              )

              -- The named-id restriction (CTX-4, ADR-0041 decision 5).
              -- Null means "sweep", which is what every inject passes.
              and ($9::uuid[] is null or id = any($9))
              -- The retention cut (MEM-6, ADR-0040 decision 2), applied
              -- here rather than after hydration so a scope past its
              -- horizon never competes for the per-(scope, kind) cap.
              -- Pinned material is never asked: seed §4.2 says it cannot
              -- be decayed, and that exemption is this clause.
              and (
                  kind = 'pinned'
                  or not exists (
                      select 1
                      from unnest($6::uuid[], $7::text[], $8::timestamptz[])
                          as horizon(scope_id, class, cutoff)
                      where horizon.scope_id = records.scope_id
                        and horizon.class = records.class
                        and records.valid_from <= horizon.cutoff
                  )
              )
        ) ranked
        where position <= $5
        "#,
        tenant_id.as_uuid(),
        &scopes,
        &sensitivities,
        at,
        per_scope_kind_limit,
        &horizon_scopes,
        &horizon_classes,
        &horizon_cutoffs,
        named.as_deref(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// [`compose_candidates`] at a **transaction-time instant** — the derived
/// sweep as the database held it at `tx_at` (CTX-5, ADR-0042 decisions 7
/// and 14).
///
/// Three differences from the present-tense read, each a decision rather
/// than an omission:
///
/// - It reads `records_versions`, so a record expired since `tx_at` still
///   composes. That is what MEM-6 chose expire-as-temporal-delete *for*
///   (ADR-0040 decision 5), and the destroy horizon is what bounds it.
/// - It applies **no retention cut**: the horizon governs the live corpus,
///   and this is a read of history (ADR-0042 decision 11).
/// - The tier predicate is the **strictest sensitivity the record has
///   carried at or since `tx_at`**, not the one the served version wore
///   (ADR-0042 decision 9). A record raised to `restricted` in April is
///   `restricted` for its March version too, so the AUTHZ-5 leak suite
///   cannot be defeated by a timestamp. At `tx_at = now` the maximum is
///   the current tier and this behaves exactly as the present-tense read.
///
/// The ceiling is computed *in this statement* rather than by the caller,
/// because a caller-side check would be a second admission path — which is
/// the thing the whole read path is arranged to not have.
#[tracing::instrument(
    name = "store.search.compose_candidates_as_of",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        pairs.count = allowed.len(),
        named = only.map_or(-1, |ids| i64::try_from(ids.len()).unwrap_or(i64::MAX)),
        tx_at = %tx_at,
        valid_at = %valid_at,
    ),
    err(Display)
)]
pub async fn compose_candidates_as_of(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    allowed: &[ScopeTier],
    tx_at: DateTime<Utc>,
    valid_at: DateTime<Utc>,
    per_scope_kind_limit: i64,
    only: Option<&[RecordId]>,
) -> Result<Vec<RecordVersion>> {
    let (scopes, sensitivities) = pair_arrays(allowed);
    let named: Option<Vec<Uuid>> = only.map(|ids| ids.iter().map(RecordId::as_uuid).collect());
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        with asof as (
            select id, tenant_id, scope_id, owner_id, kind, class, content,
                   sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
            from records_versions
            where tenant_id = $1
              and tx_from <= $4 and (tx_to is null or tx_to > $4)
              and valid_from <= $5 and (valid_to is null or valid_to > $5)
              -- PRMT-2 (ADR-0050 decision 8): a context pack's chunk is
              -- never admitted by `MemoryRead`. Chunk rows are `records`
              -- rows at the authoring scope, so without this they would be
              -- ordinary memory — an unpublished bundle composing into
              -- somebody's session marked `[unreviewed]`, which is the one
              -- thing "a pack reaches a session only through review" says
              -- cannot happen. The exclusion is here, in the query, rather
              -- than in a caller that could forget it.
              and not exists (
                  select 1 from context_pack_chunks c
                  where c.tenant_id = records_versions.tenant_id and c.record_id = records_versions.id
              )

              and ($7::uuid[] is null or id = any($7))
        ),
        -- The strictest tier each record has carried at or since `tx_at`
        -- (ADR-0042 decision 9). Ordinals rather than the text, because
        -- `confidential` sorts before `internal` as text and this is the
        -- one place that ordering must not be lexicographic.
        ceiling as (
            select v.id,
                   max(case v.sensitivity
                           when 'public' then 0 when 'internal' then 1
                           when 'confidential' then 2 else 3 end) as tier
            from records_versions v
            join asof on asof.id = v.id
            where v.tenant_id = $1 and (v.tx_to is null or v.tx_to > $4)
            group by v.id
        )
        select id as "id!", tenant_id as "tenant_id!", scope_id as "scope_id!",
               owner_id as "owner_id!", kind as "kind!", class as "class!",
               content as "content!", sensitivity as "sensitivity!",
               provenance as "provenance!", valid_from as "valid_from!",
               valid_to, tx_from as "tx_from!", tx_to
        from (
            select asof.id, asof.tenant_id, asof.scope_id, asof.owner_id,
                   asof.kind, asof.class, asof.content, asof.sensitivity,
                   asof.provenance, asof.valid_from, asof.valid_to,
                   asof.tx_from, asof.tx_to,
                   row_number() over (
                       partition by asof.scope_id, asof.kind
                       order by asof.valid_from desc, asof.tx_from desc, asof.id
                   ) as position
            from asof
            join ceiling on ceiling.id = asof.id
            where (asof.scope_id,
                   case ceiling.tier
                       when 0 then 'public' when 1 then 'internal'
                       when 2 then 'confidential' else 'restricted' end)
                  in (select * from unnest($2::uuid[], $3::text[]))
        ) ranked
        where position <= $6
        "#,
        tenant_id.as_uuid(),
        &scopes,
        &sensitivities,
        tx_at,
        valid_at,
        per_scope_kind_limit,
        named.as_deref(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// [`compose_members`] at a transaction-time instant — the published-member
/// read as the database held it at `tx_at` (CTX-5, ADR-0042 decision 14).
///
/// The **membership** is not rewound: the caller passes ids from the
/// channel's *current* tree, because publication is a judgment the
/// organisation may revise and a rewound ref would let `as_of` re-publish
/// what a FLOW-7 rollback withdrew (ADR-0042 decision 10). Only the bodies
/// come from history. Carries the same strictest-tier-since ceiling as
/// [`compose_candidates_as_of`], against the union ceiling the caller
/// passes, with the exact pair enforced where attribution happens.
#[tracing::instrument(
    name = "store.search.compose_members_as_of",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len(), tx_at = %tx_at, valid_at = %valid_at),
    err(Display)
)]
pub async fn compose_members_as_of(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
    sensitivities: &[Sensitivity],
    tx_at: DateTime<Utc>,
    valid_at: DateTime<Utc>,
) -> Result<Vec<RecordVersion>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let sensitivities: Vec<String> = sensitivities
        .iter()
        .map(|level| level.as_str().to_owned())
        .collect();
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        with asof as (
            select id, tenant_id, scope_id, owner_id, kind, class, content,
                   sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
            from records_versions
            where tenant_id = $1 and id = any($2)
              and tx_from <= $4 and (tx_to is null or tx_to > $4)
              and valid_from <= $5 and (valid_to is null or valid_to > $5)
        ),
        -- The strictest tier since `tx_at` (ADR-0042 decision 9), as
        -- ordinals for the reason `compose_candidates_as_of` gives.
        ceiling as (
            select v.id,
                   max(case v.sensitivity
                           when 'public' then 0 when 'internal' then 1
                           when 'confidential' then 2 else 3 end) as tier
            from records_versions v
            join asof on asof.id = v.id
            where v.tenant_id = $1 and (v.tx_to is null or v.tx_to > $4)
            group by v.id
        )
        select asof.id as "id!", asof.tenant_id as "tenant_id!",
               asof.scope_id as "scope_id!", asof.owner_id as "owner_id!",
               asof.kind as "kind!", asof.class as "class!",
               asof.content as "content!", asof.sensitivity as "sensitivity!",
               asof.provenance as "provenance!", asof.valid_from as "valid_from!",
               asof.valid_to, asof.tx_from as "tx_from!", asof.tx_to
        from asof
        join ceiling on ceiling.id = asof.id
        where (case ceiling.tier
                   when 0 then 'public' when 1 then 'internal'
                   when 2 then 'confidential' else 'restricted' end) = any($3)
        "#,
        tenant_id.as_uuid(),
        &ids,
        &sensitivities,
        tx_at,
        valid_at,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The composition engine's published-channel read (FLOW-2, ADR-0031
/// decision 9): the current version of each id a planned scope's
/// published tree names, through the same sensitivity and valid-time
/// predicate the derived sweep applies.
///
/// Fetched by id rather than swept, and uncapped, because a published set
/// is bounded by `MAX_CHANNEL_MEMBERS` and must not compete with derived
/// records for the per-`(scope, kind)` cap — a promoted extraction is
/// still `kind = derived`, so the capped sweep could crowd a scope's own
/// published material out of its own fetch.
///
/// **Deliberately not filtered by scope** (FLOW-5, ADR-0034 decision 6).
/// The caller builds `ids` from the published trees of the scopes the PDP
/// already permitted, and a scope's published tree may name a record that
/// lives *below* it — that is what a cross-scope promotion produces. So
/// the predicate is tree membership, which is the stronger statement: "a
/// reviewed set at a scope you may read names this record" beats "this
/// record lives at a scope you may read". Residence still decides for
/// derived material, which has crossed no boundary
/// ([`compose_candidates`] keeps its scope predicate exactly).
///
/// `sensitivities` is the **union** over the planned scopes rather than a pair
/// set, because this query deliberately has no scope predicate: the caller
/// knows which planned scope's tree named each id and verifies that scope's
/// own tier set there (ADR-0038 decision 3). The union is the hard ceiling —
/// nothing above what *some* planned scope admits ever leaves SQL — and the
/// exact pair is enforced where the attribution happens.
///
/// An id the predicate rejects — deleted, re-classified above the
/// ceiling, or outside its valid window — simply does not come back. A
/// published set can therefore only go stale by *missing* material, never
/// by resurfacing it: current Postgres truth decides, as it does for the
/// sidecar index (ADR-0024 decision 6). The caller re-checks each
/// survivor's content address against the tree that named it, so an
/// edited record is unreviewed again rather than served under a
/// publication it no longer matches (ADR-0031 decision 5).
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
    sensitivities: &[Sensitivity],
    at: DateTime<Utc>,
) -> Result<Vec<RecordVersion>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
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
          and sensitivity = any($3)
          and valid_from <= $4 and (valid_to is null or valid_to > $4)
        "#,
        tenant_id.as_uuid(),
        &ids,
        &sensitivities,
        at,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Verify-and-hydrate for the fused candidate set: re-reads `ids`
/// against current truth with the `(scope, tier)` predicate re-applied in
/// SQL, so a lagging sidecar index can only miss — never resurface a
/// deleted, re-scoped, or re-classified record (ADR-0024 decision 6). Since
/// AUTHZ-5 that includes a record whose tier moved *above* what the caller
/// may read at its scope: reclassification takes effect on the next read,
/// through this predicate, with nothing to reindex (ADR-0038 decision 3).
///
/// Since MEM-5 it includes the valid window at `at` (ADR-0039 decision 11):
/// a record a newer statement superseded is not the current assertion, so it
/// does not come back — the sidecar may still hold it, which is one more way
/// for current truth to disagree with a lagging index, and the same
/// resolution applies. Composition would have refused it anyway; what this
/// prevents is a stale fact holding a ranking slot a live one needed.
///
/// Row order is unspecified; the caller restores fused rank order.
#[tracing::instrument(
    name = "store.search.hydrate",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len(), at = %at),
    err(Display)
)]
pub async fn hydrate_verified(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[RecordId],
    allowed: &[ScopeTier],
    at: DateTime<Utc>,
) -> Result<Vec<RecordVersion>> {
    let ids: Vec<Uuid> = ids.iter().map(RecordId::as_uuid).collect();
    let (scopes, sensitivities) = pair_arrays(allowed);
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, valid_to, tx_from, tx_to
        from records
        where tenant_id = $1 and id = any($2)
          and (scope_id, sensitivity)
              in (select * from unnest($3::uuid[], $4::text[]))
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
