//! Seed → wait → promote → ask → grade, one Q&A corpus at a time
//! (EVAL-4, ADR-0047).
//!
//! The lens is the inject block, which is the inverse of EVAL-2's choice
//! for the same reason. ADR-0046 rejected the block because it is
//! budget-bounded, relevance-ranked and elides what CTX-4 demotes; those
//! three properties are exactly what this suite measures, so here absence
//! *is* the signal.
//!
//! Grading joins seed to block by **record identity** and never by
//! containment (decision 2): observe's `event_id` → the sweep's
//! `provenance.event_id` → `record_id` → its position in the block's
//! `record_ids` → `tiers[i]`. Containment could not do this job at all —
//! an index entry carries the body truncated at `index_entry_chars`
//! (ADR-0041 decision 3), so "demoted" and "absent" would be one
//! measurement.
//!
//! Nothing here writes above a leaf by any route but review. Material at
//! a team, a department or the org got there through `POST /v1/proposals`
//! and this level's own approvers, because observe lands records at the
//! caller's home scope (ADR-0020) and a service identity's home is a
//! `principal`-shaped scope under its anchor (ADR-0018 decision 2).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::client::{
    Client, InjectRequest, ObserveEvent, ObserveRequest, ProposalRequest, RecallQueryRequest,
    RecallSweepRequest,
};
use crate::extraction::{AUDITOR_ACTOR, read_committed};
use crate::qa::{Corpus, Question, TIERS};
use crate::report::{QaOutcome, QuestionOutcome, TierCounts};
use crate::scenario::Environment;

/// The reviewers every promotion goes through. Fixed names rather than
/// corpus fields: who may approve is the pack's answer at the target
/// scope, and a corpus that named its own approvers would be describing
/// the approval matrix instead of exercising it.
pub const CURATOR_ACTOR: &str = "qa-curator";
pub const STEWARD_ACTOR: &str = "qa-steward";

/// What a sweep asks for, matching EVAL-2's: the surface caps a sweep
/// here, so asking for exactly it and receiving exactly it is the
/// ambiguity ADR-0046 decision 3 refuses to measure through.
const SWEEP_LIMIT: usize = 32;

/// How many approvals to try before giving up on a proposal. The pack
/// asks for at most two distinct approvers on any memory publication
/// (the FLOW-3 matrix golden); a third pass means the requirement is
/// something this harness does not understand, and looping would hide
/// that.
const MAX_APPROVALS: usize = 3;

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
    let auditor = &environment
        .actors
        .get(AUDITOR_ACTOR)
        .ok_or_else(|| {
            format!(
                "the environment names no `{AUDITOR_ACTOR}` actor; the Q&A suite waits on \
                 `GET /v1/audit/events` for the pipeline to be done with every seeded event"
            )
        })?
        .token;

    let started = Instant::now();
    let seeded = seed(client, environment, corpus).await?;
    wait_for_pipeline(client, auditor, &seeded, options, &mut outcome).await?;
    let placed = locate(client, environment, corpus, &seeded, &mut outcome).await?;
    wait_for_index(client, environment, corpus, &placed, options, &mut outcome).await?;
    promote(client, environment, corpus, &placed, &mut outcome).await?;
    outcome.served_records = served_total(client, reader, corpus).await?;
    outcome.seed_wait_ms = round(started.elapsed().as_secs_f64() * 1000.0);

    let reference =
        tiktoken_rs::o200k_base().map_err(|err| format!("load the reference tokenizer: {err}"))?;

    for question in &corpus.questions {
        if question.is_semantic() && !options.dense_retrieval {
            outcome.skipped_semantic += 1;
            outcome.questions.push(skipped(question));
            continue;
        }
        let probe = client
            .inject(
                reader,
                &InjectRequest {
                    task: question.task.as_deref(),
                    session_id: &format!("eval:qa:{}:{}", corpus.corpus, question.name),
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
            outcome.served_records,
        ));
    }

    outcome.passed = outcome.failures.is_empty()
        && outcome
            .questions
            .iter()
            .all(|question| question.passed || question.skipped);
    Ok(outcome)
}

/// One seeded event, and where its records ended up.
struct Seeded {
    key: String,
    event_id: String,
    /// The batch's index, so a promotion knows which records are its own.
    batch: usize,
}

