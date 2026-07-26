//! Hybrid retrieval (CTX-1, ADR-0024): the dense pgvector leg and the
//! sparse Tantivy leg, fused by reciprocal rank, verified and hydrated
//! from current Postgres truth.
//!
//! The filter is mandatory and fails empty (decision 2): an empty pair set
//! returns no results without touching either index — there is no
//! unfiltered code path to call. Since AUTHZ-5 the pairs are the PDP's own
//! answer per scope *and* tier (ADR-0038 decision 3); the old blanket clamp
//! below `restricted` is gone, because a clamp decides nothing and a
//! decision that nothing asks for grants nothing. Both legs are
//! optional-but-at-least-one: no query vector → BM25 only (the
//! embedder-down degradation CTX-3 leans on); no tenant index yet →
//! dense only.
//!
//! No LLM — no network beyond Postgres — runs here, structurally: the
//! query vector is the caller's input (the gateway owns the MEM-4
//! `Embedder` seam) and this crate carries no HTTP client (decision 7).

use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_store::records::RecordVersion;
use synveda_store::search::{self, DenseHit};
use synveda_types::{Error, RecordId, Result, ScopeId, ScopeTier, Sensitivity, TenantId};

use crate::index::{SearchIndex, SparseHit};
use crate::{RETRIEVAL_LEG_SECONDS, RETRIEVAL_SEARCHES_TOTAL};

/// The reciprocal-rank-fusion constant (the literature's default;
/// ADR-0024 decision 6).
pub const RRF_K: f64 = 60.0;

/// The authz-derived pushdown predicate: the PDP-allowed `(scope, tier)`
/// pairs, from [`crate::authz::permitted_chain_scopes`] or a composition
/// plan in the product paths.
#[derive(Debug, Clone)]
pub struct SearchFilter {
    /// Pairs the caller may read. Empty = no results, without touching an
    /// index (ADR-0024 decision 2's fail-empty rule, unchanged).
    ///
    /// A pair set rather than scopes plus a ceiling: one scope on a chain
    /// may admit `confidential` through an explicit binding while the next
    /// admits only the working tiers, and no single ceiling can say that
    /// (ADR-0038 decision 3).
    pub tiers: Vec<ScopeTier>,
}

impl SearchFilter {
    /// One scope's allowed set, expanded into pairs.
    #[must_use]
    pub fn for_scope(scope_id: ScopeId, sensitivities: &[Sensitivity]) -> Self {
        SearchFilter {
            tiers: ScopeTier::expand(scope_id, sensitivities),
        }
    }

    /// The distinct scopes the pairs name — the tracing field and the
    /// sparse leg's scope term.
    #[must_use]
    pub fn scopes(&self) -> Vec<ScopeId> {
        let mut scopes: Vec<ScopeId> = self.tiers.iter().map(|pair| pair.scope_id).collect();
        scopes.sort_unstable();
        scopes.dedup();
        scopes
    }
}

/// A pre-computed query embedding (the caller's, never this crate's —
/// ADR-0024 decision 7).
#[derive(Debug, Clone)]
pub struct QueryVector {
    /// The model that produced it; only vectors written by the same
    /// model are comparable, so the dense leg filters on it.
    pub model: String,
    /// The query's vector.
    pub vector: Vec<f32>,
}

/// One hybrid search.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// The lexical query (BM25 leg).
    pub query: String,
    /// The dense leg's query embedding; `None` degrades to BM25-only.
    pub vector: Option<QueryVector>,
    /// The mandatory pushdown predicate.
    pub filter: SearchFilter,
    /// Results returned after fusion and verification.
    pub limit: usize,
    /// Candidates fetched per leg before fusion.
    pub per_leg: usize,
    /// The valid-time instant results are verified against (MEM-5,
    /// ADR-0039 decision 11): a record whose window closed before this
    /// instant is no longer the current assertion and does not come back.
    ///
    /// An explicit input, never a clock read here, for the same reason
    /// [`crate::compose::ComposeRequest::at`] is one — and so the two
    /// stages of one inject agree about *when* they are.
    pub at: DateTime<Utc>,
}

impl SearchRequest {
    /// A request with the default depths (10 results, 50 per leg), verified
    /// as of `at`.
    #[must_use]
    pub fn new(query: impl Into<String>, filter: SearchFilter, at: DateTime<Utc>) -> Self {
        Self {
            query: query.into(),
            vector: None,
            filter,
            limit: 10,
            per_leg: 50,
            at,
        }
    }
}

/// One fused, verified, hydrated result.
#[derive(Debug, Clone)]
pub struct RetrievedRecord {
    /// The record's current version, read from Postgres after fusion —
    /// never from the sidecar (ADR-0024 decision 6).
    pub record: RecordVersion,
    /// The fused reciprocal-rank score (higher is better).
    pub score: f64,
    /// 1-based rank in the dense leg, if it surfaced there.
    pub dense_rank: Option<usize>,
    /// 1-based rank in the sparse leg, if it surfaced there.
    pub sparse_rank: Option<usize>,
}

