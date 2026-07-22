//! The rule-based extractor: `ObserveKind` routing plus keyword
//! heuristics, truncation-as-summary. Honest about what it is — no
//! network, no abstraction, fixed per-rule confidence — and exactly what
//! keeps dev, demos, and the AC tests self-contained (ADR-0022
//! decision 3). The LLM implementations are the product path.

use std::sync::LazyLock;

use regex::Regex;
use synveda_types::{ObserveKind, RecordClass, Result};

use super::{CandidateRecord, ExtractionInput, ExtractionOutcome, Extractor};

/// Content longer than this is truncated on a word boundary — a
/// truncation is an honest summary only while it stays short.
const MAX_CONTENT_CHARS: usize = 300;

/// Confidence when the event's kind alone decides the class.
const KIND_CONFIDENCE: f64 = 0.9;
/// Confidence when a keyword heuristic decides the class.
const KEYWORD_CONFIDENCE: f64 = 0.6;

/// The ruleset version recorded as `model_version` in provenance. Bump
/// whenever a rule changes: provenance must name what actually ran.
const RULESET_VERSION: &str = "builtin@1";

static PREFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(prefers?|always use|never use|i like|we like|favou?rite)\b")
        .expect("static preference pattern compiles")
});
static DECISION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(decided|decision|we chose|agreed to|going with|settled on)\b")
        .expect("static decision pattern compiles")
});
static PROCEDURE: LazyLock<Regex> = LazyLock::new(|| {
    // `first,` carries its own comma boundary: `\b` cannot sit between a
    // comma and a space, so it stays outside the bounded alternation.
    Regex::new(r"(?i)\b(step \d|how to|procedure|then run|run the following)\b|(?i)\bfirst,")
        .expect("static procedure pattern compiles")
});
static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Z][A-Za-z0-9_-]* is (a|an|the|our) ").expect("static entity pattern compiles")
});

/// The rule-based extractor. A unit struct today; construction goes
/// through [`DeterministicExtractor::new`] so config can arrive later
/// without reshaping call sites.
#[derive(Debug, Clone, Default)]
pub struct DeterministicExtractor;

impl DeterministicExtractor {
    /// Builds the extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Extractor for DeterministicExtractor {
    fn method(&self) -> &'static str {
        "deterministic"
    }

    async fn extract(&self, input: &ExtractionInput) -> Result<ExtractionOutcome> {
        let text = gather_text(&input.payload);
        let candidates = if text.is_empty() {
            Vec::new()
        } else {
            let (class, confidence) = classify(input.kind, &text);
            vec![CandidateRecord {
                class,
                content: truncate(&text),
                confidence,
                sensitivity: None,
                entities: Vec::new(),
            }]
        };
        Ok(ExtractionOutcome {
            candidates,
            method: self.method().to_owned(),
            model_version: RULESET_VERSION.to_owned(),
        })
    }
}

/// Kind routes first (the client told us what this is); transcript deltas
/// fall through to keyword heuristics, default `fact`.
fn classify(kind: ObserveKind, text: &str) -> (RecordClass, f64) {
    match kind {
        ObserveKind::Decision => (RecordClass::Decision, KIND_CONFIDENCE),
        ObserveKind::ToolResult => (RecordClass::Episode, KIND_CONFIDENCE),
        ObserveKind::TranscriptDelta => {
            if PREFERENCE.is_match(text) {
                (RecordClass::Preference, KEYWORD_CONFIDENCE)
            } else if DECISION.is_match(text) {
                (RecordClass::Decision, KEYWORD_CONFIDENCE)
            } else if PROCEDURE.is_match(text) {
                (RecordClass::Procedure, KEYWORD_CONFIDENCE)
            } else if ENTITY.is_match(text) {
                (RecordClass::Entity, KEYWORD_CONFIDENCE)
            } else {
                (RecordClass::Fact, KEYWORD_CONFIDENCE)
            }
        }
    }
}

/// Concatenates every string value in the payload, in document order,
/// whitespace-collapsed — the same walk-the-strings view of a payload
/// the redaction scanner takes.
fn gather_text(payload: &serde_json::Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    collect_strings(payload, &mut parts);
    let joined = parts.join(" ");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_strings<'a>(value: &'a serde_json::Value, into: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(text) => into.push(text),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, into);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_strings(item, into);
            }
        }
        _ => {}
    }
}

/// Truncates on a word boundary with an ellipsis marker; never splits a
/// `[REDACTED:*]` placeholder because it never splits a word.
fn truncate(text: &str) -> String {
    if text.len() <= MAX_CONTENT_CHARS {
        return text.to_owned();
    }
    let cut = text[..MAX_CONTENT_CHARS]
        .rfind(' ')
        .unwrap_or(MAX_CONTENT_CHARS);
    format!("{}…", &text[..cut])
}
