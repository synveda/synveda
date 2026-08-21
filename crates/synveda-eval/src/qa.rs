//! The Q&A corpus (EVAL-4, ADR-0047 decision 4).
//!
//! A third file kind, and the reason is EVAL-2's second finding rather
//! than tidiness. `runner::wait_for_seed` waits only for the material a
//! scenario is graded on, so two scenarios sharing a tenant compose over
//! different corpora — two byte-identical runs measured `tokens_mean`
//! 129.8 and then 157 with no product change. A Q&A suite asks many
//! questions of *one* corpus, so it seeds once, waits for all of it, and
//! only then probes. Twenty questions written as twenty scenarios would
//! seed twenty times and measure twenty different corpora.
//!
//! What a corpus says that a scenario cannot: where each batch's material
//! ends up. Records land at the caller's home scope (ADR-0020) and a
//! service identity's home is a `principal`-shaped scope under its anchor
//! (ADR-0018 decision 2), so material above a leaf got there by climbing
//! through review — `promote_to` is that climb (decision 3).
//!
//! Every struct here refuses unknown fields, for EVAL-1's reason: a
//! silently-ignored expectation is an eval that passes for the wrong
//! reason.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

/// Where a batch's material ends up, which is also the axis it reports
/// into (`qa_scope_team` and friends). Closed, because the gradient it
/// names is closed (seed §4.4).
pub const TIERS: [&str; 4] = ["user", "team", "department", "org"];

/// What a question needs from the embedder before its answer is
/// reachable (decision 5).
pub const NEEDS: [&str; 2] = ["lexical", "semantic"];

/// The observe kinds MEM-1 accepts.
const KINDS: [&str; 3] = ["transcript_delta", "tool_result", "decision"];

/// Words too common to count as lexical overlap between a question and
/// its answer. Small and deliberately unclever: the guard it serves is
/// "did the fixture author mislabel `semantic`", and a stopword list that
/// tried to be complete would start making that judgement itself.
const STOPWORDS: [&str; 32] = [
    "about", "after", "again", "against", "been", "before", "does", "doing", "down", "during",
    "each", "from", "have", "here", "how", "into", "more", "most", "only", "other", "over", "same",
    "some", "such", "than", "that", "them", "then", "there", "these", "this", "what",
];

/// A word long enough to carry meaning for the overlap guard.
const MIN_CONTENT_WORD: usize = 4;

/// One corpus file: what to plant, where it ends up, and what to ask of
/// it afterwards.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub corpus: String,
    /// Why this corpus exists, for whoever adds to it next.
    pub note: String,
    /// The actor every question probes as. One reader, because the
    /// measurement is "what does *this* session get", and two readers in
    /// one file would be two corpora again.
    pub reader: String,
    pub seed: Vec<SeedBatch>,
    pub questions: Vec<Question>,
}

/// One `/v1/observe` call, as one actor, plus where its material ends up.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedBatch {
    pub actor: String,
    pub session_id: String,
    /// The scope tier this batch's material ends up at, and the axis it
    /// reports into.
    pub tier: String,
    /// The hierarchy node to climb to, named as the environment names it.
    /// Absent means the material stays at the author's own leaf, which is
    /// the `user` tier and the only tier that needs no review.
    #[serde(default)]
    pub promote_to: Option<String>,
    pub events: Vec<SeedEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedEvent {
    /// How questions refer to this event.
    pub key: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub text: String,
}

fn default_kind() -> String {
    "transcript_delta".to_owned()
}

/// One probe of the seeded corpus.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub name: String,
    /// What the session is about — the retrieval query. Absent is the
    /// taskless, recency-ordered branch (ADR-0025 decision 5), which takes
    /// no retrieval leg at all and therefore measures composition alone.
    #[serde(default)]
    pub task: Option<String>,
    /// What this question needs from the embedder before its answer is
    /// reachable. `lexical` questions share terms with their answer and
    /// the sparse leg finds them on any stack; `semantic` ones are
    /// paraphrases that share none, and only a real embedding model
    /// reaches them (decision 5).
    #[serde(default = "default_needs")]
    pub needs: String,
    /// Seed keys whose records must reach the block. Graded by record
    /// identity, never by containment (decision 2).
    pub expect_records: Vec<String>,
    /// Phrases that must not appear — another scope's material that
    /// leaked, or a record that ranked when it should not have.
    #[serde(default)]
    pub must_not_contain: Vec<String>,
    /// The caller-side budget. Tight on purpose for the questions that
    /// feed `retrieval_precision`: precision only means something when the
    /// budget is small enough that ranking decides what fits.
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    /// Why this question is interesting, or what it is expected to miss
    /// and why. Rides into the report, where an anticipated miss is
    /// otherwise indistinguishable from a regression (the EVAL-2
    /// discipline).
    #[serde(default)]
    pub note: String,
}

