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

const POLL: Duration = Duration::from_millis(500);

pub struct Options {
    pub seed_timeout: Duration,
    /// The caller-side budget, when a run wants to force ranking. Absent
    /// is the pack's own default, which is the honest shape for a
    /// published number — a budget tuned until the score improved would be
    /// a benchmark measuring its own tuning.
    pub budget_tokens: Option<u32>,
}

/// The whole run, as the report file and the gate see it.
#[derive(Debug, Serialize)]
pub struct Run {
    pub started_at: String,
    pub gateway_url: String,
    pub tenant_id: String,
    pub slice: crate::longmemeval::Slice,
    pub instances: Vec<InstanceOutcome>,
    pub metrics: BTreeMap<String, f64>,
    pub gate: Gate,
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

/// Seeds, waits, asks and grades one instance as one actor.
pub async fn run_instance(
    client: &Client,
    environment: &Environment,
    instance: &Instance,
    actor: &str,
    options: &Options,
) -> Result<InstanceOutcome, String> {
    let bearer = &environment.actor(actor)?.token;
    let auditor = &environment
        .actors
        .get(AUDITOR_ACTOR)
        .ok_or_else(|| {
            format!(
                "the environment names no `{AUDITOR_ACTOR}` actor; this suite waits on \
                 `GET /v1/audit/events` for the pipeline to be done with every seeded session"
            )
        })?
        .token;

    let mut outcome = InstanceOutcome {
        question_id: instance.question_id.clone(),
        question_type: instance.question_type.clone(),
        abstention: instance.is_abstention(),
        actor: actor.to_owned(),
        sessions: instance.haystack_session_ids.len(),
        turns: instance.turns(),
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
        failures: Vec::new(),
        passed: false,
    };

    // The corpus's clock, moved to meet this run's. One offset for the
    // whole instance, so every interval inside the haystack survives.
    let asked_at = parse_date(&instance.question_date)?;
    let offset = chrono::Utc::now() - asked_at;
    outcome.clock_offset_hours = offset.num_hours();

    let started = Instant::now();
    let seeded = seed(client, bearer, instance, offset, &mut outcome).await?;
    wait_for_pipeline(client, auditor, &seeded, options, &mut outcome).await?;
    wait_for_index(client, bearer, instance, options, &mut outcome).await?;
    outcome.seed_ms = round(started.elapsed().as_secs_f64() * 1000.0);

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

    let bound = bound_sessions(client, bearer, instance, &block.record_ids, &mut outcome).await?;
    outcome.block_sessions = bound.len();
    outcome.bound = bound.len() < instance.haystack_session_ids.len();
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

    // An abstention instance has no evidence to bind, so binding none of
    // it is not a pass and not a failure — decision 10 predicted the
    // retrieval tier could grade these, and whether it can is the thing
    // `longmemeval_abstention_empty_blocks` measures rather than assumes.
    outcome.passed = outcome.failures.is_empty()
        && (outcome.abstention || outcome.bound_evidence.len() == outcome.evidence.len());
    Ok(outcome)
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
    for session in instance.sessions() {
        let session_id = seeded_id(instance, session.session_id);
        let started = parse_date(session.date)? + offset;
        let events: Vec<ObserveEvent<'_>> = session
            .turns
            .iter()
            .enumerate()
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
        for index in 0..session.turns.len() {
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
    outcome: &mut InstanceOutcome,
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
                    outcome.failures.push(format!(
                        "the pipeline dead-lettered session `{session}`: the event was lost \
                         rather than found empty, and grading this instance would blame \
                         retrieval for a broken pipeline"
                    ));
                }
            }
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "the pipeline never finished with {} of {} session(s) within {}s",
                missing.len(),
                seeded.len(),
                options.seed_timeout.as_secs()
            ));
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Waits until the material is *rankable*, not merely composable. The
/// sparse leg is a sidecar that sweeps on a timer (ADR-0024), so a record
/// is retrievable by recency seconds before it can be ranked, and a
/// question asked in that window measures the sweep — EVAL-4 learned this
/// by getting it wrong, and the lesson is inherited rather than relearned.
///
/// It asks about the evidence sessions, or about the first session when
/// there is no evidence, and it asks with each session's own seeded text
/// rather than with the instance's question: whether the *question* ranks
/// is the thing being graded.
async fn wait_for_index(
    client: &Client,
    bearer: &str,
    instance: &Instance,
    options: &Options,
    outcome: &mut InstanceOutcome,
) -> Result<(), String> {
    let wanted: BTreeSet<&str> = if instance.answer_session_ids.is_empty() {
        instance
            .haystack_session_ids
            .first()
            .map(String::as_str)
            .into_iter()
            .collect()
    } else {
        instance
            .answer_session_ids
            .iter()
            .map(String::as_str)
            .collect()
    };

    let started = Instant::now();
    let mut pending: Vec<&str> = wanted.into_iter().collect();
    loop {
        let mut still = Vec::new();
        for session_id in pending {
            let Some(session) = instance
                .sessions()
                .find(|session| session.session_id == session_id)
            else {
                continue;
            };
            let seeded = seeded_id(instance, session_id);
            let found = client
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
                .any(|entry| entry.source_session_id() == Some(seeded.as_str()));
            if !indexed {
                still.push(session_id);
            }
        }
        if still.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "{} evidence session(s) never became rankable within {}s: {} — a question asked \
                 now measures the sparse sidecar rather than retrieval",
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

/// A whole session as one string — the readiness query, and nothing else.
/// It carries every term the session's turns did, so any one of their
/// records ranking is enough to say the session reached the index.
///
/// The speaker prefix is the same one `seed` writes onto each turn: a
/// preference stated by the user and one suggested by the assistant are
/// different facts, and three of LongMemEval's six question types turn on
/// which is which.
fn render(session: &Session<'_>) -> String {
    session
        .turns
        .iter()
        .map(|turn| format!("{}: {}", turn.role, turn.content))
        .collect::<Vec<_>>()
        .join("\n")
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
            round(per_type_mean(&graded)),
        );
    }

    // The axis that says whether any of the above measured retrieval at
    // all. A block that carried every session of its haystack ranked
    // nothing; a suite where that is common reports the budget under the
    // name of recall.
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

/// Recall averaged over question types rather than over sessions — the
/// macro view, which is what stops a category with many evidence sessions
/// from carrying one with few. The same reason EVAL-2 reports its
/// extraction axes per class and then macro-averages them.
fn per_type_mean(graded: &[&InstanceOutcome]) -> f64 {
    let mut per_type: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for outcome in graded {
        let slot = per_type
            .entry(outcome.question_type.as_str())
            .or_insert((0, 0));
        slot.0 += outcome.bound_evidence.len();
        slot.1 += outcome.evidence.len();
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
pub fn summarise(run: &Run) -> String {
    let mut out = String::new();
    out.push_str("\nlongmemeval (deterministic retrieval tier)\n");
    out.push_str(&format!("  corpus   {}\n", run.slice.describe()));
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
        for failure in &outcome.failures {
            out.push_str(&format!("       {failure}\n"));
        }
    }

    out.push_str("\n  metrics\n");
    for (metric, value) in &run.metrics {
        out.push_str(&format!("    {metric:<36} {value}\n"));
    }
    if !run.gate.breaches.is_empty() {
        out.push_str("\n  gate\n");
        for breach in &run.gate.breaches {
            out.push_str(&format!("    {}\n", breach.reason));
        }
    }
    out.push_str(&format!(
        "\n  {}\n",
        if run.gate.passed {
            "gate held"
        } else {
            "GATE BREACHED"
        }
    ));
    out
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
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
            failures: Vec::new(),
            passed: true,
        }
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
    fn a_session_renders_with_its_speakers_kept() {
        let instance = instance("aa11", "single-session-preference", &["s1"]);
        let session = instance.sessions().next().expect("a session");
        assert_eq!(render(&session), "user: packing\nassistant: noted");
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
