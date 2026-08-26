//! The Reader seam (EVAL-3, ADR-0061 decision 6): the thing that answers
//! a question **from the governed block and from nothing else**.
//!
//! **Why this exists at all.** Synveda does not answer questions; it
//! serves a block. LongMemEval grades whether a free-text answer matches
//! a reference. Something has to stand between the two, and the ADR is
//! blunt about what that means for the number: "a memory benchmark score
//! is never a measurement of the memory system alone" — it is a joint
//! property of the block, this reader, and the judge, and decision 6
//! keys the baseline to all of them.
//!
//! **From the block and nothing else** is the seam's one invariant. A
//! reader that could reach its own training knowledge, the network, or
//! the corpus would answer questions the block failed to support, and
//! the score would then measure the reader's world knowledge with
//! Synveda's name on it. Hence [`ReaderInput`] carries a question and a
//! block and has no third field, and the prompt spends most of its length
//! on that one rule.
//!
//! **Abstention is a first-class outcome, not an empty answer.** EVAL-1
//! decision 4 made it an axis because "a memory system that invents
//! context is worse than one that stays quiet", and LongMemEval's 30
//! abstention instances are that axis with an external corpus behind it.
//! A reader says it abstained; nothing downstream has to infer it from
//! prose.
//!
//! The shape is [`crate::judge`]'s, deliberately — trait, network-free
//! default, Claude implementation, `SYNVEDA_READER` selecting between
//! them the way `SYNVEDA_EXTRACTOR` selects an extractor. Two seams that
//! are read together should not have to be learned twice.

use serde::Serialize;

mod claude;
mod extractive;
mod prompt;

pub use claude::ClaudeReader;
pub use extractive::ExtractiveReader;

/// One question and the block it must be answered out of.
///
/// Borrowed rather than owned, the `client::SessionEventBatchRequest` idiom — a run
/// holds each block once and reads out of it.
#[derive(Debug, Clone, Copy)]
pub struct ReaderInput<'a> {
    /// The instance's question.
    pub question: &'a str,
    /// The governed block, exactly as a session ContextRun served it —
    /// current JSON Knowledge payloads, safety notice and address footer.
    /// Passed verbatim because trimming it here would measure a block the
    /// product does not serve.
    pub block: &'a str,
}

/// One answer, and the provenance that makes the score it feeds
/// reproducible.
#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    /// The free-text answer the judge will grade. When the reader
    /// abstained this is its decline, in words — a reference that says
    /// the block holds no answer has to be gradeable against something.
    pub text: String,
    /// Whether the reader declined because the block does not support an
    /// answer. First-class rather than inferred from `text` (EVAL-1
    /// decision 4): the abstention axis should not depend on a judge
    /// reading prose, and "I don't know" is a sentence a wrong answer can
    /// also contain.
    pub abstained: bool,
    /// `extractive` or `claude-api`.
    pub method: String,
    /// The ruleset or model version behind the answer — for a model, what
    /// the API *served* (decision 6).
    pub model_version: String,
    /// The effort the answer was produced at, for a model-backed reader.
    ///
    /// It matters more here than anywhere: a reader that reasons harder
    /// can paper over a worse block, so the reader's effort is part of
    /// what a published score depends on and part of what the baseline is
    /// keyed to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What the answer cost, for a model-backed reader. `None` for a
    /// ruleset, which costs nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::anthropic::Usage>,
}

/// The reading seam. Failures are `Err` with a named reason rather than
/// an abstention: a reader that could not reach its model has not found
/// the block unsupportive, and scoring it as an abstention would move the
/// axis EVAL-1 decision 4 built for the opposite reason.
///
/// `async fn` in a public trait is deliberate: dispatch is static through
/// [`AnyReader`], never `dyn`.
#[allow(async_fn_in_trait)]
pub trait Reader {
    /// The stable method name recorded in the report and the tally.
    fn method(&self) -> &'static str;

