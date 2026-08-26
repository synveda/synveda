//! Seed → capture → publish → ask → grade, one Q&A corpus at a time
//! (EVAL-4, ADR-0047).
//!
//! ContextRun is budget-bounded and relevance-ranked, which is exactly what
//! this suite measures. Grading joins appended session-event ids to accepted
//! Knowledge ids and then to the current addresses in the rendered block;
//! content containment is never the authority. Shared placements are created
//! only by candidate acceptance through the VedaFlow-backed command path.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::client::{
    CaptureAcceptOptions, Client, ContextRunRequest, KnowledgeQueryRequest, KnowledgeSweepRequest,
    SessionEventBatchRequest, SessionEventInput,
};
use crate::qa::{Corpus, Question, VISIBILITIES};
use crate::report::{QaOutcome, QuestionOutcome, ScopeCounts};
use crate::runner::apply_candidate;
use crate::scenario::Environment;

/// The reviewers every governed publication goes through. Fixed names rather than
/// corpus fields: who may approve is the pack's answer at the target
/// scope, and a corpus that named its own approvers would be describing
/// the approval matrix instead of exercising it.
pub const CURATOR_ACTOR: &str = "qa-curator";
pub const STEWARD_ACTOR: &str = "qa-steward";
pub const PUBLISHER_ACTOR: &str = "qa-publisher";

/// What a sweep asks for, matching EVAL-2's: the surface caps a sweep
/// here, so asking for exactly it and receiving exactly it is the
/// ambiguity ADR-0046 decision 3 refuses to measure through.
const SWEEP_LIMIT: usize = 32;

const POLL: Duration = Duration::from_millis(500);

pub struct Options {
    pub seed_timeout: Duration,
    /// Whether this run's embedder ranks by meaning. False is the
    /// deterministic hash embedder (ADR-0023 decision 6), whose geometry
    /// carries none — `semantic` questions are then skipped and counted
    /// rather than scored zero (decision 5).
    pub dense_retrieval: bool,
}

/// One corpus's measurement. Errors are the corpus's failures, not the
/// run's: a corpus that cannot be measured reads as a failed corpus with
/// a named reason, and the others still report.
pub async fn run_corpus(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    options: &Options,
) -> Result<QaOutcome, String> {
    let mut outcome = QaOutcome::new(corpus);
    let reader = &environment.actor(&corpus.reader)?.token;
    let started = Instant::now();
    let placed = seed(client, environment, corpus, options, &mut outcome).await?;
    wait_for_search(client, environment, corpus, &placed, options, &mut outcome).await?;
    outcome.served_knowledge = visible_total(client, reader, corpus).await?;
    outcome.seed_wait_ms = round(started.elapsed().as_secs_f64() * 1000.0);

    let reference =
        tiktoken_rs::o200k_base().map_err(|err| format!("load the reference tokenizer: {err}"))?;

    for question in &corpus.questions {
        if question.is_semantic() && !options.dense_retrieval {
            outcome.skipped_semantic += 1;
            outcome.questions.push(skipped(question));
            continue;
        }
        let probe_run = client
            .session_for(
                reader,
                &format!("eval:qa:{}:{}", corpus.corpus, question.name),
            )
            .await?;
        let probe = client
            .compose_context(
                reader,
                &probe_run,
                &ContextRunRequest {
                    task: question.task.as_deref(),
                    budget_tokens: question.budget_tokens,
                },
            )
            .await?;
        outcome.questions.push(grade(
            corpus,
            question,
            &placed,
            &probe,
            &reference,
            outcome.served_knowledge,
        ));
    }

    outcome.passed = outcome.failures.is_empty()
        && outcome
            .questions
            .iter()
            .all(|question| question.passed || question.skipped);
    Ok(outcome)
}

/// A seed key's immutable Knowledge revisions, as returned by the governed
/// candidate decision itself. Publication is the authority for this mapping;
/// rediscovering it through a later query would make an indexing or policy
/// result masquerade as a write-side outcome.
#[derive(Default)]
struct Placed {
    knowledge_item_ids: Vec<String>,
    /// The seeded text, used as its own retrieval query while waiting for
    /// the sparse index — an exact readiness condition, and deliberately
    /// not the question's own task (see `wait_for_search`).
    text: String,
    /// The actor that wrote it. The readiness check asks *this* identity
    /// rather than the reader, for the reason `wait_for_search` gives.
    actor: String,
}