/// A seed key's records, once the sweep has found them.
#[derive(Default)]
struct Placed {
    record_ids: Vec<String>,
    /// The author's own leaf: a climb's `source_scope_id`.
    scope_id: String,
    /// The seeded text, used as its own retrieval query while waiting for
    /// the sparse index — an exact readiness condition, and deliberately
    /// not the question's own task (see `wait_for_index`).
    text: String,
    /// The actor that wrote it. The readiness check asks *this* identity
    /// rather than the reader, for the reason `wait_for_index` gives.
    actor: String,
}

async fn seed(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
) -> Result<Vec<Seeded>, String> {
    let mut seeded = Vec::new();
    for (index, batch) in corpus.seed.iter().enumerate() {
        let bearer = &environment.actor(&batch.actor)?.token;
        let occurred_at = chrono::Utc::now().to_rfc3339();
        let events: Vec<ObserveEvent<'_>> = batch
            .events
            .iter()
            .map(|event| ObserveEvent {
                idempotency_key: format!("{}:{}", batch.session_id, event.key),
                kind: &event.kind,
                payload: serde_json::json!({ "text": event.text }),
                occurred_at: occurred_at.clone(),
            })
            .collect();
        let response = client
            .observe(
                bearer,
                &ObserveRequest {
                    session_id: &batch.session_id,
                    events,
                },
            )
            .await?;
        let acked = &response.value;
        if acked.denied > 0 || acked.quarantined > 0 {
            return Err(format!(
                "seeding `{}` was withheld: {} denied, {} quarantined — the corpus is \
                 documentation-only content and should trip neither",
                batch.session_id, acked.denied, acked.quarantined
            ));
        }
        for event in &batch.events {
            let key = format!("{}:{}", batch.session_id, event.key);
            let event_id = acked
                .events
                .iter()
                .find(|entry| entry.idempotency_key == key)
                .and_then(|entry| entry.event_id.clone())
                .ok_or_else(|| {
                    format!(
                        "seeding `{}` acked no event id, so nothing downstream can be attributed \
                         to it",
                        event.key
                    )
                })?;
            seeded.push(Seeded {
                key: event.key.clone(),
                event_id,
                batch: index,
            });
        }
    }
    Ok(seeded)
}

