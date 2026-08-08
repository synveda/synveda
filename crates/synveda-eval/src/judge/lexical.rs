//! The deterministic default: the lexical rubric, and the thing a model
//! judge has to beat to be worth its cost (ADR-0061 decision 3).
//!
//! The rule is token containment after SQuAD-style normalisation —
//! lowercase, drop punctuation, drop the articles, collapse whitespace —
//! and every content token of the reference must appear in the candidate.
//! Borrowed rather than invented, for the reason ADR-0047 gave its own
//! guards: a home-grown rubric would start making judgements of its own,
//! and this one's whole job is to be the boring baseline whose misses are
//! attributable.
//!
//! Its weakness is the point. `3` and `three`, `a fortnight` and `two
//! weeks`, any paraphrase at all — the rubric calls them wrong, and a
//! model judge that agrees with a human on exactly those is what ADR-0053
//! option 9 asked to be able to measure. Running one labelled set through
//! both is that measurement; a strawman baseline would have made the
//! model judge look good for free.

use std::collections::BTreeSet;

use super::{Judge, JudgeInput, Verdict};

/// The method name in the report and the tally.
const METHOD: &str = "lexical";

/// The rubric's version, the `builtin@1` convention. Bump it when the
/// normalisation changes: a published agreement rate is a joint property
/// of the judge *and* whatever it was compared against.
const RULESET: &str = "rubric@1";

/// Dropped before comparison. SQuAD's normalisation drops exactly these
/// three and nothing else, and the list stays that short on purpose — a
/// stopword list long enough to be interesting is a list making
/// judgements the rubric is not entitled to make.
const ARTICLES: [&str; 3] = ["a", "an", "the"];

/// How many missing tokens the rationale names before it stops. A
/// rationale is read by a person comparing two judges; a hundred-token
/// dump is not.
const MAX_NAMED: usize = 8;

/// The network-free rubric.
#[derive(Debug, Clone, Copy, Default)]
pub struct LexicalJudge;

impl LexicalJudge {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Judge for LexicalJudge {
    fn method(&self) -> &'static str {
        METHOD
    }

    async fn grade(&self, input: &JudgeInput<'_>) -> Result<Verdict, String> {
        let reference = normalise(input.reference);
        if reference.is_empty() {
            // Not a wrong answer — an unjudgeable one. Containment against
            // an empty reference is trivially true, so grading it would
            // score every candidate correct and report the run as perfect.
            return Err(format!(
                "the reference for {:?} holds no content token, so nothing can be graded against \
                 it — an empty reference passes every candidate",
                truncate(input.question)
            ));
        }
        let candidate = normalise(input.candidate);
        let missing: Vec<&str> = reference
            .iter()
            .filter(|token| !candidate.contains(*token))
            .map(String::as_str)
            .collect();

        let correct = missing.is_empty();
        let rationale = if correct {
            format!(
                "every content token of the reference ({}) appears in the candidate",
                reference.len()
            )
        } else {
            let named: Vec<&str> = missing.iter().take(MAX_NAMED).copied().collect();
            let rest = missing.len() - named.len();
            let tail = if rest > 0 {
                format!(" and {rest} more")
            } else {
                String::new()
            };
            format!(
                "the candidate is missing {} of the reference's {} content tokens: {}{tail}",
                missing.len(),
                reference.len(),
                named.join(", ")
            )
        };
        Ok(Verdict {
            correct,
            rationale,
            method: METHOD.to_owned(),
            model_version: RULESET.to_owned(),
            // A rubric has no effort to record, and an invented one would
            // read as a setting somebody could change. Same for tokens.
            effort: None,
            usage: None,
        })
    }
}

/// Lowercased alphanumeric tokens, articles dropped. A set rather than a
/// bag: the rubric asks whether the reference's vocabulary is present,
/// not how many times, and counting repetitions would fail a candidate
/// for saying something once that the reference said twice.
fn normalise(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .filter(|token| !ARTICLES.contains(&token.as_str()))
        .collect()
}

fn truncate(text: &str) -> String {
    text.chars().take(72).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn grade(reference: &str, candidate: &str) -> Verdict {
        LexicalJudge::new()
            .grade(&JudgeInput {
                question: "when did the lease end",
                reference,
                candidate,
            })
            .await
            .expect("graded")
    }

    #[tokio::test]
    async fn wording_punctuation_and_articles_do_not_decide_it() {
        let verdict = grade(
            "the lease ended in March",
            "Going by your messages, your lease ended in march — about five months ago.",
        )
        .await;
        assert!(verdict.correct, "{}", verdict.rationale);
        assert_eq!(verdict.method, "lexical");
        assert_eq!(verdict.model_version, "rubric@1");
    }

    #[tokio::test]
    async fn a_missing_reference_token_is_named_rather_than_summarised() {
        let verdict = grade("the lease ended in March", "Your lease ended recently.").await;
        assert!(!verdict.correct);
        assert!(verdict.rationale.contains("march"), "{}", verdict.rationale);
    }

    /// The rubric's known blind spot, asserted rather than assumed. This
    /// is the pair a model judge has to get right to be worth its cost,
    /// and pinning it here means a future change that quietly "fixed" the
    /// rubric would have to say so.
    #[tokio::test]
    async fn a_paraphrase_the_rubric_cannot_see_is_graded_wrong() {
        let verdict = grade("three weeks", "It took about 21 days.").await;
        assert!(
            !verdict.correct,
            "the rubric is not supposed to see paraphrases: {}",
            verdict.rationale
        );
    }

    #[tokio::test]
    async fn an_empty_reference_is_an_error_rather_than_a_pass() {
        let err = LexicalJudge::new()
            .grade(&JudgeInput {
                question: "when did the lease end",
                reference: "   ...   ",
                candidate: "anything at all",
            })
            .await
            .expect_err("an empty reference must not grade");
        assert!(err.contains("every candidate"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn extra_detail_in_the_candidate_is_not_a_miss() {
        // Containment runs one way on purpose: a reader model that answers
        // correctly and then adds context has not answered wrongly.
        let verdict = grade("March", "March, though you mentioned moving out early.").await;
        assert!(verdict.correct, "{}", verdict.rationale);
    }
}
