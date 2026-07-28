//! The knowledge graph's storage and its only traversal entry point
//! (GRPH-1, ADR-0043).
//!
//! Indexed adjacency in Postgres, not AGE: GRPH-4 measured the relational
//! baseline 3–8× faster at 2.5× less storage on the traversal the product
//! actually issues, and ADR-0043 kept ADR-0004's named graphs while
//! overturning its engine. Nothing here calls the extension.
//!
//! The shape mirrors [`crate::records`] deliberately rather than
//! incidentally. A vertex is identity — one row per thing the graph can
//! talk about, converging on `(tenant_id, graph, kind, key)`, with no
//! history because "this thing exists and is named" is not a claim about
//! the world that can be superseded (decision 5). An **edge is a claim**,
//! and every claim is a bitemporal row of exactly the records shape:
//! `valid_from`/`valid_to` are application data, `tx_from`/`tx_to` belong
//! to the triggers, and nothing in this module reads or writes them
//! directly. So [`supersede`] is MEM-5's rule restated for edges — a
//! closed window plus a new row, never a rewrite (decision 4) — and the
//! graph answers `as_of` through the same view shape `records_versions`
//! gave CTX-5.
//!
//! Two properties are load-bearing enough to be types rather than review
//! notes (decisions 2 and 9):
//!
//! * [`expand`] takes a [`Graph`] **by value**. There is no default, no
//!   `Option`, and no other traversal entry point, so a query that does
//!   not name its semantic domain does not compile — the discipline
//!   ADR-0024 decision 1 applied to tenancy, applied here to meaning.
//! * [`expand`] takes a [`Depth`] **enum**. Unbounded traversal is
//!   unrepresentable, so ADR-0043's reversal trigger for anything deeper
//!   arrives as a compile error and then as an ADR.
//!
//! The graph is never a scope producer (decision 12). [`expand`] returns
//! vertex and edge identity; turning a reached vertex into readable
//! content goes through the record it backs and therefore through
//! `composition_plan` and `admit`, which narrow the fused candidate list
//! and never widen it. There is no path from an edge to a record body in
//! this module, and that absence is the reason a knowledge graph does not
//! become a policy bypass.
//!
//! GRPH-2 (ADR-0044) added two functions and no columns. [`assert_edge`]
//! is [`insert_edge`] made idempotent against migration 0027's partial
//! unique index, so the linker can be re-driven without accumulating
//! duplicate claims; and [`supersession_edges`] is the `provenance`
//! graph, **projected** from `record_supersessions` rather than written
//! into `graph_edges` — one system of record per claim (decision 11),
//! which is why it returns [`RecordId`]s and mints no vertices.
//!
//! Every statement here is static and sqlx-checked with its seed set bound
//! as an array — the criterion (G5) the spike's Cypher path failed.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{Depth, Error, Graph, GraphEdgeId, GraphVertexId, RecordId, Result, TenantId};
use uuid::Uuid;

use crate::records::storage_error;

/// Histogram: wall time of one [`expand`] call, labelled `graph` and
/// `depth`. Emitted here, described by the gateway where the recorder
/// lives (ADR-0007).
pub const GRAPH_EXPANSION_SECONDS: &str = "synveda_graph_expansion_duration_seconds";

/// Counter: edges written, labelled `graph`. Counts claims asserted — an
/// insert and a supersession's replacement each count one; closing a
/// window is not a new claim and counts nothing.
pub const GRAPH_EDGES_TOTAL: &str = "synveda_graph_edges_total";

/// The most seeds one [`expand`] call will accept.
///
/// A bound rather than a tuning knob (ADR-0043 decision 9: "a bounded seed
/// set"). GRPH-4 measured the traversal at 10 seeds; CTX-5 caps its own
/// candidate universe at 32 scopes; 64 leaves headroom over both while
/// keeping the bound array a bound array. A caller with more candidates
/// than this is ranking them badly, and finding that out as an
/// [`Error::Invalid`] beats finding it out as a slow query.
pub const MAX_EXPANSION_SEEDS: usize = 64;

/// The mutable portion of a vertex: everything except its identity
/// (`id`, `tenant_id`, `graph`) and the row's creation stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexState {
    /// What sort of thing this is (`person`, `org`, `episode`, …). Open
    /// vocabulary: entity types are the extraction pipeline's business,
    /// not a closed product enum like `RecordClass`.
    pub kind: String,
    /// The normalised resolution key GRPH-2 converges on, unique within
    /// `(tenant, graph, kind)`.
    pub key: String,
    /// The display form the key was normalised from.
    pub label: String,
    /// The record this vertex names, when the corpus already holds the
    /// thing rather than the graph having invented it.
    pub record_id: Option<RecordId>,
}

/// A vertex as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vertex {
    /// Vertex identifier.
    pub id: GraphVertexId,
    /// Owning tenant; immutable.
    pub tenant_id: TenantId,
    /// Which named graph this vertex belongs to; immutable, and part of
    /// the composite key an edge's foreign keys point at.
    pub graph: Graph,
    /// The vertex's state.
    pub state: VertexState,
    /// When the row was first written.
    pub created_at: DateTime<Utc>,
}

