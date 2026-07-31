//! The five axes, the report, and the gate (EVAL-1, ADR-0028 decisions 4
//! and 5).
//!
//! Three axes are higher-better (accuracy, recall, abstention) and two are
//! lower-better (tokens, latency); the baseline expresses that as `min`
//! and `max` bounds rather than the runner knowing it in code, so a
//! deliberate change to what "good" means is a reviewable diff.
//!
//! A metric with no bound is reported and not gated, and the summary says
//! which is which. A harness that quietly stops gating something is worse
//! than one that never gated it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What one scenario measured.
#[derive(Debug, Serialize)]
pub struct Outcome {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The capability family this scenario measures, when it declares one
    /// (MEM-5, ADR-0039 decision 14). Its accuracy is reduced into a metric
    /// of that name as well as into the suite's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub passed: bool,
    /// 1.0 when every `must_contain` appeared and no `must_not_contain`
    /// did; 0.0 otherwise. Deliberately binary: a block that leaks one
    /// forbidden phrase is not 80% right.
    pub accuracy: f64,
    /// The fraction of expected records whose marker reached the block.
    /// Absent for scenarios that expect none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall: Option<f64>,
    /// Whether a scenario that had to compose nothing did. Absent for
    /// every other scenario.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstained: Option<bool>,
    pub tokens: u32,
    /// The budget the gateway actually applied. A tokens measurement
    /// without it is unreadable: a block that shrank because a pack
    /// narrowed the budget and a block that shrank because retrieval got
    /// worse look identical until this column is there.
    pub budget_tokens: u32,
    /// The graded block's watermark (ADR-0025 decision 7): what was
    /// measured, addressable forever from the audit chain.
    pub block_hash: String,
    pub latency_ms: Vec<f64>,
    /// How long the seeded memory took to become composable. Reported,
    /// never gated: it is the pipeline's lag, which is MEM-3's and
    /// EVAL-6's to bound.
    pub seed_wait_ms: f64,
    /// Anything the gateway degraded while serving this scenario
    /// (ADR-0026 decision 4) — an averaged-in degradation would otherwise
    /// look like a quality regression with no cause.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub degraded: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// One class's counts over one group. Precision reads `matched/produced`,
/// recall reads `matched/expected`, and both denominators ride the report
/// because a ratio without them is unreadable (EVAL-2, ADR-0046
/// decision 11).
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct ClassCounts {
    pub expected: usize,
    pub produced: usize,
    pub matched: usize,
}

/// What one fixture group measured (EVAL-2, ADR-0046 decision 11). This is
/// the dashboard: per class, what was labelled, what the pipeline produced,
/// and what matched — plus the review queue and the attribution column that
/// keeps a withheld record from reading as a missed extraction.
#[derive(Debug, Default, Serialize)]
pub struct ExtractionOutcome {
    pub group: String,
    pub actor: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub fixtures: usize,
    pub passed: bool,
    pub per_class: BTreeMap<String, ClassCounts>,
    /// How many fixtures declared bait, so `hallucination_rate` has a
    /// visible denominator.
    pub bait_fixtures: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bait_hits: Vec<String>,
    /// Records that matched no expectation — the review queue for
    /// invention the fixture author did not anticipate (decision 6). A
    /// list, deliberately not a score.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmatched: Vec<String>,
    /// Misses whose fixture said in advance why they would happen. A known
    /// structural limit and a regression are the same number without this.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub noted_misses: Vec<String>,
    /// What the pipeline committed, from the chain (decision 4).
    pub committed_records: usize,
    /// What the sweep served. The gap between the two is admission doing
    /// its job — a horizon, a tier, a shut valid window — and naming it
    /// here is what stops it reading as an extraction miss.
    pub served_records: usize,
    /// Restatements MEM-5 absorbed into records that already asserted
    /// them: the part of that gap with a specific cause.
    pub merged_records: usize,
    /// The models that actually served this group, as the pipeline recorded
    /// them — not the alias the config asked for (decision 12).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_versions: Vec<String>,
    pub scopes_considered: usize,
    pub scopes_decided: usize,
    /// The chain range the committed counts were read from, so the
    /// attribution names its source rather than floating free.
    pub chain_from: i64,
    pub chain_to: i64,
    /// How long the pipeline took to finish with every seeded event.
    /// Reported, never gated: it is MEM-3's lag and EVAL-6's to bound.
    pub seed_wait_ms: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

impl ExtractionOutcome {
    #[must_use]
    pub fn new(group: &crate::fixtures::Group) -> Self {
        Self {
            group: group.group.clone(),
            actor: group.actor.clone(),
            note: group.note.clone(),
            fixtures: group.fixtures.len(),
            ..Self::default()
        }
    }

