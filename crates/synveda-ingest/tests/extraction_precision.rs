//! The MEM-3 precision AC (ADR-0022 decision 8): per-class extraction
//! precision on the labelled fixture set, asserted against the
//! provisional macro-precision target for the deterministic path. The
//! same harness runs against a live LLM through the `#[ignore]`d
//! [`live_precision`] test — the hook EVAL-2 grows into the real target,
//! dashboard, and calibration measurement.
//!
//! Fixture discipline mirrors the redaction suite: transcript-shaped,
//! documentation-only content, `[REDACTED:*]` placeholders included —
//! never real credentials, never network access in the asserted test.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use synveda_ingest::extraction::{
    AnyExtractor, ClaudeExtractor, DeterministicExtractor, ExtractionInput, Extractor,
    VllmExtractor,
};
use synveda_types::{IdentityId, ObserveEventId, ObserveKind, RecordClass, ScopeId, TenantId};

/// The provisional macro-averaged precision target (ADR-0022 decision 8);
/// EVAL-2 owns the real target.
const PROVISIONAL_TARGET: f64 = 0.8;

const FIXTURES: &str = include_str!("fixtures/extraction/labelled.json");

#[derive(Deserialize)]
struct Fixture {
    name: String,
    input: FixtureInput,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct FixtureInput {
    kind: ObserveKind,
    session_id: String,
    occurred_at: DateTime<Utc>,
    payload: serde_json::Value,
}

#[derive(Deserialize)]
struct Expected {
    class: RecordClass,
    #[serde(default)]
    content_contains: Option<String>,
}

fn fixtures() -> Vec<Fixture> {
    serde_json::from_str(FIXTURES).expect("labelled fixture set parses")
}

fn input(fixture: &FixtureInput) -> ExtractionInput {
    ExtractionInput {
        event_id: ObserveEventId::new(),
        tenant_id: TenantId::new(),
        scope_id: ScopeId::new(),
        owner_id: IdentityId::new(),
        session_id: fixture.session_id.clone(),
        kind: fixture.kind,
        payload: fixture.payload.clone(),
        occurred_at: fixture.occurred_at,
        redactions: None,
    }
}

/// Per-class `(correct, emitted)` plus the recall numerator/denominator.
struct Report {
    per_class: BTreeMap<&'static str, (usize, usize)>,
    matched_expected: usize,
    total_expected: usize,
}

impl Report {
    /// Macro-averaged precision over the classes the extractor emitted.
    fn macro_precision(&self) -> f64 {
        if self.per_class.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .per_class
            .values()
            .map(|(correct, emitted)| *correct as f64 / *emitted as f64)
            .sum();
        sum / self.per_class.len() as f64
    }