fn default_needs() -> String {
    "lexical".to_owned()
}

impl Question {
    /// Whether a run whose embedder cannot rank may score this question.
    /// A `semantic` question there is skipped and counted, never scored
    /// zero: a question the configured path structurally cannot answer is
    /// not a regression, and scoring it as one teaches the next reader to
    /// delete it (decision 5).
    #[must_use]
    pub fn is_semantic(&self) -> bool {
        self.needs == "semantic"
    }
}

impl Corpus {
    /// The batch a seeded key belongs to.
    #[must_use]
    pub fn batch_of(&self, key: &str) -> Option<&SeedBatch> {
        self.seed
            .iter()
            .find(|batch| batch.events.iter().any(|event| event.key == key))
    }
}

/// Content words of a string, lowercased, stopwords and short words
/// dropped.
fn content_words(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.len() >= MIN_CONTENT_WORD && !STOPWORDS.contains(&word.as_str()))
        .collect()
}

/// Every `*.json` corpus in a directory, in filename order so two runs
/// report in the same order. Validated as a whole: the guards are
/// corpus-wide, not per file.
pub fn load_corpora(dir: &Path) -> Result<Vec<Corpus>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the Q&A corpus {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no Q&A corpora", dir.display()));
    }

    let mut corpora = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let corpus: Corpus = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid Q&A corpus: {err}", path.display()))?;
        corpora.push(corpus);
    }
    validate(&corpora)?;
    Ok(corpora)
}

