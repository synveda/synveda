//! CTX-1 live-model retrieval quality (ADR-0024 decision 8) — the
//! MEM-3 live-LLM pattern: the same fixture set the CI harness measures
//! with synthetic geometry, embedded here by real TEI (BGE-M3 dense,
//! 1024-d) so the dense leg's quality is the model's, not a
//! construction's. Run by `demos/ctx-1-hybrid-retrieval.sh` against the
//! dev stack:
//!
//! ```text
//! DATABASE_URL=… SYNVEDA_TEI_URL=http://localhost:8110 \
//!   cargo test -p synveda-gateway --test retrieval_live -- --ignored --nocapture
//! ```
//!
//! This crate hosts it because the layering rule forbids
//! `synveda-retrieval` from depending on `synveda-ingest` (the
//! `Embedder` seam) even for tests; the gateway sits above both.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use synveda_ingest::embedding::{Embedder as _, TeiEmbedder};
use synveda_retrieval::hybrid::{QueryVector, SearchFilter, SearchRequest, hybrid_search};
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::{
    IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeTier, Sensitivity, TenantId,
};

#[derive(Deserialize)]
struct DocsFile {
    docs: Vec<Doc>,
}

#[derive(Deserialize)]
struct Doc {
    key: String,
    content: String,
}

#[derive(Deserialize)]
struct QueriesFile {
    queries: Vec<Query>,
}

#[derive(Deserialize)]
struct Query {
    key: String,
    text: String,
    relevant: Vec<String>,
}

fn load_fixture() -> (DocsFile, QueriesFile) {
    let root = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../synveda-retrieval/tests/fixtures/quality"
    );
    let docs = std::fs::read_to_string(format!("{root}/docs.json")).expect("read docs.json");
    let queries =
        std::fs::read_to_string(format!("{root}/queries.json")).expect("read queries.json");
    (
        serde_json::from_str(&docs).expect("parse docs.json"),
        serde_json::from_str(&queries).expect("parse queries.json"),
    )
}

#[tokio::test]
#[ignore = "requires the dev stack's TEI (SYNVEDA_TEI_URL) and DATABASE_URL"]
async fn live_tei_hybrid_quality_on_the_fixture_set() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must point at the dev stack");
    let tei_url =
        std::env::var("SYNVEDA_TEI_URL").unwrap_or_else(|_| "http://localhost:8110".to_owned());
    let embedder = TeiEmbedder::new(TeiEmbedder::DEFAULT_MODEL.to_owned(), tei_url);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect");
    synveda_store::migrate(&pool).await.expect("migrations");

    let (docs, queries) = load_fixture();
    let (tenant, scope) = (TenantId::new(), ScopeId::new());
    let index = SearchIndex::open(
        std::env::temp_dir()
            .join("synveda-ctx1-live")
            .join(tenant.to_string()),
    )
    .expect("open sidecar root");

    // Embed and insert the fixture corpus through the real model.
    let contents: Vec<String> = docs.docs.iter().map(|doc| doc.content.clone()).collect();
    let vectors = embedder.embed(&contents).await.expect(
        "TEI must be reachable (dev compose: make dev-up; SYNVEDA_TEI_URL=http://localhost:8110)",
    );
    let mut ids_by_key: HashMap<String, RecordId> = HashMap::new();
    for (doc, vector) in docs.docs.iter().zip(vectors) {
        let id = RecordId::new();
        records::insert(
            &pool,
            id,
            tenant,
            &RecordState {
                scope_id: scope,
                owner_id: IdentityId::new(),
                kind: RecordKind::Derived,
                class: RecordClass::Fact,
                content: doc.content.clone(),
                sensitivity: Sensitivity::Internal,
                provenance: serde_json::json!({"source": "ctx-1 live fixture"}),
                valid_from: chrono::Utc::now(),
                valid_to: None,
            },
            &RecordEmbedding {
                model: embedder.model().to_owned(),
                vector,
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
    let swept = indexer::sweep_tenant(&pool, &index, tenant, &config)
        .await
        .expect("sweep");
    assert_eq!(swept.upserts as usize, docs.docs.len());

    // Measure hybrid and the sparse-only degradation on the same corpus.
    let mut recall_hybrid = 0.0;
    let mut recall_sparse = 0.0;
    let mut mrr_hybrid = 0.0;
    for query in &queries.queries {
        let query_vector = embedder
            .embed(std::slice::from_ref(&query.text))
            .await
            .expect("embed query")
            .remove(0);
        let relevant: Vec<RecordId> = query
            .relevant
            .iter()
            .map(|key| ids_by_key[&key.clone()])
            .collect();
        let mut request = SearchRequest::new(
            query.text.as_str(),
            SearchFilter {
                tiers: ScopeTier::expand(scope, &[Sensitivity::Public, Sensitivity::Internal]),
            },
            chrono::Utc::now(),
        );
        request.limit = 6;
        for mode in ["sparse", "hybrid"] {
            let mut modal = request.clone();
            if mode == "hybrid" {
                modal.vector = Some(QueryVector {
                    model: embedder.model().to_owned(),
                    vector: query_vector.clone(),
                });
            }
            let mut tx = rls::begin_tenant_tx(&pool, tenant)
                .await
                .expect("tenant tx");
            let results = hybrid_search(&mut tx, &index, tenant, &modal)
                .await
                .expect("search");
            drop(tx);
            let hits: Vec<usize> = results
                .iter()
                .enumerate()
                .filter(|(_, hit)| relevant.contains(&hit.record.id))
                .map(|(position, _)| position + 1)
                .collect();
            let recall = hits.len() as f64 / relevant.len() as f64;
            match mode {
                "sparse" => recall_sparse += recall,
                _ => {
                    recall_hybrid += recall;
                    mrr_hybrid += hits.first().map_or(0.0, |rank| 1.0 / *rank as f64);
                }
            }
            eprintln!("  {} [{mode}]: recall@6 {recall:.3}", query.key);
        }
    }
    let count = queries.queries.len() as f64;
    let (recall_hybrid, recall_sparse, mrr_hybrid) = (
        recall_hybrid / count,
        recall_sparse / count,
        mrr_hybrid / count,
    );
    eprintln!(
        "live TEI ({}): sparse-only recall@6 {recall_sparse:.3} | \
         hybrid recall@6 {recall_hybrid:.3} mrr {mrr_hybrid:.3}",
        embedder.model()
    );
    assert!(
        recall_hybrid + 1e-9 >= recall_sparse,
        "hybrid must not lose the lexical leg's recall: {recall_hybrid:.3} < {recall_sparse:.3}"
    );
    assert!(
        recall_hybrid >= 0.7,
        "live hybrid recall@6 {recall_hybrid:.3} under the 0.7 floor \
         (EVAL-4 owns the real targets)"
    );
}