async fn seed(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    options: &Options,
    outcome: &mut QaOutcome,
) -> Result<BTreeMap<String, Placed>, String> {
    let mut placed = BTreeMap::new();
    for batch in &corpus.seed {
        let bearer = &environment.actor(&batch.actor)?.token;
        let occurred_at = chrono::Utc::now().to_rfc3339();
        let events: Vec<SessionEventInput<'_>> = batch
            .events
            .iter()
            .map(|event| SessionEventInput {
                idempotency_key: format!("{}:{}", batch.session_id, event.key),
                kind: &event.event_type,
                payload: serde_json::json!({ "text": event.text }),
                occurred_at: occurred_at.clone(),
            })
            .collect();
        let session = client.session_for(bearer, &batch.session_id).await?;
        let response = client
            .append_events(bearer, &session, &SessionEventBatchRequest { events })
            .await?;
        let acked = &response.value;
        if acked.denied > 0 || acked.quarantined > 0 {
            return Err(format!(
                "seeding `{}` was withheld: {} denied, {} quarantined — the corpus is \
                 documentation-only content and should trip neither",
                batch.session_id, acked.denied, acked.quarantined
            ));
        }
        let mut by_event = BTreeMap::new();
        for event in &batch.events {
            let key = format!("{}:{}", batch.session_id, event.key);
            let event_id = acked
                .events
                .iter()
                .find(|entry| entry.idempotency_key == key)
                .and_then(|entry| entry.event_id().map(str::to_owned))
                .ok_or_else(|| {
                    format!(
                        "seeding `{}` acked no event id, so nothing downstream can be attributed \
                         to it",
                        event.key
                    )
                })?;
            by_event.insert(event_id, (event.key.clone(), event.text.clone()));
        }
        let target = batch
            .publish_scope
            .as_deref()
            .map(|name| environment.scope(name))
            .transpose()?;
        let reviewed = client
            .capture_and_accept(
                bearer,
                &session,
                &format!("eval-qa-{}-{}", corpus.corpus, batch.session_id),
                options.seed_timeout,
                CaptureAcceptOptions {
                    scope_id: target,
                    sensitivity: None,
                    ..CaptureAcceptOptions::default()
                },
            )
            .await?;
        for candidate in &reviewed {
            let applied =
                apply_candidate(client, environment, candidate, &batch.session_id).await?;
            let mut attributed = false;
            for source in &candidate.source_event_ids {
                let Some((key, text)) = by_event.get(source) else {
                    continue;
                };
                attributed = true;
                let slot = placed.entry(key.clone()).or_insert_with(|| Placed {
                    knowledge_item_ids: Vec::new(),
                    text: text.clone(),
                    actor: batch.actor.clone(),
                });
                if !slot.knowledge_item_ids.contains(&applied.item_id) {
                    slot.knowledge_item_ids.push(applied.item_id.clone());
                }
            }
            if !attributed {
                outcome.failures.push(format!(
                    "candidate {} for `{}` cites no event in its capture batch",
                    candidate.id, batch.session_id
                ));
            }
        }
        for event in &batch.events {
            if !placed.contains_key(&event.key) {
                outcome.failures.push(format!(
                    "seed key `{}` produced no applied Knowledge item, so no question can be graded on it",
                    event.key
                ));
            }
        }
        if let Some(scope_id) = target {
            outcome.publications.push(format!(
                "{} ({}) → {} as governed Knowledge",
                batch.session_id, batch.visibility, scope_id
            ));
        }
    }
    Ok(placed)
}

/// Wait until every graded Knowledge item is queryable. This is a readiness
/// check rather than the measurement: it asks each item's own seeded text
/// through the non-budgeted session-scoped Knowledge query, never the graded
/// question or a ContextRun. The author performs the check so publication
/// policy for another reader cannot be mistaken for indexing lag.
async fn wait_for_search(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    placed: &BTreeMap<String, Placed>,
    options: &Options,
    outcome: &mut QaOutcome,
) -> Result<(), String> {
    // Only what a task-carrying question will ask for: a taskless probe
    // takes no retrieval leg at all, so its material needs no search wait.
    let wanted: BTreeSet<&str> = corpus
        .questions
        .iter()
        .filter(|question| question.task.is_some())
        .flat_map(|question| question.expect_knowledge.iter().map(String::as_str))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let started = Instant::now();
    let mut pending: Vec<&str> = wanted.into_iter().collect();
    pending.sort_unstable();
    loop {
        let mut still = Vec::new();
        for key in pending {
            let Some(slot) = placed.get(key) else {
                continue;
            };
            let author = &environment.actor(&slot.actor)?.token;
            let query_session = client
                .session_for(author, &format!("eval:qa:query:{}", corpus.corpus))
                .await?;
            let found = client
                .knowledge_query(
                    author,
                    &query_session,
                    &KnowledgeQueryRequest {
                        query: &slot.text,
                        limit: SWEEP_LIMIT,
                    },
                )
                .await?;
            let retrievable = found
                .value
                .entries
                .iter()
                .any(|entry| slot.knowledge_item_ids.contains(&entry.knowledge_item_id));
            if !retrievable {
                still.push(key);
            }
        }
        if still.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "{} seeded Knowledge item(s) never became queryable to their own author within {}s: \
                 {} — every question that asks for them would measure readiness rather than \
                 selection",
                still.len(),
                options.seed_timeout.as_secs(),
                still.join(", ")
            ));
            return Ok(());
        }
        pending = still;
        tokio::time::sleep(POLL).await;
    }
}