/// The mutable portion of an edge: everything except its identity
/// (`id`, `tenant_id`, `graph`) and the trigger-owned transaction time.
///
/// "Mutable" is narrower here than for a record: the trigger layer refuses
/// a change to `kind` or to either endpoint, because a different relation
/// between different things is a different claim and a different claim is
/// [`supersede`], not an update (decision 4). What an update may legally
/// change is the window, the method, the confidence and the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeState {
    /// The relation type (`mentions`, `reports_to`, `occurred_during`, …).
    /// Open vocabulary, for the reason [`VertexState::kind`] is.
    pub kind: String,
    /// Where the claim starts.
    pub src_id: GraphVertexId,
    /// Where it ends. Never equal to `src_id` — a thing related to itself
    /// is a resolution bug, and the schema refuses it.
    pub dst_id: GraphVertexId,
    /// Who asserted the claim: the seam's name, exactly as
    /// `record_supersessions.method`.
    pub method: String,
    /// How sure the asserter was, integer per mille. Never a float: a
    /// number a client may reshape is a number that cannot be compared
    /// later (the MEM-5 discipline).
    pub confidence_permille: i32,
    /// The record this claim was extracted from, where it has a single
    /// source. `None` for a projection or a fused assertion.
    pub source_record_id: Option<RecordId>,
    /// When the relation started holding in the world (valid time).
    pub valid_from: DateTime<Utc>,
    /// When it stopped; `None` = no known end.
    pub valid_to: Option<DateTime<Utc>>,
}

/// One version of an edge as stored: its state plus identity and the
/// transaction period during which the database held this version as
/// truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeVersion {
    /// Edge identifier; stable across the versions of this claim.
    pub id: GraphEdgeId,
    /// Owning tenant; immutable.
    pub tenant_id: TenantId,
    /// Which named graph the claim lives in; immutable.
    pub graph: Graph,
    /// The version's state.
    pub state: EdgeState,
    /// When the database started holding this version (trigger-stamped).
    pub tx_from: DateTime<Utc>,
    /// When it stopped; `None` = this is the current version.
    pub tx_to: Option<DateTime<Utc>>,
}

/// The two edges a supersession leaves behind (decision 4): the claim that
/// stopped holding, and the one that replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supersession {
    /// The prior claim, its valid window now closed. Its open-ended
    /// version is in `graph_edges_history` and still reads through
    /// [`edge_as_of`].
    pub closed: EdgeVersion,
    /// The claim that now holds.
    pub replacement: EdgeVersion,
}

/// A vertex an expansion reached, and how far away it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reached {
    /// How many hops from the nearest seed — 1 or 2, never more.
    pub hop: u8,
    /// The vertex.
    pub vertex_id: GraphVertexId,
}

/// What one traversal found.
///
/// Both fields are totally ordered before they leave the store — edges by
/// `(kind, id)`, reached vertices by `(hop, vertex_id)` — so a
/// graph-ranked input can never make CTX-2's byte-identical
/// re-composition depend on plan order (ADR-0043's determinism note).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Every distinct edge traversed, whichever hop found it.
    pub edges: Vec<EdgeVersion>,
    /// The vertices reached, **excluding the seeds themselves**: what the
    /// graph contributes that the caller did not already have. A vertex
    /// found at both hops is reported at the nearer one.
    pub reached: Vec<Reached>,
}

/// Raw vertex row.
struct VertexRow {
    id: Uuid,
    tenant_id: Uuid,
    graph: String,
    kind: String,
    key: String,
    label: String,
    record_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

/// Raw edge row, shared by every edge query in this module.
struct EdgeRow {
    id: Uuid,
    tenant_id: Uuid,
    graph: String,
    kind: String,
    src_id: Uuid,
    dst_id: Uuid,
    method: String,
    confidence_permille: i32,
    source_record_id: Option<Uuid>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    tx_from: DateTime<Utc>,
    tx_to: Option<DateTime<Utc>>,
}

/// An edge row plus the hop at which a traversal found it.
struct ExpandRow {
    hop: i32,
    id: Uuid,
    tenant_id: Uuid,
    graph: String,
    kind: String,
    src_id: Uuid,
    dst_id: Uuid,
    method: String,
    confidence_permille: i32,
    source_record_id: Option<Uuid>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    tx_from: DateTime<Utc>,
    tx_to: Option<DateTime<Utc>>,
}

/// A stored value outside the schema's vocabulary means code and schema
/// have drifted — a bug, never a caller's fault. The same conversion
/// [`crate::records`] makes.
fn vocab(err: Error) -> Error {
    Error::Internal {
        message: format!("stored value outside vocabulary: {err}"),
    }
}

impl TryFrom<VertexRow> for Vertex {
    type Error = Error;

    fn try_from(row: VertexRow) -> Result<Self> {
        Ok(Vertex {
            id: GraphVertexId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            graph: row.graph.parse().map_err(vocab)?,
            state: VertexState {
                kind: row.kind,
                key: row.key,
                label: row.label,
                record_id: row.record_id.map(RecordId::from_uuid),
            },
            created_at: row.created_at,
        })
    }
}

impl TryFrom<EdgeRow> for EdgeVersion {
    type Error = Error;

