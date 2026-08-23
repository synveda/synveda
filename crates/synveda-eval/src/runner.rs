//! Seed → wait → probe → grade, one scenario at a time (EVAL-1,
//! ADR-0028 decision 3).
//!
//! Memory is planted through `/v1/observe` and nothing else, so a run
//! exercises MEM-1's buffer, MEM-2's redaction, MEM-3's extraction, and
//! MEM-4's embedding before it ever measures composition. That is slower
//! than an INSERT by whole seconds, and it is the difference between
//! evaluating the product and evaluating a fixture.
//!
//! Scenarios run in sequence. A concurrent runner would finish sooner and
//! would also make the latency axis a measurement of the runner.

use std::time::{Duration, Instant};

use crate::client::{Client, InjectRequest, InjectResponse, ObserveEvent, ObserveRequest};
use crate::report::Outcome;
use crate::scenario::{Environment, Scenario};

/// How long seeded material gets to become composable before the scenario
/// is graded anyway. The pipeline's own SLO is 60s (seed §10), and a
/// scenario that times out fails on its axes rather than crashing the
/// run — a stuck pipeline should read as "quality collapsed", because to
/// the person whose session it is, that is what happened.
pub const DEFAULT_SEED_TIMEOUT: Duration = Duration::from_secs(90);

/// How often to ask whether the seed has landed.
const POLL: Duration = Duration::from_millis(500);

pub struct Options {
    pub seed_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            seed_timeout: DEFAULT_SEED_TIMEOUT,
        }
    }
}

pub async fn run_scenario(
    client: &Client,
    environment: &Environment,
    scenario: &Scenario,
    options: &Options,
) -> Result<Outcome, String> {
    let mut failures = Vec::new();
    let mut degraded = Vec::new();

    let seeded = seed(client, environment, scenario, &mut degraded).await?;
    let seed_wait_ms = if seeded.is_empty() {
        0.0
    } else {
        wait_for_seed(
            client,
            environment,
            scenario,
            &seeded,
            options,
            &mut failures,
        )
        .await?
    };

    // The graded call, and then the repeats that give the latency axis
    // more than one sample. Only the first is judged: a scenario that
    // passes on the third attempt has not passed.
    let bearer = &environment.actor(&scenario.probe.actor)?.token;
    let request = InjectRequest {
        task: scenario.probe.task.as_deref(),
        budget_tokens: scenario.probe.budget_tokens,
    };
    let probe_run = client
        .session_for(bearer, &scenario.probe.session_id)
        .await?;
    let first = client.inject(bearer, &probe_run, &request).await?;
    let mut latency_ms = vec![round(first.elapsed_ms)];
    note_degraded(&mut degraded, &first.degraded);
    for _ in 1..scenario.probe.repeat {
        let repeat = client.inject(bearer, &probe_run, &request).await?;
        latency_ms.push(round(repeat.elapsed_ms));
        note_degraded(&mut degraded, &repeat.degraded);
    }

    let graded = grade(scenario, &seeded, &first.value, &mut failures);

    Ok(Outcome {
        name: scenario.name.clone(),
        description: scenario.description.clone(),
        category: scenario.category.clone(),
        passed: failures.is_empty(),
        accuracy: graded.accuracy,
        recall: graded.recall,
        abstained: graded.abstained,
        tokens: first.value.tokens,
        budget_tokens: first.value.budget_tokens,
        block_hash: first.value.block_hash.clone(),
        latency_ms,
        seed_wait_ms: round(seed_wait_ms),
        degraded,
        failures,
    })
}

/// One seeded event, as the runner needs to recognise it later.
struct Seeded {
    key: String,
    marker: String,
    actor: String,
}

async fn seed(
    client: &Client,
    environment: &Environment,
    scenario: &Scenario,
    degraded: &mut Vec<String>,
) -> Result<Vec<Seeded>, String> {
    let mut seeded = Vec::new();
    for batch in &scenario.seed {
        let bearer = &environment.actor(&batch.actor)?.token;
        let occurred_at = chrono::Utc::now().to_rfc3339();
        let events: Vec<ObserveEvent<'_>> = batch
            .events
            .iter()
            .map(|event| ObserveEvent {
                // Per-run unique, so re-running a suite against the same
                // tenant seeds again rather than deduplicating into
                // nothing (ADR-0020 decision 2 would report duplicates,
                // and the scenario would then measure the previous run).
                idempotency_key: format!("{}:{}", batch.session_id, event.key),
                kind: &event.event_type,
                payload: serde_json::json!({ "text": event.text }),
                occurred_at: occurred_at.clone(),
            })
            .collect();
        let response = client
            .observe(
                bearer,
                &client.session_for(bearer, &batch.session_id).await?,
                &ObserveRequest { events },
            )
            .await?;
        note_degraded(degraded, &response.degraded);
        if response.value.denied > 0 || response.value.quarantined > 0 {
            return Err(format!(
                "seeding `{}` was withheld: {} denied, {} quarantined",
                batch.session_id, response.value.denied, response.value.quarantined
            ));
        }
        // A batch the buffer took less of than we sent would make every
        // number downstream a measurement of the wrong corpus.
        let taken = response.value.accepted + response.value.duplicates;
        if taken != batch.events.len() {
            return Err(format!(
                "seeding `{}` sent {} event(s) and the buffer took {taken}",
                batch.session_id,
                batch.events.len()
            ));
        }
        for event in &batch.events {
            seeded.push(Seeded {
                key: event.key.clone(),
                marker: event.marker().to_owned(),
                actor: batch.actor.clone(),
            });
        }
    }
    Ok(seeded)
}

