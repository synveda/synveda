//! Seed → wait → sweep → attribute → score, one fixture group at a time
//! (EVAL-2, ADR-0046).
//!
//! Nothing here reads a record the product would not serve. Transcripts go
//! in through `/v1/observe` and the records they became come back through
//! `/v1/recall`'s sweep, so every number is a number a caller could have
//! measured for themselves — MEM-1's buffer, MEM-2's redaction, MEM-3's
//! extraction, MEM-4's embedding, MEM-5's dedup and CTX's admission all
//! included, because they are all between the two calls.
//!
//! **Two lenses, two questions** (decision 4). The sweep says what a
//! *reader is served*; `GET /v1/audit/events?action=memory.extracted` says
//! what the *pipeline committed*. `admit` applies tiers, horizons and
//! MEM-5's valid-window predicate, so those are not the same set, and the
//! difference between them is reported as its own number rather than
//! absorbed into recall. The gated axes come from the sweep, because that
//! is the product claim.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::client::{Client, ObserveEvent, ObserveRequest, RecallSweepRequest};
use crate::fixtures::{CLASSES, Fixture, Group};
use crate::report::{ClassCounts, ExtractionOutcome};
use crate::scenario::Environment;

/// The actor the audit lens reads as. A tenant-wide `auditor` binding on a
/// dev-mode *subject* — never a service identity, which AUTH-3's
/// confinement forbid denies the tenant plane however it is bound
/// (ADR-0045).
pub const AUDITOR_ACTOR: &str = "auditor";

/// What one sweep asks for. The surface caps a sweep at this, so asking
/// for exactly it and receiving exactly it is the ambiguity decision 3
/// refuses to measure through.
const SWEEP_LIMIT: usize = 32;

/// How many chain rows a page of the audit lens asks for.
const AUDIT_PAGE: usize = 500;

const POLL: Duration = Duration::from_millis(500);

pub struct Options {
    pub seed_timeout: Duration,
}

