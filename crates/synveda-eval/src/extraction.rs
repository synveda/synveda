//! Seed → wait → sweep → attribute → score, one fixture group at a time
//! (EVAL-2, ADR-0046).
//!
//! Nothing here reads Knowledge the product would not serve. Transcripts go
//! in through `POST /v1/sessions/{id}/events`; accepted Knowledge comes back
//! through the separately authorised, session-scoped Knowledge evaluation
//! lens. Every item is still decided exactly by the PDP.
//!
//! Candidate decisions provide the write-side attribution and the diagnostic
//! Knowledge sweep says what a reader is served. The two counts remain
//! separate: accepted output is not evidence that retrieval served it.
//!
//! CPR-20 restores that enumeration without restoring tenant-global recall:
//! the evaluation route derives its project and authority from a real session,
//! follows opaque cursors and is deliberately separate from the budgeted
//! context-run surface. Prompt 30 owns the corpus re-measurement and baseline
//! update after capture acceptance is wired into the evaluation setup.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::client::{
    CaptureAcceptOptions, Client, KnowledgeSweepRequest, SessionEventBatchRequest,
    SessionEventInput,
};
use crate::fixtures::{CLASSES, Fixture, Group};
use crate::report::{ClassCounts, ExtractionOutcome};
use crate::runner::apply_candidate;
use crate::scenario::Environment;

/// What one sweep asks for. The surface caps a sweep at this, so asking
/// for exactly it and receiving exactly it is the ambiguity decision 3
/// refuses to measure through.
const SWEEP_LIMIT: usize = 32;

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

    // Seed and explicitly review. Extraction creates candidates only; this
    // harness accepts them through the public VedaFlow-backed action before
    // asking the separately authorised Knowledge evaluation lens.
    let mut event_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut materialised: BTreeMap<String, Materialised> = BTreeMap::new();
    let capture_started = Instant::now();
    for fixture in &group.fixtures {
        let session = client
            .session_for(bearer, &fixture.input.session_id)
            .await?;
        let response = client
            .append_events(
                bearer,
                &session,
                &SessionEventBatchRequest {
                    events: vec![SessionEventInput {
                        idempotency_key: fixture.name.clone(),
                        kind: &fixture.input.event_type,
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
            .and_then(|entry| entry.event_id().map(str::to_owned))
            .ok_or_else(|| {
                format!(
                    "seeding `{}` acked no event id, so nothing downstream can be attributed \
                     to it",
                    fixture.name
                )
            })?;
        event_ids.insert(event_id, fixture.name.clone());
        let reviewed = client
            .capture_and_accept(
                bearer,
                &session,
                &format!("eval-extraction-{}", fixture.name),
                options.seed_timeout,
                CaptureAcceptOptions::default(),
            )
            .await?;
        for candidate in reviewed {
            if let Err(error) =
                apply_candidate(client, environment, &candidate, &fixture.name).await
            {
                outcome.failures.push(error);
                continue;
            }
            for source in candidate.source_event_ids {
                let slot = materialised.entry(source).or_default();
                slot.knowledge_items += 1;
            }
        }
    }
    outcome.seed_wait_ms = round(capture_started.elapsed().as_secs_f64() * 1000.0);

    // Enumerate current visible Knowledge as the group's own actor. The
    // valid-time instant is deliberately a little ahead of the client clock
    // so freshly applied revisions remain current despite sub-second clock
    // skew between the harness and gateway. Transaction time is omitted and
    // therefore remains the server's present.
    let as_of = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
    let session = format!("eval:extraction:{}", group.group);
    let swept = client
        .knowledge_sweep(
            bearer,
            &KnowledgeSweepRequest {
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

    // Two bounds, both unmeasurable, and neither silently absorbed: the
    // diagnostic surface's visibility traversal and this harness's item cap.
    if sweep.truncated {
        outcome.failures.push(format!(
            "the sweep's scope universe was truncated at {} of {} scopes; the answer is bounded \
             and cannot be scored",
            sweep.scopes_decided, sweep.scopes_considered
        ));
    }
    if sweep.entries.len() >= SWEEP_LIMIT {
        outcome.failures.push(format!(
            "the sweep returned {} Knowledge items against a requested limit of {SWEEP_LIMIT}: a full \
             page and a truncated one are indistinguishable from here, so this group cannot be \
             scored — split it into more groups rather than raising the limit",
            sweep.entries.len()
        ));
    }

    // Attribute. Knowledge from another group's fixture is a leak: sibling
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
                "served Knowledge item {} carries no source event in its provenance, so it can be \
                 attributed to nothing",
                entry.knowledge_item_id
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
                "served Knowledge item {} came from session {:?}, which this group did not seed — a \
                 cross-actor leak, not a measurement",
                entry.knowledge_item_id,
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
        let knowledge = served
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
        for (class, content) in &knowledge {
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
        outcome.served_knowledge += knowledge.len();

        // A fixture that missed an expectation and says why. This is what
        // the `note` field is for: without it, a known structural limit —
        // one candidate per event, truncation-as-summary, no marker phrase —
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

    // The attribution column: what candidate review materialised against
    // what the reader was served.
    for (event_id, name) in &event_ids {
        match materialised.get(event_id.as_str()) {
            Some(entry) => {
                outcome.committed_knowledge += entry.knowledge_items;
            }
            None if group
                .fixtures
                .iter()
                .find(|fixture| &fixture.name == name)
                .is_some_and(|fixture| fixture.expected.is_empty()) =>
            {
                // Zero candidates is the asserted write-side outcome for a
                // fixture whose ground truth contains no durable Knowledge.
            }
            None => outcome.failures.push(format!(
                "candidate review materialised no Knowledge for `{name}`, so the write-side \
                 outcome is unknown"
            )),
        }
    }

    outcome.passed = outcome.failures.is_empty() && outcome.bait_hits.is_empty();
    Ok(outcome)
}

/// What accepted capture candidates materialised for one source event.
#[derive(Clone, Copy, Default)]
struct Materialised {
    pub knowledge_items: usize,
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
        // The shape of a systematic mis-routing: items arrive under a
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
