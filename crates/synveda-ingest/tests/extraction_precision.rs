//! The MEM-3 precision AC (ADR-0022 decision 8) over EVAL-2's labelled
//! corpus: per-class precision and recall for the deterministic path,
//! asserted against the floor EVAL-2 registered. The same harness runs
//! against a live LLM through the `#[ignore]`d [`live_precision`] test.
//!
//! **The corpus lives at `evals/fixtures/extraction/`, not here**
//! (EVAL-2, ADR-0046 decision 7). One corpus, two readers: this test reads
//! it as a fast, hermetic, no-stack tripwire on the extractor *function*,
//! and `synveda-eval` reads the same files to measure the *product path*
//! over HTTP. Both deserialize the full format with
//! `deny_unknown_fields`, so a field added for one reader cannot be
//! silently ignored by the other. This is a data dependency and not a
//! crate one — the eval's empty dependency set (ADR-0028 decision 1)
//! is untouched.
//!
//! The two targets are deliberately different numbers with different
//! jobs (ADR-0046 decision 8): the floor here guards the extractor,
//! `evals/baseline.json` gates the product path.
//!
//! Fixture discipline mirrors the redaction suite: transcript-shaped,
//! documentation-only content, `[REDACTED:*]` placeholders included —
//! never real credentials, never network access in the asserted test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use synveda_ingest::extraction::{
    AnyExtractor, ClaudeExtractor, DeterministicExtractor, ExtractionInput, Extractor,
    VllmExtractor,
};
use synveda_types::knowledge::KnowledgeType;
use synveda_types::session::SessionEventType;
use synveda_types::{ScopeId, SessionEventId, SessionId, TenantId};

/// The macro-averaged precision floor for the deterministic path
/// (ADR-0046 decision 13), raising ADR-0022's provisional 0.8 on the
/// strength of what the pre-EVAL-2 fixture set already measured.
///
/// No recall floor is asserted: recall had never been measured anywhere
/// in this product before this corpus existed, and a floor invented
/// before a measurement is a wish. It is reported on every run and
/// `evals/baseline.json` is where it becomes a gate.
const DETERMINISTIC_PRECISION_FLOOR: f64 = 0.9;

/// One group file — one eval actor's worth of fixtures (ADR-0046
/// decision 2).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    /// The group's short name; the eval report prints it.
    #[allow(dead_code)]
    group: String,
    /// The eval actor whose home scope this group's records land at.
    /// Unused here — this reader has no gateway — and declared so the
    /// format stays one format.
    #[allow(dead_code)]
    actor: String,
    /// Why this group exists, for whoever adds to it next.
    #[allow(dead_code)]
    note: String,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
    input: FixtureInput,
    expected: Vec<Expected>,
    /// Phrases a hallucinating extractor would plausibly produce from
    /// this transcript and which the transcript does not support
    /// (ADR-0046 decision 6).
    #[serde(default)]
    must_not_extract: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureInput {
    event_type: SessionEventType,
    session_id: String,
    occurred_at: DateTime<Utc>,
    payload: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    // `class` is the shared pre-CPR-18 corpus spelling. CPR-13 owns the
    // corpus-wide re-point; this reader maps it directly to the Knowledge
    // vocabulary meanwhile, with no runtime compatibility path.
    #[serde(rename = "class")]
    knowledge_type: KnowledgeType,
    #[serde(default)]
    content_contains: Option<String>,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/fixtures/extraction")
}

/// Every group file in filename order, flattened. Filename order so two
/// runs report in the same order.
fn fixtures() -> Vec<Fixture> {
    let dir = corpus_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read the corpus {}: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{} holds no groups", dir.display());

    let mut fixtures = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let group: Group = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{} is not a valid group: {error}", path.display()));
        fixtures.extend(group.fixtures);
    }
    fixtures
}

fn input(fixture: &FixtureInput) -> ExtractionInput {
    ExtractionInput {
        event_id: SessionEventId::new(),
        tenant_id: TenantId::new(),
        scope_id: ScopeId::new(),
        session_id: SessionId::new(),
        principal_id: fixture.session_id.clone(),
        event_type: fixture.event_type,
        payload: fixture.payload.clone(),
        occurred_at: fixture.occurred_at,
        redactions: None,
    }
}

