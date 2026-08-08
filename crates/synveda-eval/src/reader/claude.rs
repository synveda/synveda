//! The Claude API reader (ADR-0061 decision 6): the shared Messages API
//! transport with this seam's prompt, schema and forced tool.

use crate::anthropic;

use super::{Answer, Reader, ReaderInput, prompt};

/// The method name in the report, the tally and errors.
const SERVICE: &str = "claude-api";

/// Stated rather than inherited, and stated here rather than shared with
/// the judge: the two seams are keyed separately because decision 6
/// records two model versions, and there is no reason their effort should
/// move together.
///
/// `high` because reading an answer out of a block is the half of the
/// score this product is not responsible for, and under-spending there
/// would show up as a Synveda regression. The hazard runs the other way
/// too — a reader that reasons harder can paper over a worse block — so
/// this level is part of the provenance on every [`Answer`], and a sweep
/// is the cost lever once there is a corpus to sweep against.
const EFFORT: &str = "high";

/// The ceiling bounds thinking plus answer, and a 115k-token LongMemEval
/// haystack is a lot to reason over. Far above what a sentence needs, and
/// an unused ceiling costs nothing.
const MAX_TOKENS: u32 = 8192;

/// The forced tool's name.
const TOOL_NAME: &str = "emit_answer";

/// The Anthropic Messages API reader.
#[derive(Debug, Clone)]
pub struct ClaudeReader {
    client: anthropic::Client,
}

impl ClaudeReader {
    /// The default model when `SYNVEDA_READER_MODEL` is unset.
    pub const DEFAULT_MODEL: &'static str = anthropic::DEFAULT_MODEL;

    /// The default endpoint when `SYNVEDA_ANTHROPIC_BASE_URL` is unset.
    pub const DEFAULT_BASE_URL: &'static str = anthropic::DEFAULT_BASE_URL;

    #[must_use]
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            client: anthropic::Client::new(api_key, model, base_url),
        }
    }

    /// The configured model id. Read by [`super::independence_note`],
    /// which has to be able to say which model is on both sides.
    #[must_use]
    pub fn model(&self) -> &str {
        self.client.model()
    }
}

impl Reader for ClaudeReader {
    fn method(&self) -> &'static str {
        SERVICE
    }

    async fn read(&self, input: &ReaderInput<'_>) -> Result<Answer, String> {
        let result = self
            .client
            .call(&anthropic::ToolCall {
                service: SERVICE,
                tool_name: TOOL_NAME,
                tool_description: "Emit the answer read out of the context block.",
                schema: prompt::answer_schema(),
                system: prompt::SYSTEM_PROMPT,
                user: prompt::user_message(input),
                max_tokens: MAX_TOKENS,
                effort: EFFORT,
            })
            .await?;
        let (text, abstained) = prompt::parse_answer(SERVICE, result.input)?;
        Ok(Answer {
            text,
            abstained,
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

    const BLOCK: &str = "## alice (user)\n- [episode] The renovation ran three weeks.\n";

    async fn reader(response: Value) -> (ClaudeReader, mock::Captured) {
        let (base_url, captured) = mock::spawn(response).await;
        (
            ClaudeReader::new(
                mock::API_KEY.to_owned(),
                ClaudeReader::DEFAULT_MODEL.to_owned(),
                base_url,
            ),
            captured,
        )
    }

    /// The transport's contract is tested in `anthropic`; this asserts
    /// the half only the reader knows — its tool, the block going in as
    /// data, and the provenance stamped on the answer.
    #[tokio::test]
    async fn the_reader_supplies_its_own_tool_and_stamps_its_own_provenance() {
        let (reader, captured) = reader(json!({
            "model": "claude-opus-5-served-build",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "name": "emit_answer",
                "input": {"answer": "  It ran three weeks.  ", "abstained": false}
            }]
        }))
        .await;

        let answer = reader
            .read(&ReaderInput {
                question: "how long did the renovation take",
                block: BLOCK,
            })
            .await
            .expect("read");
        assert_eq!(answer.text, "It ran three weeks.");
        assert!(!answer.abstained);
        assert_eq!(answer.method, "claude-api");
        assert_eq!(answer.model_version, "claude-opus-5-served-build");
        assert_eq!(answer.effort.as_deref(), Some(EFFORT));

        let body = mock::body(&captured);
        assert_eq!(body["tool_choice"]["name"], "emit_answer");
        let sent: Value = serde_json::from_str(
            body["messages"][0]["content"]
                .as_str()
                .expect("text content"),
        )
        .expect("the user message is JSON");
        assert_eq!(
            sent["context_block"], BLOCK,
            "the block goes to the model verbatim, furniture and all"
        );
    }

    #[tokio::test]
    async fn an_abstention_survives_the_wire_as_an_abstention() {
        let (reader, _) = reader(json!({
            "model": "m",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "name": "emit_answer",
                "input": {"answer": "The block does not say.", "abstained": true}
            }]
        }))
        .await;
        let answer = reader
            .read(&ReaderInput {
                question: "who supplies the beans",
                block: BLOCK,
            })
            .await
            .expect("read");
        assert!(answer.abstained);
        assert_eq!(answer.text, "The block does not say.");
    }
}