    fn try_from(row: EdgeRow) -> Result<Self> {
        Ok(EdgeVersion {
            id: GraphEdgeId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            graph: row.graph.parse().map_err(vocab)?,
            state: EdgeState {
                kind: row.kind,
                src_id: GraphVertexId::from_uuid(row.src_id),
                dst_id: GraphVertexId::from_uuid(row.dst_id),
                method: row.method,
                confidence_permille: row.confidence_permille,
                source_record_id: row.source_record_id.map(RecordId::from_uuid),
                valid_from: row.valid_from,
                valid_to: row.valid_to,
            },
            tx_from: row.tx_from,
            tx_to: row.tx_to,
        })
    }
}

impl ExpandRow {
    /// Splits the traversal row into the hop and the edge it found.
    fn split(self) -> Result<(u8, EdgeVersion)> {
        let hop = u8::try_from(self.hop).map_err(|_| Error::Internal {
            message: format!("traversal returned hop {} outside 1..=2", self.hop),
        })?;
        let edge = EdgeVersion::try_from(EdgeRow {
            id: self.id,
            tenant_id: self.tenant_id,
            graph: self.graph,
            kind: self.kind,
            src_id: self.src_id,
            dst_id: self.dst_id,
            method: self.method,
            confidence_permille: self.confidence_permille,
            source_record_id: self.source_record_id,
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            tx_from: self.tx_from,
            tx_to: self.tx_to,
        })?;
        Ok((hop, edge))
    }
}

/// Refuses a confidence the CHECK constraint would refuse, with a message
/// that names the number instead of a SQLSTATE.
fn check_confidence(permille: i32) -> Result<()> {
    if !(0..=1000).contains(&permille) {
        return Err(Error::Invalid {
            message: format!("edge confidence is {permille} per mille; the range is 0..=1000"),
        });
    }
    Ok(())
}

/// Interns `key` in `graph` and returns the vertex it resolved to
/// (ADR-0043 decision 5): the convergence point entity resolution needs.
///
/// First writer wins the identifier. A second call with the same
/// `(tenant, graph, kind, key)` returns the **existing** vertex with `id`
/// ignored — which is what makes this the place GRPH-2 converges rather
/// than a place it must first check. The display `label` is refreshed
/// (the newest observation names the thing best) and `record_id` is only
/// ever filled in, never cleared: a later mention that does not know the
/// backing record must not unlink the one that did.
#[tracing::instrument(
    name = "store.graph.upsert_vertex",
    skip_all,
    fields(tenant.id = %tenant_id, graph = %graph, vertex.kind = %state.kind),
    err(Display)
)]
pub async fn upsert_vertex(
    executor: impl PgExecutor<'_>,
    id: GraphVertexId,
    tenant_id: TenantId,
    graph: Graph,
    state: &VertexState,
) -> Result<Vertex> {
    let row = sqlx::query_as!(
        VertexRow,
        r#"
        insert into graph_vertices (id, tenant_id, graph, kind, key, label, record_id)
        values ($1, $2, $3, $4, $5, $6, $7)
        on conflict (tenant_id, graph, kind, key) do update
            set label = excluded.label,
                record_id = coalesce(excluded.record_id, graph_vertices.record_id)
        returning id as "id!", tenant_id as "tenant_id!", graph as "graph!",
                  kind as "kind!", key as "key!", label as "label!", record_id,
                  created_at as "created_at!"
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        graph.as_str(),
        state.kind,
        state.key,
        state.label,
        state.record_id.map(|id| id.as_uuid()),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// The vertex `id`, if this tenant has one.
#[tracing::instrument(
    name = "store.graph.vertex",
    skip_all,
    fields(tenant.id = %tenant_id, vertex.id = %id),
    err(Display)
)]
pub async fn vertex(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GraphVertexId,
) -> Result<Option<Vertex>> {
    let row = sqlx::query_as!(
        VertexRow,
        r#"
        select id, tenant_id, graph, kind, key, label, record_id, created_at
        from graph_vertices
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// The vertices `ids`, ordered by id — how a caller turns [`expand`]'s
/// identity-only answer into labels and backing records without joining
/// inside the traversal statement.
#[tracing::instrument(
    name = "store.graph.vertices",
    skip_all,
    fields(tenant.id = %tenant_id, ids.count = ids.len()),
    err(Display)
)]
pub async fn vertices(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    ids: &[GraphVertexId],
) -> Result<Vec<Vertex>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = ids.iter().map(GraphVertexId::as_uuid).collect();
    let rows = sqlx::query_as!(
        VertexRow,
        r#"
        select id, tenant_id, graph, kind, key, label, record_id, created_at
        from graph_vertices
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

/// The vertices of `graph` backed by any of `records`, ordered by id —
/// the seed lookup GRPH-3 makes to turn retrieved records into a
/// traversal's starting set, and the check GRPH-2 makes to see whether
/// linking already ran. Uses `graph_vertices_record_idx`.
#[tracing::instrument(
    name = "store.graph.vertices_for_records",
    skip_all,
    fields(tenant.id = %tenant_id, graph = %graph, records.count = records.len()),
    err(Display)
)]
pub async fn vertices_for_records(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    graph: Graph,
    records: &[RecordId],
) -> Result<Vec<Vertex>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let records: Vec<Uuid> = records.iter().map(|id| id.as_uuid()).collect();
    let rows = sqlx::query_as!(
        VertexRow,
        r#"
        select id, tenant_id, graph, kind, key, label, record_id, created_at
        from graph_vertices
        where tenant_id = $1 and graph = $2 and record_id = any($3)
        order by id
        "#,
        tenant_id.as_uuid(),
        graph.as_str(),
        &records,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Asserts a new claim. Fails with [`Error::Conflict`] if `id` already
/// exists, and with [`Error::Invalid`] on a confidence outside
/// `0..=1000`.
///
/// Both endpoints must already be vertices of this tenant **and this
/// graph**: the composite foreign keys make a cross-tenant or
/// cross-graph edge unrepresentable rather than merely refused
/// (decisions 6 and 7).
#[tracing::instrument(
    name = "store.graph.insert_edge",
    skip_all,
    fields(tenant.id = %tenant_id, graph = %graph, edge.id = %id, edge.kind = %state.kind),
    err(Display)
)]
pub async fn insert_edge(
    executor: impl PgExecutor<'_>,
    id: GraphEdgeId,
    tenant_id: TenantId,
    graph: Graph,
    state: &EdgeState,
) -> Result<EdgeVersion> {
    check_confidence(state.confidence_permille)?;
    let row = sqlx::query_as!(
        EdgeRow,
        r#"
        insert into graph_edges
            (id, tenant_id, graph, kind, src_id, dst_id, method,
             confidence_permille, source_record_id, valid_from, valid_to, tx_from)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
        returning id as "id!", tenant_id as "tenant_id!", graph as "graph!",
                  kind as "kind!", src_id as "src_id!", dst_id as "dst_id!",
                  method as "method!", confidence_permille as "confidence_permille!",
                  source_record_id, valid_from as "valid_from!", valid_to,
                  tx_from as "tx_from!", tx_to
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        graph.as_str(),
        state.kind,
        state.src_id.as_uuid(),
        state.dst_id.as_uuid(),
        state.method,
        state.confidence_permille,
        state.source_record_id.map(|id| id.as_uuid()),
        state.valid_from,
        state.valid_to,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    metrics::counter!(GRAPH_EDGES_TOTAL, "graph" => graph.as_str()).increment(1);
    row.try_into()
}

/// Supersedes `id` with a new claim, atomically (decision 4): the prior
/// edge's valid window closes at `valid_to` and `replacement` is inserted
/// in the same statement, so a caller cannot leave half a supersession
/// behind even by failing between two calls it never makes.
///
/// The window only ever **narrows** — the predicate refuses a `valid_to`
/// that would extend an already-closed window, exactly as
/// [`crate::records::close_window`] does, so a second supersession cannot
/// resurrect a claim by pushing its end later. When the predicate matches
/// nothing (no such current edge, or a window that already closed at or
/// before `valid_to`) **nothing is inserted either**: the replacement is
/// selected from the closing row, so a missed supersession cannot leave an
/// orphan claim asserting the same thing twice. That case is `None`.
///
/// Nothing is deleted. The prior edge's open-ended version lands in
/// `graph_edges_history` through the archive trigger and stays readable
/// through [`edge_as_of`] and [`edge_versions`] — which is the half of
/// GRPH-1's acceptance criterion that says both versions read as-of.
#[tracing::instrument(
    name = "store.graph.supersede",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        graph = %graph,
        edge.id = %id,
        replacement.id = %replacement_id,
        valid_to = %valid_to
    ),
    err(Display)
)]
pub async fn supersede(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    graph: Graph,
    id: GraphEdgeId,
    valid_to: DateTime<Utc>,
    replacement_id: GraphEdgeId,
    replacement: &EdgeState,
) -> Result<Option<Supersession>> {
    check_confidence(replacement.confidence_permille)?;
    let rows = sqlx::query_as!(
        ExpandRow,
        r#"
        with closed as (
            update graph_edges set valid_to = $3
            where tenant_id = $1 and id = $2
              and valid_from < $3
              and (valid_to is null or valid_to > $3)
            returning id, tenant_id, graph, kind, src_id, dst_id, method,
                      confidence_permille, source_record_id, valid_from,
                      valid_to, tx_from, tx_to
        ),
        inserted as (
            insert into graph_edges
                (id, tenant_id, graph, kind, src_id, dst_id, method,
                 confidence_permille, source_record_id, valid_from, valid_to, tx_from)
            select $4, $1, $5, $6, $7, $8, $9, $10, $11, $12, $13, now()
            from closed
            returning id, tenant_id, graph, kind, src_id, dst_id, method,
                      confidence_permille, source_record_id, valid_from,
                      valid_to, tx_from, tx_to
        )
        select 0 as "hop!", id as "id!", tenant_id as "tenant_id!",
               graph as "graph!", kind as "kind!", src_id as "src_id!",
               dst_id as "dst_id!", method as "method!",
               confidence_permille as "confidence_permille!", source_record_id,
               valid_from as "valid_from!", valid_to, tx_from as "tx_from!", tx_to
        from closed
        union all
        select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
               graph as "graph!", kind as "kind!", src_id as "src_id!",
               dst_id as "dst_id!", method as "method!",
               confidence_permille as "confidence_permille!", source_record_id,
               valid_from as "valid_from!", valid_to, tx_from as "tx_from!", tx_to
        from inserted
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        valid_to,
        replacement_id.as_uuid(),
        graph.as_str(),
        replacement.kind,
        replacement.src_id.as_uuid(),
        replacement.dst_id.as_uuid(),
        replacement.method,
        replacement.confidence_permille,
        replacement.source_record_id.map(|id| id.as_uuid()),
        replacement.valid_from,
        replacement.valid_to,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;

    if rows.is_empty() {
        return Ok(None);
    }
    // `hop` is the union's discriminator here, not a distance: 0 is the
    // closed claim and 1 is its replacement. Reusing the row type keeps
    // one edge-shaped decoder in this module rather than three.
    let mut closed = None;
    let mut replaced = None;
    for row in rows {
        let (marker, edge) = row.split()?;
        if marker == 0 {
            closed = Some(edge);
        } else {
            replaced = Some(edge);
        }
    }
    match (closed, replaced) {
        (Some(closed), Some(replacement)) => {
            metrics::counter!(GRAPH_EDGES_TOTAL, "graph" => graph.as_str()).increment(1);
            Ok(Some(Supersession {
                closed,
                replacement,
            }))
        }
        // The insert selects from the update, so one without the other
        // cannot happen — a statement that returned it is a schema bug.
        _ => Err(Error::Internal {
            message: format!("supersession of edge {id} returned a partial result"),
        }),
    }
}

/// Asserts a claim that may already hold (GRPH-2, ADR-0044 decision 11).
///
/// The idempotent half of [`insert_edge`]: where that one reports a
/// [`Error::Conflict`] on a second write, this returns `Ok(None)` when an
/// open claim of the same `(graph, kind, src, dst)` is already recorded —
/// no second row, no history row, and no increment of
/// [`GRAPH_EDGES_TOTAL`], because re-asserting what already holds asserts
/// nothing. That is what makes a linker safe to re-drive by design rather
/// than by care.
///
/// The conflict target is `graph_edges_open_claim_unique` (migration
/// 0027), which is partial on `valid_to is null`: a superseded claim
/// leaves its closed row behind, and only the open one is unique. A
/// collision on the primary key is *not* absorbed — a duplicate edge id is
/// a bug in the caller, and it still surfaces as an error.
#[tracing::instrument(
    name = "store.graph.assert_edge",
    skip_all,
    fields(tenant.id = %tenant_id, graph = %graph, edge.id = %id, edge.kind = %state.kind),
    err(Display)
)]
pub async fn assert_edge(
    executor: impl PgExecutor<'_>,
    id: GraphEdgeId,
    tenant_id: TenantId,
    graph: Graph,
    state: &EdgeState,
) -> Result<Option<EdgeVersion>> {
    check_confidence(state.confidence_permille)?;
    let row = sqlx::query_as!(
        EdgeRow,
        r#"
        insert into graph_edges
            (id, tenant_id, graph, kind, src_id, dst_id, method,
             confidence_permille, source_record_id, valid_from, valid_to, tx_from)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
        on conflict (tenant_id, graph, kind, src_id, dst_id)
            where valid_to is null do nothing
        returning id as "id!", tenant_id as "tenant_id!", graph as "graph!",
                  kind as "kind!", src_id as "src_id!", dst_id as "dst_id!",
                  method as "method!", confidence_permille as "confidence_permille!",
                  source_record_id, valid_from as "valid_from!", valid_to,
                  tx_from as "tx_from!", tx_to
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        graph.as_str(),
        state.kind,
        state.src_id.as_uuid(),
        state.dst_id.as_uuid(),
        state.method,
        state.confidence_permille,
        state.source_record_id.map(|id| id.as_uuid()),
        state.valid_from,
        state.valid_to,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    if row.is_some() {
        metrics::counter!(GRAPH_EDGES_TOTAL, "graph" => graph.as_str()).increment(1);
    }
    row.map(TryInto::try_into).transpose()
}

/// One supersession, seen as an edge of the `provenance` graph
/// (ADR-0044 decision 14).
///
/// Not a row of `graph_edges` and never written to one: `record_supersessions`
/// (migration 0024) stays the single system of record for this claim, because
/// the write path reads it inside the record's own transaction (ADR-0039
/// option 7). This type is the projection ADR-0043 decision 11 called for —
/// the same fact, in the graph's vocabulary, so a caller can fuse it with
/// [`expand`]'s output without knowing which table it came from.
///
/// Endpoints are [`RecordId`]s rather than [`GraphVertexId`]s because the
/// records are not vertices. Minting vertices for them would *be* the mirror
/// this projection exists to avoid — and both halves of GRPH-3's fusion trade
/// in candidate record ids anyway (ADR-0042 decision 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceEdge {
    /// The record that closed the other's window: the claim's source end.
    pub superseding_id: RecordId,
    /// The record whose window closed: its destination.
    pub superseded_id: RecordId,
    /// The judge that decided (`deterministic` today) — the same seam name
    /// [`EdgeState::method`] carries.
    pub method: String,
    /// The verdict class, machine-readable and short.
    pub reason: String,
    /// Jaccard as integer per mille, when the lexical leg scored the pair.
    pub jaccard_permille: Option<i32>,
    /// Cosine as integer per mille, when the semantic leg did.
    pub cosine_permille: Option<i32>,
    /// When the claim started holding: the instant the superseded record's
    /// window was closed at.
    pub valid_from: DateTime<Utc>,
}