/// Waits for the pipeline to be *done*, which the chain states exactly:
/// every seeded event appears in a `memory.extracted` payload whether it
/// produced records or not. Polling the sweep instead would be waiting for
/// an unknown number of records to appear, which is downstream of the
/// thing under measurement (the EVAL-2 rule).
async fn wait_for_pipeline(
    client: &Client,
    auditor: &str,
    seeded: &[Seeded],
    options: &Options,
    outcome: &mut QaOutcome,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let committed = read_committed(client, auditor).await?;
        let missing: Vec<&str> = seeded
            .iter()
            .filter(|entry| !committed.contains_key(entry.event_id.as_str()))
            .map(|entry| entry.key.as_str())
            .collect();
        if missing.is_empty() {
            for entry in seeded {
                if committed
                    .get(entry.event_id.as_str())
                    .is_some_and(|committed| committed.dead_lettered)
                {
                    outcome.failures.push(format!(
                        "the pipeline dead-lettered `{}`: the event was lost rather than found \
                         empty, and grading its questions would blame the corpus for a broken \
                         pipeline",
                        entry.key
                    ));
                }
            }
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "the pipeline never finished with {} event(s) within {}s: {}",
                missing.len(),
                options.seed_timeout.as_secs(),
                missing.join(", ")
            ));
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Finds every seeded event's records by sweeping as each *author*, which
/// is the only caller that can see its own leaf: the privacy floor closes
/// another principal's personal scope to everyone (ADR-0037/ADR-0038), and
/// that is exactly what makes a promotion mean something later.
async fn locate(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    seeded: &[Seeded],
    outcome: &mut QaOutcome,
) -> Result<BTreeMap<String, Placed>, String> {
    let mut placed: BTreeMap<String, Placed> = BTreeMap::new();
    let by_event: BTreeMap<&str, &Seeded> = seeded
        .iter()
        .map(|entry| (entry.event_id.as_str(), entry))
        .collect();

    // The instant is deliberately ahead of now, for ADR-0046 decision 1's
    // reason: the surface reads `as_of < now` as a rewind, and a rewind
    // applies no retention horizon, so a client's own `now` measures the
    // historical read rather than the live one.
    let as_of = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let authors: BTreeSet<&str> = corpus
        .seed
        .iter()
        .map(|batch| batch.actor.as_str())
        .collect();

    for author in authors {
        let bearer = &environment.actor(author)?.token;
        let swept = client
            .recall_sweep(
                bearer,
                &RecallSweepRequest {
                    as_of: &as_of,
                    session_id: &format!("eval:qa:locate:{}", corpus.corpus),
                    limit: SWEEP_LIMIT,
                },
            )
            .await?;
        let sweep = swept.value;
        if sweep.mode != "sweep" {
            outcome.failures.push(format!(
                "the surface answered `{author}`'s enumeration in `{}` mode, not `sweep`: this is \
                 a measurement of a different question",
                sweep.mode
            ));
        }
        if sweep.truncated {
            outcome.failures.push(format!(
                "`{author}`'s sweep truncated its scope universe at {} of {} scopes; where the \
                 corpus lives cannot be established from a bounded answer",
                sweep.scopes_decided, sweep.scopes_considered
            ));
        }
        if sweep.entries.len() >= SWEEP_LIMIT {
            outcome.failures.push(format!(
                "`{author}`'s sweep returned {} records against a requested limit of \
                 {SWEEP_LIMIT}: a full page and a truncated one are indistinguishable from here, \
                 so this corpus cannot be located — split it across more actors rather than \
                 raising the limit",
                sweep.entries.len()
            ));
        }

        for entry in &sweep.entries {
            let Some(event_id) = entry.source_event_id() else {
                continue;
            };
            let Some(seed) = by_event.get(event_id) else {
                // Another corpus's material in the same actor's scope. Not
                // a leak — the corpora share a stack — and not this
                // corpus's to grade either.
                continue;
            };
            let batch = &corpus.seed[seed.batch];
            let slot = placed.entry(seed.key.clone()).or_insert_with(|| Placed {
                record_ids: Vec::new(),
                scope_id: entry.scope_id.clone(),
                text: batch
                    .events
                    .iter()
                    .find(|event| event.key == seed.key)
                    .map(|event| event.text.clone())
                    .unwrap_or_default(),
                actor: batch.actor.clone(),
            });
            // Records that a climb has to carry together must live
            // together; a split would make `source_scope_id` a guess.
            if slot.scope_id != entry.scope_id {
                outcome.failures.push(format!(
                    "seed key `{}` produced records at two scopes ({} and {}), so a promotion \
                     cannot name where its material is",
                    seed.key, slot.scope_id, entry.scope_id
                ));
            }
            slot.record_ids.push(entry.record_id.clone());
        }
    }

    for entry in seeded {
        if !placed.contains_key(&entry.key) {
            outcome.failures.push(format!(
                "seed key `{}` produced no record its own author can see, so no question can be \
                 graded on it",
                entry.key
            ));
        }
    }
    Ok(placed)
}

