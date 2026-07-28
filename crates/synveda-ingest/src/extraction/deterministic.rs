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
/// `@2` added entity mentions (GRPH-2, ADR-0044 decision 2).
const RULESET_VERSION: &str = "builtin@2";

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
/// The opaque spans MEM-2 leaves behind (ADR-0021). Removed before
/// mention detection so `REDACTED` never reads as a proper noun — the
/// linker refuses a mention carrying the marker (ADR-0044 decision 9),
/// and this makes sure the marker is still attached when it looks.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[REDACTED:[^\]]*\]").expect("static placeholder pattern compiles")
});

/// Words that open a sentence rather than name a thing. A capitalised run
/// is stripped of these from the front, so "If Postgres fails" yields
/// `Postgres` and "We decided" yields nothing.
///
/// A stoplist rather than a position rule (GRPH-2, ADR-0044 decision 2):
/// refusing single capitalised tokens at sentence starts would
/// systematically miss every entity that opens a sentence, while a
/// stoplist misses only the words that are not on it — and a list is data
/// the next contributor can extend, where a position rule is behaviour
/// they would have to argue with.
const SENTENCE_OPENERS: &[&str] = &[
    "a",
    "after",
    "again",
    "all",
    "also",
    "an",
    "and",
    "another",
    "any",
    "as",
    "at",
    "be",
    "because",
    "before",
    "both",
    "but",
    "by",
    "can",
    "could",
    "did",
    "do",
    "does",
    "during",
    "each",
    "either",
    "every",
    "finally",
    "first",
    "for",
    "from",
    "he",
    "her",
    "here",
    "his",
    "how",
    "i",
    "if",
    "in",
    "is",
    "it",
    "its",
    "let",
    "may",
    "me",
    "might",
    "must",
    "my",
    "next",
    "no",
    "not",
    "note",
    "now",
    "of",
    "on",
    "once",
    "one",
    "or",
    "otherwise",
    "our",
    "per",
    "please",
    "she",
    "should",
    "since",
    "so",
    "some",
    "than",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "to",
    "today",
    "tomorrow",
    "us",
    "use",
    "using",
    "via",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "why",
    "will",
    "with",
    "would",
    "yes",
    "yesterday",
    "you",
    "your",
];

/// Punctuation that ends a capitalised run: the next capital starts a new
/// sentence, not a longer name. Without this, "We use Postgres. The team
/// decided" would read as one mention called "Postgres. The".
const RUN_TERMINATORS: [char; 6] = ['.', '!', '?', ',', ';', ':'];

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
            // Mentions come from the content that will actually be
            // persisted, not from the text before truncation: an edge
            // claiming this record names a thing must be true of the
            // record as stored (GRPH-2, ADR-0044 decision 2).
            let content = truncate(&text);
            let entities = mentions(&content);
            vec![CandidateRecord {
                class,
                content,
                confidence,
                sensitivity: None,
                entities,
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

/// The proper names this content mentions (GRPH-2, ADR-0044 decision 2):
/// runs of capitalised tokens, stripped of the sentence-opening words that
/// are capitalised by grammar rather than by name.
///
/// Honest about what it is, exactly as the classifier above is: a
/// capitalisation heuristic, no network and no model. It misses lowercase
/// names and will occasionally intern an opener that
/// [`SENTENCE_OPENERS`] does not carry — which is why GRPH-2 measures the
/// orphan rate rather than claiming a recall number, and why the LLM
/// extractors, which fill the same field from the shared prompt, are the
/// product path.
fn mentions(content: &str) -> Vec<String> {
    let text = PLACEHOLDER.replace_all(content, " ");
    let mut found: Vec<String> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for token in text.split_whitespace() {
        if token.chars().next().is_some_and(char::is_uppercase) {
            run.push(token);
            // A terminator closes the run *including* this token: the next
            // capital opens a sentence rather than continuing a name.
            if token.ends_with(RUN_TERMINATORS) {
                push_run(&mut run, &mut found);
            }
        } else {
            push_run(&mut run, &mut found);
        }
    }
    push_run(&mut run, &mut found);
    found
}

/// Drains one capitalised run into `found`, dropping its leading sentence
/// openers and any duplicate of a mention already seen.
fn push_run(run: &mut Vec<&str>, found: &mut Vec<String>) {
    let mut tokens = std::mem::take(run);
    while tokens
        .first()
        .is_some_and(|token| SENTENCE_OPENERS.contains(&bare(token).as_str()))
    {
        tokens.remove(0);
    }
    if tokens.is_empty() {
        return;
    }
    let mention = tokens.join(" ");
    if !found.contains(&mention) {
        found.push(mention);
    }
}

/// A token reduced to what a stoplist can match: lowercased, with
/// surrounding punctuation removed.
fn bare(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_lowercase()
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