/// The gathered-text view of a payload — the same walk the extractor and
/// the redaction scanner take. Used only by the corpus guards.
fn gather_text(payload: &serde_json::Value) -> String {
    fn collect<'a>(value: &'a serde_json::Value, into: &mut Vec<&'a str>) {
        match value {
            serde_json::Value::String(text) => into.push(text),
            serde_json::Value::Array(items) => items.iter().for_each(|item| collect(item, into)),
            serde_json::Value::Object(map) => map.values().for_each(|item| collect(item, into)),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    collect(payload, &mut parts);
    parts.join(" ")
}

/// Per class: `(matched, produced)` for precision and `(matched,
/// expected)` for recall.
#[derive(Default)]
struct Report {
    precision: BTreeMap<&'static str, (usize, usize)>,
    recall: BTreeMap<&'static str, (usize, usize)>,
    bait_hits: Vec<String>,
    unmatched: Vec<String>,
}

fn macro_average(counts: &BTreeMap<&'static str, (usize, usize)>) -> f64 {
    if counts.is_empty() {
        return 0.0;
    }
    let sum: f64 = counts
        .values()
        .map(|(hit, total)| {
            if *total == 0 {
                0.0
            } else {
                *hit as f64 / *total as f64
            }
        })
        .sum();
    sum / counts.len() as f64
}

impl Report {
    fn macro_precision(&self) -> f64 {
        macro_average(&self.precision)
    }

    fn macro_recall(&self) -> f64 {
        macro_average(&self.recall)
    }

    fn print(&self, method: &str) {
        println!("extraction quality ({method}):");
        println!("  class        precision        recall");
        let classes: Vec<&&str> = self
            .precision
            .keys()
            .chain(self.recall.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for class in classes {
            let show = |counts: &BTreeMap<&'static str, (usize, usize)>| match counts.get(*class) {
                Some((hit, total)) if *total > 0 => {
                    format!("{hit}/{total} = {:.3}", *hit as f64 / *total as f64)
                }
                _ => "     —".to_owned(),
            };
            println!(
                "  {:<12} {:<16} {}",
                class,
                show(&self.precision),
                show(&self.recall)
            );
        }
        println!(
            "  macro precision {:.3}; macro recall {:.3} (reported, not asserted)",
            self.macro_precision(),
            self.macro_recall()
        );
        if !self.unmatched.is_empty() {
            println!(
                "  {} record(s) matched no expectation — the review queue for unanticipated invention:",
                self.unmatched.len()
            );
            for entry in &self.unmatched {
                println!("    {entry}");
            }
        }
    }
}

/// One candidate consumes at most one expectation and one expectation is
/// consumed at most once (ADR-0046 decision 5): without that, a single
/// record can satisfy three expectations and inflate recall into
/// nonsense.
async fn measure(extractor: &AnyExtractor, fixtures: &[Fixture]) -> Report {
    let mut report = Report::default();
    for fixture in fixtures {
        let outcome = extractor
            .extract(&input(&fixture.input))
            .await
            .unwrap_or_else(|error| panic!("fixture {}: extractor failed: {error}", fixture.name));

        for expected in &fixture.expected {
            report
                .recall
                .entry(expected.knowledge_type.as_str())
                .or_default()
                .1 += 1;
        }

        let mut taken = vec![false; fixture.expected.len()];
        for candidate in &outcome.candidates {
            report
                .precision
                .entry(candidate.knowledge_type.as_str())
                .or_default()
                .1 += 1;

            let hit = fixture
                .expected
                .iter()
                .enumerate()
                .position(|(index, expected)| {
                    !taken[index]
                        && expected.knowledge_type == candidate.knowledge_type
                        && expected.content_contains.as_deref().is_none_or(|token| {
                            candidate
                                .body_markdown
                                .to_lowercase()
                                .contains(&token.to_lowercase())
                        })
                });
            match hit {
                Some(index) => {
                    taken[index] = true;
                    report
                        .precision
                        .entry(candidate.knowledge_type.as_str())
                        .or_default()
                        .0 += 1;
                    report
                        .recall
                        .entry(candidate.knowledge_type.as_str())
                        .or_default()
                        .0 += 1;
                }
                None => report.unmatched.push(format!(
                    "{} [{}] {}",
                    fixture.name,
                    candidate.knowledge_type.as_str(),
                    candidate.body_markdown.chars().take(72).collect::<String>()
                )),
            }

            let content = format!(
                "{} {} {}",
                candidate.title, candidate.body_markdown, candidate.summary
            )
            .to_lowercase();
            for bait in &fixture.must_not_extract {
                if content.contains(&bait.to_lowercase()) {
                    report
                        .bait_hits
                        .push(format!("{}: fabricated {bait:?}", fixture.name));
                }
            }
        }
    }
    report
}

/// The AC: the deterministic extractor's macro precision meets the floor
/// EVAL-2 registered, over the shared corpus, through the exact seam the
/// pipeline uses. Recall is printed and not asserted.
#[tokio::test]
async fn deterministic_precision_meets_the_registered_floor() {
    let extractor = AnyExtractor::Deterministic(DeterministicExtractor::new());
    let report = measure(&extractor, &fixtures()).await;
    report.print("deterministic");
    assert!(
        report.macro_precision() >= DETERMINISTIC_PRECISION_FLOOR,
        "macro precision {:.3} under the registered floor {DETERMINISTIC_PRECISION_FLOOR}",
        report.macro_precision()
    );
}

/// A rule-based extractor copies spans; it cannot invent. That is a
/// property worth asserting rather than assuming (ADR-0046 decision 6):
/// this is what fails if a future templating or summarisation step
/// breaks it.
#[tokio::test]
async fn the_deterministic_path_fabricates_nothing() {
    let extractor = AnyExtractor::Deterministic(DeterministicExtractor::new());
    let report = measure(&extractor, &fixtures()).await;
    assert!(
        report.bait_hits.is_empty(),
        "the deterministic path fabricated content: {:?}",
        report.bait_hits
    );
}

/// A corpus guard, not an extractor test: an expected token absent from
/// its own source can never be matched, so a mislabelled fixture would
/// depress recall forever and silently. Both readers share this corpus,
/// so both would report the same wrong number.
#[test]
fn every_expected_token_appears_in_its_own_source() {
    for fixture in fixtures() {
        let text = gather_text(&fixture.input.payload).to_lowercase();
        for expected in &fixture.expected {
            if let Some(token) = &expected.content_contains {
                assert!(
                    text.contains(&token.to_lowercase()),
                    "{}: expected token {token:?} is absent from the source — a mislabelled \
                     fixture, not a real miss",
                    fixture.name
                );
            }
        }
    }
}

/// The mirror guard: bait present in its own source is not bait at all —
/// any faithful extractor would reproduce it, and the hallucination axis
/// would be measuring copying.
#[test]
fn every_bait_phrase_is_absent_from_its_own_source() {
    for fixture in fixtures() {
        let text = gather_text(&fixture.input.payload).to_lowercase();
        for bait in &fixture.must_not_extract {
            assert!(
                !text.contains(&bait.to_lowercase()),
                "{}: bait {bait:?} appears in its own source, so it can never be a \
                 hallucination",
                fixture.name
            );
        }
    }
}

/// Session ids are the harness's attribution key from a served record
/// back to the fixture that produced it (`provenance.session_id`), so a
/// collision would silently merge two fixtures' results.
#[test]
fn session_ids_are_unique_across_the_whole_corpus() {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for fixture in fixtures() {
        if let Some(previous) = seen.insert(fixture.input.session_id.clone(), fixture.name.clone())
        {
            panic!(
                "session id {:?} is used by both {previous} and {}",
                fixture.input.session_id, fixture.name
            );
        }
    }
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
            .body_markdown
            .contains("[REDACTED:github-pat]"),
        "placeholder must survive verbatim: {:?}",
        outcome.candidates[0].body_markdown
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

/// The live-LLM hook (ADR-0022 decision 8): the same corpus and the same
/// scoring against a real endpoint. Ignored by default — run it
/// deliberately:
///
/// ```sh
/// SYNVEDA_EXTRACTOR=claude ANTHROPIC_API_KEY=... \
///   cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture
/// # or, air-gapped:
/// SYNVEDA_EXTRACTOR=vllm SYNVEDA_VLLM_BASE_URL=http://... SYNVEDA_EXTRACTOR_MODEL=... \
///   cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture
/// ```
///
/// The live measurement over the *product path*, with its own committed
/// baseline, is EVAL-2's (ADR-0046 decision 12); this stays the
/// extractor-level hook it has been since MEM-3.
#[tokio::test]
#[ignore = "network + credentials; the deliberate live-LLM measurement"]
async fn live_precision() {
    let selected = std::env::var("SYNVEDA_EXTRACTOR").unwrap_or_default();
    let model = std::env::var("SYNVEDA_EXTRACTOR_MODEL").ok();
    let extractor = match selected.as_str() {
        "claude" => AnyExtractor::Claude(
            ClaudeExtractor::new(
                std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY"),
                model.unwrap_or_else(|| ClaudeExtractor::DEFAULT_MODEL.to_owned()),
                std::env::var("SYNVEDA_ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|_| ClaudeExtractor::DEFAULT_BASE_URL.to_owned()),
            )
            .expect("configure Claude client"),
        ),
        "vllm" => AnyExtractor::Vllm(
            VllmExtractor::new(
                model.expect("SYNVEDA_EXTRACTOR_MODEL"),
                std::env::var("SYNVEDA_VLLM_BASE_URL").expect("SYNVEDA_VLLM_BASE_URL"),
            )
            .expect("configure vLLM client"),
        ),
        other => panic!("set SYNVEDA_EXTRACTOR=claude|vllm (got {other:?})"),
    };
    let report = measure(&extractor, &fixtures()).await;
    report.print(extractor.method());
    // The bait outcome is the interesting half here: unlike the
    // deterministic path, a model *can* invent.
    if !report.bait_hits.is_empty() {
        println!("  fabrications: {:?}", report.bait_hits);
    }
    assert!(
        report.macro_precision() >= DETERMINISTIC_PRECISION_FLOOR,
        "macro precision {:.3} under the registered floor {DETERMINISTIC_PRECISION_FLOOR}",
        report.macro_precision()
    );
}