/// Climbs every batch that declares a target, through the product's own
/// review: open, approve until the pack says nothing is outstanding, then
/// run the effect. This is the only way material reaches a scope above a
/// leaf, and it is why a per-scope answer rate asserts FLOW-5 as well as
/// CTX-2 (decision 3).
async fn promote(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    placed: &BTreeMap<String, Placed>,
    outcome: &mut QaOutcome,
) -> Result<(), String> {
    let curator = &environment.actor(CURATOR_ACTOR)?.token;
    let steward = &environment.actor(STEWARD_ACTOR)?.token;

    for batch in &corpus.seed {
        let Some(target) = &batch.promote_to else {
            continue;
        };
        let scope_id = environment.scope(target)?;
        let author = &environment.actor(&batch.actor)?.token;

        let mut record_ids = Vec::new();
        let mut source = String::new();
        for event in &batch.events {
            let Some(slot) = placed.get(&event.key) else {
                continue;
            };
            source.clone_from(&slot.scope_id);
            record_ids.extend(slot.record_ids.iter().cloned());
        }
        if record_ids.is_empty() {
            outcome.failures.push(format!(
                "batch `{}` has nothing to promote to `{target}`: none of its events produced a \
                 record",
                batch.session_id
            ));
            continue;
        }

        let opened = client
            .propose(
                author,
                &ProposalRequest {
                    scope_id,
                    source_scope_id: &source,
                    record_ids,
                    title: format!("eval: {} to {target}", corpus.corpus),
                    effect: None,
                    sensitivity: None,
                },
            )
            .await?;
        let proposal = opened.value;

        // Approve until the pack says nothing is outstanding. Which roles
        // and how many distinct approvers is the target scope's answer
        // (ADR-0032), so the harness reads the requirement rather than
        // restating it: a pack that asks for a different set is followed.
        let mut state = proposal.state.clone();
        let mut approvals = 0;
        while state == "open" && approvals < MAX_APPROVALS {
            let bearer = if approvals == 0 { curator } else { steward };
            let voted = client.approve(bearer, &proposal.id).await?;
            state.clone_from(&voted.value.state);
            approvals += 1;
            if state == "open" && voted.value.outstanding.is_empty() {
                outcome.failures.push(format!(
                    "the climb to `{target}` stayed open and the surface named nothing \
                     outstanding, so this harness cannot tell what it still needs"
                ));
                break;
            }
        }
        if state != "approved" {
            outcome.failures.push(format!(
                "the climb to `{target}` ended `{state}` after {approvals} approval(s) rather \
                 than approved, so its tier has no material and every question about it would \
                 measure the review rather than the composition"
            ));
            continue;
        }

        // The curator runs the effect, never the steward: publishing takes
        // `MemoryRead` too, and nobody publishes what they cannot read
        // (ADR-0031 decision 12).
        let published = client.publish(curator, &proposal.id).await?;
        // Checked rather than assumed: a publication that landed
        // somewhere else would put the material outside the reader's
        // chain, and every question about that tier would then measure a
        // misdirected climb while reporting a composition number.
        if published.value.scope_id != scope_id {
            outcome.failures.push(format!(
                "the climb to `{target}` published at scope {} rather than {scope_id}",
                published.value.scope_id
            ));
        }
        outcome.promotions.push(format!(
            "{} ({}) → {} : {} record(s) at commit {}",
            batch.session_id,
            batch.tier,
            proposal.target_scope_path,
            published.value.added,
            published.value.commit.chars().take(12).collect::<String>()
        ));
    }
    Ok(())
}

/// Waits until the corpus is *retrievable*, not merely served: the sparse
/// leg is a sidecar that sweeps on a timer (ADR-0024), so a record is
/// composable by recency seconds before it is rankable, and grading a
/// task-carrying question in that window would measure the sweep.
///
/// Three things make this a readiness check rather than the measurement,
/// and each one was learned by getting it wrong first.
///
/// It asks each record's **own seeded text**, which is exact — "is this
/// record in the index" — and never a question's task, which is a
/// paraphrase or a different phrasing and whether it ranks is the thing
/// being graded (the EVAL-2 rule about waiting on the chain rather than
/// on the sweep, one layer out).
///
/// It asks through `POST /v1/recall`'s query form rather than through a
/// block. Recall ranks with no composition budget and no scope gradient,
/// so "indexed" cannot be confused with "did not fit" — where an inject
/// probe becomes unsatisfiable the moment a pack narrows the budget below
/// what the far end of the chain needs, and the wait then burns its whole
/// timeout and reports an indexing failure for what is a composition
/// change.
///
/// And it asks as each record's **author**, before any climb, rather than
/// as the reader. A promotion publishes a channel that *names* a record
/// at its current address (ADR-0034 decision 3); the record itself stays
/// on its author's leaf. So a reader composes promoted material through
/// the published channel but a query-shaped recall, which searches the
/// scopes the caller may read, does not reach it. The author always can,
/// and the sparse index is one per tenant (ADR-0024 decision 3) — so
/// readiness established for the author is readiness full stop.
async fn wait_for_index(
    client: &Client,
    environment: &Environment,
    corpus: &Corpus,
    placed: &BTreeMap<String, Placed>,
    options: &Options,
    outcome: &mut QaOutcome,
) -> Result<(), String> {
    // Only what a task-carrying question will ask for: a taskless probe
    // takes no retrieval leg at all, so its material needs no index.
    let wanted: BTreeSet<&str> = corpus
        .questions
        .iter()
        .filter(|question| question.task.is_some())
        .flat_map(|question| question.expect_records.iter().map(String::as_str))
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
            let found = client
                .recall_query(
                    author,
                    &RecallQueryRequest {
                        query: &slot.text,
                        session_id: &format!("eval:qa:index:{}", corpus.corpus),
                        limit: SWEEP_LIMIT,
                    },
                )
                .await?;
            let indexed = found
                .value
                .entries
                .iter()
                .any(|entry| slot.record_ids.contains(&entry.record_id));
            if !indexed {
                still.push(key);
            }
        }
        if still.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "{} seeded record(s) never became retrievable to their own author within {}s: \
                 {} — every question that asks for them measures the index rather than the \
                 composition",
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

