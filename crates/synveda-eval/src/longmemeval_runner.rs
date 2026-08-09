//! Seed → wait → ask → grade, one LongMemEval instance at a time: the
//! deterministic retrieval tier (EVAL-3, ADR-0061 decision 5).
//!
//! This is the half of the benchmark Synveda is actually responsible for.
//! The question it grades has one right answer that is reproducible from
//! bytes — *did the block bind the evidence sessions the instance names in
//! `answer_session_ids`* — so it gates, against its own baseline, and it
//! reaches no model and costs nothing per run. The QA accuracy that
//! LongMemEval is better known for is the other tier: published, gated by
//! nothing, and dependent on two external models (decision 6).
//!
//! Grading joins by **session identity**, which is the only join this
//! corpus offers. Each haystack session is seeded under a session id this
//! harness owns, and the block's `record_ids` are handed back to
//! `POST /v1/recall` in its ids shape to read each record's
//! `provenance.session_id`. EVAL-4 does the same join through a sweep, and
//! a sweep cannot be used here for the reason ADR-0046 decision 3 gives
//! and ADR-0061 decision 8 restates: an instance is ~40 sessions against a
//! 32-record cap, and a full page and a truncated one are
//! indistinguishable from the client. The ids shape has no such ambiguity
//! — it answers about records the caller named — and it is chunked at the
//! cap rather than asked past it.
//!
//! Two measurement decisions the ADR left open, both recorded here because
//! both move numbers.
//!
//! **One event per turn, batched one call per session.** The first version
//! of this seeded a whole session as a single `transcript_delta`, on the
//! argument that a delta is a batch of turns. A live run refuted it:
//! `DeterministicExtractor` emits exactly one record per event and
//! truncates its content at `MAX_CONTENT_CHARS` (300), so a
//! forty-turn session arrives as three hundred characters and the evidence
//! turn is thrown away before retrieval is ever asked anything. A
//! benchmark run that way measures a truncation. Turns are the largest
//! unit the pipeline preserves whole, so turns are the unit — and they
//! ride in one observe call per session, so the call count is the same
//! either way. The session id still carries the join, which is what
//! `answer_session_ids` grades.
//!
//! **The corpus's clock is aligned to the run's, not replayed in 2023.**
//! Every timestamp is shifted by one offset per instance — the difference
//! between now and that instance's `question_date` — so the *relative*
//! structure the temporal-reasoning questions depend on is preserved
//! exactly while the material is not three years old on arrival. Seeding
//! it at its literal dates would put every record on the wrong side of
//! whatever retention horizon and staleness curve the pack configures
//! (MEM-6, ADR-0040), and the suite would then be measuring a retention
//! policy under the name of retrieval. The offset rides into the report,
//! because a shifted clock that nobody stated is a shifted clock somebody
//! will later mistake for a bug.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::client::{
    Client, InjectRequest, ObserveEvent, ObserveRequest, RecallIdsRequest, RecallQueryRequest,
};
use crate::extraction::{AUDITOR_ACTOR, read_committed};
use crate::longmemeval::{Instance, Session, parse_date};
use crate::report::Gate;
use crate::scenario::Environment;

/// The actor pool's naming convention. `evals/lib.sh` registers one
/// identity per instance (decision 8) and this harness discovers them by
/// prefix rather than by count, so growing the pool is a shell loop's
/// bound and not a constant in two places.
pub const ACTOR_PREFIX: &str = "lme-";

/// Every seeded session id starts here, so a run's material is
/// recognisable in an audit chain shared with four other suites.
const SESSION_PREFIX: &str = "lme";

/// What a haystack session is observed as.
const KIND: &str = "transcript_delta";

/// The surface's cap on an ids-shaped recall (`MAX_RECALL_IDS`). Chunked
/// at it rather than asked past it: the rule ADR-0046 decision 3 and
/// ADR-0048 trigger (f) both state is that a corpus outgrowing this splits
/// across more actors, and a *join* over ids the caller already holds is
/// the one shape where paging is exact rather than ambiguous.
const MAX_RECALL_IDS: usize = 32;

/// What a readiness query may carry. `POST /v1/recall` caps a query at
/// `MAX_QUERY_CHARS` (4,000) and refuses anything longer with a 400
/// (`crates/synveda-gateway/src/recall.rs`), and a real haystack session
/// runs well past that — the first run against the fetched corpus died on
/// the first instance, forty-seven sessions in. Held below the cap rather
/// than at it: the surface's bound is the surface's to change, and a
/// client sitting exactly on it breaks when it moves by one.
const MAX_QUERY_CHARS: usize = 2_000;

const POLL: Duration = Duration::from_millis(500);

/// Where each tier's gate lives. Two files rather than decision 6's one,
/// and the reason is mechanical: `report::gate` treats a bounded metric
/// this run did not measure as a breach, which is what stops a suite
/// quietly losing coverage — so a single file bounding both tiers would
/// fail whichever tier ran. Decision 7 gave the tiers separate targets;
/// separate baselines follow from that rather than from a preference.
pub const RETRIEVAL_BASELINE: &str = "evals/baseline-longmemeval-retrieval.json";
pub const JUDGED_BASELINE: &str = "evals/baseline-longmemeval-judged.json";

pub struct Options {
    pub seed_timeout: Duration,
    /// The caller-side budget, when a run wants to force ranking. Absent
    /// is the pack's own default, which is the honest shape for a
    /// published number — a budget tuned until the score improved would be
    /// a benchmark measuring its own tuning.
    pub budget_tokens: Option<u32>,
}

/// The reader and the judge, when a run is measuring the model-judged
/// tier. Absent is the deterministic tier, which reaches no model at all.
///
/// They stay two seams even when they are the same model family, because
/// option 7 rejected using one for both: a model grading answers produced
/// from its own reading is a measurement with a known bias and no way to
/// bound it. `reader::independence_note` says so in the report when the
/// run does it anyway.
pub struct Graders<'a> {
    pub reader: &'a crate::reader::AnyReader,
    pub judge: &'a crate::judge::AnyJudge,
}

/// One instance's run, minus the instance: everything the loop holds.
pub struct Suite<'a> {
    pub client: &'a Client,
    pub environment: &'a Environment,
    pub options: &'a Options,
    pub graders: Option<Graders<'a>>,
}

/// The counters both seams are called through, carried across instances.
#[derive(Debug, Default, Serialize)]
pub struct Tallies {
    pub reader: crate::reader::Tally,
    pub judge: crate::judge::Tally,
}

/// The whole run, as the report file and the gate see it.
#[derive(Debug, Serialize)]
pub struct Report {
    pub started_at: String,
    pub gateway_url: String,
    pub tenant_id: String,
    pub slice: crate::longmemeval::Slice,
    /// `retrieval` or `judged`, so a report file says which tier produced
    /// it without anybody inferring it from which axes are present.
    pub tier: String,
    /// The reader and judge as the API *served* them, which is what the
    /// judged baseline is keyed to and what the published row names
    /// (decision 6). Empty for the deterministic tier.
    pub models: BTreeMap<String, String>,
    /// Where the baseline's models and this run's disagree. Reported and
    /// never fatal: a model that moved is a different measurement, not a
    /// regression.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_drift: Vec<String>,
    /// The self-reading bias, named when the run has it (option 7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independence: Option<String>,
    /// The judge measured against its labelled sets, in the same run that
    /// used it (decision 4). Structural rather than a convention: a
    /// benchmark number produced by an unmeasured judge is not a
    /// measurement, and running the two together is what makes publishing
    /// one without the other impossible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_agreement: Option<crate::agreement::JudgeReport>,
    pub instances: Vec<InstanceOutcome>,
    pub tallies: Tallies,
    pub metrics: BTreeMap<String, f64>,
    pub gate: Gate,
    /// The deterministic tier's floors, checked by a judged run against
    /// the same measurements. Absent on a deterministic run, where `gate`
    /// already is them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_gate: Option<Gate>,
}