/// Runs the hybrid search inside the caller's tenant transaction
/// (`rls::begin_tenant_tx` — the RLS discipline, and the dense leg's
/// transaction-local HNSW tuning needs it).
#[tracing::instrument(
    name = "retrieval.hybrid_search",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        pairs.count = request.filter.tiers.len(),
        limit = request.limit,
        mode = tracing::field::Empty,
        results = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn hybrid_search(
    conn: &mut PgConnection,
    index: &SearchIndex,
    tenant_id: TenantId,
    request: &SearchRequest,
) -> Result<Vec<RetrievedRecord>> {
    let span = tracing::Span::current();
    if request.filter.tiers.is_empty() {
        span.record("mode", "empty_filter");
        span.record("results", 0);
        metrics::counter!(RETRIEVAL_SEARCHES_TOTAL, "mode" => "empty_filter").increment(1);
        return Ok(vec![]);
    }
    let allowed = &request.filter.tiers;
    let scopes = request.filter.scopes();
    // The sparse leg indexes scope and tier as separate terms, so it takes
    // the two unions and can admit a pair no scope actually permits — a
    // *candidate* generator, exactly as it was for scope alone. Hydration
    // re-applies the pairs against current Postgres truth, which is where
    // the predicate is enforced (ADR-0024 decision 6, ADR-0038 decision 3).
    let sensitivities = union_sensitivities(allowed);
    let per_leg = request.per_leg.max(request.limit).max(1);

    let started = Instant::now();
    let sparse =
        index.search_sparse(tenant_id, &request.query, &scopes, &sensitivities, per_leg)?;
    metrics::histogram!(RETRIEVAL_LEG_SECONDS, "leg" => "sparse")
        .record(started.elapsed().as_secs_f64());

    let dense = match &request.vector {
        Some(query_vector) => {
            if query_vector.vector.is_empty() {
                return Err(Error::Invalid {
                    message: "query vector must not be empty".to_owned(),
                });
            }
            let started = Instant::now();
            let hits = search::dense_candidates(
                conn,
                tenant_id,
                &query_vector.model,
                &query_vector.vector,
                allowed,
                per_leg as i64,
            )
            .await?;
            metrics::histogram!(RETRIEVAL_LEG_SECONDS, "leg" => "dense")
                .record(started.elapsed().as_secs_f64());
            hits
        }
        None => vec![],
    };

    let mode = match (request.vector.is_some(), sparse.is_empty()) {
        (true, _) => "hybrid",
        (false, _) => "sparse_only",
    };
    // A tenant with no sidecar index yet has an empty sparse leg even
    // in hybrid mode; keep the label honest for dashboards.
    let mode = if request.vector.is_some() && sparse.is_empty() && !dense.is_empty() {
        "dense_only"
    } else {
        mode
    };
    span.record("mode", mode);
    metrics::counter!(RETRIEVAL_SEARCHES_TOTAL, "mode" => mode).increment(1);

    let fused = fuse(&dense, &sparse);
    if fused.is_empty() {
        span.record("results", 0);
        return Ok(vec![]);
    }
    // Hydrate with headroom: the verify re-check may drop candidates the
    // sidecar remembered but current truth no longer permits.
    let candidates: Vec<RecordId> = fused
        .iter()
        .take(request.limit.saturating_mul(2).max(request.limit))
        .map(|entry| entry.record_id)
        .collect();
    let started = Instant::now();
    let hydrated =
        search::hydrate_verified(conn, tenant_id, &candidates, allowed, request.at).await?;
    metrics::histogram!(RETRIEVAL_LEG_SECONDS, "leg" => "hydrate")
        .record(started.elapsed().as_secs_f64());
    let mut by_id: HashMap<RecordId, RecordVersion> = hydrated
        .into_iter()
        .map(|version| (version.id, version))
        .collect();
    let results: Vec<RetrievedRecord> = fused
        .into_iter()
        .filter_map(|entry| {
            by_id
                .remove(&entry.record_id)
                .map(|record| RetrievedRecord {
                    record,
                    score: entry.score,
                    dense_rank: entry.dense_rank,
                    sparse_rank: entry.sparse_rank,
                })
        })
        .take(request.limit)
        .collect();
    span.record("results", results.len());
    Ok(results)
}

/// One fused candidate, ordered best-first.
#[derive(Debug)]
struct FusedCandidate {
    record_id: RecordId,
    score: f64,
    dense_rank: Option<usize>,
    sparse_rank: Option<usize>,
}

