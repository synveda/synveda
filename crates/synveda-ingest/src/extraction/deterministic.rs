//! The rule-based extractor: event-type routing plus keyword
//! heuristics, truncation-as-summary. Honest about what it is — no
//! network, no abstraction, fixed per-rule confidence — and exactly what
//! keeps dev, demos, and the AC tests self-contained (ADR-0022
//! decision 3). The LLM implementations are the product path.

use std::sync::LazyLock;

use regex::Regex;
use synveda_types::Result;
use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
use synveda_types::session::SessionEventType;

use super::{CandidateKnowledge, ExtractionInput, ExtractionOutcome, Extractor};

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
const RULESET_VERSION: &str = "builtin@3";

static PREFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(prefers?|always use|never use|i like|we like|favou?rite)\b")
        .expect("static preference pattern compiles")
});
static DECISION: LazyLock<Regex> = LazyLock::new(|| {
    // `chose`/`chosen` bare, not only `we chose`: the pronoun was never the
    // signal, and requiring it meant "Chose BLAKE3 over SHA-256" read as a
    // fact. Found when the observe cutover removed `ObserveKind::Decision`
    // and the keyword path had to carry cases the client used to classify.
    Regex::new(r"(?i)\b(decided|decision|chose|chosen|we picked|agreed to|going with|settled on)\b")
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
            let (class, confidence) = classify(input.event_type, &text);
            // Mentions come from the content that will actually be proposed,
            // not from the text before truncation: a future relation claiming
            // this revision names a thing must be true of its stored content.
            let content = truncate(&text);
            let entities = mentions(&content);
            vec![CandidateKnowledge {
                knowledge_type: class,
                origin: if input.event_type == SessionEventType::MemoryAsserted {
                    KnowledgeOrigin::Asserted
                } else {
                    KnowledgeOrigin::Observed
                },
                title: proposed_title(&content),
                summary: content.clone(),
                body_markdown: content,
                tags: Vec::new(),
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

/// The event type routes first (the client told us what this is); anything
/// that is somebody's words falls through to keyword heuristics, default
/// `fact`.
///
/// **Something that happened is an episode.** A tool call, a tool answer, a
/// file change and a shell command are all evidence of an act, and their class
/// is decided by their type at `KIND_CONFIDENCE` — nothing in the text can
/// make a `command.executed` into a preference.
///
/// **`memory.asserted` takes the keyword path**, and deliberately. It is the
/// one type that says nothing about *class*: "a model composed this and chose
/// to store it" (ADR-0057 decision 8) is a claim about who put the content on
/// the wire, not about what the content is — a model asserts preferences,
/// procedures and decisions as readily as bare facts. Routing it to a fixed
/// class at `KIND_CONFIDENCE` would assert a classification the type does not
/// carry, so the text is read the same way a message's is and the provenance
/// claim rides on the candidate's source event instead.
///
/// Types that answer `false` to [`SessionEventType::capture_eligible`] never
/// enter a frozen batch.
fn classify(event_type: SessionEventType, text: &str) -> (KnowledgeType, f64) {
    match event_type {
        SessionEventType::ToolInvoked
        | SessionEventType::ToolResult
        | SessionEventType::FileChanged
        | SessionEventType::CommandExecuted => (KnowledgeType::Episode, KIND_CONFIDENCE),
        _ => {
            if PREFERENCE.is_match(text) {
                (KnowledgeType::Preference, KEYWORD_CONFIDENCE)
            } else if DECISION.is_match(text) {
                (KnowledgeType::Decision, KEYWORD_CONFIDENCE)
            } else if PROCEDURE.is_match(text) {
                (KnowledgeType::Procedure, KEYWORD_CONFIDENCE)
            } else if ENTITY.is_match(text) {
                (KnowledgeType::Entity, KEYWORD_CONFIDENCE)
            } else {
                (KnowledgeType::Fact, KEYWORD_CONFIDENCE)
            }
        }
    }
}

/// A deterministic title from the first sentence/line, bounded below the
/// Knowledge title limit without splitting UTF-8.
fn proposed_title(content: &str) -> String {
    let sentence = content
        .split(['\n', '.', '!', '?'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(content)
        .trim();
    let mut title: String = sentence.chars().take(100).collect();
    if sentence.chars().count() > 100 {
        title.push('…');
    }
    title
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
///
/// Counted in **characters**, which is what the constant has always been
/// called and was not what this measured. It compared `text.len()` —
/// bytes — and then sliced at that byte index, so any transcript whose
/// three-hundredth byte landed inside a multi-byte character panicked the
/// extraction worker and stalled the pipeline behind it. A LongMemEval
/// haystack found it on the first run: `end byte index 300 is not a char
/// boundary; it is inside '💡'`. Real chat has emoji in it.
///
/// The byte reading also truncated non-ASCII content early and silently —
/// three hundred bytes of accented text is well under three hundred
/// characters — so a record in one language kept less than the same record
/// in another.
fn truncate(text: &str) -> String {
    let mut boundaries = text.char_indices().map(|(index, _)| index);
    let Some(limit) = boundaries.nth(MAX_CONTENT_CHARS) else {
        return text.to_owned();
    };
    // `limit` is a character boundary by construction, and so is the
    // result of `rfind(' ')` within it — a space is one byte and cannot
    // sit inside another character.
    let cut = text[..limit].rfind(' ').unwrap_or(limit);
    format!("{}…", &text[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panic a LongMemEval haystack found on the first real run: the
    /// three-hundredth *byte* landing inside a multi-byte character.
    /// `truncate` sliced there and took the extraction worker with it,
    /// stalling every event queued behind it.
    #[test]
    fn a_multi_byte_character_on_the_boundary_does_not_panic() {
        // 299 bytes of ASCII, then an emoji straddling byte 300.
        let text = format!("{}💡 and then some more text after it", "a".repeat(299));
        let out = truncate(&text);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= MAX_CONTENT_CHARS + 1);
    }

    /// Counted in characters, as the constant has always been named. Under
    /// the byte reading, three hundred characters of accented text was
    /// nearly six hundred bytes and got cut in half — a record kept less
    /// for being written in one language rather than another.
    #[test]
    fn the_limit_counts_characters_rather_than_bytes() {
        let short = "é".repeat(MAX_CONTENT_CHARS);
        assert_eq!(
            truncate(&short),
            short,
            "{} characters is under the limit however many bytes it is",
            MAX_CONTENT_CHARS
        );
        assert!(
            short.len() > MAX_CONTENT_CHARS,
            "and it is over it in bytes"
        );

        let long = "é".repeat(MAX_CONTENT_CHARS + 50);
        let out = truncate(&long);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), MAX_CONTENT_CHARS + 1);
    }

    #[test]
    fn a_word_boundary_is_still_preferred_and_nothing_short_is_touched() {
        assert_eq!(truncate("short enough"), "short enough");
        let text = format!("{} tail", "word ".repeat(80));
        let out = truncate(&text);
        assert!(out.ends_with('…') && !out.contains("wor…"), "{out}");
    }

    /// The types whose class the event itself decides, and the class each one
    /// means. `memory.asserted` is absent by design — see below.
    #[test]
    fn type_routing_is_fixed_for_the_types_that_carry_a_class() {
        for event_type in [
            SessionEventType::ToolInvoked,
            SessionEventType::ToolResult,
            SessionEventType::FileChanged,
            SessionEventType::CommandExecuted,
        ] {
            let (class, confidence) = classify(event_type, "anything at all");
            assert_eq!(
                class,
                KnowledgeType::Episode,
                "{} should route to episode whatever the text says",
                event_type.as_str()
            );
            assert!((confidence - KIND_CONFIDENCE).abs() < f64::EPSILON);
        }
    }

    /// The gate that keeps the extractor away from bookkeeping. A type that
    /// answers `false` never enters a batch, so a change that made one of
    /// these `true` would start spending an LLM call on "session started".
    #[test]
    fn only_durable_content_types_are_capture_eligible() {
        let carrying: Vec<&str> = SessionEventType::ALL
            .iter()
            .filter(|event_type| event_type.capture_eligible())
            .map(SessionEventType::as_str)
            .collect();
        assert_eq!(
            carrying,
            [
                "message.user",
                "message.assistant",
                "tool.invoked",
                "tool.result",
                "file.changed",
                "command.executed",
                "memory.asserted",
            ]
        );
    }

    /// ADR-0057 decision 8: `memory.asserted` is a provenance claim, not a
    /// class claim. It must read the text exactly as a plain message does —
    /// identical class *and* identical confidence — because a model asserts
    /// preferences and procedures as readily as bare facts, and pinning it to
    /// one class would assert a classification the type never carried.
    #[test]
    fn an_assertion_classifies_exactly_as_a_message_does() {
        let texts = [
            "I prefer tabs over spaces",
            "we decided to ship on Friday",
            "to deploy, run make release then tag it",
            "Acme Corp is the customer",
            "the staging cluster is in eu-west-1",
            "",
        ];
        for text in texts {
            assert_eq!(
                classify(SessionEventType::MemoryAsserted, text),
                classify(SessionEventType::MessageUser, text),
                "memory.asserted and message.user disagreed on {text:?}"
            );
        }
    }

    /// The keyword path is what both of them share, so it is worth pinning
    /// that it actually discriminates — otherwise the test above passes on
    /// a routing that collapsed everything to `fact`.
    #[test]
    fn the_shared_keyword_path_still_discriminates() {
        let classes = [
            "I prefer tabs over spaces",
            "we decided to ship on Friday",
            "how to deploy: then run make release",
        ]
        .map(|text| classify(SessionEventType::MemoryAsserted, text).0);
        assert_eq!(
            classes,
            [
                KnowledgeType::Preference,
                KnowledgeType::Decision,
                KnowledgeType::Procedure
            ]
        );
        assert_eq!(
            classify(SessionEventType::MemoryAsserted, "the sky is a colour").0,
            KnowledgeType::Fact,
            "the default"
        );
    }
}