/// One instance's measurement.
#[derive(Debug, Serialize)]
pub struct InstanceOutcome {
    pub question_id: String,
    pub question_type: String,
    /// One of LongMemEval's 30 abstention instances. Measured, and
    /// excluded from the retrieval rates by upstream's own convention
    /// (decision 5) — excluded out loud rather than filtered quietly.
    pub abstention: bool,
    pub actor: String,
    pub sessions: usize,
    pub turns: usize,
    /// Turns skipped for being blank, and sessions skipped for repeating
    /// an id already planted. Both are properties of the real corpus, both
    /// are tiny, and both are counted rather than dropped quietly — a
    /// haystack that shrank without saying so is a denominator nobody can
    /// check (decision 7).
    pub empty_turns: usize,
    pub duplicate_sessions: usize,
    /// How far the corpus's clock was moved to meet this run's, in hours.
    pub clock_offset_hours: i64,
    pub seed_ms: f64,
    /// The evidence sessions the instance names.
    pub evidence: Vec<String>,
    /// Of those, the ones a record in the block came from.
    pub bound_evidence: Vec<String>,
    pub block_records: usize,
    /// Distinct haystack sessions the block carried material from.
    pub block_sessions: usize,
    /// Block records whose provenance named no session this run seeded.
    /// Never silently dropped: a join that quietly lost records would
    /// report a recall over a denominator it had shrunk itself.
    pub unattributed_records: usize,
    /// Whether the block carried fewer sessions than the haystack holds.
    /// A block that carried all of them made no ranking decision, so its
    /// perfect recall is a statement about the budget rather than about
    /// retrieval — the `bound` predicate EVAL-4 measures rather than
    /// declares, with the haystack as the denominator a sweep cannot give.
    pub bound: bool,
    pub tokens: u32,
    pub budget_tokens: u32,
    pub block_hash: String,
    pub latency_ms: f64,
    /// The reader's answer out of the block, on the judged tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<crate::reader::Answer>,
    /// The judge's verdict on it. Absent where no judge was asked, which
    /// is not the same as a judge that said no — an abstention instance is
    /// graded on the reader's own flag, and a reader that abstained where
    /// evidence exists is wrong without anybody paying to confirm it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<crate::judge::Verdict>,
    /// Whether the answer was right. `None` on the deterministic tier and
    /// for an instance the reader could not be run on at all — never
    /// defaulted to false, because "unmeasured" and "wrong" are the two
    /// things a published accuracy must not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correct: Option<bool>,
    /// Why `correct` is what it is, in one line: the judge's rationale, or
    /// the abstention rule that decided it without a judge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grading: Option<String>,
    pub failures: Vec<String>,
    pub passed: bool,
}

/// The actors this run may use, in a stable order.
///
/// Discovered from the environment rather than counted: `evals/lib.sh`
/// decides how many exist, and a run asking for more instances than there
/// are actors is refused by name rather than quietly doubling two
/// instances onto one identity — which would put one instance's haystack
/// inside another's, and measure retrieval over a corpus twice the size
/// the benchmark specifies.
pub fn actors(environment: &Environment) -> Vec<String> {
    environment
        .actors
        .keys()
        .filter(|name| name.starts_with(ACTOR_PREFIX))
        .cloned()
        .collect()
}

/// One instance seeded and not yet measured.
pub struct Seeded {
    pub outcome: InstanceOutcome,
    /// Every acked event id, mapped to the haystack session it came from.
    pub events: BTreeMap<String, String>,
}

/// Plants one instance's haystack as one actor, and stops there.
///
/// Seeding is separated from measuring because the first run against the
/// real corpus proved they cannot be interleaved. Instances 1–3 seeded in
/// 6–14 seconds and measured fine; from instance 4 on, the extraction
/// queue those three had filled was still draining, every per-instance
/// wait burned its whole timeout, six blocks came back empty, and the run
/// reported `retrieval_recall 0.214` — a number about queue depth wearing
/// the name of retrieval.
///
/// This is EVAL-2's finding, which EVAL-4 already paid for once: "two
/// byte-identical runs measured `tokens_mean` 129.8 and then 157 with no
/// product change", because a suite that seeds and probes in the same loop
/// measures a different corpus each time. The Q&A suite fixed it by
/// seeding once, waiting for all of it, and only then probing. So does
/// this one now.
pub async fn seed_instance(
    suite: &Suite<'_>,
    instance: &Instance,
    actor: &str,
) -> Result<Seeded, String> {
    let (client, environment) = (suite.client, suite.environment);
    let bearer = &environment.actor(actor)?.token;

    let mut outcome = InstanceOutcome {
        question_id: instance.question_id.clone(),
        question_type: instance.question_type.clone(),
        abstention: instance.is_abstention(),
        actor: actor.to_owned(),
        sessions: instance.haystack_session_ids.len(),
        turns: instance.turns(),
        empty_turns: 0,
        duplicate_sessions: 0,
        clock_offset_hours: 0,
        seed_ms: 0.0,
        evidence: instance.answer_session_ids.clone(),
        bound_evidence: Vec::new(),
        block_records: 0,
        block_sessions: 0,
        unattributed_records: 0,
        bound: false,
        tokens: 0,
        budget_tokens: 0,
        block_hash: String::new(),
        latency_ms: 0.0,
        answer: None,
        verdict: None,
        correct: None,
        grading: None,
        failures: Vec::new(),
        passed: false,
    };

    // The corpus's clock, moved to meet this run's. One offset for the
    // whole instance, so every interval inside the haystack survives.
    let asked_at = parse_date(&instance.question_date)?;
    let offset = chrono::Utc::now() - asked_at;
    outcome.clock_offset_hours = offset.num_hours();

    let started = Instant::now();
    let events = seed(client, bearer, instance, offset, &mut outcome).await?;
    outcome.seed_ms = round(started.elapsed().as_secs_f64() * 1000.0);
    Ok(Seeded { outcome, events })
}