    fn print(&self, method: &str) {
        println!("extraction precision ({method}):");
        for (class, (correct, emitted)) in &self.per_class {
            println!(
                "  {class:<12} {correct}/{emitted} = {:.3}",
                *correct as f64 / *emitted as f64
            );
        }
        println!(
            "  macro precision {:.3}; recall (informational) {}/{}",
            self.macro_precision(),
            self.matched_expected,
            self.total_expected
        );
    }
}

/// An emission is correct when the ground truth expects its class and the
/// expected content token (a distinctive term any faithful summary keeps)
/// appears in it, case-insensitively.
async fn measure(extractor: &AnyExtractor, fixtures: &[Fixture]) -> Report {
    let mut per_class: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut matched_expected = 0usize;
    let mut total_expected = 0usize;
    for fixture in fixtures {
        let outcome = extractor
            .extract(&input(&fixture.input))
            .await
            .unwrap_or_else(|error| panic!("fixture {}: extractor failed: {error}", fixture.name));
        total_expected += fixture.expected.len();
        let mut expected_hit = vec![false; fixture.expected.len()];
        for candidate in &outcome.candidates {
            let entry = per_class.entry(candidate.class.as_str()).or_insert((0, 0));
            entry.1 += 1;
            let hit = fixture.expected.iter().position(|expected| {
                expected.class == candidate.class
                    && expected.content_contains.as_deref().is_none_or(|token| {
                        candidate
                            .content
                            .to_lowercase()
                            .contains(&token.to_lowercase())
                    })
            });
            if let Some(index) = hit {
                entry.0 += 1;
                if !expected_hit[index] {
                    expected_hit[index] = true;
                    matched_expected += 1;
                }
            }
        }
    }
    Report {
        per_class,
        matched_expected,
        total_expected,
    }
}

/// The AC: the deterministic extractor's macro precision meets the
/// provisional target on the labelled fixtures, and every fixture flows
/// through the exact seam the pipeline uses.
#[tokio::test]
async fn deterministic_precision_meets_provisional_target() {
    let extractor = AnyExtractor::Deterministic(DeterministicExtractor::new());
    let report = measure(&extractor, &fixtures()).await;
    report.print("deterministic");
    assert!(
        report.macro_precision() >= PROVISIONAL_TARGET,
        "macro precision {:.3} under the provisional target {PROVISIONAL_TARGET}",
        report.macro_precision()
    );
}

/// Redaction opacity (ADR-0021, STATUS's MEM-3 obligation): a
/// `[REDACTED:*]` placeholder in the input survives verbatim in the
/// extracted content — never dropped, never "filled in".
#[tokio::test]
async fn placeholders_survive_extraction_verbatim() {
    let extractor = AnyExtractor::Deterministic(DeterministicExtractor::new());
    let fixtures = fixtures();
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture.name == "fact-redaction-placeholder")
        .expect("placeholder fixture present");
    let outcome = extractor
        .extract(&input(&fixture.input))
        .await
        .expect("extract");
    assert_eq!(outcome.candidates.len(), 1);
    assert!(
        outcome.candidates[0]
            .content
            .contains("[REDACTED:github-pat]"),
        "placeholder must survive verbatim: {:?}",
        outcome.candidates[0].content
    );
}

/// Empty observations extract nothing — zero candidates is a legal,
/// non-error outcome.
#[tokio::test]
async fn empty_payload_extracts_nothing() {
    let extractor = AnyExtractor::Deterministic(DeterministicExtractor::new());
    let fixtures = fixtures();
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture.name == "empty-payload")
        .expect("empty fixture present");
    let outcome = extractor
        .extract(&input(&fixture.input))
        .await
        .expect("extract");
    assert!(outcome.candidates.is_empty());
}

/// The live-LLM hook (ADR-0022 decision 8): the same fixtures and the
/// same scoring against a real endpoint. Ignored by default — run it
/// deliberately:
///
/// ```sh
/// SYNVEDA_EXTRACTOR=claude ANTHROPIC_API_KEY=... \
///   cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture
/// # or, air-gapped:
/// SYNVEDA_EXTRACTOR=vllm SYNVEDA_VLLM_BASE_URL=http://... SYNVEDA_EXTRACTOR_MODEL=... \
///   cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "network + credentials; the deliberate live-LLM measurement"]
async fn live_precision() {
    let selected = std::env::var("SYNVEDA_EXTRACTOR").unwrap_or_default();
    let model = std::env::var("SYNVEDA_EXTRACTOR_MODEL").ok();
    let extractor = match selected.as_str() {
        "claude" => AnyExtractor::Claude(ClaudeExtractor::new(
            std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY"),
            model.unwrap_or_else(|| ClaudeExtractor::DEFAULT_MODEL.to_owned()),
            std::env::var("SYNVEDA_ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| ClaudeExtractor::DEFAULT_BASE_URL.to_owned()),
        )),
        "vllm" => AnyExtractor::Vllm(VllmExtractor::new(
            model.expect("SYNVEDA_EXTRACTOR_MODEL"),
            std::env::var("SYNVEDA_VLLM_BASE_URL").expect("SYNVEDA_VLLM_BASE_URL"),
        )),
        other => panic!("set SYNVEDA_EXTRACTOR=claude|vllm (got {other:?})"),
    };
    let report = measure(&extractor, &fixtures()).await;
    report.print(extractor.method());
    assert!(
        report.macro_precision() >= PROVISIONAL_TARGET,
        "macro precision {:.3} under the provisional target {PROVISIONAL_TARGET}",
        report.macro_precision()
    );
}
