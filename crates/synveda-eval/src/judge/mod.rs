//! The Judge seam (EVAL-3, ADR-0061 decision 3): one trait, a
//! deterministic default, and a Claude-backed implementation selected by
//! `SYNVEDA_JUDGE` exactly as `SYNVEDA_EXTRACTOR` selects an extractor.
//!
//! **Why a seam rather than a function.** Five ADRs deferred a
//! model-backed judge, and each named a property it has to have. The
//! binding one is ADR-0046 option 6: "a judge whose own precision nobody
//! has measured should not be the thing that decides whether the product
//! regressed." Two implementations behind one trait is what turns that
//! from a warning into a measurement — the default is the lexical rubric
//! this harness already grades by, the Claude one is the thing under
//! test, and the question ADR-0053 option 9 actually asked (whether a
//! model judge predicts anything the rubric does not) is answered by
//! running one labelled set through both and comparing.
//!
//! **The default reaches no network, and that is deliberate.** A
//! benchmark number produced by an unmeasured judge is a second opinion
//! with a decimal point (decision 4), so the judged path costs money only
//! when someone asks for it by name — the same disposition ADR-0046
//! decision 12 gave live extraction.

use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;

mod claude;
mod lexical;
mod prompt;

pub use claude::ClaudeJudge;
pub use lexical::LexicalJudge;

/// One claim of equivalence to grade: does `candidate` answer `question`
/// the way `reference` does?
///
/// Borrowed rather than owned, the `client::SessionEventBatchRequest` idiom — a
/// judged run holds its corpus once and grades out of it.
///
/// The three fields carry both labelled sets decision 4 names. For
/// LongMemEval they are the instance's question, its reference answer,
/// and what the reader model produced from the governed block. For
/// EVAL-2's unmatched-record list, `question` is what the expectation
/// asked of the event, `reference` is the expected content and
/// `candidate` the record the pipeline actually served. One shape,
/// because both sets ask the same thing of a judge.
#[derive(Debug, Clone, Copy)]
pub struct JudgeInput<'a> {
    pub question: &'a str,
    pub reference: &'a str,
    pub candidate: &'a str,
}

/// One verdict, and the provenance that makes it quotable.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    /// Whether the candidate conveys the reference's answer. Binary on
    /// purpose: LongMemEval publishes a QA accuracy, and a judge that
    /// graded on a scale would produce a number nobody else's is
    /// comparable with.
    pub correct: bool,
    /// One sentence naming what decided it. It rides into the report
    /// because reversal trigger (a) makes the disagreements the next
    /// feature's input rather than a footnote — a bare bit would leave
    /// nothing to work from.
    pub rationale: String,
    /// `lexical` or `claude-api`.
    pub method: String,
    /// The ruleset or model version behind the verdict. For a model it is
    /// what the API *served*, never the alias requested (decision 6, which
    /// is ADR-0046 decision 12's mechanism applied to the judge).
    pub model_version: String,
    /// The effort the verdict was produced at, for a model-backed judge;
    /// absent for a rubric that has none.
    ///
    /// Decision 6 keys the baseline to every model the number depends on,
    /// and a model's effort changes how it behaves as surely as its
    /// version does — the same request at `low` and at `high` is two
    /// judges. A figure quoted with the version and not the effort is
    /// half a provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What the verdict cost, for a model-backed judge. `None` for a
    /// rubric, which costs nothing and should not report a zero that
    /// reads like a measured one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::anthropic::Usage>,
}

/// The grading seam. Failures are `Err` with a named reason rather than a
/// verdict: a judge that could not reach its model has not decided the
/// answer is wrong, and scoring it as one would move a published number
/// for an operational reason.
///
/// `async fn` in a public trait is deliberate, for [`AnyExtractor`]'s
/// reason: dispatch is static through [`AnyJudge`], never `dyn`, so the
/// auto-trait caveat the lint warns about cannot bite.
///
/// [`AnyExtractor`]: https://docs.rs/synveda-ingest
#[allow(async_fn_in_trait)]
pub trait Judge {
    /// The stable method name recorded in the report and the tally.
    fn method(&self) -> &'static str;

