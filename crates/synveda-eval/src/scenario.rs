//! The scenario format (EVAL-1, ADR-0028 decision 2).
//!
//! Scenarios are data, not code: a file names its actors, seeds memory
//! through `/v1/observe`, probes through `/v1/inject`, and declares what
//! the block must and must not contain. Adding coverage is adding a file,
//! which is what lets EVAL-2, EVAL-4, and EVAL-5 grow this suite without
//! touching the runner.
//!
//! Every struct here refuses unknown fields. A silently-ignored
//! expectation is an eval that passes for the wrong reason — the one
//! failure mode a harness must not have.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// The live stack a run measures, as `evals/bootstrap` prints it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub gateway_url: String,
    /// Display only: the runner never sends a tenant, because the token
    /// carries one (ADR-0008).
    pub tenant_id: String,
    pub actors: BTreeMap<String, Actor>,
    /// Hierarchy nodes by name, for the one thing a corpus has to say in
    /// UUIDs: where a promotion lands (EVAL-4, ADR-0047 decision 3). A
    /// fixture names `payments`; the bootstrap knows what that is. Empty
    /// for an environment that runs no Q&A corpus.
    #[serde(default)]
    pub scopes: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    /// The `/v1` bearer this actor calls with.
    pub token: String,
    /// Where the actor sits, for the report to be readable.
    #[serde(default)]
    pub scope: Option<String>,
}

impl Environment {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read the environment {}: {err}", path.display()))?;
        let environment: Self = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid environment: {err}", path.display()))?;
        if environment.actors.is_empty() {
            return Err(format!("{} names no actors", path.display()));
        }
        Ok(environment)
    }

    pub fn actor(&self, name: &str) -> Result<&Actor, String> {
        self.actors
            .get(name)
            .ok_or_else(|| format!("no actor `{name}` in this environment"))
    }

    pub fn scope(&self, name: &str) -> Result<&str, String> {
        self.scopes.get(name).map(String::as_str).ok_or_else(|| {
            format!(
                "no scope `{name}` in this environment; the Q&A corpus promotes into named \
                 hierarchy nodes and `evals/lib.sh` is what names them"
            )
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    /// What this scenario is for, in one sentence. It rides into the
    /// report, where a failing metric is otherwise just a number.
    #[serde(default)]
    pub description: String,
    /// The capability family this scenario measures, e.g.
    /// `knowledge-update` (MEM-5, ADR-0039 decision 14).
    ///
    /// Scenarios that declare one contribute their accuracy to a metric of
    /// that name as well as to the suite's, so a baseline can bound a
    /// *category* and a gate can fail naming it — which is what "category
    /// score ≥ baseline" has to mean in a harness whose discipline is
    /// pre-registered gates. The names are LongMemEval's, so EVAL-3's
    /// benchmark adapters report into the same axes rather than inventing
    /// a second set.
    #[serde(default)]
    pub category: Option<String>,
    /// Memory to plant before probing. Empty for the scenarios whose
    /// whole point is that nothing is known.
    #[serde(default)]
    pub seed: Vec<SeedBatch>,
    pub probe: Probe,
    pub expect: Expect,
}

/// One `/v1/observe` call, as one actor.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedBatch {
    pub actor: String,
    pub session_id: String,
    pub events: Vec<SeedEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedEvent {
    /// How expectations refer to this event.
    pub key: String,
    /// `transcript_delta` | `tool_result` | `decision` (MEM-1's vocabulary).
    #[serde(default = "default_kind")]
    pub kind: String,
    pub text: String,
    /// The phrase that must survive extraction and summarisation. The
    /// full text is the default, which is right for short fixtures and
    /// wrong the moment a fixture is long enough to be summarised — so
    /// scenarios that seed prose set it explicitly.
    #[serde(default)]
    pub marker: Option<String>,
}

impl SeedEvent {
    pub fn marker(&self) -> &str {
        self.marker.as_deref().unwrap_or(&self.text)
    }
}

fn default_kind() -> String {
    "transcript_delta".to_owned()
}

/// The `/v1/inject` call under measurement.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub actor: String,
    pub session_id: String,
    /// Absent is the taskless, recency-ordered branch (ADR-0025
    /// decision 5) — the cold session start, which is a real case and
    /// not a degraded one.
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    /// How many times to call it. The first response is graded; all of
    /// them feed the latency axis, because a median over one sample is
    /// not a median.
    #[serde(default = "default_repeat")]
    pub repeat: usize,
}

fn default_repeat() -> usize {
    3
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Seed keys whose material must reach the block. This is the recall
    /// axis: the fraction of these whose marker appears.
    #[serde(default)]
    pub records: Vec<String>,
    /// Phrases that must appear, beyond the seeded keys.
    #[serde(default)]
    pub must_contain: Vec<String>,
    /// Phrases that must not — an irrelevant record that ranked anyway,
    /// or another identity's memory that leaked.
    #[serde(default)]
    pub must_not_contain: Vec<String>,
    /// The block must compose nothing at all. A memory system that
    /// invents context is worse than one that stays quiet, and this is
    /// the only axis that says so (ADR-0028 decision 4).
    #[serde(default)]
    pub abstain: bool,
}

/// The metric a category reduces into: its name with separators folded to
/// underscores, so `knowledge-update` bounds `knowledge_update` and the
/// baseline reads like the rest of the axes.
#[must_use]
pub fn metric_name(category: &str) -> String {
    category.trim().replace(['-', ' '], "_").to_lowercase()
}

/// Every `*.json` in a directory, in filename order so two runs of the
/// same suite report in the same order.
pub fn load_suite(dir: &Path) -> Result<Vec<Scenario>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the suite {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no scenarios", dir.display()));
    }

    let mut scenarios = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let scenario: Scenario = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid scenario: {err}", path.display()))?;
        validate(&scenario).map_err(|err| format!("{}: {err}", path.display()))?;
        scenarios.push(scenario);
    }
    Ok(scenarios)
}