    /// Answers one question out of one block.
    async fn read(&self, input: &ReaderInput<'_>) -> Result<Answer, String>;
}

/// The configured reader, dispatched statically.
#[derive(Debug, Clone)]
pub enum AnyReader {
    /// The rule-based, network-free default.
    Extractive(ExtractiveReader),
    /// The Anthropic Messages API (decision 6).
    Claude(ClaudeReader),
}

impl Reader for AnyReader {
    fn method(&self) -> &'static str {
        match self {
            AnyReader::Extractive(inner) => inner.method(),
            AnyReader::Claude(inner) => inner.method(),
        }
    }

    async fn read(&self, input: &ReaderInput<'_>) -> Result<Answer, String> {
        match self {
            AnyReader::Extractive(inner) => inner.read(input).await,
            AnyReader::Claude(inner) => inner.read(input).await,
        }
    }
}

/// Builds the configured reader from `SYNVEDA_READER` and its companions.
///
/// There is no `off`: a run without a reader produces no answers, and a
/// harness that silently declined to read would report a QA accuracy of
/// nothing. `SYNVEDA_ANTHROPIC_BASE_URL` is shared with the judge and the
/// extractor on purpose — it names one endpoint.
pub fn from_env() -> Result<AnyReader, String> {
    from_vars(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

/// The selection itself, over a lookup rather than the process
/// environment — [`crate::judge::from_env`]'s arrangement, for the same
/// reason: the tests exercise every branch without the `unsafe` that
/// `std::env::set_var` now needs, which this crate forbids.
fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<AnyReader, String> {
    let selected = var("SYNVEDA_READER").unwrap_or_else(|| "extractive".to_owned());
    match selected.as_str() {
        "extractive" => Ok(AnyReader::Extractive(ExtractiveReader::new())),
        "claude" => {
            let api_key = var("ANTHROPIC_API_KEY")
                .ok_or("SYNVEDA_READER=claude requires ANTHROPIC_API_KEY")?;
            let base_url = var("SYNVEDA_ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| ClaudeReader::DEFAULT_BASE_URL.to_owned());
            let model = var("SYNVEDA_READER_MODEL")
                .unwrap_or_else(|| ClaudeReader::DEFAULT_MODEL.to_owned());
            Ok(AnyReader::Claude(ClaudeReader::new(
                api_key, model, base_url,
            )))
        }
        other => Err(format!(
            "SYNVEDA_READER must be extractive|claude, got {other:?}"
        )),
    }
}

/// `synveda_eval_reader_calls_total` by outcome, and
/// `synveda_eval_reader_seconds`.
///
/// The sibling of [`crate::judge::Tally`], which ADR-0061 decision 12
/// names; this one it does not, because the ADR wrote the judge's DoD
/// before the reader had a seam. A paid path with no call count is one
/// nobody can budget, and the outcome vocabulary is genuinely its own —
/// a reader abstains where a judge disagrees, and those are not the same
/// event with different words.
#[derive(Debug, Default, Serialize)]
pub struct Tally {
    /// By outcome: `answered`, `abstained`, `error`. An abstention is not
    /// a failure — EVAL-1 decision 4 makes it the correct outcome for a
    /// block that supports nothing — and an error is not an abstention.
    pub calls: std::collections::BTreeMap<String, usize>,
    /// `synveda_eval_reader_tokens`, summed over every call that reached
    /// a model. The reader is the expensive half on LongMemEval: a
    /// 115k-token haystack is the prompt, so its per-instance cost is
    /// what decides whether the full 500 is a run or a plan.
    pub tokens: crate::anthropic::Usage,
    /// Summed over every call including the ones that failed.
    pub seconds: f64,
}

impl Tally {
    /// Reads through the tally, which is the only way this crate calls a
    /// reader. Structural rather than a convention: a call site that
    /// could bypass the counters would eventually be one that did.
    pub async fn read(
        &mut self,
        reader: &AnyReader,
        input: &ReaderInput<'_>,
    ) -> Result<Answer, String> {
        let started = std::time::Instant::now();
        let outcome = reader.read(input).await;
        self.seconds = round(self.seconds + started.elapsed().as_secs_f64());
        if let Ok(answer) = &outcome
            && let Some(usage) = answer.usage
        {
            self.tokens.add(usage);
        }
        let label = match &outcome {
            Ok(answer) if answer.abstained => "abstained",
            Ok(_) => "answered",
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

/// The reader and the judge must not be the same model instance, and
/// ADR-0061 option 7 says why: "a model grading answers produced from its
/// own reading is a measurement with a known bias and no way to bound
/// it." They may be the same *family* — the ADR allows that explicitly —
/// so this does not refuse a shared model id. It says so out loud, once,
/// in the report, because a bias nobody wrote down is one nobody
/// discounts when quoting the number.
#[must_use]
pub fn independence_note(reader: &AnyReader, judge: &crate::judge::AnyJudge) -> Option<String> {
    use crate::judge::Judge;
    let (AnyReader::Claude(reader), crate::judge::AnyJudge::Claude(_)) = (reader, judge) else {
        return None;
    };
    Some(format!(
        "reader `{}` and judge `{}` are both model-backed; ADR-0061 option 7 keeps them separate \
         calls but allows the same family, so a shared model id means the grade carries a known \
         self-reading bias with no bound on it (reader model: {})",
        reader.method(),
        judge.method(),
        reader.model()
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let table: BTreeMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name: &str| table.get(name).cloned()
    }

    #[test]
    fn the_default_is_the_network_free_reader() {
        let reader = from_vars(vars(&[])).expect("an unset environment must still read");
        assert_eq!(reader.method(), "extractive");
    }

    #[test]
    fn claude_without_a_key_is_a_configuration_error_rather_than_a_silent_fallback() {
        let err =
            from_vars(vars(&[("SYNVEDA_READER", "claude")])).expect_err("no key must not build");
        assert!(err.contains("ANTHROPIC_API_KEY"), "unhelpful error: {err}");
    }

    #[test]
    fn an_unknown_reader_names_the_vocabulary() {
        let err = from_vars(vars(&[("SYNVEDA_READER", "psychic")]))
            .expect_err("unknown reader must not build");
        assert!(err.contains("extractive|claude"), "unhelpful error: {err}");
    }

    #[test]
    fn the_reader_takes_its_own_model_variable_not_the_judges() {
        let reader = from_vars(vars(&[
            ("SYNVEDA_READER", "claude"),
            ("ANTHROPIC_API_KEY", "test-key-never-real"),
            ("SYNVEDA_READER_MODEL", "claude-opus-4-8"),
            // Set, and deliberately ignored here: the two seams are keyed
            // separately because decision 6 records two model versions.
            ("SYNVEDA_JUDGE_MODEL", "claude-sonnet-5"),
        ]))
        .expect("builds");
        let AnyReader::Claude(claude) = reader else {
            panic!("SYNVEDA_READER=claude must select the Claude reader");
        };
        assert_eq!(claude.model(), "claude-opus-4-8");
        assert!(
            !format!("{claude:?}").contains("test-key-never-real"),
            "a debug print must not carry the key"
        );
    }

    #[test]
    fn two_model_backed_seams_are_allowed_but_never_silent() {
        let reader = AnyReader::Claude(ClaudeReader::new(
            "k".to_owned(),
            ClaudeReader::DEFAULT_MODEL.to_owned(),
            ClaudeReader::DEFAULT_BASE_URL.to_owned(),
        ));
        let judge = crate::judge::AnyJudge::Claude(crate::judge::ClaudeJudge::new(
            "k".to_owned(),
            crate::judge::ClaudeJudge::DEFAULT_MODEL.to_owned(),
            crate::judge::ClaudeJudge::DEFAULT_BASE_URL.to_owned(),
        ));
        let note = independence_note(&reader, &judge).expect("a note is owed here");
        assert!(note.contains("self-reading bias"), "{note}");

        // The default pairing has no such bias to declare.
        let rubric = crate::judge::AnyJudge::Lexical(crate::judge::LexicalJudge::new());
        assert!(independence_note(&reader, &rubric).is_none());
    }
}