    /// Grades one claim of equivalence.
    async fn grade(&self, input: &JudgeInput<'_>) -> Result<Verdict, String>;
}

/// The configured judge, dispatched statically.
#[derive(Debug, Clone)]
pub enum AnyJudge {
    /// The rubric-based, network-free default.
    Lexical(LexicalJudge),
    /// The Anthropic Messages API (decision 3).
    Claude(ClaudeJudge),
}

impl Judge for AnyJudge {
    fn method(&self) -> &'static str {
        match self {
            AnyJudge::Lexical(inner) => inner.method(),
            AnyJudge::Claude(inner) => inner.method(),
        }
    }

    async fn grade(&self, input: &JudgeInput<'_>) -> Result<Verdict, String> {
        match self {
            AnyJudge::Lexical(inner) => inner.grade(input).await,
            AnyJudge::Claude(inner) => inner.grade(input).await,
        }
    }
}

/// Builds the configured judge from `SYNVEDA_JUDGE` and its companions,
/// the shape `extractor_from_env` already established in the gateway.
///
/// There is no `off`. The lexical judge needs nothing, so the zero-config
/// path always grades — a harness that silently declined to would report
/// an agreement of nothing.
///
/// `SYNVEDA_ANTHROPIC_BASE_URL` is shared with the extractor on purpose:
/// it names one endpoint, and a second variable for the same host would
/// be a second thing to get wrong.
pub fn from_env() -> Result<AnyJudge, String> {
    from_vars(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

/// The selection itself, over a lookup rather than the process
/// environment. Split so the tests below can exercise every branch
/// without mutating a process-wide table three threads share — and
/// without the `unsafe` that `std::env::set_var` now needs, which this
/// crate forbids outright.
fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<AnyJudge, String> {
    let selected = var("SYNVEDA_JUDGE").unwrap_or_else(|| "lexical".to_owned());
    match selected.as_str() {
        "lexical" => Ok(AnyJudge::Lexical(LexicalJudge::new())),
        "claude" => {
            let api_key = var("ANTHROPIC_API_KEY")
                .ok_or("SYNVEDA_JUDGE=claude requires ANTHROPIC_API_KEY")?;
            let base_url = var("SYNVEDA_ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| ClaudeJudge::DEFAULT_BASE_URL.to_owned());
            let model =
                var("SYNVEDA_JUDGE_MODEL").unwrap_or_else(|| ClaudeJudge::DEFAULT_MODEL.to_owned());
            Ok(AnyJudge::Claude(ClaudeJudge::new(api_key, model, base_url)))
        }
        other => Err(format!(
            "SYNVEDA_JUDGE must be lexical|claude, got {other:?}"
        )),
    }
}

/// The two numbers ADR-0061 decision 12 names —
/// `synveda_eval_judge_calls_total` labelled by outcome, and
/// `synveda_eval_judge_seconds`.
///
/// They are report fields rather than Prometheus counters because this
/// harness is a client CLI with no metrics endpoint to scrape and no
/// process to scrape it: ADR-0028 decision 1 gave it an empty dependency
/// set, and a recorder nothing collects from would be instrumentation
/// that only looked like instrumentation. The names are kept verbatim so
/// the day a judged run happens under a scraper, the axis is already the
/// one the ADR named.
#[derive(Debug, Default, Serialize)]
pub struct Tally {
    /// `synveda_eval_judge_calls_total`, by outcome: `correct`,
    /// `incorrect`, `error`. An errored call is its own outcome rather
    /// than an incorrect verdict, because the two mean opposite things
    /// about the product.
    pub calls: BTreeMap<String, usize>,
    /// `synveda_eval_judge_tokens`, summed over every call that reached a
    /// model. This path bills per pair, and decision 7's slice-versus-full
    /// choice cannot be made from a call count alone.
    pub tokens: crate::anthropic::Usage,
    /// `synveda_eval_judge_seconds`, summed over every call including the
    /// ones that failed — a judge that is slow to fail still costs the
    /// run its wall clock.
    pub seconds: f64,
}

impl Tally {
    /// Grades through the tally, which is the only way this crate calls a
    /// judge. Structural rather than a convention: a call site that could
    /// bypass the counters would eventually be one that did.
    pub async fn grade(
        &mut self,
        judge: &AnyJudge,
        input: &JudgeInput<'_>,
    ) -> Result<Verdict, String> {
        let started = Instant::now();
        let outcome = judge.grade(input).await;
        self.seconds = round(self.seconds + started.elapsed().as_secs_f64());
        if let Ok(verdict) = &outcome
            && let Some(usage) = verdict.usage
        {
            self.tokens.add(usage);
        }
        let label = match &outcome {
            Ok(verdict) if verdict.correct => "correct",
            Ok(_) => "incorrect",
            Err(_) => "error",
        };
        *self.calls.entry(label.to_owned()).or_default() += 1;
        outcome
    }

    /// Every call this tally saw, whatever the outcome.
    #[must_use]
    pub fn total(&self) -> usize {
        self.calls.values().sum()
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lookup over a fixed table, standing in for the environment.
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let table: BTreeMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name: &str| table.get(name).cloned()
    }

    #[test]
    fn the_default_is_the_network_free_rubric() {
        let judge = from_vars(vars(&[])).expect("an unset environment must still grade");
        assert_eq!(judge.method(), "lexical");
    }

    #[test]
    fn claude_without_a_key_is_a_configuration_error_rather_than_a_silent_fallback() {
        // The gateway's discipline: a misconfigured extractor is a startup
        // error, not a quietly different pipeline. A judge that fell back
        // to the rubric here would publish a lexical number under a model
        // judge's name.
        let err =
            from_vars(vars(&[("SYNVEDA_JUDGE", "claude")])).expect_err("no key must not build");
        assert!(err.contains("ANTHROPIC_API_KEY"), "unhelpful error: {err}");
    }

    #[test]
    fn claude_takes_its_model_and_endpoint_from_the_environment() {
        let judge = from_vars(vars(&[
            ("SYNVEDA_JUDGE", "claude"),
            ("ANTHROPIC_API_KEY", "test-key-never-real"),
            ("SYNVEDA_JUDGE_MODEL", "claude-opus-4-8"),
            ("SYNVEDA_ANTHROPIC_BASE_URL", "http://127.0.0.1:9/"),
        ]))
        .expect("builds");
        let AnyJudge::Claude(claude) = judge else {
            panic!("SYNVEDA_JUDGE=claude must select the Claude judge");
        };
        assert_eq!(claude.method(), "claude-api");
        let shown = format!("{claude:?}");
        // The trailing slash is trimmed, so the request path cannot end
        // up with a double slash the endpoint would 404.
        assert!(
            shown.contains("http://127.0.0.1:9\""),
            "the base url keeps no trailing slash: {shown}"
        );
        assert!(shown.contains("claude-opus-4-8"), "{shown}");
        // The key is held, sent, and never printable.
        assert!(
            !shown.contains("test-key-never-real"),
            "a debug print must not carry the key: {shown}"
        );
    }

    #[test]
    fn an_unknown_judge_names_the_vocabulary() {
        let err = from_vars(vars(&[("SYNVEDA_JUDGE", "vibes")]))
            .expect_err("unknown judge must not build");
        assert!(err.contains("lexical|claude"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn the_tally_counts_by_outcome_and_never_conflates_error_with_incorrect() {
        let judge = AnyJudge::Lexical(LexicalJudge::new());
        let mut tally = Tally::default();

        tally
            .grade(
                &judge,
                &JudgeInput {
                    question: "how long",
                    reference: "three weeks",
                    candidate: "about three weeks",
                },
            )
            .await
            .expect("graded");
        tally
            .grade(
                &judge,
                &JudgeInput {
                    question: "how long",
                    reference: "three weeks",
                    candidate: "no idea",
                },
            )
            .await
            .expect("graded");
        // An empty reference is unjudgeable, not wrong.
        tally
            .grade(
                &judge,
                &JudgeInput {
                    question: "how long",
                    reference: "  ",
                    candidate: "anything at all",
                },
            )
            .await
            .expect_err("an empty reference must not grade");

        assert_eq!(tally.calls.get("correct"), Some(&1));
        assert_eq!(tally.calls.get("incorrect"), Some(&1));
        assert_eq!(tally.calls.get("error"), Some(&1));
        assert_eq!(tally.total(), 3);
    }
}