/// One group's measurement. Errors are the group's failures, not the run's:
/// a group that cannot be measured reads as a failed group with a named
/// reason, and the other groups still report.
pub async fn run_group(
    client: &Client,
    environment: &Environment,
    group: &Group,
    options: &Options,
) -> Result<ExtractionOutcome, String> {
    let mut outcome = ExtractionOutcome::new(group);
    let bearer = &environment.actor(&group.actor)?.token;
    let auditor = &environment
        .actors
        .get(AUDITOR_ACTOR)
        .ok_or_else(|| {
            format!(
                "the environment names no `{AUDITOR_ACTOR}` actor; the extraction suite reads \
                 `GET /v1/audit/events` to attribute what the pipeline committed (ADR-0046 \
                 decision 4)"
            )
        })?
        .token;

    // Seed. One call per fixture, because a batch carries one session id
    // and the session is the fixture's own label.
    let mut event_ids: BTreeMap<String, String> = BTreeMap::new();
    for fixture in &group.fixtures {
        let response = client
            .observe(
                bearer,
                &ObserveRequest {
                    session_id: &fixture.input.session_id,
                    events: vec![ObserveEvent {
                        idempotency_key: fixture.name.clone(),
                        kind: &fixture.input.kind,
                        payload: fixture.input.payload.clone(),
                        occurred_at: fixture.input.occurred_at.clone(),
                    }],
                },
            )
            .await?;
        let acked = &response.value;
        if acked.denied > 0 || acked.quarantined > 0 {
            return Err(format!(
                "seeding `{}` was withheld: {} denied, {} quarantined — the corpus is \
                 documentation-only content and should trip neither",
                fixture.name, acked.denied, acked.quarantined
            ));
        }
        let event_id = acked
            .events
            .iter()
            .find(|entry| entry.idempotency_key == fixture.name)
            .and_then(|entry| entry.event_id.clone())
            .ok_or_else(|| {
                format!(
                    "seeding `{}` acked no event id, so nothing downstream can be attributed \
                     to it",
                    fixture.name
                )
            })?;
        event_ids.insert(event_id, fixture.name.clone());
    }

    // Wait for the pipeline to be *done*, which the chain states exactly:
    // every seeded event appears in a `memory.extracted` payload whether it
    // produced records or not. Polling the sweep instead would be waiting
    // for an unknown number of records to appear, which is the thing under
    // measurement.
    let committed = wait_for_pipeline(client, auditor, &event_ids, options, &mut outcome).await?;

    // Sweep, as the group's own actor.
    //
    // The instant is deliberately a little AHEAD of now, and this is the
    // subtlest thing in the file. A sweep must carry an `as_of` — it is
    // how the surface distinguishes the shape from a malformed request —
    // and the surface reads `as_of < now` as a *rewind*: `tx_at` becomes
    // `Some`, the body fetches move to `records_versions`, and with them
    // "no retention horizon is applied, because the horizon governs the
    // live corpus" (ADR-0042 decision 11, stated on `ComposeRequest
    // ::tx_at`). Sending `Utc::now()` therefore measures the *historical*
    // read: by the time the gateway evaluates its own `now`, the client's
    // instant is already milliseconds behind it.
    //
    // That is not the read a caller gets, and it would quietly hollow out
    // the attribution column this suite exists for — a horizon or a
    // supersession would withhold nothing from a rewind, so committed and
    // served could never differ for those reasons and the column would
    // report "nothing withheld" forever. An instant at or after the
    // server's `now` keeps the sweep on the live tables, where every
    // admission rule applies. A minute is far more than any handling
    // delay and far less than the age of anything the corpus seeds.
    let as_of = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let session = format!("eval:extraction:{}", group.group);
    let swept = client
        .recall_sweep(
            bearer,
            &RecallSweepRequest {
                as_of: &as_of,
                session_id: &session,
                limit: SWEEP_LIMIT,
            },
        )
        .await?;
    let sweep = swept.value;
    outcome.scopes_considered = sweep.scopes_considered;
    outcome.scopes_decided = sweep.scopes_decided;

    // Checked, not assumed: a request the surface read as `ids` or `query`
    // would have answered a different question, and a measurement of the
    // wrong question is worse than none.
    if sweep.mode != "sweep" {
        outcome.failures.push(format!(
            "the surface answered in `{}` mode, not `sweep`: this is a measurement of a \
             different question",
            sweep.mode
        ));
    }

    // Two truncations, both unmeasurable, and neither one silently
    // absorbed: the scope cap the surface reports, and the record cap it
    // does not (decision 3).
    if sweep.truncated {
        outcome.failures.push(format!(
            "the sweep's scope universe was truncated at {} of {} scopes; the answer is bounded \
             and cannot be scored",
            sweep.scopes_decided, sweep.scopes_considered
        ));
    }
    if sweep.entries.len() >= SWEEP_LIMIT {
        outcome.failures.push(format!(
            "the sweep returned {} records against a requested limit of {SWEEP_LIMIT}: a full \
             page and a truncated one are indistinguishable from here, so this group cannot be \
             scored — split it into more groups rather than raising the limit",
            sweep.entries.len()
        ));
    }

    // Attribute. A record from another group's fixture is a leak: sibling
    // isolation is a policy property (no pack opens another principal's
    // personal scope) and this suite depends on it, so it asserts it
    // rather than assuming it (decision 2).
    let known: BTreeSet<&str> = group
        .fixtures
        .iter()
        .map(|fixture| fixture.name.as_str())
        .collect();
    let mut served: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for entry in &sweep.entries {
        let Some(event_id) = entry.source_event_id() else {
            outcome.failures.push(format!(
                "served record {} carries no source event in its provenance, so it can be \
                 attributed to nothing",
                entry.record_id
            ));
            continue;
        };
        match event_ids.get(event_id) {
            Some(name) if known.contains(name.as_str()) => {
                served
                    .entry(name.as_str())
                    .or_default()
                    .push((&entry.class, &entry.content));
            }
            _ => outcome.failures.push(format!(
                "served record {} came from session {:?}, which this group did not seed — a \
                 cross-actor leak, not a measurement",
                entry.record_id,
                entry.source_session_id().unwrap_or("unknown")
            )),
        }
        if let Some(model) = entry.model_version()
            && !outcome.model_versions.iter().any(|seen| seen == model)
        {
            outcome.model_versions.push(model.to_owned());
        }
    }

    // Score. One candidate consumes at most one expectation and one
    // expectation is consumed at most once (decision 5).
    for fixture in &group.fixtures {
        let records = served
            .get(fixture.name.as_str())
            .cloned()
            .unwrap_or_default();
        for expected in &fixture.expected {
            outcome.class_mut(&expected.class).expected += 1;
        }
        if !fixture.must_not_extract.is_empty() {
            outcome.bait_fixtures += 1;
        }

        let mut taken = vec![false; fixture.expected.len()];
        for (class, content) in &records {
            outcome.class_mut(class).produced += 1;
            let hit = fixture
                .expected
                .iter()
                .enumerate()
                .position(|(index, expected)| {
                    !taken[index] && Fixture::matches(expected, class, content)
                });
            match hit {
                Some(index) => {
                    taken[index] = true;
                    outcome.class_mut(class).matched += 1;
                }
                None => outcome.unmatched.push(format!(
                    "{} [{class}] {}",
                    fixture.name,
                    content.chars().take(72).collect::<String>()
                )),
            }
            let lower = content.to_lowercase();
            for bait in &fixture.must_not_extract {
                if lower.contains(&bait.to_lowercase()) {
                    outcome
                        .bait_hits
                        .push(format!("{}: fabricated {bait:?}", fixture.name));
                }
            }
        }
        outcome.served_records += records.len();

        // A fixture that missed an expectation and says why. This is what
        // the `note` field is for: without it, a known structural limit —
        // one record per event, truncation-as-summary, no marker phrase —
        // is indistinguishable in the report from a regression.
        let missed = taken.iter().filter(|hit| !**hit).count();
        if missed > 0 && !fixture.note.is_empty() {
            outcome.noted_misses.push(format!(
                "{} missed {missed} of {}: {}",
                fixture.name,
                fixture.expected.len(),
                fixture.note
            ));
        }
    }

    // The attribution column: what the pipeline committed against what the
    // reader was served, and the merges that explain part of any gap.
    for (event_id, name) in &event_ids {
        match committed.get(event_id.as_str()) {
            Some(entry) => {
                outcome.committed_records += entry.records;
                outcome.merged_records += entry.merged;
                if entry.dead_lettered {
                    outcome.failures.push(format!(
                        "the pipeline dead-lettered `{name}` at chain seq {}: the event was lost \
                         rather than found empty, and grading it as a miss would blame the \
                         corpus for a broken pipeline",
                        entry.seq
                    ));
                }
                outcome.chain_from = match outcome.chain_from {
                    0 => entry.seq,
                    lowest => lowest.min(entry.seq),
                };
                outcome.chain_to = outcome.chain_to.max(entry.seq);
            }
            None => outcome.failures.push(format!(
                "the chain records no extraction for `{name}`, so what the pipeline did with it \
                 is unknown"
            )),
        }
    }

    outcome.passed = outcome.failures.is_empty() && outcome.bait_hits.is_empty();
    Ok(outcome)
}