/// Metric names the reduction always produces. A category may not take one
/// of them: the category's mean would overwrite the suite's axis, and the
/// gate would then bound something other than what its name says.
const RESERVED_METRICS: [&str; 12] = [
    "accuracy",
    "recall",
    "abstention",
    "tokens_mean",
    "tokens_max",
    "latency_p50_ms",
    "latency_p95_ms",
    "hallucination_rate",
    // EVAL-4's axes that sit outside the `qa_` namespace (ADR-0047).
    "tokens_per_answer",
    "retrieval_precision",
    "estimator_bias_p95",
    "staleness_p50_permille",
];

/// Suites that own a whole namespace rather than a fixed list, because
/// their axes are per class (EVAL-2, ADR-0046) or per scope tier (EVAL-4,
/// ADR-0047) and either can be added. A prefix rule covers the names that
/// do not exist yet.
const RESERVED_PREFIXES: [&str; 2] = ["extraction_", "qa_"];

/// Whether a folded metric name belongs to something other than a
/// scenario category.
fn is_reserved(metric: &str) -> bool {
    RESERVED_METRICS.contains(&metric)
        || RESERVED_PREFIXES
            .iter()
            .any(|prefix| metric.starts_with(prefix))
}

/// The checks serde cannot make: that expectations refer to keys the
/// scenario actually seeds, and that it asks for something.
fn validate(scenario: &Scenario) -> Result<(), String> {
    if scenario.probe.repeat == 0 {
        return Err("probe.repeat must be at least 1".to_owned());
    }
    if let Some(category) = &scenario.category {
        if category.trim().is_empty() {
            return Err("category is present but empty".to_owned());
        }
        if is_reserved(&metric_name(category)) {
            return Err(format!(
                "category `{category}` collides with a built-in axis of the same name"
            ));
        }
    }
    let keys: Vec<&str> = scenario
        .seed
        .iter()
        .flat_map(|batch| batch.events.iter())
        .map(|event| event.key.as_str())
        .collect();
    for key in &scenario.expect.records {
        if !keys.contains(&key.as_str()) {
            return Err(format!("expect.records names `{key}`, which nothing seeds"));
        }
    }
    if scenario.expect.abstain && !scenario.expect.records.is_empty() {
        return Err("a scenario cannot both abstain and expect records".to_owned());
    }
    if !scenario.expect.abstain
        && scenario.expect.records.is_empty()
        && scenario.expect.must_contain.is_empty()
        && scenario.expect.must_not_contain.is_empty()
    {
        return Err("a scenario that expects nothing measures nothing".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Scenario, String> {
        let scenario: Scenario = serde_json::from_str(json).map_err(|err| err.to_string())?;
        validate(&scenario)?;
        Ok(scenario)
    }

    const MINIMAL: &str = r#"{
        "name": "own memory",
        "seed": [{"actor": "curator", "session_id": "s1",
                  "events": [{"key": "deploy", "text": "Deploys go through make deploy.",
                              "marker": "make deploy"}]}],
        "probe": {"actor": "curator", "session_id": "p1"},
        "expect": {"records": ["deploy"]}
    }"#;

    #[test]
    fn a_scenario_round_trips_with_its_defaults() {
        let scenario = parse(MINIMAL).expect("parses");
        assert_eq!(scenario.probe.repeat, 3);
        assert_eq!(scenario.seed[0].events[0].kind, "transcript_delta");
        assert_eq!(scenario.seed[0].events[0].marker(), "make deploy");
        assert!(!scenario.expect.abstain);
    }

    #[test]
    fn a_marker_defaults_to_the_seeded_text() {
        let scenario = parse(
            r#"{
                "name": "own memory",
                "seed": [{"actor": "curator", "session_id": "s1",
                          "events": [{"key": "deploy", "text": "Deploys go through make deploy."}]}],
                "probe": {"actor": "curator", "session_id": "p1"},
                "expect": {"records": ["deploy"]}
            }"#,
        )
        .expect("parses");
        assert_eq!(
            scenario.seed[0].events[0].marker(),
            "Deploys go through make deploy."
        );
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // The failure mode this guards: a typo'd expectation that reads
        // as "no expectation" and turns the scenario green.
        let json = MINIMAL.replace(r#""records": ["deploy"]"#, r#""recods": ["deploy"]"#);
        let err = parse(&json).expect_err("unknown field must not parse");
        assert!(err.contains("recods"), "unhelpful error: {err}");
    }

    #[test]
    fn expectations_must_refer_to_something_the_scenario_seeds() {
        let json = MINIMAL.replace(r#""records": ["deploy"]"#, r#""records": ["ghost"]"#);
        let err = parse(&json).expect_err("dangling key must not validate");
        assert!(err.contains("ghost"), "unhelpful error: {err}");
    }

    #[test]
    fn a_scenario_that_expects_nothing_is_refused() {
        let err = parse(
            r#"{
                "name": "measures nothing",
                "probe": {"actor": "curator", "session_id": "p1"},
                "expect": {}
            }"#,
        )
        .expect_err("an empty expectation measures nothing");
        assert!(err.contains("measures nothing"), "unhelpful error: {err}");
    }

    /// A category that folds onto a built-in axis would have its mean
    /// silently overwrite that axis, and the gate would then bound
    /// something other than what its name says. The extraction namespace is
    /// reserved by prefix so the classes EVAL-2 has not added yet are
    /// covered too.
    #[test]
    fn a_category_cannot_take_a_built_in_axis_name() {
        for category in [
            "accuracy",
            "hallucination rate",
            "extraction precision macro",
            "extraction-recall-fact",
            // EVAL-4's namespace and its axes outside it (ADR-0047).
            "qa answer rate",
            "qa-scope-department",
            "tokens per answer",
            "retrieval precision",
        ] {
            let json = MINIMAL.replace(
                r#""probe""#,
                &format!(r#""category": "{category}", "probe""#),
            );
            let err = parse(&json).expect_err(&format!("category {category:?} must not validate"));
            assert!(err.contains("collides"), "unhelpful error: {err}");
        }
        // A category that merely mentions a class is still fine.
        let json = MINIMAL.replace(r#""probe""#, r#""category": "fact recall", "probe""#);
        assert!(
            parse(&json).is_ok(),
            "an ordinary category must still parse"
        );
    }

    #[test]
    fn abstaining_and_expecting_records_is_a_contradiction() {
        let json = MINIMAL.replace(
            r#""records": ["deploy"]"#,
            r#""records": ["deploy"], "abstain": true"#,
        );
        let err = parse(&json).expect_err("contradiction must not validate");
        assert!(err.contains("abstain"), "unhelpful error: {err}");
    }
}