impl ProvenanceEdge {
    /// The graph this projection belongs to. A constant rather than a
    /// field: every row of it is a `provenance` claim.
    pub const GRAPH: Graph = Graph::Provenance;

    /// The relation type, in the same open vocabulary
    /// [`EdgeState::kind`] uses.
    pub const KIND: &'static str = "supersedes";
}

/// Every supersession naming any of `records` on either side, projected as
/// `provenance` edges and totally ordered (ADR-0044 decision 14).
///
/// The batch form of [`crate::dedup::supersessions_for`], in the edge
/// model: GRPH-3 holds a candidate set rather than one record. There is no
/// seed cap here and there should not be — this is an indexed read by
/// record id, not a traversal, so nothing fans out and nothing needs
/// bounding.
#[tracing::instrument(
    name = "store.graph.supersession_edges",
    skip_all,
    fields(tenant.id = %tenant_id, records.count = records.len()),
    err(Display)
)]
pub async fn supersession_edges(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    records: &[RecordId],
) -> Result<Vec<ProvenanceEdge>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let records: Vec<Uuid> = records.iter().map(|id| id.as_uuid()).collect();
    let rows = sqlx::query!(
        r#"
        select superseding_id, superseded_id, method, reason,
               jaccard_permille, cosine_permille, closed_at
        from record_supersessions
        where tenant_id = $1
          and (superseded_id = any($2) or superseding_id = any($2))
        order by closed_at, superseding_id, superseded_id
        "#,
        tenant_id.as_uuid(),
        &records,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| ProvenanceEdge {
            superseding_id: RecordId::from_uuid(row.superseding_id),
            superseded_id: RecordId::from_uuid(row.superseded_id),
            method: row.method,
            reason: row.reason,
            jaccard_permille: row.jaccard_permille,
            cosine_permille: row.cosine_permille,
            valid_from: row.closed_at,
        })
        .collect())
}

