//! CTX-1 retrieval quality on the fixture set (the AC; ADR-0024
//! decision 8).
//!
//! CI mode: vectors are synthetic topic-mixture geometry — meaningful
//! by construction — so this asserts the *engine*: reciprocal-rank
//! fusion must recall at least what either leg recalls alone, and the
//! fixture's absolute floor. Real-model quality runs the same fixture
//! through live TEI in the gateway's `#[ignore]`d harness
//! (`crates/synveda-gateway/tests/retrieval_live.rs`, the MEM-3
//! live-LLM pattern); EVAL-4 owns quality targets and regression gates.
//!
//! The fixture is adversarial to *each* leg by design (the textbook
//! hybrid case): a topic's lexically-reachable docs carry vectors
//! leaning toward a partner topic (the dense leg ranks the partner's
//! docs above them), and its paraphrase docs share no query terms (the
//! sparse leg cannot see them). Either leg alone plateaus near 0.5
//! recall; fusion must recover both halves.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_retrieval::hybrid::{QueryVector, SearchFilter, SearchRequest, hybrid_search};
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::{
    IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeTier, Sensitivity, TenantId,
};

const MODEL: &str = "fixture@1";

#[derive(Deserialize)]
struct DocsFile {
    docs: Vec<Doc>,
}

#[derive(Deserialize)]
struct Doc {
    key: String,
    topic_mix: Vec<f32>,
    content: String,
}

#[derive(Deserialize)]
struct QueriesFile {
    queries: Vec<Query>,
}

#[derive(Deserialize)]
struct Query {
    key: String,
    topic_mix: Vec<f32>,
    text: String,
    relevant: Vec<String>,
}

fn load_fixture() -> (DocsFile, QueriesFile) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/quality");
    let docs = std::fs::read_to_string(format!("{root}/docs.json")).expect("read docs.json");
    let queries =
        std::fs::read_to_string(format!("{root}/queries.json")).expect("read queries.json");
    (
        serde_json::from_str(&docs).expect("parse docs.json"),
        serde_json::from_str(&queries).expect("parse queries.json"),
    )
}

/// The synthetic geometry: topic weights on axes 0–3 plus a
/// key-derived jitter axis, L2-normalised. Deterministic; similar
/// topic mixes attract, distinct ones do not — exactly the property a
/// real embedding model supplies, minus the network (ADR-0023
/// decision 6 stands: `hash@1` noise is never a quality substrate;
/// this constructed geometry is one).
fn synthetic_vector(topic_mix: &[f32], key: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; 16];
    vector[..topic_mix.len()].copy_from_slice(topic_mix);
    let jitter = key.bytes().map(u32::from).sum::<u32>() as usize % 12;
    vector[4 + jitter] = 0.25;
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

struct Harness {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn harness() -> Option<&'static Harness> {
    static DB: OnceLock<Option<Harness>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping retrieval quality tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            synveda_store::migrate(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Harness { rt, pool })
    })
    .as_ref()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Metrics {
    /// Recall at the relevant-set size (6): the strict "found the whole
    /// topic" measure — top-10-of-24 would forgive a diluted ranking.
    recall_at_6: f64,
    mrr: f64,
}

async fn measure(
    pool: &PgPool,
    index: &SearchIndex,
    tenant: TenantId,
    scope: ScopeId,
    queries: &QueriesFile,
    ids_by_key: &HashMap<String, RecordId>,
    mode: &str,
) -> Metrics {
    let mut recall_sum = 0.0;
    let mut mrr_sum = 0.0;
    for query in &queries.queries {
        let mut request = SearchRequest::new(
            // Dense-only isolates the vector leg by matching no terms.
            if mode == "dense" {
                ""
            } else {
                query.text.as_str()
            },
            SearchFilter {
                tiers: ScopeTier::expand(scope, &[Sensitivity::Public, Sensitivity::Internal]),
            },
            chrono::Utc::now(),
        );
        request.limit = 6;
        if mode != "sparse" {
            request.vector = Some(QueryVector {
                model: MODEL.to_owned(),
                vector: synthetic_vector(&query.topic_mix, &query.key),
            });
        }
        let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
        let results = hybrid_search(&mut tx, index, tenant, &request)
            .await
            .expect("search");
        drop(tx);
        let relevant: Vec<RecordId> = query.relevant.iter().map(|key| ids_by_key[key]).collect();
        let hit_ranks: Vec<usize> = results
            .iter()
            .enumerate()
            .filter(|(_, hit)| relevant.contains(&hit.record.id))
            .map(|(position, _)| position + 1)
            .collect();
        if std::env::var("CTX1_QUALITY_DEBUG").is_ok() {
            let key_of = |id: RecordId| {
                ids_by_key
                    .iter()
                    .find(|(_, candidate)| **candidate == id)
                    .map(|(key, _)| key.as_str())
                    .unwrap_or("?")
            };
            let listing: Vec<String> = results
                .iter()
                .map(|hit| key_of(hit.record.id).to_string())
                .collect();
            eprintln!("  {} [{mode}]: {}", query.key, listing.join(" "));
        }
        recall_sum += hit_ranks.len() as f64 / relevant.len() as f64;
        mrr_sum += hit_ranks.first().map_or(0.0, |rank| 1.0 / *rank as f64);
    }
    let count = queries.queries.len() as f64;
    Metrics {
        recall_at_6: recall_sum / count,
        mrr: mrr_sum / count,
    }
}