/// The checks serde cannot make. All of them run in `synveda-eval check`,
/// with no database and no gateway, because a mislabelled question would
/// otherwise move a gated number forever — and the two that matter most
/// are the `needs` guards, which are this corpus's version of EVAL-2's
/// "an expected token absent from its own source".
fn validate(corpora: &[Corpus]) -> Result<(), String> {
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    let mut sessions: BTreeMap<&str, &str> = BTreeMap::new();
    let mut corpus_names: BTreeSet<&str> = BTreeSet::new();

    for corpus in corpora {
        if !corpus_names.insert(corpus.corpus.as_str()) {
            return Err(format!("two corpora are both named `{}`", corpus.corpus));
        }
        if corpus.seed.is_empty() {
            return Err(format!("corpus `{}` plants nothing", corpus.corpus));
        }
        if corpus.questions.is_empty() {
            return Err(format!("corpus `{}` asks nothing", corpus.corpus));
        }

        let mut keys: BTreeMap<&str, &SeedEvent> = BTreeMap::new();
        for batch in &corpus.seed {
            let at = |what: &str| format!("{}/{}: {what}", corpus.corpus, batch.session_id);
            if !TIERS.contains(&batch.tier.as_str()) {
                return Err(at(&format!(
                    "tier `{}` is not one of {TIERS:?}",
                    batch.tier
                )));
            }
            // The two halves of decision 3, stated as an equivalence so
            // neither can drift: `user` is the tier that needs no review,
            // and every other tier is one a climb produced.
            match (batch.tier.as_str(), batch.promote_to.as_deref()) {
                ("user", Some(scope)) => {
                    return Err(at(&format!(
                        "tier `user` is the author's own leaf and promotes nowhere, but this \
                         batch climbs to `{scope}`"
                    )));
                }
                (tier, None) if tier != "user" => {
                    return Err(at(&format!(
                        "tier `{tier}` sits above a leaf, and nothing but a promotion can put \
                         material there — name the scope in `promote_to`"
                    )));
                }
                _ => {}
            }
            if batch.events.is_empty() {
                return Err(at("a batch that seeds nothing measures nothing"));
            }
            if let Some(previous) = sessions.insert(&batch.session_id, &corpus.corpus) {
                return Err(format!(
                    "session id `{}` is used by both corpus `{previous}` and corpus `{}`",
                    batch.session_id, corpus.corpus
                ));
            }
            for event in &batch.events {
                if !KINDS.contains(&event.kind.as_str()) {
                    return Err(at(&format!(
                        "kind `{}` is not one of {KINDS:?}",
                        event.kind
                    )));
                }
                if keys.insert(&event.key, event).is_some() {
                    return Err(at(&format!(
                        "seed key `{}` is used twice in one corpus",
                        event.key
                    )));
                }
            }
        }

        for question in &corpus.questions {
            let at = |what: &str| format!("{}/{}: {what}", corpus.corpus, question.name);
            if let Some(previous) = names.insert(&question.name, &corpus.corpus) {
                return Err(format!(
                    "question name `{}` is used by both corpus `{previous}` and corpus `{}`",
                    question.name, corpus.corpus
                ));
            }
            if !NEEDS.contains(&question.needs.as_str()) {
                return Err(at(&format!(
                    "needs `{}` is not one of {NEEDS:?}",
                    question.needs
                )));
            }
            if question.expect_records.is_empty() {
                return Err(at(
                    "a question that expects no records measures nothing — this suite grades by \
                     record identity, so there is nothing else for it to grade",
                ));
            }
            if question.budget_tokens == Some(0) {
                return Err(at("budget_tokens must be at least 1"));
            }
            let mut answer = String::new();
            for key in &question.expect_records {
                let Some(event) = keys.get(key.as_str()) else {
                    return Err(at(&format!(
                        "expect_records names `{key}`, which this corpus does not seed"
                    )));
                };
                answer.push(' ');
                answer.push_str(&event.text);
            }

            // The `needs` guards. A question declares which retrieval leg
            // has to find its answer, and the corpus is only honest if the
            // declaration matches the text: `lexical` means the sparse leg
            // can see it, which takes shared terms, and `semantic` means
            // it cannot, which takes none. Mislabel either and the
            // question fails on the wrong path for a corpus reason.
            let Some(task) = &question.task else {
                if question.is_semantic() {
                    return Err(at(
                        "a taskless probe takes no retrieval leg at all (ADR-0025 decision 5's \
                         else-branch), so it cannot be `semantic` — it is recency-ordered \
                         composition and reaches its answer on any stack",
                    ));
                }
                continue;
            };
            let shared: Vec<String> = content_words(task)
                .intersection(&content_words(&answer))
                .cloned()
                .collect();
            if question.is_semantic() && !shared.is_empty() {
                return Err(at(&format!(
                    "declared `semantic` but shares {shared:?} with its own answer, so the sparse \
                     leg can reach it and the question would pass without the dense one ever \
                     working"
                )));
            }
            if !question.is_semantic() && shared.is_empty() {
                return Err(at(
                    "declared `lexical` but shares no content word with its own answer, so the \
                     sparse leg cannot reach it — it would fail on the deterministic path for a \
                     corpus reason rather than a product one",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = r#"{
        "corpus": "c1",
        "note": "n",
        "reader": "qa-reader",
        "seed": [
            {"actor": "qa-reader", "session_id": "s-own", "tier": "user",
             "events": [{"key": "own", "text": "I always run cargo nextest before pushing."}]},
            {"actor": "qa-team", "session_id": "s-team", "tier": "team",
             "promote_to": "payments",
             "events": [{"key": "team", "kind": "decision",
                         "text": "Payments retries are capped at three attempts."}]}
        ],
        "questions": [
            {"name": "q-team", "task": "what are payments retries capped at",
             "expect_records": ["team"]}
        ]
    }"#;

    fn parse(json: &str) -> Result<Vec<Corpus>, String> {
        let corpus: Corpus = serde_json::from_str(json).map_err(|err| err.to_string())?;
        let corpora = vec![corpus];
        validate(&corpora)?;
        Ok(corpora)
    }

    #[test]
    fn a_corpus_round_trips_with_its_defaults() {
        let corpora = parse(CLEAN).expect("parses");
        let corpus = &corpora[0];
        assert_eq!(corpus.seed[0].events[0].kind, "transcript_delta");
        assert_eq!(corpus.questions[0].needs, "lexical");
        assert!(!corpus.questions[0].is_semantic());
        assert_eq!(
            corpus.batch_of("team").map(|b| b.tier.as_str()),
            Some("team")
        );
        assert!(corpus.batch_of("nothing").is_none());
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let json = CLEAN.replace(r#""expect_records""#, r#""expect_recods""#);
        let err = parse(&json).expect_err("unknown field must not parse");
        assert!(err.contains("expect_recods"), "unhelpful error: {err}");
    }

    #[test]
    fn a_question_naming_an_unseeded_key_is_refused() {
        let json = CLEAN.replace(r#"["team"]"#, r#"["ghost"]"#);
        let err = parse(&json).expect_err("dangling key must not validate");
        assert!(err.contains("ghost"), "unhelpful error: {err}");
    }

    /// The equivalence decision 3 turns on, in both directions: `user` is
    /// the only tier a leaf can hold, and every other tier is a climb.
    #[test]
    fn a_tier_and_a_promotion_must_agree() {
        let climbing_user = CLEAN.replace(
            r#""tier": "user","#,
            r#""tier": "user", "promote_to": "payments","#,
        );
        let err = parse(&climbing_user).expect_err("a user tier promotes nowhere");
        assert!(err.contains("promotes nowhere"), "{err}");

        let unpromoted_team = CLEAN.replace(r#""promote_to": "payments","#, "");
        let err = parse(&unpromoted_team).expect_err("a team tier needs a climb");
        assert!(err.contains("nothing but a promotion"), "{err}");
    }

    /// The guard that keeps the two paths honest. A `semantic` question
    /// sharing terms with its answer would pass on the deterministic path
    /// through the sparse leg, and the dense-leg axis would then be
    /// measuring nothing while reporting a number.
    #[test]
    fn a_semantic_question_sharing_words_with_its_answer_is_refused() {
        let json = CLEAN.replace(
            r#""task": "what are payments retries capped at","#,
            r#""task": "what are payments retries capped at", "needs": "semantic","#,
        );
        let err = parse(&json).expect_err("mislabelled semantic must not validate");
        assert!(err.contains("shares"), "unhelpful error: {err}");
        assert!(err.contains("retries") || err.contains("payments"), "{err}");
    }

    #[test]
    fn a_lexical_question_sharing_nothing_with_its_answer_is_refused() {
        let json = CLEAN.replace(
            r#""task": "what are payments retries capped at""#,
            r#""task": "how do we treat unsuccessful charge reattempts""#,
        );
        let err = parse(&json).expect_err("mislabelled lexical must not validate");
        assert!(
            err.contains("shares no content word"),
            "unhelpful error: {err}"
        );

        // …and the same question declared honestly is fine.
        let honest = json.replace(
            r#""expect_records""#,
            r#""needs": "semantic", "expect_records""#,
        );
        assert!(parse(&honest).is_ok(), "an honest semantic question parses");
    }

    #[test]
    fn a_taskless_question_cannot_be_semantic() {
        let json = CLEAN.replace(
            r#""task": "what are payments retries capped at","#,
            r#""needs": "semantic","#,
        );
        let err = parse(&json).expect_err("taskless semantic must not validate");
        assert!(err.contains("no retrieval leg"), "unhelpful error: {err}");
    }

    #[test]
    fn a_question_that_expects_no_records_is_refused() {
        let json = CLEAN.replace(r#""expect_records": ["team"]"#, r#""expect_records": []"#);
        let err = parse(&json).expect_err("an empty expectation measures nothing");
        assert!(err.contains("measures nothing"), "unhelpful error: {err}");
    }

    #[test]
    fn the_vocabularies_are_closed() {
        assert!(
            parse(&CLEAN.replace(r#""tier": "team""#, r#""tier": "division""#)).is_err(),
            "unknown tier must not validate"
        );
        assert!(
            parse(&CLEAN.replace(r#""kind": "decision""#, r#""kind": "thought""#)).is_err(),
            "unknown kind must not validate"
        );
        assert!(
            parse(&CLEAN.replace(
                r#""expect_records": ["team"]"#,
                r#""needs": "vibes", "expect_records": ["team"]"#
            ))
            .is_err(),
            "unknown needs must not validate"
        );
    }

    #[test]
    fn duplicate_names_across_corpora_are_refused() {
        let first: Corpus = serde_json::from_str(CLEAN).expect("parses");
        let second: Corpus =
            serde_json::from_str(&CLEAN.replace(r#""corpus": "c1""#, r#""corpus": "c2""#))
                .expect("parses");
        let err = validate(&[first, second]).expect_err("collision must not validate");
        assert!(err.contains("session id `s-own`"), "{err}");
    }

    #[test]
    fn content_words_drop_stopwords_and_short_words() {
        let words = content_words("What are the payments retries capped at?");
        assert!(words.contains("payments"));
        assert!(words.contains("retries"));
        assert!(!words.contains("what"), "stopword");
        assert!(!words.contains("the"), "too short");
        assert!(!words.contains("at"), "too short");
    }
}