/// How many current Knowledge items the reader can enumerate after every
/// governed publication. The diagnostic lens, rather than a ContextRun,
/// supplies the denominator that proves whether a budget actually bound.
async fn visible_total(client: &Client, reader: &str, corpus: &Corpus) -> Result<usize, String> {
    let as_of = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let swept = client
        .knowledge_sweep(
            reader,
            &KnowledgeSweepRequest {
                as_of: &as_of,
                session_id: &format!("eval:qa:served:{}", corpus.corpus),
                limit: SWEEP_LIMIT,
            },
        )
        .await?;
    Ok(swept.value.entries.len())
}

fn skipped(question: &Question) -> QuestionOutcome {
    QuestionOutcome {
        name: question.name.clone(),
        note: question.note.clone(),
        needs: question.needs.clone(),
        task: question.task.clone(),
        skipped: true,
        passed: false,
        per_scope: BTreeMap::new(),
        missing: Vec::new(),
        selected_knowledge: 0,
        relevant_knowledge: 0,
        bound: false,
        explicit_budget: question.budget_tokens.is_some(),
        tokens: 0,
        budget_tokens: 0,
        reference_tokens: 0,
        block_hash: String::new(),
        latency_ms: 0.0,
        degraded: Vec::new(),
        failures: Vec::new(),
    }
}

/// Grades one block against one question, by Knowledge identity throughout.
fn grade(
    corpus: &Corpus,
    question: &Question,
    placed: &BTreeMap<String, Placed>,
    probe: &crate::client::Timed<crate::client::ContextRunResponse>,
    reference: &tiktoken_rs::CoreBPE,
    served_knowledge: usize,
) -> QuestionOutcome {
    let block = &probe.value;
    let mut failures = Vec::new();
    let mut per_scope: BTreeMap<String, ScopeCounts> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut relevant = 0usize;

    for key in &question.expect_knowledge {
        let visibility = corpus
            .batch_of(key)
            .map_or("principal", |batch| batch.visibility.as_str());
        let counts = per_scope.entry(visibility.to_owned()).or_default();
        counts.expected += 1;

        let Some(slot) = placed.get(key) else {
            missing.push(key.clone());
            failures.push(format!(
                "expected Knowledge `{key}` has no item id, so the block could not select it"
            ));
            continue;
        };
        // A seed event can become more than one Knowledge item; the material
        // reached the reader if any of them was selected.
        let selected = slot
            .knowledge_item_ids
            .iter()
            .filter(|item| block.knowledge_item_ids.contains(item))
            .count();
        if selected == 0 {
            missing.push(key.clone());
            failures.push(format!("expected Knowledge `{key}` was not selected"));
            continue;
        }
        relevant += selected;
        counts.selected += 1;
    }

    for phrase in &question.must_not_contain {
        if block.text.contains(phrase) {
            failures.push(format!("the block leaked `{phrase}`"));
        }
    }
    // The invariant every question gets for nothing: a block may narrow a
    // requested budget and may never widen it (ADR-0026 decision 7).
    if let Some(requested) = question.budget_tokens
        && block.tokens > requested
    {
        failures.push(format!(
            "the block spent {} tokens against a requested budget of {requested}",
            block.tokens
        ));
    }

    QuestionOutcome {
        name: question.name.clone(),
        note: question.note.clone(),
        needs: question.needs.clone(),
        task: question.task.clone(),
        skipped: false,
        passed: failures.is_empty(),
        per_scope,
        missing,
        selected_knowledge: block.knowledge_item_ids.len(),
        relevant_knowledge: relevant,
        bound: block.knowledge_item_ids.len() < served_knowledge,
        explicit_budget: question.budget_tokens.is_some(),
        tokens: block.tokens,
        budget_tokens: block.budget_tokens,
        reference_tokens: reference.encode_ordinary(&block.text).len(),
        block_hash: block.block_hash.clone(),
        latency_ms: round(probe.elapsed_ms),
        degraded: probe.degraded.clone(),
        failures,
    }
}