/// The AC assertion: on the fixture set, fusion recalls at least what
/// either leg recalls alone, beats the single-leg plateau the fixture
/// builds in, and clears the absolute floor.
#[test]
fn hybrid_fusion_beats_either_leg_on_the_fixture_set() {
    let Some(db) = harness() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (docs, queries) = load_fixture();
        let (tenant, scope) = (TenantId::new(), ScopeId::new());
        let index = SearchIndex::open(
            std::env::temp_dir()
                .join("synveda-ctx1-quality")
                .join(tenant.to_string()),
        )
        .expect("open sidecar root");

        let mut ids_by_key = HashMap::new();
        for doc in &docs.docs {
            let id = RecordId::new();
            records::insert(
                pool,
                id,
                tenant,
                &RecordState {
                    scope_id: scope,
                    owner_id: IdentityId::new(),
                    kind: RecordKind::Derived,
                    class: RecordClass::Fact,
                    content: doc.content.clone(),
                    sensitivity: Sensitivity::Internal,
                    provenance: serde_json::json!({"source": "ctx-1 quality fixture"}),
                    valid_from: chrono::Utc::now(),
                    valid_to: None,
                },
                &RecordEmbedding {
                    model: MODEL.to_owned(),
                    vector: synthetic_vector(&doc.topic_mix, &doc.key),
                },
            )
            .await
            .expect("insert fixture doc");
            ids_by_key.insert(doc.key.clone(), id);
        }
        let config = IndexerConfig {
            overlap: Duration::ZERO,
            ..IndexerConfig::default()
        };
        let swept = indexer::sweep_tenant(pool, &index, tenant, &config)
            .await
            .expect("sweep");
        assert_eq!(swept.upserts as usize, docs.docs.len());

        let sparse = measure(pool, &index, tenant, scope, &queries, &ids_by_key, "sparse").await;
        let dense = measure(pool, &index, tenant, scope, &queries, &ids_by_key, "dense").await;
        let hybrid = measure(pool, &index, tenant, scope, &queries, &ids_by_key, "hybrid").await;
        eprintln!(
            "quality (synthetic geometry): sparse recall@6 {:.3} mrr {:.3} | \
             dense recall@6 {:.3} mrr {:.3} | hybrid recall@6 {:.3} mrr {:.3}",
            sparse.recall_at_6,
            sparse.mrr,
            dense.recall_at_6,
            dense.mrr,
            hybrid.recall_at_6,
            hybrid.mrr,
        );

        // Fixture sanity first: each leg alone must plateau, or the
        // fixture has stopped testing fusion.
        assert!(
            sparse.recall_at_6 < 0.75,
            "fixture sanity: the lexical leg alone plateaus (got {:.3})",
            sparse.recall_at_6
        );
        assert!(
            dense.recall_at_6 < 0.75,
            "fixture sanity: the dense leg alone plateaus (got {:.3})",
            dense.recall_at_6
        );
        // The fusion promise: strictly better than either blind leg,
        // and near-complete on the fixture.
        assert!(
            hybrid.recall_at_6 + 1e-9 >= sparse.recall_at_6,
            "fusion must not lose sparse recall: hybrid {:.3} < sparse {:.3}",
            hybrid.recall_at_6,
            sparse.recall_at_6
        );
        assert!(
            hybrid.recall_at_6 + 1e-9 >= dense.recall_at_6,
            "fusion must not lose dense recall: hybrid {:.3} < dense {:.3}",
            hybrid.recall_at_6,
            dense.recall_at_6
        );
        assert!(
            hybrid.recall_at_6 >= 0.9,
            "hybrid recall@6 {:.3} under the 0.9 fixture floor",
            hybrid.recall_at_6
        );
        assert!(
            hybrid.mrr >= 0.8,
            "hybrid MRR {:.3} under the 0.8 fixture floor",
            hybrid.mrr
        );
    });
}
