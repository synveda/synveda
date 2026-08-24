//! The one prompt and the one parser both LLM extractors share (ADR-0022
//! decision 3). Keeping them in a single module is what makes "the Claude
//! impl and the vLLM impl extract the same things" a structural property
//! rather than a hope.

use serde::Deserialize;
use synveda_types::knowledge::{KnowledgeOrigin, KnowledgeType};
use synveda_types::session::SessionEventType;
use synveda_types::{Error, Result, Sensitivity};

use super::{CandidateKnowledge, ExtractionInput};

/// The system prompt: the six class definitions, summarise-at-write, the
/// redaction-opacity rule, and the output contract.
pub(crate) const SYSTEM_PROMPT: &str = "You propose reviewable Knowledge from one observed \
event of an AI-agent work session.\n\
\n\
Classify each extractable proposal into exactly one knowledge_type:\n\
- fact: a stable statement about the world, the project, or a person.\n\
- decision: a choice that was made, with its subject.\n\
- preference: how a person or team likes things done.\n\
- procedure: how to accomplish something, as reusable steps.\n\
- entity: a person, system, or thing worth remembering in itself.\n\
- episode: something that happened — an action and its outcome.\n\
- convention: a durable project or team practice.\n\
- warning: a hazard, caveat, or expiring caution.\n\
- reference: a durable pointer to external material.\n\
\n\
Rules:\n\
- Provide a short title, a self-contained Markdown body, a short summary and \
zero or more lower-case tags. A future reviewer must understand each proposal \
without this event.\n\
- Extract zero candidates when the event holds nothing worth remembering. \
Never invent information that is not in the event.\n\
- Tokens of the form [REDACTED:rule-id] are opaque redaction placeholders. \
Preserve them verbatim when quoting; never guess what they replaced.\n\
- confidence is your own estimate in [0,1] that the candidate is a \
faithful, durable memory of what the event says.\n\
- sensitivity is optional: internal or confidential. Omit it when unsure. \
There is no third option here: the tier above these two means a compliance \
approver signed for it, which is a review rather than a judgement you can \
make (AUTHZ-5).\n\
- entities lists proper names mentioned by a candidate; may be empty.";

/// The candidates JSON schema both impls request. Kept as a function
/// returning a fresh value: `serde_json::json!` cannot be a `const`.
pub(crate) fn candidates_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "candidates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "knowledge_type": {
                            "type": "string",
                            "enum": ["fact", "decision", "preference",
                                     "procedure", "entity", "episode",
                                     "convention", "warning", "reference"]
                        },
                        "title": { "type": "string" },
                        "body_markdown": { "type": "string" },
                        "summary": { "type": "string" },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "confidence": { "type": "number" },
                        "sensitivity": {
                            "type": "string",
                            // The pipeline clamps at `confidential`
                            // anyway (ADR-0038 decision 8); asking for a
                            // tier we would refuse is an invitation to
                            // argue with the clamp later.
                            "enum": ["internal", "confidential"]
                        },
                        "entities": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["knowledge_type", "title", "body_markdown",
                                 "summary", "confidence"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["candidates"],
        "additionalProperties": false
    })
}

/// The user message: the event's type, time, and redacted payload, as
/// compact JSON the model reads as data.
pub(crate) fn user_message(input: &ExtractionInput) -> String {
    serde_json::json!({
        "event_type": input.event_type.as_str(),
        "occurred_at": input.occurred_at.to_rfc3339(),
        "payload": input.payload,
    })
    .to_string()
}

/// The wire shape of one candidate as the schema elicits it. Separate
/// from [`CandidateKnowledge`] so lenient parsing (clamping, defaults)
/// happens in exactly one place.
#[derive(Deserialize)]
struct WireCandidate {
    knowledge_type: KnowledgeType,
    title: String,
    body_markdown: String,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    confidence: f64,
    #[serde(default)]
    sensitivity: Option<Sensitivity>,
    #[serde(default)]
    entities: Vec<String>,
}

#[derive(Deserialize)]
struct WireCandidates {
    candidates: Vec<WireCandidate>,
}

/// Parses a candidates JSON value (the forced tool call's input, or a
/// chat completion's JSON body) into candidates. Confidence is clamped
/// into `[0,1]`; blank content is dropped rather than persisted.
pub(crate) fn parse_candidates(
    service: &str,
    value: serde_json::Value,
    event_type: SessionEventType,
) -> Result<Vec<CandidateKnowledge>> {
    let wire: WireCandidates = serde_json::from_value(value).map_err(|err| Error::Dependency {
        service: service.to_owned(),
        message: format!("extractor returned candidates outside the contract: {err}"),
    })?;
    Ok(wire
        .candidates
        .into_iter()
        .filter(|candidate| {
            !candidate.title.trim().is_empty()
                && !candidate.body_markdown.trim().is_empty()
                && !candidate.summary.trim().is_empty()
        })
        .map(|candidate| CandidateKnowledge {
            knowledge_type: candidate.knowledge_type,
            origin: if event_type == SessionEventType::MemoryAsserted {
                KnowledgeOrigin::Asserted
            } else {
                KnowledgeOrigin::Observed
            },
            title: candidate.title.trim().to_owned(),
            body_markdown: candidate.body_markdown.trim().to_owned(),
            summary: candidate.summary.trim().to_owned(),
            tags: candidate.tags,
            confidence: candidate.confidence.clamp(0.0, 1.0),
            sensitivity: candidate.sensitivity,
            entities: candidate.entities,
        })
        .collect())
}

/// Strips an optional Markdown code fence: chat-completion models wrap
/// JSON in ```json fences often enough that refusing to would turn a
/// formatting tic into a retry loop. Anything beyond that stays strict.
pub(crate) fn strip_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}