/// Polls until every seeded marker is composable for the actor that wrote
/// it, so the graded probe measures a warm system rather than the
/// pipeline's lag. The wait itself is reported; what it is not is graded.
async fn wait_for_seed(
    client: &Client,
    environment: &Environment,
    scenario: &Scenario,
    seeded: &[Seeded],
    options: &Options,
    failures: &mut Vec<String>,
) -> Result<f64, String> {
    let started = Instant::now();
    // Only the material this scenario will be judged on has to land; a
    // deliberately-irrelevant seed may never rank, and waiting for it
    // would be waiting forever.
    let wanted: Vec<&Seeded> = seeded
        .iter()
        .filter(|entry| scenario.expect.records.contains(&entry.key))
        .collect();
    if wanted.is_empty() {
        return Ok(0.0);
    }

    let session = format!("eval:seed-wait:{}", scenario.probe.session_id);
    loop {
        let mut missing = Vec::new();
        for entry in &wanted {
            // First: is it in memory at all? Taskless is the
            // recency-ordered branch, so this asks only whether the
            // pipeline has made a record, with no retrieval involved.
            let bearer = &environment.actor(&entry.actor)?.token;
            let run = client.session_for(bearer, &session).await?;
            let landed = client
                .inject(
                    bearer,
                    &run,
                    &InjectRequest {
                        task: None,
                        budget_tokens: None,
                    },
                )
                .await?;
            if !landed.value.text.contains(&entry.marker) {
                missing.push(entry.key.clone());
                continue;
            }
            // Then, for a scenario that probes with a task: can retrieval
            // find it? The sparse leg is a sidecar that sweeps on a timer
            // (ADR-0024), so a record can be composable-by-recency
            // seconds before it is rankable. Measuring the graded probe
            // in that window would be measuring the sweep.
            if scenario.probe.task.is_some() {
                let probe = &environment.actor(&scenario.probe.actor)?.token;
                let probe_run = client.session_for(probe, &session).await?;
                let ranked = client
                    .inject(
                        probe,
                        &probe_run,
                        &InjectRequest {
                            task: scenario.probe.task.as_deref(),
                            budget_tokens: scenario.probe.budget_tokens,
                        },
                    )
                    .await?;
                if !ranked.value.text.contains(&entry.marker) {
                    missing.push(entry.key.clone());
                }
            }
        }
        if missing.is_empty() {
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
        if started.elapsed() >= options.seed_timeout {
            // Grade it anyway. A pipeline that never delivered is a
            // quality collapse, and the axes are where that belongs —
            // not in a crash that reports nothing at all.
            failures.push(format!(
                "seeded material never became composable within {}s: {}",
                options.seed_timeout.as_secs(),
                missing.join(", ")
            ));
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
        tokio::time::sleep(POLL).await;
    }
}

struct Graded {
    accuracy: f64,
    recall: Option<f64>,
    abstained: Option<bool>,
}

fn grade(
    scenario: &Scenario,
    seeded: &[Seeded],
    block: &InjectResponse,
    failures: &mut Vec<String>,
) -> Graded {
    let text = &block.text;
    let before = failures.len();

    let recall = if scenario.expect.records.is_empty() {
        None
    } else {
        let mut found = 0usize;
        for key in &scenario.expect.records {
            let marker = seeded
                .iter()
                .find(|entry| &entry.key == key)
                .map_or("", |entry| entry.marker.as_str());
            if text.contains(marker) {
                found += 1;
            } else {
                failures.push(format!("expected record `{key}` never reached the block"));
            }
        }
        Some(found as f64 / scenario.expect.records.len() as f64)
    };

    for phrase in &scenario.expect.must_contain {
        if !text.contains(phrase) {
            failures.push(format!("the block is missing `{phrase}`"));
        }
    }
    for phrase in &scenario.expect.must_not_contain {
        if text.contains(phrase) {
            failures.push(format!("the block leaked `{phrase}`"));
        }
    }

    // An invariant every scenario gets for nothing: a block may narrow a
    // requested budget and may never widen it (ADR-0026 decision 7). No
    // scenario should have to remember to ask for this.
    if let Some(requested) = scenario.probe.budget_tokens
        && block.tokens > requested
    {
        failures.push(format!(
            "the block spent {} tokens against a requested budget of {requested}",
            block.tokens
        ));
    }

    let abstained = scenario.expect.abstain.then(|| {
        let empty = block.record_ids.is_empty();
        if !empty {
            failures.push(format!(
                "expected an empty block, got {} record(s) and {} token(s)",
                block.record_ids.len(),
                block.tokens
            ));
        }
        empty
    });

    Graded {
        // Binary by design: a block that leaks one forbidden phrase is
        // not mostly right.
        accuracy: if failures.len() == before { 1.0 } else { 0.0 },
        recall,
        abstained,
    }
}

fn note_degraded(into: &mut Vec<String>, degraded: &[String]) {
    for entry in degraded {
        if !into.contains(entry) {
            into.push(entry.clone());
        }
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Scenario;

    fn block(text: &str, records: usize, tokens: u32) -> InjectResponse {
        InjectResponse {
            text: text.to_owned(),
            block_hash: "b3-test".to_owned(),
            record_ids: (0..records).map(|index| index.to_string()).collect(),
            tiers: vec!["body".to_owned(); records],
            index_entries: 0,
            index_tokens: 0,
            staleness_permille: vec![1000; records],
            tokens,
            budget_tokens: 1500,
        }
    }

    fn scenario(json: &str) -> Scenario {
        serde_json::from_str(json).expect("scenario parses")
    }

    fn seeded() -> Vec<Seeded> {
        vec![Seeded {
            key: "deploy".to_owned(),
            marker: "make deploy".to_owned(),
            actor: "curator".to_owned(),
        }]
    }

    const RECALLING: &str = r#"{
        "name": "recall",
        "seed": [{"actor": "curator", "session_id": "s1",
                  "events": [{"key": "deploy", "text": "Deploys go through make deploy.",
                              "marker": "make deploy"}]}],
        "probe": {"actor": "curator", "session_id": "p1"},
        "expect": {"records": ["deploy"], "must_not_contain": ["push to main"]}
    }"#;

    #[test]
    fn a_block_carrying_the_marker_recalls_and_is_accurate() {
        let mut failures = Vec::new();
        let graded = grade(
            &scenario(RECALLING),
            &seeded(),
            &block("- [procedure] Deploys go through make deploy.", 1, 40),
            &mut failures,
        );
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(graded.recall, Some(1.0));
        assert!((graded.accuracy - 1.0).abs() < f64::EPSILON);
        assert_eq!(graded.abstained, None);
    }

    #[test]
    fn a_missing_record_costs_recall_and_accuracy_both() {
        let mut failures = Vec::new();
        let graded = grade(
            &scenario(RECALLING),
            &seeded(),
            &block("- [fact] something else entirely", 1, 20),
            &mut failures,
        );
        assert_eq!(graded.recall, Some(0.0));
        assert!(graded.accuracy.abs() < f64::EPSILON);
        assert!(failures[0].contains("deploy"), "{failures:?}");
    }

    #[test]
    fn a_leaked_phrase_zeroes_accuracy_even_with_perfect_recall() {
        let mut failures = Vec::new();
        let graded = grade(
            &scenario(RECALLING),
            &seeded(),
            &block("make deploy, and never push to main directly", 1, 40),
            &mut failures,
        );
        assert_eq!(graded.recall, Some(1.0));
        assert!(
            graded.accuracy.abs() < f64::EPSILON,
            "a leak is not partial credit"
        );
        assert!(failures[0].contains("leaked"), "{failures:?}");
    }

    #[test]
    fn a_block_that_outspends_its_requested_budget_fails_without_being_asked() {
        let budgeted = scenario(
            r#"{
                "name": "budgeted",
                "probe": {"actor": "curator", "session_id": "p1", "task": "anything",
                          "budget_tokens": 100},
                "expect": {"must_contain": ["kept"]}
            }"#,
        );
        let mut failures = Vec::new();
        let graded = grade(&budgeted, &[], &block("kept", 3, 140), &mut failures);
        assert!(graded.accuracy.abs() < f64::EPSILON);
        assert!(failures[0].contains("requested budget"), "{failures:?}");

        let mut failures = Vec::new();
        grade(&budgeted, &[], &block("kept", 3, 90), &mut failures);
        assert!(
            failures.is_empty(),
            "inside the budget is not a failure: {failures:?}"
        );
    }

    #[test]
    fn an_abstention_scenario_grades_on_emptiness() {
        let abstaining = scenario(
            r#"{
                "name": "abstain",
                "probe": {"actor": "newcomer", "session_id": "p1", "task": "anything"},
                "expect": {"abstain": true, "must_not_contain": ["make deploy"]}
            }"#,
        );

        let mut failures = Vec::new();
        let graded = grade(&abstaining, &[], &block("", 0, 0), &mut failures);
        assert_eq!(graded.abstained, Some(true));
        assert!((graded.accuracy - 1.0).abs() < f64::EPSILON);
        assert!(failures.is_empty());

        let mut failures = Vec::new();
        let graded = grade(
            &abstaining,
            &[],
            &block("make deploy", 1, 12),
            &mut failures,
        );
        assert_eq!(graded.abstained, Some(false));
        assert!(graded.accuracy.abs() < f64::EPSILON);
        assert_eq!(
            failures.len(),
            2,
            "both the leak and the non-empty block: {failures:?}"
        );
    }
}