/// The current version of edge `id`, if it has one.
#[tracing::instrument(
    name = "store.graph.edge",
    skip_all,
    fields(tenant.id = %tenant_id, edge.id = %id),
    err(Display)
)]
pub async fn edge(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GraphEdgeId,
) -> Result<Option<EdgeVersion>> {
    let row = sqlx::query_as!(
        EdgeRow,
        r#"
        select id, tenant_id, graph, kind, src_id, dst_id, method,
               confidence_permille, source_record_id, valid_from, valid_to,
               tx_from, tx_to
        from graph_edges
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Transaction-time as-of: the version of edge `id` the database held as
/// truth at `tx_at` — "what did we claim at time T". Transaction periods
/// are half-open `[tx_from, tx_to)`, so a version is visible from the
/// exact instant it was written.
#[tracing::instrument(
    name = "store.graph.edge_as_of",
    skip_all,
    fields(tenant.id = %tenant_id, edge.id = %id, tx_at = %tx_at),
    err(Display)
)]
pub async fn edge_as_of(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GraphEdgeId,
    tx_at: DateTime<Utc>,
) -> Result<Option<EdgeVersion>> {
    let row = sqlx::query_as!(
        EdgeRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", graph as "graph!",
               kind as "kind!", src_id as "src_id!", dst_id as "dst_id!",
               method as "method!", confidence_permille as "confidence_permille!",
               source_record_id, valid_from as "valid_from!", valid_to,
               tx_from as "tx_from!", tx_to
        from graph_edges_versions
        where tenant_id = $1 and id = $2
          and tx_from <= $3 and (tx_to is null or tx_to > $3)
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        tx_at,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Every version of edge `id` the database has ever known, oldest first.
#[tracing::instrument(
    name = "store.graph.edge_versions",
    skip_all,
    fields(tenant.id = %tenant_id, edge.id = %id),
    err(Display)
)]
pub async fn edge_versions(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GraphEdgeId,
) -> Result<Vec<EdgeVersion>> {
    let rows = sqlx::query_as!(
        EdgeRow,
        r#"
        select id as "id!", tenant_id as "tenant_id!", graph as "graph!",
               kind as "kind!", src_id as "src_id!", dst_id as "dst_id!",
               method as "method!", confidence_permille as "confidence_permille!",
               source_record_id, valid_from as "valid_from!", valid_to,
               tx_from as "tx_from!", tx_to
        from graph_edges_versions
        where tenant_id = $1 and id = $2
        order by tx_from
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// **The only traversal in the product** (ADR-0043 decisions 2 and 9).
///
/// Walks `graph` outward from `seeds` for `depth` hops, undirected — a
/// seed matches either endpoint, because "who does Ada report to" and
/// "who reports to Ada" are one question about one edge. Returns the
/// claims traversed and the vertices they reached, both totally ordered.
///
/// The two instants are the bitemporal pair the corpus already answers
/// with. `valid_at` selects claims whose window covers that moment;
/// `as_of` is transaction time — `None` reads current truth from
/// `graph_edges`, and `Some(t)` rewinds through `graph_edges_versions` to
/// what the database claimed at `t`. Rewinding the graph rewinds the
/// corpus and never the authority: this call decides nothing, and the
/// candidates it produces are narrowed by admission downstream
/// (decision 12).
///
/// # Errors
///
/// [`Error::Invalid`] if `seeds` exceeds [`MAX_EXPANSION_SEEDS`]. An
/// empty seed set returns an empty expansion **without touching an
/// index** — the CTX-1 property, inherited: there is no traversal that
/// walks everything.
#[tracing::instrument(
    name = "store.graph.expand",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        graph = %graph,
        depth = %depth,
        seeds.count = seeds.len(),
        edges = tracing::field::Empty,
        reached = tracing::field::Empty
    ),
    err(Display)
)]
pub async fn expand(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    graph: Graph,
    seeds: &[GraphVertexId],
    depth: Depth,
    valid_at: DateTime<Utc>,
    as_of: Option<DateTime<Utc>>,
) -> Result<Expansion> {
    if seeds.len() > MAX_EXPANSION_SEEDS {
        return Err(Error::Invalid {
            message: format!(
                "expansion takes at most {MAX_EXPANSION_SEEDS} seeds; {} were given",
                seeds.len()
            ),
        });
    }
    if seeds.is_empty() {
        return Ok(Expansion::default());
    }
    let seed_uuids: Vec<Uuid> = seeds.iter().map(GraphVertexId::as_uuid).collect();
    let tenant = tenant_id.as_uuid();
    let graph_name = graph.as_str();

    let started = std::time::Instant::now();
    // Four shipped statements, one per (depth, time mode). Written out
    // rather than assembled, because ADR-0001's "enumerate every SQL
    // statement in the binary" claim is only worth having if it is
    // literally true — and because each of these four is what the AC
    // suite reads `explain (format json)` for.
    //
    // The `-- shipped-traversal:` marker on each is load-bearing, not
    // decoration: `tests/graph.rs` finds the statements *in this source
    // file* by that marker and explains exactly the text that ships, so
    // the plan assertion cannot drift from the query the way a copy in a
    // test would. A fifth statement here without a marker fails that test.
    let rows = match (depth, as_of) {
        (Depth::One, None) => {
            sqlx::query_as!(
                ExpandRow,
                r#"
                -- shipped-traversal: depth=one time=current
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from graph_edges
                where tenant_id = $1 and graph = $2 and src_id = any($3)
                  and valid_from <= $4 and (valid_to is null or valid_to > $4)
                union all
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from graph_edges
                where tenant_id = $1 and graph = $2 and dst_id = any($3)
                  and valid_from <= $4 and (valid_to is null or valid_to > $4)
                "#,
                tenant,
                graph_name,
                &seed_uuids,
                valid_at,
            )
            .fetch_all(&mut *conn)
            .await
        }
        (Depth::Two, None) => {
            sqlx::query_as!(
                ExpandRow,
                r#"
                -- shipped-traversal: depth=two time=current
                with hop1 as (
                    select id, tenant_id, graph, kind, src_id, dst_id, method,
                           confidence_permille, source_record_id, valid_from,
                           valid_to, tx_from, tx_to
                    from graph_edges
                    where tenant_id = $1 and graph = $2 and src_id = any($3)
                      and valid_from <= $4 and (valid_to is null or valid_to > $4)
                    union all
                    select id, tenant_id, graph, kind, src_id, dst_id, method,
                           confidence_permille, source_record_id, valid_from,
                           valid_to, tx_from, tx_to
                    from graph_edges
                    where tenant_id = $1 and graph = $2 and dst_id = any($3)
                      and valid_from <= $4 and (valid_to is null or valid_to > $4)
                ),
                frontier as (
                    select distinct vid
                    from (
                        select src_id as vid from hop1
                        union all
                        select dst_id as vid from hop1
                    ) endpoints
                ),
                hop2 as (
                    select e.id, e.tenant_id, e.graph, e.kind, e.src_id, e.dst_id,
                           e.method, e.confidence_permille, e.source_record_id,
                           e.valid_from, e.valid_to, e.tx_from, e.tx_to
                    from graph_edges e join frontier f on e.src_id = f.vid
                    where e.tenant_id = $1 and e.graph = $2
                      and e.valid_from <= $4 and (e.valid_to is null or e.valid_to > $4)
                    union all
                    select e.id, e.tenant_id, e.graph, e.kind, e.src_id, e.dst_id,
                           e.method, e.confidence_permille, e.source_record_id,
                           e.valid_from, e.valid_to, e.tx_from, e.tx_to
                    from graph_edges e join frontier f on e.dst_id = f.vid
                    where e.tenant_id = $1 and e.graph = $2
                      and e.valid_from <= $4 and (e.valid_to is null or e.valid_to > $4)
                )
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from hop1
                union all
                select 2 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from hop2
                "#,
                tenant,
                graph_name,
                &seed_uuids,
                valid_at,
            )
            .fetch_all(&mut *conn)
            .await
        }
        (Depth::One, Some(tx_at)) => {
            sqlx::query_as!(
                ExpandRow,
                r#"
                -- shipped-traversal: depth=one time=as-of
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from graph_edges_versions
                where tenant_id = $1 and graph = $2 and src_id = any($3)
                  and valid_from <= $4 and (valid_to is null or valid_to > $4)
                  and tx_from <= $5 and (tx_to is null or tx_to > $5)
                union all
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from graph_edges_versions
                where tenant_id = $1 and graph = $2 and dst_id = any($3)
                  and valid_from <= $4 and (valid_to is null or valid_to > $4)
                  and tx_from <= $5 and (tx_to is null or tx_to > $5)
                "#,
                tenant,
                graph_name,
                &seed_uuids,
                valid_at,
                tx_at,
            )
            .fetch_all(&mut *conn)
            .await
        }
        (Depth::Two, Some(tx_at)) => {
            sqlx::query_as!(
                ExpandRow,
                r#"
                -- shipped-traversal: depth=two time=as-of
                with hop1 as (
                    select id, tenant_id, graph, kind, src_id, dst_id, method,
                           confidence_permille, source_record_id, valid_from,
                           valid_to, tx_from, tx_to
                    from graph_edges_versions
                    where tenant_id = $1 and graph = $2 and src_id = any($3)
                      and valid_from <= $4 and (valid_to is null or valid_to > $4)
                      and tx_from <= $5 and (tx_to is null or tx_to > $5)
                    union all
                    select id, tenant_id, graph, kind, src_id, dst_id, method,
                           confidence_permille, source_record_id, valid_from,
                           valid_to, tx_from, tx_to
                    from graph_edges_versions
                    where tenant_id = $1 and graph = $2 and dst_id = any($3)
                      and valid_from <= $4 and (valid_to is null or valid_to > $4)
                      and tx_from <= $5 and (tx_to is null or tx_to > $5)
                ),
                frontier as (
                    select distinct vid
                    from (
                        select src_id as vid from hop1
                        union all
                        select dst_id as vid from hop1
                    ) endpoints
                ),
                hop2 as (
                    select e.id, e.tenant_id, e.graph, e.kind, e.src_id, e.dst_id,
                           e.method, e.confidence_permille, e.source_record_id,
                           e.valid_from, e.valid_to, e.tx_from, e.tx_to
                    from graph_edges_versions e join frontier f on e.src_id = f.vid
                    where e.tenant_id = $1 and e.graph = $2
                      and e.valid_from <= $4 and (e.valid_to is null or e.valid_to > $4)
                      and e.tx_from <= $5 and (e.tx_to is null or e.tx_to > $5)
                    union all
                    select e.id, e.tenant_id, e.graph, e.kind, e.src_id, e.dst_id,
                           e.method, e.confidence_permille, e.source_record_id,
                           e.valid_from, e.valid_to, e.tx_from, e.tx_to
                    from graph_edges_versions e join frontier f on e.dst_id = f.vid
                    where e.tenant_id = $1 and e.graph = $2
                      and e.valid_from <= $4 and (e.valid_to is null or e.valid_to > $4)
                      and e.tx_from <= $5 and (e.tx_to is null or e.tx_to > $5)
                )
                select 1 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from hop1
                union all
                select 2 as "hop!", id as "id!", tenant_id as "tenant_id!",
                       graph as "graph!", kind as "kind!", src_id as "src_id!",
                       dst_id as "dst_id!", method as "method!",
                       confidence_permille as "confidence_permille!",
                       source_record_id, valid_from as "valid_from!", valid_to,
                       tx_from as "tx_from!", tx_to
                from hop2
                "#,
                tenant,
                graph_name,
                &seed_uuids,
                valid_at,
                tx_at,
            )
            .fetch_all(&mut *conn)
            .await
        }
    }
    .map_err(storage_error)?;

    metrics::histogram!(
        GRAPH_EXPANSION_SECONDS,
        "graph" => graph.as_str(),
        "depth" => depth.as_str(),
    )
    .record(started.elapsed().as_secs_f64());

    let expansion = fold(rows, seeds)?;
    tracing::Span::current().record("edges", expansion.edges.len());
    tracing::Span::current().record("reached", expansion.reached.len());
    Ok(expansion)
}