    pub fn class_mut(&mut self, class: &str) -> &mut ClassCounts {
        self.per_class.entry(class.to_owned()).or_default()
    }
}

/// One scope tier's counts over one corpus (EVAL-4, ADR-0047 decision 9).
/// `reached` counts an expected record that arrived at any tier and `body`
/// the ones that arrived whole, so `body <= reached` always — the index
/// tier names what it could not carry (ADR-0041 decision 13) and the gap
/// between the two is the displacement CTX-4 parked here.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct TierCounts {
    pub expected: usize,
    pub reached: usize,
    pub body: usize,
}

/// What one question measured.
#[derive(Debug, Serialize)]
pub struct QuestionOutcome {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// `lexical` or `semantic` (decision 5).
    pub needs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Not measured on this run because the configured embedder cannot
    /// rank. Counted rather than scored zero, and named in the report:
    /// a question the path structurally cannot answer is not a
    /// regression.
    pub skipped: bool,
    pub passed: bool,
    pub per_tier: BTreeMap<String, TierCounts>,
    /// Expected records that arrived at the index tier — named, not
    /// carried. Not a failure on its own: the block still says they
    /// exist and recall fetches them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub demoted: Vec<String>,
    /// Expected records the block did not carry at all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Every record the block carried, and how many of them this question
    /// judges relevant — the two halves of `retrieval_precision`.
    pub block_records: usize,
    pub relevant_records: usize,
    /// What the index tier cost this block, as the product counts it
    /// (ADR-0041 decision 14). Reported beside the displacement so the
    /// two halves of CTX-4's trade — what was named, what it cost — sit
    /// together.
    pub index_entries: usize,
    pub index_tokens: u32,
    /// Whether something bound this block: it carried fewer records than
    /// the reader is served, because the budget ran out or because
    /// retrieval offered fewer candidates. Either way a choice was made,
    /// which is the condition under which what the block *did* carry is a
    /// ranking decision rather than a restatement of the corpus size —
    /// and it is what `retrieval_precision` reads (ADR-0047 decision 8).
    /// Exact rather than declared, so a corpus cannot opt a question into
    /// or out of the axis by mistake.
    pub bound: bool,
    pub tokens: u32,
    pub budget_tokens: u32,
    /// What a real BPE tokenizer counts for the same text. CTX-2 ships
    /// `ceil(chars/4)` and ADR-0025 parked the bias here; this is the
    /// denominator that measures it.
    pub reference_tokens: usize,
    pub block_hash: String,
    pub latency_ms: f64,
    /// Freshness of what the block carried, in block order (MEM-6).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub staleness_permille: Vec<u16>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub degraded: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// What one Q&A corpus measured (EVAL-4, ADR-0047).
#[derive(Debug, Default, Serialize)]
pub struct QaOutcome {
    pub corpus: String,
    pub reader: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub passed: bool,
    /// The climbs this corpus made to exist at all, each naming the tier
    /// it reached and the commit that carried it. A per-scope answer rate
    /// is an assertion about FLOW-5 as much as about CTX-2, and this is
    /// where that shows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub promotions: Vec<String>,
    pub questions: Vec<QuestionOutcome>,
    /// Questions this run could not measure, because the configured
    /// embedder cannot rank a paraphrase.
    pub skipped_semantic: usize,
    /// How many records the reader is served in total, from its own
    /// sweep. The denominator that says whether a block was bound, and
    /// the number that makes a block's record count readable: 8 of 12 is
    /// a ranking decision, 12 of 12 is the whole corpus fitting.
    pub served_records: usize,
    /// How long the whole corpus took to become composable. Reported,
    /// never gated: MEM-3's lag and EVAL-6's to bound.
    pub seed_wait_ms: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

impl QaOutcome {
    #[must_use]
    pub fn new(corpus: &crate::qa::Corpus) -> Self {
        Self {
            corpus: corpus.corpus.clone(),
            reader: corpus.reader.clone(),
            note: corpus.note.clone(),
            ..Self::default()
        }
    }
}

/// One disclosure that should not have happened (EVAL-5, ADR-0048).
/// Every field is here because a leak nobody can reproduce is a leak
/// nobody acts on: the probe index and the query are enough to re-run the
/// run's first N probes and see it again.
#[derive(Debug, Serialize)]
pub struct Leak {
    /// `sensitivity` | `scope` | `tenant` — derived per (record, reader)
    /// pair, never declared, so a corpus author cannot file a leak under
    /// the wrong axis (ADR-0048 decision 12's note).
    pub boundary: String,
    pub record: String,
    pub reader: String,
    pub surface: String,
    /// Which grader fired: `identity`, `content`, or both. A content-only
    /// hit is a block that rendered bytes its watermark does not name,
    /// which is a different defect with a different owner (decision 6).
    pub predicate: String,
    pub probe: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// One line of a composed block that is neither the renderer's own
/// furniture nor an entry it accounts for (ADR-0048 decision 9).
#[derive(Debug, Serialize)]
pub struct Unattributed {
    pub reader: String,
    pub probe: usize,
    pub line: String,
    pub block_hash: String,
}

/// What one security corpus measured.
#[derive(Debug, Default, Serialize)]
pub struct SecurityOutcome {
    pub corpus: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub passed: bool,
    /// How the corpus got its premise: the climbs and the
    /// reclassifications, each naming what it installed. A leak suite
    /// whose material was placed rather than governed asserts that a tier
    /// no product path produced does not cross a boundary no product path
    /// opened (decision 7).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub premise: Vec<String>,
    /// The generated space, and how much of it this run asked. Both,
    /// because a floor on the second is meaningless without the first.
    pub variants_generated: usize,
    pub variants_asked: usize,
    /// Probes issued, per surface and in total — the denominator every
    /// zero above sits on.
    pub probes: usize,
    pub probes_by_surface: BTreeMap<String, usize>,
    /// Declared-readable (record, reader) pairs, and how many actually
    /// reached their reader over the whole run. The positive control:
    /// without it a run of zeros is indistinguishable from an empty
    /// corpus (decision 4).
    pub controls_expected: usize,
    pub controls_met: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub controls_missed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub leaks: Vec<Leak>,
    /// Leaks the content grader saw and the identity grader did not — a
    /// block carrying material its own watermark does not name.
    pub watermark_gaps: usize,
    /// Block lines the renderer's vocabulary does not account for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unattributed: Vec<Unattributed>,
    /// Entry lines whose *content* reproduces one of the block's marker
    /// forms — ` [confidential]`, `(recall <id>)` — inside the region the
    /// renderer left to content. Reported and gated by nothing on the
    /// first run (decision 11).
    pub marker_echoes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marker_echo_lines: Vec<String>,
    /// How long the corpus took to seed, classify and climb. Reported,
    /// never gated: MEM-3's lag and EVAL-6's to bound.
    pub seed_wait_ms: f64,
    /// What the probe half cost, which is the number that decides whether
    /// this suite stays sequential (ADR-0048 option 8).
    pub probe_ms: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

impl SecurityOutcome {
    #[must_use]
    pub fn new(corpus: &crate::security::Corpus) -> Self {
        Self {
            corpus: corpus.corpus.clone(),
            note: corpus.note.clone(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub suite: String,
    pub tenant_id: String,
    pub gateway_url: String,
    pub started_at: String,
    /// Which actor sat where. Without it, "the outsider abstained" is a
    /// sentence nobody can check.
    pub actors: BTreeMap<String, String>,
    pub scenarios: Vec<Outcome>,
    /// The extraction suite's groups (EVAL-2). Absent when no corpus ran.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extraction: Vec<ExtractionOutcome>,
    /// The Q&A suite's corpora (EVAL-4). Absent when none ran.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub qa: Vec<QaOutcome>,
    /// The security suite's corpora (EVAL-5). Absent when none ran.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<SecurityOutcome>,
    pub metrics: BTreeMap<String, f64>,
    pub gate: Gate,
}

#[derive(Debug, Serialize)]
pub struct Gate {
    pub passed: bool,
    /// Metrics the baseline bounds, in the order it bounds them.
    pub gated: Vec<String>,
    /// Metrics measured but bounded by nothing — visible on purpose.
    pub ungated: Vec<String>,
    pub breaches: Vec<Breach>,
}

#[derive(Debug, Serialize)]
pub struct Breach {
    pub metric: String,
    pub bound: String,
    pub baseline: Option<f64>,
    pub measured: Option<f64>,
    pub delta: Option<f64>,
    pub reason: String,
}

/// What `--update-baseline` leaves above a measured cost. Half again is
/// wide enough to absorb a loaded CI runner and narrow enough that a
/// doubling still trips — the two things a cost ceiling is for.
const CEILING_HEADROOM: f64 = 1.5;

/// The committed gate (`evals/baseline.json`).
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    /// Why these numbers are these numbers. It is the first thing anyone
    /// updating them should read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub metrics: BTreeMap<String, Bound>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// How far below a measured value `--update-baseline` writes a floor —
    /// EVAL-2's "gate on regression >2pts" as `slack: 0.02` (ADR-0046
    /// decision 9).
    ///
    /// It affects only how a floor is *written*, never how the gate
    /// compares: the tolerance lands in the committed number where a
    /// reviewer sees it, rather than in a comparison nobody reads. A metric
    /// that declares no slack keeps EVAL-1's zero-tolerance behaviour
    /// exactly, which is why every axis that predates this field is
    /// unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slack: Option<f64>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read the baseline {}: {err}", path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid baseline: {err}", path.display()))
    }

    /// Rewrites the file from a run's measurements, keeping each metric's
    /// direction. Deliberate, and a diff someone has to look at.
    ///
    /// A quality floor becomes exactly what was measured — the suite just
    /// achieved it, so it is achievable — unless it declares `slack`, in
    /// which case it lands that far below (ADR-0046 decision 9). A cost
    /// ceiling gets [`CEILING_HEADROOM`] on top, because a ceiling pinned
    /// to the last measurement fails on the next run's jitter, and a gate
    /// that cries wolf nightly is a gate someone turns off.
    pub fn updated(&self, metrics: &BTreeMap<String, f64>) -> Self {
        let mut updated = Baseline {
            note: self.note.clone(),
            metrics: BTreeMap::new(),
        };
        for (metric, bound) in &self.metrics {
            let measured = metrics.get(metric).copied();
            updated.metrics.insert(
                metric.clone(),
                match (measured, bound.min, bound.max) {
                    (Some(value), Some(_), None) => Bound {
                        // Never below zero: a slack wider than the
                        // measurement would write a floor no run can fail.
                        min: Some(round((value - bound.slack.unwrap_or(0.0)).max(0.0))),
                        max: None,
                        slack: bound.slack,
                    },
                    (Some(value), None, Some(_)) => Bound {
                        min: None,
                        max: Some(round(value * CEILING_HEADROOM)),
                        slack: bound.slack,
                    },
                    // Two-sided or unmeasured: leave it exactly as it was
                    // rather than guess which side moved.
                    _ => *bound,
                },
            );
        }
        updated
    }
}

/// The five axes, plus one per declared category, reduced from the
/// scenario outcomes.
pub fn metrics(outcomes: &[Outcome]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if outcomes.is_empty() {
        return metrics;
    }

    let accuracy: f64 = outcomes.iter().map(|outcome| outcome.accuracy).sum();
    metrics.insert("accuracy".to_owned(), accuracy / outcomes.len() as f64);

    // One axis per capability family the suite declares (ADR-0039
    // decision 14). Only over the scenarios that measure it — the same rule
    // the recall and abstention axes follow, and what lets a gate say
    // "knowledge_update fell to 0.5" instead of averaging the regression
    // away across a suite that mostly measures something else.
    let mut by_category: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for outcome in outcomes {
        if let Some(category) = &outcome.category {
            by_category
                .entry(crate::scenario::metric_name(category))
                .or_default()
                .push(outcome.accuracy);
        }
    }
    for (metric, scores) in by_category {
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;
        metrics.insert(metric, mean);
    }

    let recalls: Vec<f64> = outcomes
        .iter()
        .filter_map(|outcome| outcome.recall)
        .collect();
    if !recalls.is_empty() {
        metrics.insert(
            "recall".to_owned(),
            recalls.iter().sum::<f64>() / recalls.len() as f64,
        );
    }

    let abstentions: Vec<f64> = outcomes
        .iter()
        .filter_map(|outcome| outcome.abstained)
        .map(|abstained| if abstained { 1.0 } else { 0.0 })
        .collect();
    if !abstentions.is_empty() {
        metrics.insert(
            "abstention".to_owned(),
            abstentions.iter().sum::<f64>() / abstentions.len() as f64,
        );
    }

    let tokens: Vec<f64> = outcomes
        .iter()
        .map(|outcome| f64::from(outcome.tokens))
        .collect();
    metrics.insert(
        "tokens_mean".to_owned(),
        tokens.iter().sum::<f64>() / tokens.len() as f64,
    );
    metrics.insert(
        "tokens_max".to_owned(),
        tokens.iter().copied().fold(0.0, f64::max),
    );

    let mut latencies: Vec<f64> = outcomes
        .iter()
        .flat_map(|outcome| outcome.latency_ms.iter().copied())
        .collect();
    if !latencies.is_empty() {
        latencies.sort_by(f64::total_cmp);
        metrics.insert("latency_p50_ms".to_owned(), percentile(&latencies, 50.0));
        metrics.insert("latency_p95_ms".to_owned(), percentile(&latencies, 95.0));
    }

    metrics
        .iter()
        .map(|(k, v)| (k.clone(), round(*v)))
        .collect()
}

/// Nearest-rank on a sorted sample. Boring on purpose: an eval that
/// interpolates its own percentiles invites an argument about the
/// interpolation instead of about the regression.
pub fn percentile(sorted: &[f64], percent: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (percent / 100.0 * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

pub fn gate(baseline: &Baseline, metrics: &BTreeMap<String, f64>) -> Gate {
    let mut breaches = Vec::new();
    let gated: Vec<String> = baseline.metrics.keys().cloned().collect();
    let ungated: Vec<String> = metrics
        .keys()
        .filter(|metric| !baseline.metrics.contains_key(*metric))
        .cloned()
        .collect();

    for (metric, bound) in &baseline.metrics {
        let Some(&measured) = metrics.get(metric) else {
            // A bounded metric that stopped being measured is a breach,
            // not a pass. This is how a suite quietly loses coverage.
            breaches.push(Breach {
                metric: metric.clone(),
                bound: describe(bound),
                baseline: bound.min.or(bound.max),
                measured: None,
                delta: None,
                reason: format!(
                    "`{metric}` is bounded by the baseline but this run measured it nowhere"
                ),
            });
            continue;
        };
        if let Some(min) = bound.min
            && measured < min
        {
            breaches.push(Breach {
                metric: metric.clone(),
                bound: format!("min {}", round(min)),
                baseline: Some(min),
                measured: Some(measured),
                delta: Some(round(measured - min)),
                reason: format!(
                    "`{metric}` fell to {} against a floor of {}",
                    round(measured),
                    round(min)
                ),
            });
        }
        if let Some(max) = bound.max
            && measured > max
        {
            breaches.push(Breach {
                metric: metric.clone(),
                bound: format!("max {}", round(max)),
                baseline: Some(max),
                measured: Some(measured),
                delta: Some(round(measured - max)),
                reason: format!(
                    "`{metric}` rose to {} against a ceiling of {}",
                    round(measured),
                    round(max)
                ),
            });
        }
    }

    Gate {
        passed: breaches.is_empty(),
        gated,
        ungated,
        breaches,
    }
}

fn describe(bound: &Bound) -> String {
    match (bound.min, bound.max) {
        (Some(min), Some(max)) => format!("min {} / max {}", round(min), round(max)),
        (Some(min), None) => format!("min {}", round(min)),
        (None, Some(max)) => format!("max {}", round(max)),
        (None, None) => "unbounded".to_owned(),
    }
}

/// Three decimals is more than any of these axes means, and it keeps the
/// committed baseline from churning on floating-point noise.
fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// The human summary. Goes to stderr so the JSON report keeps stdout.
pub fn summarise(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "eval: {} scenarios against {}\n",
        report.scenarios.len(),
        report.gateway_url
    ));
    for outcome in &report.scenarios {
        out.push_str(&format!(
            "  {} {}\n",
            if outcome.passed { "✓" } else { "✗" },
            outcome.name
        ));
        for failure in &outcome.failures {
            out.push_str(&format!("      {failure}\n"));
        }
        if !outcome.degraded.is_empty() {
            out.push_str(&format!(
                "      served degraded: {}\n",
                outcome.degraded.join(", ")
            ));
        }
    }
    if !report.extraction.is_empty() {
        out.push_str(&extraction_summary(&report.extraction));
    }
    if !report.qa.is_empty() {
        out.push_str(&qa_summary(&report.qa));
    }
    if !report.security.is_empty() {
        out.push_str(&security_summary(&report.security));
    }
    out.push_str("\n  axis                       measured   gate\n");
    for (metric, value) in &report.metrics {
        let bound = if report.gate.gated.contains(metric) {
            report
                .gate
                .breaches
                .iter()
                .find(|breach| &breach.metric == metric)
                .map_or_else(
                    || "held".to_owned(),
                    |breach| format!("BREACH ({})", breach.bound),
                )
        } else {
            "reported only".to_owned()
        };
        out.push_str(&format!("  {metric:<26} {value:>8.3}   {bound}\n"));
    }
    for breach in &report.gate.breaches {
        out.push_str(&format!("\n  gate: {}\n", breach.reason));
    }
    out.push_str(&format!(
        "\n  {}\n",
        if report.gate.passed {
            "gate held".to_owned()
        } else {
            format!("gate FAILED on {} metric(s)", report.gate.breaches.len())
        }
    ));
    out
}

/// The per-class table EVAL-2's AC calls a dashboard (ADR-0046
/// decision 11): what was labelled, what the pipeline produced, what
/// matched, and the attribution column that keeps a withheld record from
/// reading as a missed extraction.
fn extraction_summary(groups: &[ExtractionOutcome]) -> String {
    let mut out = String::new();
    let fixtures: usize = groups.iter().map(|group| group.fixtures).sum();
    out.push_str(&format!(
        "\n  extraction: {fixtures} fixtures across {} group(s)\n",
        groups.len()
    ));
    for group in groups {
        out.push_str(&format!(
            "  {} {} ({}): {} committed → {} served",
            if group.passed { "✓" } else { "✗" },
            group.group,
            group.actor,
            group.committed_records,
            group.served_records
        ));
        if group.merged_records > 0 {
            out.push_str(&format!(", {} merged", group.merged_records));
        }
        if group.chain_to > 0 {
            out.push_str(&format!(
                " (chain {}..{})",
                group.chain_from, group.chain_to
            ));
        }
        out.push('\n');
        for failure in &group.failures {
            out.push_str(&format!("      {failure}\n"));
        }
        for hit in &group.bait_hits {
            out.push_str(&format!("      {hit}\n"));
        }
    }

    let mut totals: BTreeMap<&str, ClassCounts> = BTreeMap::new();
    for group in groups {
        for (class, counts) in &group.per_class {
            let slot = totals.entry(class.as_str()).or_default();
            slot.expected += counts.expected;
            slot.produced += counts.produced;
            slot.matched += counts.matched;
        }
    }
    out.push_str("\n  class        precision          recall\n");
    for (class, counts) in &totals {
        let ratio = |hit: usize, total: usize| {
            if total == 0 {
                "     —".to_owned()
            } else {
                format!("{hit}/{total} = {:.3}", hit as f64 / total as f64)
            }
        };
        out.push_str(&format!(
            "  {:<12} {:<18} {}\n",
            class,
            ratio(counts.matched, counts.produced),
            ratio(counts.matched, counts.expected)
        ));
    }

    // Misses the corpus predicted, before the ones it did not: a reader
    // scanning this table needs to know which numbers are already
    // explained.
    let noted: Vec<&str> = groups
        .iter()
        .flat_map(|group| group.noted_misses.iter().map(String::as_str))
        .collect();
    if !noted.is_empty() {
        out.push_str("\n  misses the corpus predicted, with the reason it gave:\n");
        for entry in noted {
            out.push_str(&format!("    {entry}\n"));
        }
    }

    let unmatched: usize = groups.iter().map(|group| group.unmatched.len()).sum();
    if unmatched > 0 {
        out.push_str(&format!(
            "\n  {unmatched} record(s) matched no expectation — the review queue for \
             unanticipated invention:\n"
        ));
        for group in groups {
            for entry in &group.unmatched {
                out.push_str(&format!("    {entry}\n"));
            }
        }
    }
    let models: Vec<&str> = groups
        .iter()
        .flat_map(|group| group.model_versions.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !models.is_empty() {
        out.push_str(&format!("\n  extracted by: {}\n", models.join(", ")));
    }
    out
}

/// The per-tier table EVAL-4's AC asks for (ADR-0047 decisions 8 and 9):
/// what each scope tier was asked for, what reached the reader, and what
/// reached it whole — plus the climbs that put the material there, because
/// a per-scope answer rate is an assertion about FLOW-5 as much as about
/// composition.
fn qa_summary(corpora: &[QaOutcome]) -> String {
    let mut out = String::new();
    let questions: usize = corpora.iter().map(|corpus| corpus.questions.len()).sum();
    out.push_str(&format!(
        "\n  qa: {questions} question(s) across {} corpus/corpora\n",
        corpora.len()
    ));
    for corpus in corpora {
        out.push_str(&format!(
            "  {} {} (read by {}): {} record(s) served",
            if corpus.passed { "✓" } else { "✗" },
            corpus.corpus,
            corpus.reader,
            corpus.served_records
        ));
        if corpus.skipped_semantic > 0 {
            out.push_str(&format!(
                ", {} semantic question(s) skipped — this run's embedder cannot rank a paraphrase",
                corpus.skipped_semantic
            ));
        }
        out.push('\n');
        for promotion in &corpus.promotions {
            out.push_str(&format!("      climbed: {promotion}\n"));
        }
        for failure in &corpus.failures {
            out.push_str(&format!("      {failure}\n"));
        }
        for question in &corpus.questions {
            for failure in &question.failures {
                out.push_str(&format!("      {}: {failure}\n", question.name));
            }
        }
    }

    let mut totals: BTreeMap<&str, TierCounts> = BTreeMap::new();
    for question in corpora
        .iter()
        .flat_map(|corpus| corpus.questions.iter())
        .filter(|question| !question.skipped)
    {
        for (tier, counts) in &question.per_tier {
            let slot = totals.entry(tier.as_str()).or_default();
            slot.expected += counts.expected;
            slot.reached += counts.reached;
            slot.body += counts.body;
        }
    }
    out.push_str("\n  scope tier   reached            whole\n");
    for (tier, counts) in &totals {
        let ratio = |hit: usize| {
            if counts.expected == 0 {
                "     —".to_owned()
            } else {
                format!(
                    "{hit}/{} = {:.3}",
                    counts.expected,
                    hit as f64 / counts.expected as f64
                )
            }
        };
        out.push_str(&format!(
            "  {:<12} {:<18} {}\n",
            tier,
            ratio(counts.reached),
            ratio(counts.body)
        ));
    }

    // Demotions before misses: a record the index tier named is a
    // different fact from one the block never carried, and a reader
    // scanning this needs to know which is which.
    let demoted: Vec<&str> = corpora
        .iter()
        .flat_map(|corpus| corpus.questions.iter())
        .flat_map(|question| question.demoted.iter().map(String::as_str))
        .collect();
    if !demoted.is_empty() {
        out.push_str(&format!(
            "\n  {} expected record(s) reached the reader as index lines rather than bodies — \
             named, not carried:\n    {}\n",
            demoted.len(),
            demoted.join(", ")
        ));
    }
    let noted: Vec<String> = corpora
        .iter()
        .flat_map(|corpus| corpus.questions.iter())
        .filter(|question| !question.note.is_empty() && !question.passed && !question.skipped)
        .map(|question| format!("{} — {}", question.name, question.note))
        .collect();
    if !noted.is_empty() {
        out.push_str("\n  misses the corpus predicted, with the reason it gave:\n");
        for entry in noted {
            out.push_str(&format!("    {entry}\n"));
        }
    }
    out
}

/// The table EVAL-5's AC asks for (ADR-0048): what the run asked, over
/// which surfaces, what it disclosed, and — first, because every zero
/// below depends on it — that the material a reader was *supposed* to
/// have actually reached them.
fn security_summary(corpora: &[SecurityOutcome]) -> String {
    let mut out = String::new();
    let probes: usize = corpora.iter().map(|corpus| corpus.probes).sum();
    out.push_str(&format!(
        "\n  security: {probes} probe(s) across {} corpus/corpora\n",
        corpora.len()
    ));
    for corpus in corpora {
        out.push_str(&format!(
            "  {} {}: {} of {} variant(s) asked over {} probe(s) in {:.0}ms\n",
            if corpus.passed { "✓" } else { "✗" },
            corpus.corpus,
            corpus.variants_asked,
            corpus.variants_generated,
            corpus.probes,
            corpus.probe_ms
        ));
        for entry in &corpus.premise {
            out.push_str(&format!("      governed: {entry}\n"));
        }
        out.push_str(&format!(
            "      controls {}/{} — material a reader is supposed to have, that reached them\n",
            corpus.controls_met, corpus.controls_expected
        ));
        for missed in &corpus.controls_missed {
            out.push_str(&format!("        missed: {missed}\n"));
        }
        let surfaces: Vec<String> = corpus
            .probes_by_surface
            .iter()
            .map(|(surface, count)| format!("{surface} {count}"))
            .collect();
        if !surfaces.is_empty() {
            out.push_str(&format!("      surfaces: {}\n", surfaces.join(", ")));
        }
        for failure in &corpus.failures {
            out.push_str(&format!("      {failure}\n"));
        }
        // Leaks before anything else a reader might scan past. A count in
        // the axis table says a boundary broke; this says which record
        // reached whom, under what phrasing, and at which probe — the
        // three things needed to reproduce it.
        for leak in &corpus.leaks {
            out.push_str(&format!(
                "      LEAK [{}] {} → {} via {} ({}) at probe {}{}\n",
                leak.boundary,
                leak.record,
                leak.reader,
                leak.surface,
                leak.predicate,
                leak.probe,
                leak.query
                    .as_ref()
                    .map(|query| format!(" asking {query:?}"))
                    .unwrap_or_default()
            ));
        }
        for line in &corpus.unattributed {
            out.push_str(&format!(
                "      UNATTRIBUTED LINE in {}'s block {}: {:?}\n",
                line.reader,
                line.block_hash.chars().take(12).collect::<String>(),
                line.line
            ));
        }
        if corpus.marker_echoes > 0 {
            out.push_str(&format!(
                "      {} entry line(s) whose content reproduces a marker form — reported, \
                 bounded by nothing (ADR-0048 decision 11):\n",
                corpus.marker_echoes
            ));
            for line in &corpus.marker_echo_lines {
                out.push_str(&format!("        {line}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        accuracy: f64,
        recall: Option<f64>,
        abstained: Option<bool>,
        tokens: u32,
    ) -> Outcome {
        Outcome {
            name: "scenario".to_owned(),
            description: String::new(),
            category: None,
            passed: accuracy == 1.0,
            accuracy,
            recall,
            abstained,
            tokens,
            budget_tokens: 1500,
            block_hash: "b3-test".to_owned(),
            latency_ms: vec![10.0, 20.0, 30.0, 40.0],
            seed_wait_ms: 0.0,
            degraded: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn categorised(category: &str, accuracy: f64) -> Outcome {
        Outcome {
            category: Some(category.to_owned()),
            ..outcome(accuracy, None, None, 100)
        }
    }

    fn baseline(metrics: &[(&str, Bound)]) -> Baseline {
        Baseline {
            note: String::new(),
            metrics: metrics
                .iter()
                .map(|(name, bound)| ((*name).to_owned(), *bound))
                .collect(),
        }
    }

    fn min(value: f64) -> Bound {
        Bound {
            min: Some(value),
            max: None,
            slack: None,
        }
    }

    fn max(value: f64) -> Bound {
        Bound {
            min: None,
            max: Some(value),
            slack: None,
        }
    }

    fn min_with_slack(value: f64, slack: f64) -> Bound {
        Bound {
            min: Some(value),
            max: None,
            slack: Some(slack),
        }
    }

    /// A category is its own axis over its own scenarios (ADR-0039
    /// decision 14): the point is that a regression in one capability
    /// family cannot be averaged away by a suite that mostly measures
    /// something else.
    #[test]
    fn a_category_reduces_over_its_own_scenarios_only() {
        let metrics = metrics(&[
            categorised("knowledge-update", 0.0),
            categorised("knowledge-update", 1.0),
            outcome(1.0, None, None, 100),
            outcome(1.0, None, None, 100),
        ]);
        assert_eq!(metrics.get("knowledge_update"), Some(&0.5));
        assert_eq!(metrics.get("accuracy"), Some(&0.75), "the suite's own axis");
        assert!(
            !metrics.contains_key("knowledge-update"),
            "the name is folded"
        );

        // A suite with no categories grows no category axes.
        let plain = metrics_of_plain();
        assert!(plain.keys().all(|metric| metric != "knowledge_update"));
    }

    fn metrics_of_plain() -> BTreeMap<String, f64> {
        metrics(&[outcome(1.0, Some(1.0), None, 100)])
    }

    /// The gate names the category, which is the whole reason a category
    /// is an axis rather than a label in the report.
    #[test]
    fn a_category_floor_breaches_naming_the_category() {
        let measured = metrics(&[
            categorised("knowledge-update", 0.0),
            categorised("knowledge-update", 1.0),
        ]);
        let gate = gate(&baseline(&[("knowledge_update", min(1.0))]), &measured);
        assert!(!gate.passed);
        assert_eq!(gate.breaches.len(), 1);
        assert_eq!(gate.breaches[0].metric, "knowledge_update");
        assert_eq!(gate.breaches[0].measured, Some(0.5));
        assert_eq!(gate.breaches[0].delta, Some(-0.5));
    }

    #[test]
    fn each_axis_averages_only_the_scenarios_that_measure_it() {
        let outcomes = vec![
            outcome(1.0, Some(1.0), None, 100),
            outcome(0.0, Some(0.5), None, 300),
            // An abstention scenario recalls nothing by definition; it
            // must not drag the recall average to zero.
            outcome(1.0, None, Some(true), 0),
        ];
        let metrics = metrics(&outcomes);
        assert!((metrics["accuracy"] - 0.667).abs() < 0.001);
        assert!((metrics["recall"] - 0.75).abs() < 0.001);
        assert!((metrics["abstention"] - 1.0).abs() < f64::EPSILON);
        assert!((metrics["tokens_mean"] - 133.333).abs() < 0.001);
        assert!((metrics["tokens_max"] - 300.0).abs() < f64::EPSILON);
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let samples = [10.0, 20.0, 30.0, 40.0];
        assert!((percentile(&samples, 50.0) - 20.0).abs() < f64::EPSILON);
        assert!((percentile(&samples, 95.0) - 40.0).abs() < f64::EPSILON);
        assert!((percentile(&samples, 0.0) - 10.0).abs() < f64::EPSILON);
        assert!((percentile(&[], 95.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn a_floor_breach_names_the_axis_and_the_delta() {
        let metrics = metrics(&[outcome(0.0, Some(0.5), None, 100)]);
        let gate = gate(&baseline(&[("recall", min(1.0))]), &metrics);
        assert!(!gate.passed);
        let breach = &gate.breaches[0];
        assert_eq!(breach.metric, "recall");
        assert_eq!(breach.measured, Some(0.5));
        assert_eq!(breach.delta, Some(-0.5));
        assert!(
            breach.reason.contains("floor"),
            "unclear: {}",
            breach.reason
        );
    }

    #[test]
    fn a_ceiling_breach_reads_the_other_way_round() {
        let metrics = metrics(&[outcome(1.0, Some(1.0), None, 900)]);
        let gate = gate(&baseline(&[("tokens_mean", max(400.0))]), &metrics);
        assert!(!gate.passed);
        assert_eq!(gate.breaches[0].delta, Some(500.0));
        assert!(gate.breaches[0].reason.contains("ceiling"));
    }

    #[test]
    fn a_bounded_metric_that_stopped_being_measured_is_a_breach() {
        // The quiet way a suite loses coverage: the scenarios that fed an
        // axis are removed, the axis disappears, and every remaining
        // number still looks fine.
        let metrics = metrics(&[outcome(1.0, None, None, 100)]);
        let gate = gate(&baseline(&[("recall", min(1.0))]), &metrics);
        assert!(!gate.passed);
        assert_eq!(gate.breaches[0].measured, None);
        assert!(gate.breaches[0].reason.contains("measured it nowhere"));
    }

    #[test]
    fn metrics_the_baseline_does_not_bound_are_reported_and_named() {
        let metrics = metrics(&[outcome(1.0, Some(1.0), None, 100)]);
        let gate = gate(&baseline(&[("accuracy", min(1.0))]), &metrics);
        assert!(gate.passed);
        assert_eq!(gate.gated, vec!["accuracy"]);
        assert!(gate.ungated.contains(&"tokens_mean".to_owned()));
        assert!(gate.ungated.contains(&"latency_p95_ms".to_owned()));
    }

    #[test]
    fn updating_a_baseline_keeps_each_bound_on_its_own_side() {
        let before = baseline(&[("recall", min(1.0)), ("tokens_mean", max(400.0))]);
        let metrics = metrics(&[outcome(1.0, Some(0.8), None, 500)]);
        let after = before.updated(&metrics);
        // A floor lands on what the suite just achieved…
        assert_eq!(after.metrics["recall"].min, Some(0.8));
        assert_eq!(after.metrics["recall"].max, None);
        // …and a ceiling lands above what it just cost, or the next run's
        // jitter fails a gate that measured nothing new.
        assert_eq!(after.metrics["tokens_mean"].max, Some(750.0));
        assert_eq!(after.metrics["tokens_mean"].min, None);
    }

    /// EVAL-2's "gate on regression >2pts" (ADR-0046 decision 9): slack
    /// changes how a floor is *written* and nothing about how it is
    /// compared, and it carries forward so the tolerance stays declared.
    #[test]
    fn a_declared_slack_writes_the_floor_that_far_below_the_measurement() {
        let before = baseline(&[
            ("extraction_precision_macro", min_with_slack(0.9, 0.02)),
            ("recall", min(1.0)),
        ]);
        let measured: BTreeMap<String, f64> = [
            ("extraction_precision_macro".to_owned(), 0.983),
            ("recall".to_owned(), 1.0),
        ]
        .into_iter()
        .collect();
        let after = before.updated(&measured);
        assert_eq!(after.metrics["extraction_precision_macro"].min, Some(0.963));
        assert_eq!(
            after.metrics["extraction_precision_macro"].slack,
            Some(0.02),
            "the tolerance has to survive the rewrite or the next update loses it"
        );
        // An axis that declares no slack is untouched by the feature.
        assert_eq!(after.metrics["recall"].min, Some(1.0));
        assert_eq!(after.metrics["recall"].slack, None);
    }

    #[test]
    fn a_slack_wider_than_the_measurement_writes_a_floor_of_zero() {
        // Never a negative floor: that would be a gate no run can fail,
        // which is worse than a gate nobody set.
        let before = baseline(&[("hallucination_rate", min_with_slack(0.5, 0.9))]);
        let measured: BTreeMap<String, f64> = [("hallucination_rate".to_owned(), 0.1)]
            .into_iter()
            .collect();
        let after = before.updated(&measured);
        assert_eq!(after.metrics["hallucination_rate"].min, Some(0.0));
    }

    /// The gate is unchanged by slack: it compares against `min` exactly as
    /// committed, so a run that dips below the written floor still fails.
    #[test]
    fn the_gate_reads_the_written_floor_and_not_the_slack() {
        let bounded = baseline(&[("extraction_recall_macro", min_with_slack(0.9, 0.02))]);
        let measured: BTreeMap<String, f64> = [("extraction_recall_macro".to_owned(), 0.89)]
            .into_iter()
            .collect();
        let gate = gate(&bounded, &measured);
        assert!(!gate.passed, "0.89 is under the written floor of 0.9");
        assert_eq!(gate.breaches[0].measured, Some(0.89));
    }
}