/// How many records the reader is served in total, once every climb has
/// landed. A sweep rather than a block, because a block is the thing under
/// measurement and a sweep is the enumerator (ADR-0046 decision 1) — this
/// is the denominator that says whether something bound a block or the
/// whole corpus simply fitted.
async fn served_total(client: &Client, reader: &str, corpus: &Corpus) -> Result<usize, String> {
    let as_of = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let swept = client
        .recall_sweep(
            reader,
            &RecallSweepRequest {
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
        per_tier: BTreeMap::new(),
        demoted: Vec::new(),
        missing: Vec::new(),
        block_records: 0,
        relevant_records: 0,
        index_entries: 0,
        index_tokens: 0,
        bound: false,
        tokens: 0,
        budget_tokens: 0,
        reference_tokens: 0,
        block_hash: String::new(),
        latency_ms: 0.0,
        staleness_permille: Vec::new(),
        degraded: Vec::new(),
        failures: Vec::new(),
    }
}

/// Grades one block against one question, by record identity throughout.
fn grade(
    corpus: &Corpus,
    question: &Question,
    placed: &BTreeMap<String, Placed>,
    probe: &crate::client::Timed<crate::client::InjectResponse>,
    reference: &tiktoken_rs::CoreBPE,
    served_records: usize,
) -> QuestionOutcome {
    let block = &probe.value;
    let mut failures = Vec::new();
    let mut per_tier: BTreeMap<String, TierCounts> = BTreeMap::new();
    let mut demoted = Vec::new();
    let mut missing = Vec::new();
    let mut relevant = 0usize;

    for key in &question.expect_records {
        let tier = corpus
            .batch_of(key)
            .map_or("user", |batch| batch.tier.as_str());
        let counts = per_tier.entry(tier.to_owned()).or_default();
        counts.expected += 1;

        let Some(slot) = placed.get(key) else {
            missing.push(key.clone());
            failures.push(format!(
                "expected record `{key}` has no record id, so the block could not have carried it"
            ));
            continue;
        };
        // A seed event can become more than one record; the material
        // reached the reader if any of them did.
        let carried: Vec<&str> = slot
            .record_ids
            .iter()
            .filter_map(|record| block.tier_of(record))
            .collect();
        if carried.is_empty() {
            missing.push(key.clone());
            failures.push(format!("expected record `{key}` never reached the block"));
            continue;
        }
        relevant += carried.len();
        counts.reached += 1;
        if carried.contains(&"body") {
            counts.body += 1;
        } else {
            // Named but not carried. Not a failure on its own — the index
            // line exists precisely so the reader knows it is there and
            // can recall it — and the gap between reach and body is the
            // number ADR-0041 parked here.
            demoted.push(key.clone());
        }
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
        per_tier,
        demoted,
        missing,
        block_records: block.record_ids.len(),
        relevant_records: relevant,
        index_entries: block.index_entries,
        index_tokens: block.index_tokens,
        bound: block.record_ids.len() < served_records,
        tokens: block.tokens,
        budget_tokens: block.budget_tokens,
        reference_tokens: reference.encode_ordinary(&block.text).len(),
        block_hash: block.block_hash.clone(),
        latency_ms: round(probe.elapsed_ms),
        staleness_permille: block.staleness_permille.clone(),
        degraded: probe.degraded.clone(),
        failures,
    }
}

/// The Q&A axes, reduced over every corpus.
///
/// Rates are over *records* rather than over questions, and over the whole
/// suite rather than per corpus averaged: a tier with two expectations in
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

    let mut totals: BTreeMap<&str, TierCounts> = BTreeMap::new();
    for question in measured() {
        for (tier, counts) in &question.per_tier {
            let slot = totals.entry(tier.as_str()).or_default();
            slot.expected += counts.expected;
            slot.reached += counts.reached;
            slot.body += counts.body;
        }
    }

    let mut whole = TierCounts::default();
    for tier in TIERS {
        let Some(counts) = totals.get(tier) else {
            continue;
        };
        whole.expected += counts.expected;
        whole.reached += counts.reached;
        whole.body += counts.body;
        if counts.expected > 0 {
            metrics.insert(
                format!("qa_scope_{tier}"),
                round(counts.reached as f64 / counts.expected as f64),
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
        "qa_answer_rate".to_owned(),
        round(whole.reached as f64 / whole.expected as f64),
    );
    metrics.insert(
        "qa_body_rate".to_owned(),
        round(whole.body as f64 / whole.expected as f64),
    );

    // The exchange rate a composition change actually moves: tokens spent
    // per expected record carried whole. `tokens_mean` moves for reasons
    // nobody can attribute; this one moves when a budget narrows, a
    // channel rule closes, a demotion threshold shifts, or ranking gets
    // worse (decision 8).
    let tokens: u32 = measured().map(|question| question.tokens).sum();
    if whole.body > 0 {
        metrics.insert(
            "tokens_per_answer".to_owned(),
            round(f64::from(tokens) / whole.body as f64),
        );
    }

    // Precision over the questions that carry a task **and** whose budget
    // actually bound the block. A taskless probe takes no retrieval leg
    // at all, and a block that carried everything the reader is served
    // made no ranking decision to measure — counted there, this axis
    // reports corpus size under the name of precision and moves whenever
    // a fixture is added.
    let ranked: Vec<&QuestionOutcome> = measured()
        .filter(|question| question.task.is_some() && question.bound)
        .collect();
    let carried: usize = ranked.iter().map(|question| question.block_records).sum();
    if carried > 0 {
        let relevant: usize = ranked
            .iter()
            .map(|question| question.relevant_records)
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

    // MEM-6's unvalidated heuristic, measured for the first time over what
    // a reader was actually served (ADR-0040's parked obligation).
    let mut staleness: Vec<f64> = measured()
        .flat_map(|question| {
            question
                .staleness_permille
                .iter()
                .map(|value| f64::from(*value))
        })
        .collect();
    if !staleness.is_empty() {
        staleness.sort_by(f64::total_cmp);
        metrics.insert(
            "staleness_p50_permille".to_owned(),
            round(percentile(&staleness, 50.0)),
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

    fn question(
        name: &str,
        task: bool,
        per_tier: &[(&str, usize, usize, usize)],
    ) -> QuestionOutcome {
        QuestionOutcome {
            name: name.to_owned(),
            note: String::new(),
            needs: "lexical".to_owned(),
            task: task.then(|| "a task".to_owned()),
            skipped: false,
            passed: true,
            per_tier: per_tier
                .iter()
                .map(|(tier, expected, reached, body)| {
                    (
                        (*tier).to_owned(),
                        TierCounts {
                            expected: *expected,
                            reached: *reached,
                            body: *body,
                        },
                    )
                })
                .collect(),
            demoted: Vec::new(),
            missing: Vec::new(),
            block_records: 0,
            relevant_records: 0,
            index_entries: 0,
            index_tokens: 0,
            bound: task,
            tokens: 0,
            budget_tokens: 1500,
            reference_tokens: 0,
            block_hash: "b3-test".to_owned(),
            latency_ms: 1.0,
            staleness_permille: Vec::new(),
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
    fn rates_reduce_over_records_and_the_tiers_are_their_own_axes() {
        let metrics = metrics(&[corpus(vec![
            question("a", true, &[("team", 2, 2, 2)]),
            question("b", true, &[("department", 2, 1, 0), ("org", 1, 1, 1)]),
        ])]);
        assert_eq!(metrics.get("qa_scope_team"), Some(&1.0));
        assert_eq!(metrics.get("qa_scope_department"), Some(&0.5));
        assert_eq!(metrics.get("qa_scope_org"), Some(&1.0));
        // 4 of 5 reached, 3 of 5 whole.
        assert_eq!(metrics.get("qa_answer_rate"), Some(&0.8));
        assert_eq!(metrics.get("qa_body_rate"), Some(&0.6));
        // A tier nothing exercised is absent rather than zero.
        assert!(!metrics.contains_key("qa_scope_user"));
    }

    /// The displacement number ADR-0041 parked here: reach holds while
    /// body falls, which is the index tier taking bodies that mattered.
    #[test]
    fn a_demotion_moves_the_body_rate_and_leaves_the_answer_rate_alone() {
        let whole = metrics(&[corpus(vec![question("a", true, &[("team", 4, 4, 4)])])]);
        let demoted = metrics(&[corpus(vec![question("a", true, &[("team", 4, 4, 1)])])]);
        assert_eq!(whole.get("qa_answer_rate"), demoted.get("qa_answer_rate"));
        assert_eq!(whole.get("qa_body_rate"), Some(&1.0));
        assert_eq!(demoted.get("qa_body_rate"), Some(&0.25));
    }

    #[test]
    fn a_skipped_question_is_counted_and_never_scored() {
        // The whole point of decision 5: a question the configured path
        // cannot answer must not drag an axis down, because no code
        // change would fix it.
        let mut skipped = question("semantic", true, &[("team", 2, 0, 0)]);
        skipped.skipped = true;
        skipped.needs = "semantic".to_owned();
        let metrics = metrics(&[corpus(vec![
            question("lexical", true, &[("team", 2, 2, 2)]),
            skipped,
        ])]);
        assert_eq!(
            metrics.get("qa_answer_rate"),
            Some(&1.0),
            "the skipped question's expectations are not in the denominator"
        );
        assert_eq!(metrics.get("qa_scope_team"), Some(&1.0));
    }

    #[test]
    fn tokens_per_answer_is_the_exchange_rate_and_absent_without_a_numerator() {
        let mut spent = question("a", true, &[("team", 2, 2, 2)]);
        spent.tokens = 300;
        assert_eq!(
            metrics(&[corpus(vec![spent])]).get("tokens_per_answer"),
            Some(&150.0)
        );

        // Nothing carried whole: absent rather than a division by zero
        // dressed up as a number.
        let mut nothing = question("a", true, &[("team", 2, 0, 0)]);
        nothing.tokens = 300;
        let metrics = metrics(&[corpus(vec![nothing])]);
        assert!(!metrics.contains_key("tokens_per_answer"));
        assert_eq!(metrics.get("qa_answer_rate"), Some(&0.0));
    }

    #[test]
    fn precision_reads_only_the_questions_that_rank() {
        let mut ranked = question("ranked", true, &[("team", 1, 1, 1)]);
        ranked.block_records = 4;
        ranked.relevant_records = 1;
        let mut taskless = question("taskless", false, &[("user", 1, 1, 1)]);
        taskless.block_records = 100;
        taskless.relevant_records = 1;
        // A block that carried everything the reader is served made no
        // ranking decision, so it reports corpus size rather than
        // precision and is out of the denominator too.
        let mut unbounded = question("unbounded", true, &[("org", 1, 1, 1)]);
        unbounded.bound = false;
        unbounded.block_records = 100;
        unbounded.relevant_records = 1;

        let metrics = metrics(&[corpus(vec![ranked, taskless, unbounded])]);
        assert_eq!(
            metrics.get("retrieval_precision"),
            Some(&0.25),
            "only the block whose budget bound it decides this axis"
        );
    }

    #[test]
    fn the_estimator_bias_and_staleness_axes_report_from_what_was_served() {
        let mut question = question("a", true, &[("team", 1, 1, 1)]);
        question.tokens = 120;
        question.reference_tokens = 100;
        question.staleness_permille = vec![1000, 500, 250];
        let metrics = metrics(&[corpus(vec![question])]);
        assert_eq!(metrics.get("estimator_bias_p95"), Some(&1.2));
        assert_eq!(metrics.get("staleness_p50_permille"), Some(&500.0));
    }

    #[test]
    fn a_suite_that_measured_nothing_reports_nothing() {
        let mut skipped = question("semantic", true, &[("team", 2, 0, 0)]);
        skipped.skipped = true;
        let metrics = metrics(&[corpus(vec![skipped])]);
        assert!(metrics.is_empty(), "reported: {metrics:?}");
    }
}