/// What one `memory.extracted` payload says about one observe event.
#[derive(Clone, Copy, Default)]
pub struct Committed {
    pub records: usize,
    pub merged: usize,
    /// The event reached a `failure` outcome — retries exhausted, and the
    /// pipeline gave up on it. Deliberately distinct from "committed zero
    /// records": one is a legal outcome and the other is a lost event, and
    /// a suite that conflated them would grade a broken pipeline as a
    /// corpus the extractor found nothing in.
    pub dead_lettered: bool,
    /// Where on the chain this was read, so the attribution names its
    /// source (ADR-0045 decision 9's discipline, from the consumer's side).
    pub seq: i64,
}

/// Polls the chain until every seeded event has been extracted — or the
/// timeout, at which point the group is graded anyway and fails on a named
/// reason. A stuck pipeline should read as "quality collapsed", because to
/// the person whose session it is, that is what happened.
async fn wait_for_pipeline(
    client: &Client,
    auditor: &str,
    event_ids: &BTreeMap<String, String>,
    options: &Options,
    outcome: &mut ExtractionOutcome,
) -> Result<BTreeMap<String, Committed>, String> {
    let started = Instant::now();
    loop {
        let committed = read_committed(client, auditor).await?;
        let missing: Vec<&str> = event_ids
            .iter()
            .filter(|(event_id, _)| !committed.contains_key(event_id.as_str()))
            .map(|(_, name)| name.as_str())
            .collect();
        if missing.is_empty() {
            outcome.seed_wait_ms = round(started.elapsed().as_secs_f64() * 1000.0);
            return Ok(committed);
        }
        if started.elapsed() >= options.seed_timeout {
            outcome.failures.push(format!(
                "the pipeline never finished with {} event(s) within {}s: {}",
                missing.len(),
                options.seed_timeout.as_secs(),
                missing.join(", ")
            ));
            outcome.seed_wait_ms = round(started.elapsed().as_secs_f64() * 1000.0);
            return Ok(committed);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Every `memory.extracted` entry on the chain, keyed by observe event.
/// Pages explicitly: a helper that swallowed `truncated` would report a
/// partial chain as a complete one.
pub async fn read_committed(
    client: &Client,
    auditor: &str,
) -> Result<BTreeMap<String, Committed>, String> {
    let mut found: BTreeMap<String, Committed> = BTreeMap::new();
    let mut after: Option<i64> = None;
    loop {
        let page = client
            .audit_events(auditor, "memory.extracted", after, AUDIT_PAGE)
            .await?;
        for event in &page.events {
            let entries = event
                .payload
                .get("events")
                .and_then(serde_json::Value::as_array);
            for entry in entries.into_iter().flatten() {
                let Some(event_id) = entry.get("event_id").and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                let records = entry
                    .get("records")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let merged = entry
                    .get("merged")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let slot = found.entry(event_id.to_owned()).or_default();
                slot.records += records;
                slot.merged += merged;
                slot.dead_lettered |= event.outcome != "success";
                slot.seq = event.seq;
            }
        }
        match page.next_cursor {
            Some(cursor) if page.truncated => after = Some(cursor),
            _ => return Ok(found),
        }
    }
}

/// The extraction axes, reduced over every group (decision 5 and 6).
///
/// Per class over the whole corpus rather than per group averaged: a class
/// with two fixtures in one group and eight in another should weigh by
/// what it measured, not by which file it sat in.
pub fn metrics(outcomes: &[ExtractionOutcome]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if outcomes.is_empty() {
        return metrics;
    }

    let mut totals: BTreeMap<&str, ClassCounts> = BTreeMap::new();
    for outcome in outcomes {
        for (class, counts) in &outcome.per_class {
            let slot = totals.entry(class.as_str()).or_default();
            slot.expected += counts.expected;
            slot.produced += counts.produced;
            slot.matched += counts.matched;
        }
    }

    let mut precisions = Vec::new();
    let mut recalls = Vec::new();
    for class in CLASSES {
        let Some(counts) = totals.get(class) else {
            continue;
        };
        if counts.produced > 0 {
            let precision = counts.matched as f64 / counts.produced as f64;
            metrics.insert(format!("extraction_precision_{class}"), round(precision));
            precisions.push(precision);
        }
        if counts.expected > 0 {
            let recall = counts.matched as f64 / counts.expected as f64;
            metrics.insert(format!("extraction_recall_{class}"), round(recall));
            recalls.push(recall);
        }
    }
    if !precisions.is_empty() {
        metrics.insert(
            "extraction_precision_macro".to_owned(),
            round(precisions.iter().sum::<f64>() / precisions.len() as f64),
        );
    }
    if !recalls.is_empty() {
        metrics.insert(
            "extraction_recall_macro".to_owned(),
            round(recalls.iter().sum::<f64>() / recalls.len() as f64),
        );
    }

    // Bait over the fixtures that carry bait — the same rule the recall and
    // abstention axes follow. Against the deterministic extractor this is
    // zero by construction, and that is the point: it asserts that a
    // span-copying extractor cannot invent, so a future summarisation step
    // that breaks the property fails here.
    let bait_fixtures: usize = outcomes.iter().map(|outcome| outcome.bait_fixtures).sum();
    if bait_fixtures > 0 {
        let hits: usize = outcomes.iter().map(|outcome| outcome.bait_hits.len()).sum();
        metrics.insert(
            "hallucination_rate".to_owned(),
            round(hits as f64 / bait_fixtures as f64),
        );
    }
    metrics
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(per_class: &[(&str, usize, usize, usize)]) -> ExtractionOutcome {
        let mut outcome = ExtractionOutcome::default();
        for (class, expected, produced, matched) in per_class {
            let counts = outcome.class_mut(class);
            counts.expected = *expected;
            counts.produced = *produced;
            counts.matched = *matched;
        }
        outcome
    }

    #[test]
    fn per_class_axes_reduce_over_the_whole_corpus_not_per_group() {
        // One class split across two groups weighs by what it measured, not
        // by which file it sat in: 1/1 and 1/9 is 2/10, never the 0.556 a
        // mean of means would give.
        let metrics = metrics(&[outcome(&[("fact", 1, 1, 1)]), outcome(&[("fact", 9, 9, 1)])]);
        assert_eq!(metrics.get("extraction_precision_fact"), Some(&0.2));
        assert_eq!(metrics.get("extraction_recall_fact"), Some(&0.2));
    }

    #[test]
    fn a_class_the_corpus_does_not_exercise_is_absent_rather_than_zero() {
        let metrics = metrics(&[outcome(&[("fact", 2, 2, 2)])]);
        assert_eq!(metrics.get("extraction_precision_fact"), Some(&1.0));
        assert!(!metrics.contains_key("extraction_precision_entity"));
        assert!(!metrics.contains_key("extraction_recall_entity"));
        // The macro is over the classes that were measured, so one perfect
        // class is not diluted by five that nothing exercised.
        assert_eq!(metrics.get("extraction_precision_macro"), Some(&1.0));
    }

    #[test]
    fn a_class_produced_but_never_expected_scores_precision_and_no_recall() {
        // The shape of a systematic mis-routing: records arrive under a
        // class the corpus never labels. Precision catches it; recall has
        // no denominator to catch it with.
        let metrics = metrics(&[outcome(&[("fact", 0, 4, 0), ("preference", 4, 0, 0)])]);
        assert_eq!(metrics.get("extraction_precision_fact"), Some(&0.0));
        assert!(!metrics.contains_key("extraction_recall_fact"));
        assert_eq!(metrics.get("extraction_recall_preference"), Some(&0.0));
        assert!(!metrics.contains_key("extraction_precision_preference"));
    }

    #[test]
    fn the_hallucination_axis_is_bait_over_the_fixtures_that_carry_bait() {
        let mut clean = outcome(&[("fact", 1, 1, 1)]);
        clean.bait_fixtures = 3;
        assert_eq!(metrics(&[clean]).get("hallucination_rate"), Some(&0.0));

        let mut fabricating = outcome(&[("fact", 1, 1, 1)]);
        fabricating.bait_fixtures = 4;
        fabricating.bait_hits = vec!["f1: fabricated \"chose Qdrant\"".to_owned()];
        assert_eq!(
            metrics(&[fabricating]).get("hallucination_rate"),
            Some(&0.25)
        );
    }

    #[test]
    fn a_corpus_with_no_bait_reports_no_hallucination_axis() {
        // Absent, never 0.0: a zero here would read as "nothing was
        // fabricated" when what happened is that nothing was asked.
        let metrics = metrics(&[outcome(&[("fact", 1, 1, 1)])]);
        assert!(!metrics.contains_key("hallucination_rate"));
    }
}