/// The Q&A axes, reduced over every corpus.
///
/// Rates are over expected Knowledge items rather than questions, and over the whole
/// suite rather than per corpus averaged: a visibility with two expectations in
/// one file and eight in another should weigh by what it measured, not by
/// which file it sat in (the EVAL-2 rule).
pub fn metrics(outcomes: &[QaOutcome]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if outcomes.is_empty() {
        return metrics;
    }

    let measured = || {
        outcomes
            .iter()
            .flat_map(|outcome| outcome.questions.iter())
            .filter(|question| !question.skipped)
    };

    let mut totals: BTreeMap<&str, ScopeCounts> = BTreeMap::new();
    for question in measured() {
        for (visibility, counts) in &question.per_scope {
            let slot = totals.entry(visibility.as_str()).or_default();
            slot.expected += counts.expected;
            slot.selected += counts.selected;
        }
    }

    let mut whole = ScopeCounts::default();
    for visibility in VISIBILITIES {
        let Some(counts) = totals.get(visibility) else {
            continue;
        };
        whole.expected += counts.expected;
        whole.selected += counts.selected;
        if counts.expected > 0 {
            metrics.insert(
                format!("qa_scope_{visibility}"),
                round(counts.selected as f64 / counts.expected as f64),
            );
        }
    }
    if whole.expected == 0 {
        // Nothing was measured, so nothing is reported. A zero here would
        // read as "the product answered nothing" when what happened is
        // that nothing was asked.
        return metrics;
    }
    metrics.insert(
        "qa_selection_rate".to_owned(),
        round(whole.selected as f64 / whole.expected as f64),
    );

    // The exchange rate a composition change actually moves: tokens spent
    // per expected Knowledge item selected. `tokens_mean` moves for reasons
    // nobody can attribute; this one moves when a budget narrows, a
    // channel rule closes, a demotion threshold shifts, or ranking gets
    // worse (decision 8).
    let tokens: u32 = measured().map(|question| question.tokens).sum();
    if whole.selected > 0 {
        metrics.insert(
            "tokens_per_answer".to_owned(),
            round(f64::from(tokens) / whole.selected as f64),
        );
    }

    // Precision over the questions that carry a task **and** whose budget
    // actually bound the block. A taskless probe takes no retrieval leg
    // at all, and a block that carried everything the reader is served
    // made no ranking decision to measure — counted there, this axis
    // reports corpus size under the name of precision and moves whenever
    // a fixture is added.
    let ranked: Vec<&QuestionOutcome> = measured()
        .filter(|question| question.task.is_some() && question.explicit_budget && question.bound)
        .collect();
    let carried: usize = ranked
        .iter()
        .map(|question| question.selected_knowledge)
        .sum();
    if carried > 0 {
        let relevant: usize = ranked
            .iter()
            .map(|question| question.relevant_knowledge)
            .sum();
        metrics.insert(
            "retrieval_precision".to_owned(),
            round(relevant as f64 / carried as f64),
        );
    }

    // CTX-2's estimator, measured rather than trusted (ADR-0025's parked
    // obligation). Reported and gated by nothing: the reference vocabulary
    // is not the product's consumer, which is ADR-0025 option 2's own
    // objection kept rather than argued with.
    let mut biases: Vec<f64> = measured()
        .filter(|question| question.reference_tokens > 0)
        .map(|question| f64::from(question.tokens) / question.reference_tokens as f64)
        .collect();
    if !biases.is_empty() {
        biases.sort_by(f64::total_cmp);
        metrics.insert(
            "estimator_bias_p95".to_owned(),
            round(percentile(&biases, 95.0)),
        );
    }

    metrics
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

use crate::report::percentile;

#[cfg(test)]
mod tests {
    use super::*;

    fn question(name: &str, task: bool, per_scope: &[(&str, usize, usize)]) -> QuestionOutcome {
        QuestionOutcome {
            name: name.to_owned(),
            note: String::new(),
            needs: "lexical".to_owned(),
            task: task.then(|| "a task".to_owned()),
            skipped: false,
            passed: true,
            per_scope: per_scope
                .iter()
                .map(|(visibility, expected, selected)| {
                    (
                        (*visibility).to_owned(),
                        ScopeCounts {
                            expected: *expected,
                            selected: *selected,
                        },
                    )
                })
                .collect(),
            missing: Vec::new(),
            selected_knowledge: 0,
            relevant_knowledge: 0,
            bound: task,
            explicit_budget: task,
            tokens: 0,
            budget_tokens: 1500,
            reference_tokens: 0,
            block_hash: "b3-test".to_owned(),
            latency_ms: 1.0,
            degraded: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn corpus(questions: Vec<QuestionOutcome>) -> QaOutcome {
        QaOutcome {
            questions,
            ..QaOutcome::default()
        }
    }

    #[test]
    fn rates_reduce_over_knowledge_and_placements_are_their_own_axes() {
        let metrics = metrics(&[corpus(vec![
            question("a", true, &[("project", 2, 2)]),
            question("b", true, &[("workspace", 2, 1), ("tenant", 1, 1)]),
        ])]);
        assert_eq!(metrics.get("qa_scope_project"), Some(&1.0));
        assert_eq!(metrics.get("qa_scope_workspace"), Some(&0.5));
        assert_eq!(metrics.get("qa_scope_tenant"), Some(&1.0));
        assert_eq!(metrics.get("qa_selection_rate"), Some(&0.8));
        // A visibility nothing exercised is absent rather than zero.
        assert!(!metrics.contains_key("qa_scope_principal"));
    }

    #[test]
    fn a_skipped_question_is_counted_and_never_scored() {
        // The whole point of decision 5: a question the configured path
        // cannot answer must not drag an axis down, because no code
        // change would fix it.
        let mut skipped = question("semantic", true, &[("project", 2, 0)]);
        skipped.skipped = true;
        skipped.needs = "semantic".to_owned();
        let metrics = metrics(&[corpus(vec![
            question("lexical", true, &[("project", 2, 2)]),
            skipped,
        ])]);
        assert_eq!(
            metrics.get("qa_selection_rate"),
            Some(&1.0),
            "the skipped question's expectations are not in the denominator"
        );
        assert_eq!(metrics.get("qa_scope_project"), Some(&1.0));
    }

    #[test]
    fn tokens_per_answer_is_the_exchange_rate_and_absent_without_a_numerator() {
        let mut spent = question("a", true, &[("project", 2, 2)]);
        spent.tokens = 300;
        assert_eq!(
            metrics(&[corpus(vec![spent])]).get("tokens_per_answer"),
            Some(&150.0)
        );

        // Nothing selected: absent rather than a division by zero
        // dressed up as a number.
        let mut nothing = question("a", true, &[("project", 2, 0)]);
        nothing.tokens = 300;
        let metrics = metrics(&[corpus(vec![nothing])]);
        assert!(!metrics.contains_key("tokens_per_answer"));
        assert_eq!(metrics.get("qa_selection_rate"), Some(&0.0));
    }

    #[test]
    fn precision_reads_only_the_questions_that_rank() {
        let mut ranked = question("ranked", true, &[("project", 1, 1)]);
        ranked.selected_knowledge = 4;
        ranked.relevant_knowledge = 1;
        let mut taskless = question("taskless", false, &[("principal", 1, 1)]);
        taskless.selected_knowledge = 100;
        taskless.relevant_knowledge = 1;
        // A block that carried everything the reader is served made no
        // ranking decision, so it reports corpus size rather than
        // precision and is out of the denominator too.
        let mut unbounded = question("unbounded", true, &[("tenant", 1, 1)]);
        unbounded.bound = false;
        unbounded.selected_knowledge = 100;
        unbounded.relevant_knowledge = 1;

        let metrics = metrics(&[corpus(vec![ranked, taskless, unbounded])]);
        assert_eq!(
            metrics.get("retrieval_precision"),
            Some(&0.25),
            "only the block whose budget bound it decides this axis"
        );
    }

    #[test]
    fn the_estimator_bias_reports_from_what_was_served() {
        let mut question = question("a", true, &[("project", 1, 1)]);
        question.tokens = 120;
        question.reference_tokens = 100;
        let metrics = metrics(&[corpus(vec![question])]);
        assert_eq!(metrics.get("estimator_bias_p95"), Some(&1.2));
    }

    #[test]
    fn a_suite_that_measured_nothing_reports_nothing() {
        let mut skipped = question("semantic", true, &[("project", 2, 0)]);
        skipped.skipped = true;
        let metrics = metrics(&[corpus(vec![skipped])]);
        assert!(metrics.is_empty(), "reported: {metrics:?}");
    }
}
