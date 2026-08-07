//! The one prompt, the one schema and the one parser every model-backed
//! judge shares — the `extraction::prompt` arrangement, for the same
//! reason it exists there (ADR-0022 decision 3): keeping them in a single
//! module is what makes "two model judges grade the same way" a
//! structural property rather than a hope. There is one implementation
//! today; a second (a vLLM judge for the air-gapped path) would arrive
//! here rather than beside it.
//!
//! The prompt says what decides a verdict and why, and says it once. It
//! does not shout: the grading rule is the whole instruction, and an
//! emphatic prompt buys a judge that agrees with its own emphasis rather
//! than with the reference.

use serde::Deserialize;

/// What the judge is asked to do, and the four ways a candidate is wrong.
pub(crate) const SYSTEM_PROMPT: &str = "You grade one candidate answer against a reference \
answer for one question.\n\
\n\
Decide whether the candidate conveys the same answer as the reference. Judge the substance \
rather than the wording: a paraphrase, a different unit, a different date format, or extra \
correct detail around the answer is still correct.\n\
\n\
The candidate is incorrect when it states something the reference contradicts, when it \
answers a different question than the one asked, when it declines or says it does not know, \
or when the reference's answer cannot be read out of it.\n\
\n\
When the reference says the record holds no answer, only a candidate that declines is \
correct; one that supplies an answer anyway is incorrect.\n\
\n\
rationale is one sentence naming what decided it. A person comparing two judges reads it, \
so say what differed between the two answers rather than restating the verdict.";

/// The verdict schema the forced tool call is validated against. A
/// function rather than a `const`: `serde_json::json!` cannot be one.
pub(crate) fn verdict_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "correct": { "type": "boolean" },
            "rationale": { "type": "string" }
        },
        "required": ["correct", "rationale"],
        "additionalProperties": false
    })
}

/// The user message: the three texts as compact JSON the model reads as
/// data rather than as instructions. A corpus answer that happens to
/// contain "ignore the above" is a string here, not a turn.
pub(crate) fn user_message(input: &super::JudgeInput<'_>) -> String {
    serde_json::json!({
        "question": input.question,
        "reference_answer": input.reference,
        "candidate_answer": input.candidate,
    })
    .to_string()
}

/// The wire shape the schema elicits. Separate from [`super::Verdict`] so
/// the lenient half — trimming, the blank-rationale default — happens in
/// exactly one place.
#[derive(Deserialize)]
struct WireVerdict {
    correct: bool,
    rationale: String,
}

/// Parses the forced tool call's input into a verdict's two decided
/// fields. The caller supplies method and model version, because only it
/// knows what the API served.
pub(crate) fn parse_verdict(
    service: &str,
    value: serde_json::Value,
) -> Result<(bool, String), String> {
    let wire: WireVerdict = serde_json::from_value(value).map_err(|err| {
        format!("{service}: judge returned a verdict outside the contract: {err}")
    })?;
    let rationale = wire.rationale.trim();
    let rationale = if rationale.is_empty() {
        // Reported, never silently blank: an empty rationale in the
        // disagreement list is a row nobody can act on, and reversal
        // trigger (a) turns that list into the next feature's input.
        "the judge gave no rationale".to_owned()
    } else {
        rationale.to_owned()
    };
    Ok((wire.correct, rationale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_round_trips_and_is_trimmed() {
        let (correct, rationale) = parse_verdict(
            "claude-api",
            serde_json::json!({"correct": true, "rationale": "  both name March.  "}),
        )
        .expect("parses");
        assert!(correct);
        assert_eq!(rationale, "both name March.");
    }

    #[test]
    fn a_blank_rationale_says_so_rather_than_reading_as_a_reason() {
        let (_, rationale) = parse_verdict(
            "claude-api",
            serde_json::json!({"correct": false, "rationale": "   "}),
        )
        .expect("parses");
        assert_eq!(rationale, "the judge gave no rationale");
    }

    #[test]
    fn a_verdict_outside_the_contract_is_refused_rather_than_guessed() {
        let err = parse_verdict("claude-api", serde_json::json!({"verdict": "yes"}))
            .expect_err("off-contract must not parse");
        assert!(err.contains("outside the contract"), "{err}");
    }

    /// The schema is what `strict: true` validates against; an object that
    /// admitted extra properties would let a wordier model smuggle fields
    /// past the contract this parser assumes.
    #[test]
    fn the_schema_is_closed() {
        let schema = verdict_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"][0], "correct");
        assert_eq!(schema["required"][1], "rationale");
    }
}
