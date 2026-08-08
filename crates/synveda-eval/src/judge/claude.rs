//! The Claude API judge (ADR-0061 decision 3): the shared Messages API
//! transport with this seam's prompt, schema and forced tool.
//!
//! `crates/synveda-ingest/src/extraction/claude.rs` is the shape this
//! copies, down to the `model_version`-from-the-response honesty decision
//! 6 requires — the published score is keyed to the model the API
//! *served*, and an alias recorded as if it were a version is a benchmark
//! figure nobody, us included, can reproduce.

use crate::anthropic;

use super::{Judge, JudgeInput, Verdict, prompt};

/// The method name in the report, the tally and errors.
const SERVICE: &str = "claude-api";

/// Grading is intelligence-sensitive and its agreement rate is published
/// (decision 4), so the effort level is stated rather than inherited.
/// Lower levels are the cost lever a sweep would test; that sweep belongs
/// with the measurement, not with the seam.
const EFFORT: &str = "high";

/// Far above what a two-field verdict needs, because the ceiling bounds
/// thinking too.
const MAX_TOKENS: u32 = 8192;

/// The forced tool's name.
const TOOL_NAME: &str = "emit_verdict";

/// The Anthropic Messages API judge.
#[derive(Debug, Clone)]
pub struct ClaudeJudge {
    client: anthropic::Client,
}

impl ClaudeJudge {
    /// The default model when `SYNVEDA_JUDGE_MODEL` is unset.
    pub const DEFAULT_MODEL: &'static str = anthropic::DEFAULT_MODEL;

    /// The default endpoint when `SYNVEDA_ANTHROPIC_BASE_URL` is unset.
    pub const DEFAULT_BASE_URL: &'static str = anthropic::DEFAULT_BASE_URL;

    #[must_use]
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            client: anthropic::Client::new(api_key, model, base_url),
        }
    }
}

impl Judge for ClaudeJudge {
    fn method(&self) -> &'static str {
        SERVICE
    }

    async fn grade(&self, input: &JudgeInput<'_>) -> Result<Verdict, String> {
        let result = self
            .client
            .call(&anthropic::ToolCall {
                service: SERVICE,
                tool_name: TOOL_NAME,
                tool_description: "Emit the verdict for this candidate answer.",
                schema: prompt::verdict_schema(),
                system: prompt::SYSTEM_PROMPT,
                user: prompt::user_message(input),
                max_tokens: MAX_TOKENS,
                effort: EFFORT,
            })
            .await?;
        let (correct, rationale) = prompt::parse_verdict(SERVICE, result.input)?;
        Ok(Verdict {
            correct,
            rationale,
            method: SERVICE.to_owned(),
            model_version: result.model_version,
            effort: Some(EFFORT.to_owned()),
            usage: Some(result.usage),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::anthropic::mock;

    use super::*;

    fn sample<'a>() -> JudgeInput<'a> {
        JudgeInput {
            question: "when did the lease end",
            reference: "March",
            candidate: "It took about 21 days.",
        }
    }

    /// The transport's own contract is tested in `anthropic`; what this
    /// asserts is the half only the judge knows — its prompt, its schema,
    /// its tool, and the provenance it stamps on a verdict.
    #[tokio::test]
    async fn the_judge_supplies_its_own_tool_and_stamps_its_own_provenance() {
        let (base_url, captured) = mock::spawn(json!({
            "model": "claude-opus-5-served-build",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "name": "emit_verdict",
                "input": {"correct": true, "rationale": "  21 days is three weeks.  "}
            }]
        }))
        .await;
        let judge = ClaudeJudge::new(
            mock::API_KEY.to_owned(),
            ClaudeJudge::DEFAULT_MODEL.to_owned(),
            base_url,
        );

        let verdict = judge.grade(&sample()).await.expect("graded");
        assert!(verdict.correct);
        assert_eq!(verdict.rationale, "21 days is three weeks.");
        assert_eq!(verdict.method, "claude-api");
        assert_eq!(
            verdict.model_version, "claude-opus-5-served-build",
            "the verdict carries the model the API served, not the alias requested"
        );
        assert_eq!(verdict.effort.as_deref(), Some(EFFORT));

        let body = mock::body(&captured);
        assert_eq!(body["tool_choice"]["name"], "emit_verdict");
        assert_eq!(
            body["tools"][0]["input_schema"]["additionalProperties"],
            false
        );
        // The three texts go as data in one user turn, never as separate
        // turns a corpus string could impersonate.
        let sent: Value = serde_json::from_str(
            body["messages"][0]["content"]
                .as_str()
                .expect("text content"),
        )
        .expect("the user message is JSON");
        assert_eq!(sent["reference_answer"], "March");
        assert_eq!(sent["candidate_answer"], "It took about 21 days.");
    }
}