/// Turns the traversal's rows into the ordered answer: one entry per
/// distinct edge at the nearest hop that found it, and the non-seed
/// vertices those edges touch.
///
/// Both orderings are total and computed here rather than in SQL, so they
/// hold identically across the four statements above and cannot drift
/// with a plan (the determinism note).
fn fold(rows: Vec<ExpandRow>, seeds: &[GraphVertexId]) -> Result<Expansion> {
    let seeds: BTreeSet<GraphVertexId> = seeds.iter().copied().collect();
    // Keyed by edge id so the same claim found by both legs, or at both
    // hops, is one answer at its nearest hop.
    let mut edges: BTreeMap<GraphEdgeId, (u8, EdgeVersion)> = BTreeMap::new();
    for row in rows {
        let (hop, edge) = row.split()?;
        match edges.get_mut(&edge.id) {
            Some((known, _)) if *known <= hop => {}
            Some(slot) => slot.0 = hop,
            None => {
                edges.insert(edge.id, (hop, edge));
            }
        }
    }

    let mut reached: BTreeMap<GraphVertexId, u8> = BTreeMap::new();
    for (hop, edge) in edges.values() {
        for endpoint in [edge.state.src_id, edge.state.dst_id] {
            if seeds.contains(&endpoint) {
                continue;
            }
            reached
                .entry(endpoint)
                .and_modify(|known| *known = (*known).min(*hop))
                .or_insert(*hop);
        }
    }

    let mut edges: Vec<EdgeVersion> = edges.into_values().map(|(_, edge)| edge).collect();
    edges.sort_by(|a, b| a.state.kind.cmp(&b.state.kind).then(a.id.cmp(&b.id)));
    let mut reached: Vec<Reached> = reached
        .into_iter()
        .map(|(vertex_id, hop)| Reached { hop, vertex_id })
        .collect();
    reached.sort();

    Ok(Expansion { edges, reached })
}