/// Waits for the pipeline to be done with **every** instance's events, in
/// one wait rather than ten (see `seed_instance`).
pub async fn wait_for_all(
    suite: &Suite<'_>,
    instances: &[&Instance],
    seeded: &mut [Seeded],
    started: Instant,
) -> Result<(), String> {
    let auditor = &suite
        .environment
        .actors
        .get(AUDITOR_ACTOR)
        .ok_or_else(|| {
            format!(
                "the environment names no `{AUDITOR_ACTOR}` actor; this suite waits on \
                 `GET /v1/audit/events` for the pipeline to be done with every seeded turn"
            )
        })?
        .token;
    let all: BTreeMap<String, String> = seeded
        .iter()
        .flat_map(|entry| entry.events.iter().map(|(id, s)| (id.clone(), s.clone())))
        .collect();
    let mut shared = Vec::new();
    wait_for_pipeline(suite.client, auditor, &all, suite.options, &mut shared).await?;
    // A pipeline that never finished is every instance's problem, so it is
    // recorded on every instance rather than on whichever one was asked
    // last.
    for entry in seeded.iter_mut() {
        entry.outcome.failures.extend(shared.iter().cloned());
    }

    // …and then one index catch-up for the whole run, for the same reason
    // the pipeline wait is shared. The sparse leg is a sidecar sweeping on
    // a one-second timer (ADR-0024), and after ~5,000 new records every
    // instance is waiting on the *same* sweeper to catch up. Waiting per
    // instance made ten sequential waits out of one shared condition: the
    // first real run spent two minutes seeding and then twenty-six minutes
    // here, about 2.6 per instance, while the injects it was waiting to
    // make totalled 0.6 seconds.
    wait_for_index(suite, instances, seeded, started).await?;
    for entry in seeded.iter_mut() {
        entry.outcome.seed_ms = round(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}

/// Asks one seeded instance its question and grades the block.
pub async fn measure_instance(
    suite: &Suite<'_>,
    instance: &Instance,
    actor: &str,
    outcome: &mut InstanceOutcome,
    tallies: &mut Tallies,
) -> Result<(), String> {
    let (client, environment, options) = (suite.client, suite.environment, suite.options);
    let bearer = &environment.actor(actor)?.token;

    // The measurement. The question is the task, which is the whole point:
    // a memory system is being asked to bring back what a reader would
    // need in order to answer it.
    let probe = client
        .inject(
            bearer,
            &InjectRequest {
                task: Some(&instance.question),
                session_id: &format!("eval:{SESSION_PREFIX}:{}", instance.question_id),
                budget_tokens: options.budget_tokens,
            },
        )
        .await?;
    let block = &probe.value;
    outcome.block_records = block.record_ids.len();
    outcome.tokens = block.tokens;
    outcome.budget_tokens = block.budget_tokens;
    outcome.block_hash = block.block_hash.clone();
    outcome.latency_ms = round(probe.elapsed_ms);

    let bound = bound_sessions(client, bearer, instance, &block.record_ids, outcome).await?;
    outcome.block_sessions = bound.len();
    // An empty block trivially carries fewer sessions than its haystack,
    // so the first version of this reported `bound = true` for six blocks
    // that held nothing at all — a validity guard that passed precisely
    // when there was nothing to validate. A block has ranked something
    // only if it carried something.
    outcome.bound = !bound.is_empty() && bound.len() < instance.haystack_session_ids.len();
    outcome.bound_evidence = instance
        .answer_session_ids
        .iter()
        .filter(|session| bound.contains(session.as_str()))
        .cloned()
        .collect();

    if let Some(requested) = options.budget_tokens
        && block.tokens > requested
    {
        // The invariant every probe gets for nothing: a block may narrow a
        // requested budget and may never widen it (ADR-0026 decision 7).
        outcome.failures.push(format!(
            "the block spent {} tokens against a requested budget of {requested}",
            block.tokens
        ));
    }

    // The judged tier. It reads and grades the block this run just
    // composed, so the QA number and the retrieval number are properties
    // of the same block rather than of two runs somebody compared.
    if let Some(graders) = &suite.graders {
        grade(graders, instance, &block.text, tallies, outcome).await;
    }

    // An abstention instance has no evidence to bind, so binding none of
    // it is not a pass and not a failure — decision 10 predicted the
    // retrieval tier could grade these, and the amendment of 2026-08-08
    // records what measuring it found.
    outcome.passed = outcome.failures.is_empty()
        && (outcome.abstention || outcome.bound_evidence.len() == outcome.evidence.len())
        && outcome.correct != Some(false);
    Ok(())
}

/// Reads the block and grades the answer.
///
/// Errors here are the instance's, never the run's: a reader that failed
/// on one instance leaves `correct` unset — *unmeasured*, which is not
/// *wrong* — and the accuracy is reported over what was actually graded
/// with the shortfall named beside it.
async fn grade(
    graders: &Graders<'_>,
    instance: &Instance,
    block: &str,
    tallies: &mut Tallies,
    outcome: &mut InstanceOutcome,
) {
    let answer = match tallies
        .reader
        .read(
            graders.reader,
            &crate::reader::ReaderInput {
                question: &instance.question,
                block,
            },
        )
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            outcome.failures.push(format!("the reader failed: {error}"));
            return;
        }
    };

    if instance.is_abstention() {
        // Graded on the reader's own flag and no judge is asked. The
        // reference answer for an abstention instance is a sentence saying
        // the history does not discuss it, and grading prose against prose
        // would put a judge between this product and an axis EVAL-1
        // decision 4 already made first-class. The flag is the contract
        // the reader prompt states; a reader that answers in prose instead
        // of setting it has broken that contract, and that is worth
        // failing rather than papering over.
        outcome.correct = Some(answer.abstained);
        outcome.grading = Some(if answer.abstained {
            "the reader abstained, which is the correct answer to a question the haystack never \
             discusses"
                .to_owned()
        } else {
            "the reader answered a question its haystack never discusses — an invention, which \
             EVAL-1 decision 4 calls worse than staying quiet"
                .to_owned()
        });
        outcome.answer = Some(answer);
        return;
    }

    if answer.abstained {
        // Evidence exists, so an abstention is a miss by construction and
        // paying a judge to confirm it would buy nothing.
        outcome.correct = Some(false);
        outcome.grading = Some(
            "the reader abstained on a question whose evidence the corpus names, so no judge was \
             asked"
                .to_owned(),
        );
        outcome.answer = Some(answer);
        return;
    }

    match tallies
        .judge
        .grade(
            graders.judge,
            &crate::judge::JudgeInput {
                question: &instance.question,
                reference: &instance.answer(),
                candidate: &answer.text,
            },
        )
        .await
    {
        Ok(verdict) => {
            outcome.correct = Some(verdict.correct);
            outcome.grading = Some(verdict.rationale.clone());
            outcome.verdict = Some(verdict);
        }
        Err(error) => outcome.failures.push(format!("the judge failed: {error}")),
    }
    outcome.answer = Some(answer);
}

/// The reader and judge as the API served them, collected across every
/// instance (decision 6).
///
/// A run that was served two different models under one alias reports both
/// — joined rather than collapsed to the first — because that is exactly
/// the condition under which a score is a joint property of something
/// nobody wrote down.
#[must_use]
pub fn served_models(outcomes: &[InstanceOutcome]) -> BTreeMap<String, String> {
    let mut seen: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for outcome in outcomes {
        if let Some(answer) = &outcome.answer {
            seen.entry("reader")
                .or_default()
                .insert(answer.model_version.as_str());
            if let Some(effort) = &answer.effort {
                seen.entry("reader_effort").or_default().insert(effort);
            }
        }
        if let Some(verdict) = &outcome.verdict {
            seen.entry("judge")
                .or_default()
                .insert(verdict.model_version.as_str());
            if let Some(effort) = &verdict.effort {
                seen.entry("judge_effort").or_default().insert(effort);
            }
        }
    }
    seen.into_iter()
        .map(|(role, values)| {
            (
                role.to_owned(),
                values.into_iter().collect::<Vec<_>>().join("; "),
            )
        })
        .collect()
}

