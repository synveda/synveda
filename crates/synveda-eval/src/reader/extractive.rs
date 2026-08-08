//! The deterministic default: pick the block's best-matching entry line
//! and return it (ADR-0061 decision 6).
//!
//! It is not a question-answerer and does not pretend to be. What it is:
//! the network-free path that lets dev, tests and demos exercise the seam
//! with no key and no spend (seed §2.1), and a floor — an answer built by
//! *selection* rather than by generation, so any margin a model reader
//! shows over it is margin from reading rather than from retrieval.
//!
//! It reads the block's structural vocabulary rather than guessing at it:
//! `- [<class>] <content>` entry lines are candidates, and the `##
//! <path> (<kind>)` headings, the index legend and the
//! `<!-- synveda:watermark … -->` comment are not. Scoring those would let
//! a question about "the org" match a scope heading and return the
//! renderer's own furniture as an answer.
//!
//! Its abstention is the honest part. No entry line sharing a content
//! word with the question means the block does not support an answer, and
//! saying so is the correct outcome — EVAL-1 decision 4's axis, reachable
//! without a model.

use std::collections::BTreeSet;

use super::{Answer, Reader, ReaderInput};

/// The method name in the report and the tally.
const METHOD: &str = "extractive";

/// The ruleset version, the `builtin@1` convention. Bump it when the
/// selection changes: a score is a joint property of the reader too.
const RULESET: &str = "selection@1";

/// Dropped before comparison, as in the lexical judge — SQuAD's
/// normalisation drops exactly these and nothing else.
const ARTICLES: [&str; 3] = ["a", "an", "the"];

/// What the reader says when the block does not support an answer. Fixed
/// wording so a judge grading against an abstention reference sees the
/// same sentence every time.
const DECLINE: &str = "The context block holds no answer to this question.";

/// The block's entry-line prefix (`crates/synveda-retrieval/src/compose
/// .rs`): one entry, one line, class in brackets.
const ENTRY_PREFIX: &str = "- [";

/// The network-free reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractiveReader;

impl ExtractiveReader {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Reader for ExtractiveReader {
    fn method(&self) -> &'static str {
        METHOD
    }

    async fn read(&self, input: &ReaderInput<'_>) -> Result<Answer, String> {
        let asked = normalise(input.question);
        if asked.is_empty() {
            // Every entry line would score zero, so the reader would
            // abstain — and the abstention would say "the block holds no
            // answer" when what happened is that nothing was asked.
            return Err(format!(
                "{METHOD}: the question holds no content word, so every entry line scores the \
                 same and an abstention here would blame the block for an empty question"
            ));
        }

        // First best wins, so two equally-scoring lines resolve in block
        // order rather than by hash iteration — two runs of one corpus
        // must produce one answer.
        let mut best: Option<(usize, &str)> = None;
        for line in input.block.lines().filter_map(entry_content) {
            let score = normalise(line).intersection(&asked).count();
            if score > 0 && best.is_none_or(|(seen, _)| score > seen) {
                best = Some((score, line));
            }
        }

        let (text, abstained) = match best {
            Some((_, line)) => (line.to_owned(), false),
            None => (DECLINE.to_owned(), true),
        };
        Ok(Answer {
            text,
            abstained,
            method: METHOD.to_owned(),
            model_version: RULESET.to_owned(),
            // A ruleset has no effort and no tokens to record.
            effort: None,
            usage: None,
        })
    }
}

/// The content of one `- [class] content` entry line, or `None` for the
/// block's structural furniture — headings, legend, watermark, blanks.
fn entry_content(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix(ENTRY_PREFIX)?;
    let (_class, content) = rest.split_once("] ")?;
    let content = content.trim();
    (!content.is_empty()).then_some(content)
}

/// Lowercased alphanumeric tokens, articles dropped.
fn normalise(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .filter(|token| !ARTICLES.contains(&token.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "\
## acme (org)
- [decision] Payments retries are capped at three attempts.
- [fact] The rota is public.

## alice (user)
- [preference] Alice runs cargo nextest before pushing.
- [episode] The kitchen renovation ran for three weeks in March.

Summarised entries end with a recall handle; `synveda recall <id>` fetches the full text.

<!-- synveda:watermark v1 blake3=deadbeef records=r1,r2,r3,r4 -->";

    async fn read(question: &str) -> Answer {
        ExtractiveReader::new()
            .read(&ReaderInput {
                question,
                block: BLOCK,
            })
            .await
            .expect("read")
    }

    #[tokio::test]
    async fn the_best_matching_entry_line_is_the_answer() {
        let answer = read("how long did the kitchen renovation take").await;
        assert!(!answer.abstained);
        assert_eq!(
            answer.text,
            "The kitchen renovation ran for three weeks in March."
        );
        assert_eq!(answer.method, "extractive");
        assert_eq!(answer.model_version, "selection@1");
        assert_eq!(answer.effort, None);
    }

    /// The block's own vocabulary is furniture, not content. A question
    /// about "the org" must not be answered with the renderer's scope
    /// heading, and a question about "recall" must not return the legend.
    #[tokio::test]
    async fn the_blocks_structure_is_never_returned_as_an_answer() {
        let heading = read("what is the acme org").await;
        assert!(
            heading.abstained
                || heading.text.starts_with("Payments")
                || heading.text.starts_with("The rota"),
            "a scope heading leaked into the answer: {heading:?}"
        );
        assert!(!heading.text.contains("##"), "{heading:?}");

        let legend = read("how do I recall the full text with a handle").await;
        assert!(
            !legend.text.contains("synveda recall"),
            "the legend leaked into the answer: {legend:?}"
        );

        let watermark = read("what is the blake3 watermark").await;
        assert!(
            !watermark.text.contains("blake3"),
            "the watermark leaked into the answer: {watermark:?}"
        );
    }

    /// EVAL-1 decision 4's axis, reachable with no model: a block that
    /// does not support an answer produces a decline, not an invention.
    #[tokio::test]
    async fn a_block_that_supports_nothing_abstains_rather_than_inventing() {
        let answer = read("which vendor supplies the espresso beans").await;
        assert!(answer.abstained, "{answer:?}");
        assert_eq!(answer.text, DECLINE);
    }

    #[tokio::test]
    async fn an_empty_question_is_an_error_rather_than_an_abstention() {
        // Otherwise the abstention axis would count a broken probe as a
        // block that held nothing.
        let err = ExtractiveReader::new()
            .read(&ReaderInput {
                question: "  ...  ",
                block: BLOCK,
            })
            .await
            .expect_err("an empty question must not read");
        assert!(err.contains("empty question"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn selection_is_deterministic_across_ties() {
        // Two lines score 1 on `three`; block order decides, both times.
        let first = read("three").await;
        let second = read("three").await;
        assert_eq!(first.text, second.text);
        assert_eq!(first.text, "Payments retries are capped at three attempts.");
    }

    #[tokio::test]
    async fn an_empty_block_abstains() {
        let answer = ExtractiveReader::new()
            .read(&ReaderInput {
                question: "anything at all",
                block: "",
            })
            .await
            .expect("read");
        assert!(answer.abstained);
    }
}