/// Reciprocal-rank fusion: `score(d) = Σ legs 1/(RRF_K + rank(d))`,
/// 1-based ranks. Ties break on record id for deterministic output.
fn fuse(dense: &[DenseHit], sparse: &[SparseHit]) -> Vec<FusedCandidate> {
    let mut merged: HashMap<RecordId, FusedCandidate> = HashMap::new();
    for (position, hit) in dense.iter().enumerate() {
        let rank = position + 1;
        let entry = merged
            .entry(hit.record_id)
            .or_insert_with(|| FusedCandidate {
                record_id: hit.record_id,
                score: 0.0,
                dense_rank: None,
                sparse_rank: None,
            });
        entry.score += 1.0 / (RRF_K + rank as f64);
        entry.dense_rank = Some(rank);
    }
    for (position, hit) in sparse.iter().enumerate() {
        let rank = position + 1;
        let entry = merged
            .entry(hit.record_id)
            .or_insert_with(|| FusedCandidate {
                record_id: hit.record_id,
                score: 0.0,
                dense_rank: None,
                sparse_rank: None,
            });
        entry.score += 1.0 / (RRF_K + rank as f64);
        entry.sparse_rank = Some(rank);
    }
    let mut fused: Vec<FusedCandidate> = merged.into_values().collect();
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    fused
}

/// Every tier some pair permits, ascending — the union the sidecar's own
/// term filter takes, and the hard ceiling on what can leave either index
/// before hydration re-applies the pairs (ADR-0038 decision 3).
pub(crate) fn union_sensitivities(allowed: &[ScopeTier]) -> Vec<Sensitivity> {
    Sensitivity::ALL
        .into_iter()
        .filter(|tier| allowed.iter().any(|pair| pair.sensitivity == *tier))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> RecordId {
        RecordId::from_uuid(uuid::Uuid::from_bytes([byte; 16]))
    }

    fn scope(byte: u8) -> ScopeId {
        ScopeId::from_uuid(uuid::Uuid::from_bytes([byte; 16]))
    }

    /// The union is what the sidecar's term filter takes, and it is a
    /// ceiling on what either index can yield — never a statement about any
    /// one scope, which is why hydration re-applies the pairs
    /// (ADR-0038 decision 3).
    #[test]
    fn the_union_is_every_tier_some_pair_names_and_no_more() {
        let mixed = SearchFilter {
            tiers: [
                ScopeTier::expand(scope(1), &[Sensitivity::Public, Sensitivity::Internal]),
                ScopeTier::expand(scope(2), &[Sensitivity::Confidential]),
            ]
            .concat(),
        };
        assert_eq!(
            union_sensitivities(&mixed.tiers),
            vec![
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential
            ],
            "ascending, deduplicated, and never a tier no pair named"
        );
        assert!(!union_sensitivities(&mixed.tiers).contains(&Sensitivity::Restricted));
        assert_eq!(union_sensitivities(&[]), Vec::<Sensitivity>::new());
    }

    /// There is no clamp here any more, and that is the feature: the engine
    /// executes the plan it is handed. `restricted` reaches these pairs only
    /// when the PDP put it there, which takes a lapse that declared the tier
    /// and therefore cleared the compliance floor.
    #[test]
    fn the_engine_carries_whatever_tier_the_plan_permitted() {
        let top = SearchFilter::for_scope(scope(1), &Sensitivity::ALL);
        assert!(union_sensitivities(&top.tiers).contains(&Sensitivity::Restricted));
        assert_eq!(top.scopes(), vec![scope(1)]);
        assert_eq!(top.tiers.len(), 4, "one pair per tier");
    }

    /// The RRF promise: a candidate on both legs outranks one that tops
    /// a single leg (1/61 + 1/62 > 1/61), and ranks are recorded 1-based.
    #[test]
    fn fusion_rewards_agreement_and_is_deterministic() {
        let dense = vec![
            DenseHit {
                record_id: id(1),
                distance: 0.1,
            },
            DenseHit {
                record_id: id(2),
                distance: 0.2,
            },
        ];
        let sparse = vec![
            SparseHit {
                record_id: id(3),
                score: 9.0,
            },
            SparseHit {
                record_id: id(2),
                score: 5.0,
            },
        ];
        let fused = fuse(&dense, &sparse);
        assert_eq!(fused[0].record_id, id(2), "on both legs → first");
        assert_eq!(fused[0].dense_rank, Some(2));
        assert_eq!(fused[0].sparse_rank, Some(2));
        // ids 1 and 3 both scored 1/61: the tie breaks on record id.
        assert_eq!(fused[1].record_id, id(1));
        assert_eq!(fused[2].record_id, id(3));
        let expected = 1.0 / (RRF_K + 2.0) + 1.0 / (RRF_K + 2.0);
        assert!((fused[0].score - expected).abs() < 1e-12);
    }
}