/// Every haystack session, observed as one call carrying one event per
/// turn. Returns the acked event ids, each mapped to the haystack session
/// it belongs to.
async fn seed(
    client: &Client,
    bearer: &str,
    instance: &Instance,
    offset: chrono::TimeDelta,
    outcome: &mut InstanceOutcome,
) -> Result<BTreeMap<String, String>, String> {
    let mut seeded: BTreeMap<String, String> = BTreeMap::new();
    let mut planted: BTreeSet<&str> = BTreeSet::new();
    for session in instance.sessions() {
        // The real corpus repeats thirteen session ids across
        // `longmemeval_s`, each duplicate byte-identical to its twin
        // (`validate` refuses any that differ). Planting one twice would
        // send the same idempotency keys again and measure the pipeline's
        // deduplication rather than the benchmark, so the id is planted
        // once and the skip is counted.
        if !planted.insert(session.session_id) {
            outcome.duplicate_sessions += 1;
            continue;
        }
        let session_id = seeded_id(instance, session.session_id);
        let started = parse_date(session.date)? + offset;
        // Twelve of the corpus's 246,750 turns are blank, none of them in
        // a session any instance names. An event carrying nothing is not
        // worth a round trip, and the count rides into the report so the
        // haystack does not shrink in silence. The surviving indices are
        // kept because the idempotency key is built from the turn's
        // position, and the ack loop below has to ask for exactly the keys
        // that were sent.
        let sent: Vec<usize> = session
            .turns
            .iter()
            .enumerate()
            .filter(|(_, turn)| !turn.content.trim().is_empty())
            .map(|(index, _)| index)
            .collect();
        outcome.empty_turns += session.turns.len() - sent.len();
        let events: Vec<ObserveEvent<'_>> = sent
            .iter()
            .map(|&index| (index, &session.turns[index]))
            .map(|(index, turn)| ObserveEvent {
                idempotency_key: turn_key(session.session_id, index),
                kind: KIND,
                payload: serde_json::json!({
                    "text": format!("{}: {}", turn.role, turn.content),
                }),
                // A minute apart, so the turns of one session keep their
                // order under any recency rule. The corpus dates a session
                // and not a turn, and inventing the same instant for ten
                // turns would make their order the database's to choose.
                occurred_at: (started + chrono::Duration::minutes(index as i64)).to_rfc3339(),
            })
            .collect();
        let response = client
            .observe(
                bearer,
                &ObserveRequest {
                    session_id: &session_id,
                    events,
                },
            )
            .await?;
        let acked = &response.value;
        if acked.denied > 0 || acked.quarantined > 0 {
            // Not a corpus we wrote, so this is a finding rather than an
            // assertion: chat transcripts should trip neither, and a
            // benchmark scored over material the product withheld is a
            // benchmark scored over a corpus nobody can reproduce.
            outcome.failures.push(format!(
                "seeding session `{}` was withheld: {} denied, {} quarantined",
                session.session_id, acked.denied, acked.quarantined
            ));
            continue;
        }
        for index in sent {
            let key = turn_key(session.session_id, index);
            let Some(event_id) = acked
                .events
                .iter()
                .find(|entry| entry.idempotency_key == key)
                .and_then(|entry| entry.event_id.clone())
            else {
                outcome.failures.push(format!(
                    "seeding turn `{key}` acked no event id, so nothing downstream can be \
                     attributed to it"
                ));
                continue;
            };
            seeded.insert(event_id, session.session_id.to_owned());
        }
    }
    Ok(seeded)
}

/// One turn's idempotency key, unique within its observe call.
fn turn_key(session_id: &str, index: usize) -> String {
    format!("{session_id}:{index}")
}

/// Waits for the pipeline to be *done* with every seeded session, which
/// the audit chain states exactly (the EVAL-2 rule): a `memory.extracted`
/// payload names each event whether it produced records or not. Polling
/// for records instead would be waiting on the thing under measurement.
async fn wait_for_pipeline(
    client: &Client,
    auditor: &str,
    seeded: &BTreeMap<String, String>,
    options: &Options,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let committed = read_committed(client, auditor).await?;
        let missing: Vec<&str> = seeded
            .iter()
            .filter(|(event_id, _)| !committed.contains_key(event_id.as_str()))
            .map(|(_, session)| session.as_str())
            .collect();
        if missing.is_empty() {
            for (event_id, session) in seeded {
                if committed
                    .get(event_id.as_str())
                    .is_some_and(|entry| entry.dead_lettered)
                {
                    failures.push(format!(
                        "the pipeline dead-lettered session `{session}`: the event was lost \
                         rather than found empty, and grading this instance would blame \
                         retrieval for a broken pipeline"
                    ));
                }
            }
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            // Turns, not sessions: `seeded` is keyed by event and an
            // event is one turn. The first real run said "544 of 560
            // session(s)" for a 47-session instance, which is how a
            // message can be both alarming and wrong.
            failures.push(format!(
                "the pipeline never finished with {} of {} turn(s) within {}s — every block \
                 composed now is missing material the corpus planted",
                missing.len(),
                seeded.len(),
                options.seed_timeout.as_secs()
            ));
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Waits until the material is *rankable*, not merely composable — for
/// every instance at once.
///
/// The sparse leg is a sidecar that sweeps on a timer (ADR-0024), so a
/// record is retrievable by recency seconds before it can be ranked, and a
/// question asked in that window measures the sweep. EVAL-4 learned that
/// and this inherits it; what this does *not* inherit is waiting per
/// probe, because every instance here waits on one shared sweeper and
/// doing it ten times in sequence multiplies one wait by ten.
///
/// It asks about each instance's evidence sessions — or its first session
/// where it names none — with that session's own opening turns as the
/// query, never the instance's question: whether the *question* ranks is
/// the thing being graded.
async fn wait_for_index(
    suite: &Suite<'_>,
    instances: &[&Instance],
    seeded: &mut [Seeded],
    started: Instant,
) -> Result<(), String> {
    // One flat worklist over the whole run, so a round of polling covers
    // every instance rather than one.
    let mut pending: Vec<(usize, &str)> = Vec::new();
    for (index, instance) in instances.iter().enumerate() {
        if instance.answer_session_ids.is_empty() {
            pending.extend(
                instance
                    .haystack_session_ids
                    .first()
                    .map(|id| (index, id.as_str())),
            );
        } else {
            pending.extend(
                instance
                    .answer_session_ids
                    .iter()
                    .map(|id| (index, id.as_str())),
            );
        }
    }

    loop {
        let mut still = Vec::new();
        for (index, session_id) in pending {
            let instance = instances[index];
            let bearer = &suite.environment.actor(&seeded[index].outcome.actor)?.token;
            let Some(session) = instance
                .sessions()
                .find(|session| session.session_id == session_id)
            else {
                continue;
            };
            let planted = seeded_id(instance, session_id);
            let found = suite
                .client
                .recall_query(
                    bearer,
                    &RecallQueryRequest {
                        query: &render(&session),
                        session_id: &format!(
                            "eval:{SESSION_PREFIX}:index:{}",
                            instance.question_id
                        ),
                        limit: MAX_RECALL_IDS,
                    },
                )
                .await?;
            let indexed = found
                .value
                .entries
                .iter()
                .any(|entry| entry.source_session_id() == Some(planted.as_str()));
            if !indexed {
                still.push((index, session_id));
            }
        }
        if still.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= suite.options.seed_timeout {
            // Recorded against the instance that owns each session, so a
            // run that timed out says which measurements it spoiled.
            for (index, session_id) in &still {
                seeded[*index].outcome.failures.push(format!(
                    "evidence session `{session_id}` never became rankable within {}s — a \
                     question asked now measures the sparse sidecar rather than retrieval",
                    suite.options.seed_timeout.as_secs()
                ));
            }
            return Ok(());
        }
        pending = still;
        tokio::time::sleep(POLL).await;
    }
}

/// The haystack sessions the block carried material from, by asking the
/// product about the records it just served.
async fn bound_sessions(
    client: &Client,
    bearer: &str,
    instance: &Instance,
    record_ids: &[String],
    outcome: &mut InstanceOutcome,
) -> Result<BTreeSet<String>, String> {
    let mut bound = BTreeSet::new();
    let mut attributed = 0usize;
    for chunk in record_ids.chunks(MAX_RECALL_IDS) {
        let answered = client
            .recall_ids(
                bearer,
                &RecallIdsRequest {
                    ids: chunk.to_vec(),
                    session_id: &format!("eval:{SESSION_PREFIX}:join:{}", instance.question_id),
                },
            )
            .await?;
        if answered.value.mode != "ids" {
            return Err(format!(
                "the surface answered the join in `{}` mode, not `ids`: this would attribute the \
                 block's records from a different question's answer",
                answered.value.mode
            ));
        }
        for entry in &answered.value.entries {
            let Some(session) = entry.source_session_id() else {
                continue;
            };
            let Some(haystack) = haystack_id(instance, session) else {
                continue;
            };
            attributed += 1;
            bound.insert(haystack.to_owned());
        }
    }
    outcome.unattributed_records = record_ids.len().saturating_sub(attributed);
    Ok(bound)
}

/// The session id a haystack session is seeded under.
fn seeded_id(instance: &Instance, session_id: &str) -> String {
    format!("{SESSION_PREFIX}:{}:{session_id}", instance.question_id)
}

/// The inverse, and it checks the instance rather than merely stripping a
/// prefix: another instance's material reaching this actor would otherwise
/// be counted as this haystack's, and the recall would be over a corpus
/// the run did not seed.
fn haystack_id<'a>(instance: &Instance, seeded: &'a str) -> Option<&'a str> {
    let rest = seeded.strip_prefix(SESSION_PREFIX)?.strip_prefix(':')?;
    let rest = rest
        .strip_prefix(instance.question_id.as_str())?
        .strip_prefix(':')?;
    instance
        .haystack_session_ids
        .iter()
        .any(|id| id == rest)
        .then_some(rest)
}

/// A session's opening turns as one string — the readiness query, and
/// nothing else.
///
/// Bounded at `MAX_QUERY_CHARS` because the surface bounds it, and the
/// truncation costs nothing here: the check asks whether *any* record from
/// this session has reached the sparse index, one record is written per
/// turn, and the early turns' terms rank their own records. It is not the
/// measurement — whether the *question* ranks is what the block is graded
/// on, and that goes through `inject` untouched.
///
/// The speaker prefix is the same one `seed` writes onto each turn: a
/// preference stated by the user and one suggested by the assistant are
/// different facts, and three of LongMemEval's six question types turn on
/// which is which.
fn render(session: &Session<'_>) -> String {
    let mut out = String::new();
    for turn in session.turns {
        if out.chars().count() >= MAX_QUERY_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{}: {}", turn.role, turn.content));
    }
    // A turn can exceed the bound on its own, so the whole is clipped on a
    // character boundary rather than trusting the per-turn break.
    out.chars().take(MAX_QUERY_CHARS).collect()
}

/// The deterministic tier's axes.
///
/// Rates are over evidence *sessions* rather than over instances, for the
/// EVAL-2 reason EVAL-4 restated: an instance naming three evidence
/// sessions and one naming a single session should weigh by what they
/// measured. `longmemeval_instances_complete` is the instance-level view
/// beside it, because "found two of three" and "found nothing" are the
/// same row in a recall number and very different findings.
pub fn metrics(outcomes: &[InstanceOutcome]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if outcomes.is_empty() {
        return metrics;
    }

    // Upstream excludes the abstention instances from retrieval scoring;
    // so do we (decision 5), and the count of what was excluded is in the
    // slice line rather than implied by a smaller denominator.
    let graded: Vec<&InstanceOutcome> = outcomes
        .iter()
        .filter(|outcome| !outcome.abstention)
        .collect();

    let expected: usize = graded.iter().map(|outcome| outcome.evidence.len()).sum();
    if expected > 0 {
        let reached: usize = graded
            .iter()
            .map(|outcome| outcome.bound_evidence.len())
            .sum();
        metrics.insert(
            "longmemeval_retrieval_recall".to_owned(),
            round(reached as f64 / expected as f64),
        );
        let complete = graded
            .iter()
            .filter(|outcome| outcome.bound_evidence.len() == outcome.evidence.len())
            .count();
        metrics.insert(
            "longmemeval_instances_complete".to_owned(),
            round(complete as f64 / graded.len() as f64),
        );
        metrics.insert(
            "longmemeval_per_type".to_owned(),
            round(macro_mean(&graded, |outcome| {
                (outcome.bound_evidence.len(), outcome.evidence.len())
            })),
        );
    }

    // The axis that says whether there was anything to measure at all,
    // and it exists because the first run against the real corpus needed
    // it: six of ten blocks came back empty because the extraction queue
    // had not drained, and the run still reported a recall. An empty block
    // is not a retrieval result, it is a run that asked too early.
    let empty = outcomes
        .iter()
        .filter(|outcome| outcome.block_records == 0)
        .count();
    metrics.insert(
        "longmemeval_empty_blocks".to_owned(),
        round(empty as f64 / outcomes.len() as f64),
    );

    // And the axis that says whether any of the above measured *retrieval*
    // rather than budget. A block that carried every session of its
    // haystack ranked nothing; a suite where that is common reports the
    // budget under the name of recall.
    let bound = outcomes.iter().filter(|outcome| outcome.bound).count();
    metrics.insert(
        "longmemeval_bound_instances".to_owned(),
        round(bound as f64 / outcomes.len() as f64),
    );

    // Decision 10 claimed the abstention instances are gradeable here
    // because "the correct block binds nothing". This is that claim
    // measured rather than assumed: a haystack holds forty sessions of
    // ordinary chat whether or not it holds the answer, so a block with a
    // budget will bind *something* unless the product applies a relevance
    // floor. If this reports 0.0 on the first live run, abstention belongs
    // to the judged tier and the decision needs amending.
    let abstention: Vec<&InstanceOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.abstention)
        .collect();
    if !abstention.is_empty() {
        let empty = abstention
            .iter()
            .filter(|outcome| outcome.block_records == 0)
            .count();
        metrics.insert(
            "longmemeval_abstention_empty_blocks".to_owned(),
            round(empty as f64 / abstention.len() as f64),
        );
    }

    // Absolute rather than a rate, and gated at zero: a join that lost
    // records would shrink its own denominator and report the loss as
    // precision.
    let unattributed: usize = outcomes
        .iter()
        .map(|outcome| outcome.unattributed_records)
        .sum();
    metrics.insert(
        "longmemeval_unattributed_records".to_owned(),
        unattributed as f64,
    );

    // The judged tier's axes. Absent entirely on the deterministic tier,
    // which is why the two tiers cannot share a baseline file: a bound on
    // a metric this run did not measure is a breach, and that rule is what
    // stops a suite losing coverage quietly.
    let judged: Vec<&InstanceOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.correct.is_some())
        .collect();
    if !judged.is_empty() {
        let correct = judged
            .iter()
            .filter(|outcome| outcome.correct == Some(true))
            .count();
        metrics.insert(
            "longmemeval_qa_accuracy".to_owned(),
            round(correct as f64 / judged.len() as f64),
        );
        metrics.insert(
            "longmemeval_qa_per_type".to_owned(),
            round(macro_mean(&judged, |outcome| {
                (usize::from(outcome.correct == Some(true)), 1)
            })),
        );

        // The abstention axis, now that decision 10's amendment puts it
        // here: of the questions whose haystack never discusses them, the
        // fraction the reader declined rather than answered. EVAL-1
        // decision 4's axis with an external corpus behind it.
        let abstention_judged: Vec<&&InstanceOutcome> =
            judged.iter().filter(|outcome| outcome.abstention).collect();
        if !abstention_judged.is_empty() {
            let held = abstention_judged
                .iter()
                .filter(|outcome| outcome.correct == Some(true))
                .count();
            metrics.insert(
                "longmemeval_abstention_accuracy".to_owned(),
                round(held as f64 / abstention_judged.len() as f64),
            );
        }

        // Its inverse, and the one a memory system talks itself into: a
        // question the corpus *does* answer, declined. A reader that
        // abstains everywhere would score well above on the axis above and
        // this is what says so.
        let answerable: Vec<&&InstanceOutcome> = judged
            .iter()
            .filter(|outcome| !outcome.abstention)
            .collect();
        if !answerable.is_empty() {
            let declined = answerable
                .iter()
                .filter(|outcome| {
                    outcome
                        .answer
                        .as_ref()
                        .is_some_and(|answer| answer.abstained)
                })
                .count();
            metrics.insert(
                "longmemeval_over_abstention".to_owned(),
                round(declined as f64 / answerable.len() as f64),
            );
        }
    }
    // Instances the judged tier reached and could not grade. Absolute and
    // gated at zero: an accuracy whose denominator shrank because the
    // reader or the judge errored is an accuracy over the easy half.
    let ungraded = outcomes
        .iter()
        .filter(|outcome| outcome.answer.is_some() && outcome.correct.is_none())
        .count();
    if !judged.is_empty() || ungraded > 0 {
        metrics.insert("longmemeval_qa_ungraded".to_owned(), ungraded as f64);
    }

    let tokens: u32 = outcomes.iter().map(|outcome| outcome.tokens).sum();
    metrics.insert(
        "longmemeval_tokens_mean".to_owned(),
        round(f64::from(tokens) / outcomes.len() as f64),
    );
    let mut latencies: Vec<f64> = outcomes.iter().map(|outcome| outcome.latency_ms).collect();
    latencies.sort_by(f64::total_cmp);
    metrics.insert(
        "longmemeval_latency_p95_ms".to_owned(),
        round(crate::report::percentile(&latencies, 95.0)),
    );
    metrics
}

/// A rate averaged over question types rather than over the whole run —
/// the macro view, which is what stops a category with many rows from
/// carrying one with few. The same reason EVAL-2 reports its extraction
/// axes per class and then macro-averages them, and it matters more here:
/// LongMemEval's six types are six different abilities, and a mean over
/// instances would let the commonest one speak for all of them.
///
/// `counts` returns each instance's (numerator, denominator) contribution,
/// so retrieval recall and QA accuracy reduce through one function.
fn macro_mean(
    outcomes: &[&InstanceOutcome],
    counts: impl Fn(&InstanceOutcome) -> (usize, usize),
) -> f64 {
    let mut per_type: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for outcome in outcomes {
        let (numerator, denominator) = counts(outcome);
        let slot = per_type
            .entry(outcome.question_type.as_str())
            .or_insert((0, 0));
        slot.0 += numerator;
        slot.1 += denominator;
    }
    let rates: Vec<f64> = per_type
        .values()
        .filter(|(_, expected)| *expected > 0)
        .map(|(reached, expected)| *reached as f64 / *expected as f64)
        .collect();
    if rates.is_empty() {
        return 0.0;
    }
    rates.iter().sum::<f64>() / rates.len() as f64
}

/// The human summary. Written to stderr beside the JSON report, because a
/// number nobody reads gates nothing.
#[must_use]
pub fn summarise(run: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!("\nlongmemeval ({} tier)\n", run.tier));
    out.push_str(&format!("  corpus   {}\n", run.slice.describe()));
    for (role, model) in &run.models {
        out.push_str(&format!("  {role:<8} {model}\n"));
    }
    for line in &run.model_drift {
        out.push_str(&format!("  drift    {line}\n"));
    }
    if let Some(note) = &run.independence {
        out.push_str(&format!("  bias     {note}\n"));
    }
    // Each instance carries its own offset, because each names its own
    // question date; the range is what a reader needs to see that the
    // shift happened at all.
    let offsets: Vec<i64> = run
        .instances
        .iter()
        .map(|outcome| outcome.clock_offset_hours)
        .collect();
    if let (Some(low), Some(high)) = (offsets.iter().min(), offsets.iter().max()) {
        out.push_str(&format!(
            "  clock    shifted {low}–{high} hour(s), per instance, so each question date is now\n"
        ));
    }

    for outcome in &run.instances {
        let verdict = if outcome.abstention {
            "abstention".to_owned()
        } else {
            format!(
                "{}/{} evidence",
                outcome.bound_evidence.len(),
                outcome.evidence.len()
            )
        };
        out.push_str(&format!(
            "  {} {:<28} {:<26} {} block record(s) from {} of {} session(s){}\n",
            if outcome.passed { "ok  " } else { "FAIL" },
            outcome.question_id,
            verdict,
            outcome.block_records,
            outcome.block_sessions,
            outcome.sessions,
            if outcome.bound {
                ""
            } else {
                " — unbound, nothing ranked"
            },
        ));
        // The judged line under the retrieval one, because the two are
        // properties of the same block and reading them apart is how
        // reversal trigger (d) gets missed — retrieval holding at 1.0 while
        // the answer stays wrong is CTX-2/CTX-4's problem, not MEM's, and
        // only these two lines together say so.
        if let Some(answer) = &outcome.answer {
            out.push_str(&format!(
                "       {} {}\n",
                match outcome.correct {
                    Some(true) => "correct  ",
                    Some(false) => "INCORRECT",
                    None => "ungraded ",
                },
                truncate(if answer.abstained {
                    "(abstained)"
                } else {
                    &answer.text
                })
            ));
            if let Some(grading) = &outcome.grading {
                out.push_str(&format!("                 {}\n", truncate(grading)));
            }
        }
        for failure in &outcome.failures {
            out.push_str(&format!("       {failure}\n"));
        }
    }

    if let Some(agreement) = &run.judge_agreement {
        out.push_str(&format!(
            "\n  judge    measured before it measured: {}\n",
            crate::agreement::summarise(agreement)
                .lines()
                .find(|line| line.contains("agreement"))
                .unwrap_or("(see the report)")
                .trim()
        ));
    }
    // What the run cost, in the three places it was spent. The agreement
    // pass is listed separately rather than folded in: it is a fixed cost
    // per run and the other two scale with the slice, so one total would
    // make the per-instance figure — the number decision 7's slice choice
    // turns on — unrecoverable.
    let costed = [
        ("reader", &run.tallies.reader.tokens),
        ("judge", &run.tallies.judge.tokens),
        (
            "judge/agreement",
            run.judge_agreement
                .as_ref()
                .map_or(&crate::anthropic::Usage::ZERO, |report| {
                    &report.tally.tokens
                }),
        ),
    ];
    if costed.iter().any(|(_, tokens)| tokens.prompt_tokens() > 0) {
        out.push('\n');
        for (label, tokens) in costed {
            out.push_str(&crate::agreement::tokens_line(label, tokens));
        }
    }

    out.push_str("\n  metrics\n");
    for (metric, value) in &run.metrics {
        out.push_str(&format!("    {metric:<36} {value}\n"));
    }
    for (label, gate) in [
        (run.tier.as_str(), &run.gate),
        (
            "retrieval floors",
            run.retrieval_gate.as_ref().unwrap_or(&run.gate),
        ),
    ] {
        if run.retrieval_gate.is_none() && label == "retrieval floors" {
            continue;
        }
        if !gate.breaches.is_empty() {
            out.push_str(&format!("\n  gate ({label})\n"));
            for breach in &gate.breaches {
                out.push_str(&format!("    {}\n", breach.reason));
            }
        }
    }
    // The exit status follows the retrieval floors on a judged run, so
    // the summary names which gate decided it rather than leaving a reader
    // to work out that the judged bounds are advisory.
    let (deciding, passed) = match &run.retrieval_gate {
        Some(retrieval) => ("retrieval floors", retrieval.passed),
        None => (run.tier.as_str(), run.gate.passed),
    };
    out.push_str(&format!(
        "\n  {} ({deciding})\n",
        if passed { "gate held" } else { "GATE BREACHED" }
    ));
    if run.retrieval_gate.is_some() && !run.gate.passed {
        out.push_str("  judged bounds breached, reported and not gating (decision 5)\n");
    }
    out
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// One line's worth of a free-text answer or rationale. The whole thing is
/// in the report; this is the summary somebody reads on a terminal.
fn truncate(text: &str) -> String {
    const WIDTH: usize = 96;
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= WIDTH {
        return flat;
    }
    format!("{}…", flat.chars().take(WIDTH - 1).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(question_id: &str, question_type: &str, evidence: &[&str]) -> Instance {
        let sessions: Vec<String> = (0..4).map(|index| format!("s{index}")).collect();
        let json = serde_json::json!({
            "question_id": question_id,
            "question_type": question_type,
            "question": "when did the lease end",
            "answer": "March",
            "question_date": "2023/05/20 (Sat) 02:29",
            "haystack_dates": vec!["2023/04/01 (Sat) 10:00"; 4],
            "haystack_session_ids": sessions,
            "haystack_sessions": vec![vec![
                serde_json::json!({"role": "user", "content": "packing"}),
                serde_json::json!({"role": "assistant", "content": "noted"}),
            ]; 4],
            "answer_session_ids": evidence,
        });
        serde_json::from_value(json).expect("an instance")
    }

    fn outcome(
        question_id: &str,
        question_type: &str,
        evidence: &[&str],
        bound: &[&str],
    ) -> InstanceOutcome {
        InstanceOutcome {
            question_id: question_id.to_owned(),
            question_type: question_type.to_owned(),
            abstention: question_id.ends_with("_abs"),
            actor: "lme-000".to_owned(),
            sessions: 4,
            turns: 8,
            empty_turns: 0,
            duplicate_sessions: 0,
            clock_offset_hours: 27_000,
            seed_ms: 10.0,
            evidence: evidence.iter().map(|id| (*id).to_owned()).collect(),
            bound_evidence: bound.iter().map(|id| (*id).to_owned()).collect(),
            block_records: 2,
            block_sessions: 2,
            unattributed_records: 0,
            bound: true,
            tokens: 100,
            budget_tokens: 1500,
            block_hash: "b3-test".to_owned(),
            latency_ms: 5.0,
            answer: None,
            verdict: None,
            correct: None,
            grading: None,
            failures: Vec::new(),
            passed: true,
        }
    }

    /// The judged tier's half of an outcome: what the reader said and
    /// whether it was right.
    fn judged(
        mut outcome: InstanceOutcome,
        text: &str,
        abstained: bool,
        correct: bool,
    ) -> InstanceOutcome {
        outcome.answer = Some(crate::reader::Answer {
            text: text.to_owned(),
            abstained,
            method: "claude-api".to_owned(),
            model_version: "claude-opus-5-test".to_owned(),
            effort: Some("high".to_owned()),
            usage: None,
        });
        outcome.correct = Some(correct);
        outcome
    }

    /// The join is the whole tier. A record from another instance's
    /// material must not count as this haystack's, or the recall is over a
    /// corpus the run did not seed.
    #[test]
    fn a_session_id_round_trips_and_a_foreign_one_does_not() {
        let instance = instance("aa11", "multi-session", &["s1"]);
        let seeded = seeded_id(&instance, "s1");
        assert_eq!(seeded, "lme:aa11:s1");
        assert_eq!(haystack_id(&instance, &seeded), Some("s1"));

        assert_eq!(
            haystack_id(&instance, "lme:bb22:s1"),
            None,
            "another instance"
        );
        assert_eq!(
            haystack_id(&instance, "lme:aa11:s9"),
            None,
            "not in this haystack"
        );
        assert_eq!(haystack_id(&instance, "qa:acme:own"), None, "another suite");
        assert_eq!(haystack_id(&instance, "lme:aa11"), None, "no session part");
    }

    /// Turns are seeded as separate events, so their idempotency keys have
    /// to be distinct — a collision would read as a duplicate and drop a
    /// turn out of the haystack silently.
    #[test]
    fn every_turn_of_an_instance_gets_its_own_idempotency_key() {
        let instance = instance("aa11", "multi-session", &["s1"]);
        let keys: BTreeSet<String> = instance
            .sessions()
            .flat_map(|session| {
                (0..session.turns.len())
                    .map(|index| turn_key(session.session_id, index))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(keys.len(), instance.turns(), "a key collided: {keys:?}");
        assert!(keys.contains("s0:0") && keys.contains("s3:1"), "{keys:?}");
    }

    #[test]
    fn a_session_renders_with_its_speakers_kept_and_within_the_surfaces_bound() {
        let instance = instance("aa11", "single-session-preference", &["s1"]);
        let session = instance.sessions().next().expect("a session");
        assert_eq!(render(&session), "user: packing\nassistant: noted");

        // The bound the first real run died on: a 47-session instance's
        // turns run far past `POST /v1/recall`'s 4,000-character cap.
        let long = serde_json::from_value::<crate::longmemeval::Instance>(serde_json::json!({
            "question_id": "aa11", "question_type": "multi-session",
            "question": "q", "answer": "a",
            "question_date": "2023/05/20 (Sat) 02:29",
            "haystack_dates": ["2023/04/01 (Sat) 10:00"],
            "haystack_session_ids": ["s0"],
            "haystack_sessions": [[
                {"role": "user", "content": "packing ".repeat(900)},
                {"role": "assistant", "content": "noted"},
            ]],
            "answer_session_ids": ["s0"],
        }))
        .expect("an instance");
        let rendered = render(&long.sessions().next().expect("a session"));
        // Clipped, and clipped below the surface's own 4,000-character
        // cap rather than at it — a client sitting exactly on a bound
        // breaks when the bound moves by one.
        assert_eq!(rendered.chars().count(), MAX_QUERY_CHARS);
        const SURFACE_CAP: usize = 4_000;
        assert!(rendered.chars().count() < SURFACE_CAP);
    }

    #[test]
    fn recall_is_over_evidence_sessions_and_completeness_is_over_instances() {
        let metrics = metrics(&[
            outcome("a", "multi-session", &["s1", "s2", "s3"], &["s1", "s2"]),
            outcome("b", "temporal-reasoning", &["s1"], &["s1"]),
        ]);
        // 3 of 4 evidence sessions, but only 1 of 2 instances complete.
        assert_eq!(metrics.get("longmemeval_retrieval_recall"), Some(&0.75));
        assert_eq!(metrics.get("longmemeval_instances_complete"), Some(&0.5));
        // Macro over types: (2/3 + 1/1) / 2.
        assert_eq!(metrics.get("longmemeval_per_type"), Some(&0.833));
    }

    /// Upstream's own exclusion, and it has to leave the denominator
    /// rather than score zero — an abstention instance has no evidence, so
    /// counting it would drag the recall down for having nothing to find.
    #[test]
    fn abstention_instances_leave_the_retrieval_denominator() {
        let mut abstention = outcome("b_abs", "single-session-user", &[], &[]);
        abstention.block_records = 0;
        let metrics = metrics(&[outcome("a", "multi-session", &["s1"], &["s1"]), abstention]);
        assert_eq!(metrics.get("longmemeval_retrieval_recall"), Some(&1.0));
        assert_eq!(metrics.get("longmemeval_instances_complete"), Some(&1.0));
        // …and decision 10's prediction is reported rather than assumed.
        assert_eq!(
            metrics.get("longmemeval_abstention_empty_blocks"),
            Some(&1.0)
        );
    }

    /// A block that carried its whole haystack ranked nothing, and a
    /// perfect recall over such blocks is a statement about the budget.
    #[test]
    fn an_unbound_block_is_counted_so_a_recall_over_nothing_is_visible() {
        let mut unbound = outcome("a", "multi-session", &["s1"], &["s1"]);
        unbound.bound = false;
        unbound.block_sessions = 4;
        let metrics = metrics(&[unbound, outcome("b", "multi-session", &["s1"], &["s1"])]);
        assert_eq!(metrics.get("longmemeval_retrieval_recall"), Some(&1.0));
        assert_eq!(
            metrics.get("longmemeval_bound_instances"),
            Some(&0.5),
            "half the run ranked nothing, and the recall alone does not say so"
        );
    }

    /// The axis the first real run needed and did not have. Six of ten
    /// blocks came back with no records because the extraction queue had
    /// not drained, and the suite reported a retrieval recall anyway.
    #[test]
    fn an_empty_block_is_a_run_that_asked_too_early_rather_than_a_result() {
        let mut empty = outcome("a", "multi-session", &["s1"], &[]);
        empty.block_records = 0;
        empty.block_sessions = 0;
        let metrics = metrics(&[empty, outcome("b", "multi-session", &["s1"], &["s1"])]);
        assert_eq!(
            metrics.get("longmemeval_empty_blocks"),
            Some(&0.5),
            "half the run composed nothing at all"
        );
        // …and the recall it would otherwise have published quietly.
        assert_eq!(metrics.get("longmemeval_retrieval_recall"), Some(&0.5));
    }

    #[test]
    fn unattributed_records_are_reported_rather_than_dropped() {
        let mut lost = outcome("a", "multi-session", &["s1"], &["s1"]);
        lost.unattributed_records = 3;
        let metrics = metrics(&[lost]);
        assert_eq!(metrics.get("longmemeval_unattributed_records"), Some(&3.0));
    }

    #[test]
    fn a_run_that_measured_nothing_reports_nothing() {
        assert!(metrics(&[]).is_empty());
    }

    /// The published figure, and the two things that must not be merged
    /// into it: an instance nobody could grade is not an instance graded
    /// wrong.
    #[test]
    fn qa_accuracy_reads_only_what_was_graded_and_names_what_was_not() {
        let mut ungraded = outcome("c", "multi-session", &["s1"], &["s1"]);
        ungraded.answer = Some(crate::reader::Answer {
            text: String::new(),
            abstained: false,
            method: "claude-api".to_owned(),
            model_version: "claude-opus-5-test".to_owned(),
            effort: None,
            usage: None,
        });
        ungraded.failures.push("the judge failed: 529".to_owned());

        let metrics = metrics(&[
            judged(
                outcome("a", "multi-session", &["s1"], &["s1"]),
                "March",
                false,
                true,
            ),
            judged(
                outcome("b", "temporal-reasoning", &["s1"], &["s1"]),
                "last spring",
                false,
                false,
            ),
            ungraded,
        ]);
        assert_eq!(
            metrics.get("longmemeval_qa_accuracy"),
            Some(&0.5),
            "the ungraded instance must not sit in the denominator"
        );
        assert_eq!(metrics.get("longmemeval_qa_ungraded"), Some(&1.0));
        // Macro over types: (1/1 + 0/1) / 2 — the same as the micro rate
        // here only because each type has one instance.
        assert_eq!(metrics.get("longmemeval_qa_per_type"), Some(&0.5));
    }

    /// Decision 10's axis, in the tier the amendment of 2026-08-08 moved it
    /// to — and its inverse beside it, because a reader that abstains
    /// everywhere scores perfectly on the first one.
    #[test]
    fn abstention_is_graded_both_ways_round() {
        let metrics = metrics(&[
            // Held its tongue where it should: correct.
            judged(
                outcome("a_abs", "single-session-user", &[], &[]),
                "(declined)",
                true,
                true,
            ),
            // Answered a question its haystack never discusses: invention.
            judged(
                outcome("b_abs", "multi-session", &[], &[]),
                "About four hundred pounds",
                false,
                false,
            ),
            // Declined a question the corpus does answer: over-abstention.
            judged(
                outcome("c", "multi-session", &["s1"], &["s1"]),
                "(declined)",
                true,
                false,
            ),
            judged(
                outcome("d", "temporal-reasoning", &["s1"], &["s1"]),
                "March",
                false,
                true,
            ),
        ]);
        assert_eq!(metrics.get("longmemeval_abstention_accuracy"), Some(&0.5));
        assert_eq!(
            metrics.get("longmemeval_over_abstention"),
            Some(&0.5),
            "one of the two answerable questions was declined"
        );
    }

    /// Decision 6: the score is a joint property of two models, and a run
    /// served two different ones under one alias says so rather than
    /// picking the first.
    #[test]
    fn served_models_name_both_roles_and_refuse_to_collapse_a_split_run() {
        let mut first = judged(
            outcome("a", "multi-session", &["s1"], &["s1"]),
            "March",
            false,
            true,
        );
        first.verdict = Some(crate::judge::Verdict {
            correct: true,
            rationale: "conveys the reference".to_owned(),
            method: "claude-api".to_owned(),
            model_version: "claude-opus-5-judge".to_owned(),
            effort: Some("high".to_owned()),
            usage: None,
        });
        let mut second = judged(
            outcome("b", "multi-session", &["s1"], &["s1"]),
            "March",
            false,
            true,
        );
        if let Some(answer) = second.answer.as_mut() {
            answer.model_version = "claude-opus-5-other".to_owned();
        }

        let served = served_models(&[first, second]);
        assert_eq!(served.get("judge"), Some(&"claude-opus-5-judge".to_owned()));
        assert_eq!(served.get("reader_effort"), Some(&"high".to_owned()));
        assert_eq!(
            served.get("reader"),
            Some(&"claude-opus-5-other; claude-opus-5-test".to_owned()),
            "a run served two models reports both"
        );

        // And the deterministic tier names none, because it reached none.
        assert!(served_models(&[outcome("a", "multi-session", &["s1"], &["s1"])]).is_empty());
    }

    #[test]
    fn the_actor_pool_is_discovered_by_prefix_and_ordered() {
        let environment: Environment = serde_json::from_str(
            r#"{
                "gateway_url": "http://localhost:8080",
                "tenant_id": "t",
                "actors": {
                    "qa-reader": {"token": "x"},
                    "lme-002": {"token": "c"},
                    "lme-000": {"token": "a"},
                    "lme-001": {"token": "b"}
                }
            }"#,
        )
        .expect("an environment");
        assert_eq!(actors(&environment), ["lme-000", "lme-001", "lme-002"]);
    }
}
