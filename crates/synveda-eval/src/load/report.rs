//! The content-free load report, its atomic checkpoint, budget assessment
//! and the reviewed-baseline ledger check.
//!
//! A report holds operation names, outcome counts, latency quantiles,
//! server-side metric deltas, process and database samples. It never holds
//! a bearer, request body, Knowledge text or identifier of tenant content.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::observe::{ProcessSample, ServerQuantiles};
use super::profile::Profile;
use super::stats::{Measure, OperationSummary, Outcome, Saved};

pub const SCHEMA: &str = "synveda.load-report.v1";
pub const LEDGER_SCHEMA: &str = "synveda.load-baseline-ledger.v1";

/// Server signals every phase must show, so a renamed or dropped metric
/// fails the smoke instead of silently emptying the report.
const REQUIRED_GATEWAY_HISTOGRAMS: [&str; 2] = [
    "synveda_http_request_duration_seconds",
    "synveda_db_pool_acquire_seconds",
];
const REQUIRED_GAUGE: &str = "synveda_db_pool_connections";

/// Facts that bound what a report may claim. Supplied by the operator
/// wrapper; the harness does not guess hardware.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub source_commit: String,
    pub product_build: String,
    pub topology: String,
    pub hardware: String,
    pub database: String,
    pub embedding: String,
    pub identities: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderWindow {
    pub requests: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub index: u64,
    pub offset_seconds: f64,
    pub seconds: f64,
    pub measured: BTreeMap<String, OperationSummary>,
    pub noisy: BTreeMap<String, OperationSummary>,
    pub gateway: BTreeMap<String, ServerQuantiles>,
    pub worker: BTreeMap<String, ServerQuantiles>,
    pub gauges: BTreeMap<String, f64>,
    pub provider: ProviderWindow,
    pub processes: BTreeMap<String, ProcessSample>,
    pub database: Option<Value>,
    pub unsent: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhaseReport {
    pub name: String,
    pub completed: bool,
    pub windows: Vec<Window>,
    pub measured: BTreeMap<String, OperationSummary>,
    pub noisy: BTreeMap<String, OperationSummary>,
    /// Cumulative histogram state, so a resumed phase keeps its tails.
    pub saved_measured: Saved,
    pub saved_noisy: Saved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub profile_digest: String,
    pub profile: Profile,
    pub environment: Environment,
    pub started_at: String,
    pub updated_at: String,
    pub status: Status,
    pub resumed: u32,
    pub phases: Vec<PhaseReport>,
    pub assessment: Vec<Assessment>,
}

/// Content digest of the canonical profile JSON.
pub fn profile_digest(profile: &Profile) -> Result<String, String> {
    let bytes = serde_json::to_vec(profile).map_err(|err| format!("digest profile: {err}"))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

impl Report {
    pub fn new(profile: &Profile, environment: Environment) -> Result<Self, String> {
        let now = chrono::Utc::now().to_rfc3339();
        Ok(Self {
            schema: SCHEMA.to_owned(),
            profile_digest: profile_digest(profile)?,
            profile: profile.clone(),
            environment,
            started_at: now.clone(),
            updated_at: now,
            status: Status::Running,
            resumed: 0,
            phases: Vec::new(),
            assessment: Vec::new(),
        })
    }

    /// Loads a checkpoint to resume. The profile must be byte-for-byte the
    /// one the checkpoint ran, so a resumed run cannot change its duration,
    /// rates or seed.
    pub fn resume(path: &Path, profile: &Profile) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read checkpoint {}: {err}", path.display()))?;
        let mut report: Self = serde_json::from_str(&raw)
            .map_err(|err| format!("parse checkpoint {}: {err}", path.display()))?;
        if report.schema != SCHEMA {
            return Err(format!("checkpoint schema must be {SCHEMA}"));
        }
        if report.profile_digest != profile_digest(profile)? || report.profile != *profile {
            return Err("the checkpoint was produced by a different profile".to_owned());
        }
        if report.status == Status::Completed {
            return Err("the checkpoint is already complete".to_owned());
        }
        report.resumed += 1;
        Ok(report)
    }

    /// Writes through a sibling temporary file and a rename, so an
    /// interrupted run always leaves the previous complete checkpoint.
    pub fn write(&mut self, path: &Path) -> Result<(), String> {
        self.updated_at = chrono::Utc::now().to_rfc3339();
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| format!("serialise load report: {err}"))?;
        let temporary = path.with_extension("json.partial");
        std::fs::write(&temporary, format!("{json}\n"))
            .map_err(|err| format!("write {}: {err}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .map_err(|err| format!("replace {}: {err}", path.display()))
    }
}

/// One budget comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    pub phase: String,
    pub check: String,
    pub observed: f64,
    pub budget: f64,
    pub n: u64,
    pub passed: bool,
}

/// Unexpected outcomes per thousand across every measured operation.
pub fn unexpected_rate(operations: &BTreeMap<String, OperationSummary>) -> (f64, u64) {
    let mut total = 0;
    let mut unexpected = 0;
    for summary in operations.values() {
        for (outcome, count) in &summary.outcomes {
            total += count;
            if matches!(
                outcome,
                Outcome::Unexpected | Outcome::Timeout | Outcome::Transport
            ) {
                unexpected += count;
            }
        }
    }
    if total == 0 {
        return (0.0, 0);
    }
    (unexpected as f64 * 1_000.0 / total as f64, total)
}

fn response(operations: &BTreeMap<String, OperationSummary>, op: &str) -> Option<(f64, f64, u64)> {
    let quantiles = operations.get(op)?.latency.get(&Measure::Response)?;
    Some((quantiles.p95_ms, quantiles.p99_ms, quantiles.n))
}

/// Compares every assessed phase with the profile's proposed budgets.
///
/// Only completed phases the profile marks `assess` are judged; profile
/// validation already refuses an assessed warm-up or faulted phase, and a
/// 10x phase is reported, not assessed. Isolation is assessed in every
/// phase.
pub fn assess(report: &Report) -> Vec<Assessment> {
    let budgets = &report.profile.budgets;
    let mut out = Vec::new();
    for (phase, measured) in report.profile.phases.iter().zip(&report.phases) {
        let breaches: u64 = measured
            .measured
            .get("isolation_breach")
            .map_or(0, |summary| summary.outcomes.values().sum());
        out.push(Assessment {
            phase: phase.name.clone(),
            check: "cross_tenant_isolation_breaches".to_owned(),
            observed: breaches as f64,
            budget: 0.0,
            n: measured
                .measured
                .get("denied_probe")
                .map_or(0, |summary| summary.outcomes.values().sum()),
            passed: breaches == 0,
        });
        let assessed = measured.completed && phase.assess;
        if !assessed {
            continue;
        }
        let mut check = |name: &str, observed: Option<(f64, u64)>, budget: f64| {
            let (value, n) = observed.unwrap_or((f64::INFINITY, 0));
            out.push(Assessment {
                phase: phase.name.clone(),
                check: name.to_owned(),
                observed: value,
                budget,
                n,
                passed: n > 0 && value <= budget,
            });
        };
        let ops = &measured.measured;
        check(
            "append_p95_ms",
            response(ops, "append").map(|(p95, _, n)| (p95, n)),
            budgets.append_p95_ms,
        );
        check(
            "append_p99_ms",
            response(ops, "append").map(|(_, p99, n)| (p99, n)),
            budgets.append_p99_ms,
        );
        check(
            "context_run_p95_ms",
            response(ops, "context_run").map(|(p95, _, n)| (p95, n)),
            budgets.context_run_p95_ms,
        );
        check(
            "context_run_p99_ms",
            response(ops, "context_run").map(|(_, p99, n)| (p99, n)),
            budgets.context_run_p99_ms,
        );
        check(
            "knowledge_search_p99_ms",
            response(ops, "knowledge_search").map(|(_, p99, n)| (p99, n)),
            budgets.knowledge_search_p99_ms,
        );
        let (rate, total) = unexpected_rate(ops);
        check(
            "unexpected_per_thousand",
            Some((rate, total)),
            budgets.unexpected_per_thousand,
        );
    }
    out
}

/// One reviewed baseline in the ledger: the file name, its BLAKE3 digest
/// and who reviewed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    pub file: String,
    pub blake3: String,
    pub profile_digest: String,
    pub reviewed_by: String,
    pub reviewed_on: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub schema: String,
    pub baselines: Vec<LedgerEntry>,
}

/// Refuses a report that is not a complete, assessable run of a known
/// profile, and any baseline whose bytes no longer match its reviewed
/// ledger entry.
pub fn check(
    report: &Report,
    ledger: &Ledger,
    baselines: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let mut problems = Vec::new();
    if report.schema != SCHEMA {
        problems.push(format!("report schema must be {SCHEMA}"));
    }
    if report.status != Status::Completed {
        problems.push("the run did not complete".to_owned());
    }
    if profile_digest(&report.profile).ok().as_deref() != Some(report.profile_digest.as_str()) {
        problems.push("the embedded profile does not match its digest".to_owned());
    }
    for (field, value) in [
        ("source_commit", &report.environment.source_commit),
        ("product_build", &report.environment.product_build),
        ("topology", &report.environment.topology),
        ("hardware", &report.environment.hardware),
        ("database", &report.environment.database),
        ("embedding", &report.environment.embedding),
        ("identities", &report.environment.identities),
    ] {
        if value.trim().is_empty() {
            problems.push(format!("environment axis {field} is missing"));
        }
    }
    if report.phases.len() != report.profile.phases.len() {
        problems.push("the report does not cover every profile phase".to_owned());
    }
    for phase in &report.phases {
        phase_problems(phase, &mut problems);
    }
    ledger_problems(ledger, baselines, &mut problems);
    problems
}

/// A phase must have sampled each core operation and observed every
/// required gateway signal at least once.
fn phase_problems(phase: &PhaseReport, problems: &mut Vec<String>) {
    for op in ["append", "context_run", "knowledge_search"] {
        let samples = phase
            .measured
            .get(op)
            .and_then(|summary| summary.latency.get(&Measure::Response))
            .map_or(0, |quantiles| quantiles.n);
        if samples == 0 {
            problems.push(format!("phase {} has no {op} latency samples", phase.name));
        }
    }
    for family in REQUIRED_GATEWAY_HISTOGRAMS {
        if !phase
            .windows
            .iter()
            .any(|window| window.gateway.keys().any(|key| key.starts_with(family)))
        {
            problems.push(format!(
                "phase {} never observed gateway metric {family}",
                phase.name
            ));
        }
    }
    if !phase.windows.iter().any(|window| {
        window
            .gauges
            .keys()
            .any(|key| key.starts_with(REQUIRED_GAUGE))
    }) {
        problems.push(format!(
            "phase {} never observed gauge {REQUIRED_GAUGE}",
            phase.name
        ));
    }
}

/// Every baseline file must have a reviewed ledger entry with matching bytes.
fn ledger_problems(
    ledger: &Ledger,
    baselines: &BTreeMap<String, Vec<u8>>,
    problems: &mut Vec<String>,
) {
    if ledger.schema != LEDGER_SCHEMA {
        problems.push(format!("baseline ledger schema must be {LEDGER_SCHEMA}"));
    }
    for entry in &ledger.baselines {
        match baselines.get(&entry.file) {
            None => problems.push(format!("ledger baseline {} is missing", entry.file)),
            Some(bytes) if blake3::hash(bytes).to_hex().as_str() != entry.blake3 => {
                problems.push(format!("baseline {} changed after review", entry.file));
            }
            Some(_) => {}
        }
        if entry.reviewed_by.trim().is_empty() || entry.reviewed_on.trim().is_empty() {
            problems.push(format!("baseline {} has no reviewer", entry.file));
        }
    }
    for file in baselines.keys() {
        if !ledger.baselines.iter().any(|entry| &entry.file == file) {
            problems.push(format!("baseline {file} is not in the reviewed ledger"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::profile::sample;
    use crate::load::stats::Quantiles;

    fn summary(n: u64, p95: f64, p99: f64, outcomes: &[(Outcome, u64)]) -> OperationSummary {
        OperationSummary {
            outcomes: outcomes.iter().copied().collect(),
            latency: [(
                Measure::Response,
                Quantiles {
                    n,
                    p50_ms: p95 / 2.0,
                    p95_ms: p95,
                    p99_ms: p99,
                    max_ms: p99,
                },
            )]
            .into_iter()
            .collect(),
            unexpected_statuses: BTreeMap::new(),
        }
    }

    fn complete_report() -> Report {
        let profile = sample();
        let mut report = Report::new(
            &profile,
            Environment {
                source_commit: "abc".to_owned(),
                product_build: "release".to_owned(),
                topology: "one gateway, one worker".to_owned(),
                hardware: "unit".to_owned(),
                database: "PostgreSQL 17.11".to_owned(),
                embedding: "stub 1024".to_owned(),
                identities: "service".to_owned(),
                notes: Vec::new(),
            },
        )
        .unwrap();
        let mut measured = BTreeMap::new();
        measured.insert(
            "append".to_owned(),
            summary(100, 10.0, 20.0, &[(Outcome::Ok, 100)]),
        );
        measured.insert(
            "context_run".to_owned(),
            summary(10, 90.0, 120.0, &[(Outcome::Ok, 10)]),
        );
        measured.insert(
            "knowledge_search".to_owned(),
            summary(5, 300.0, 400.0, &[(Outcome::Ok, 5)]),
        );
        measured.insert(
            "denied_probe".to_owned(),
            summary(3, 5.0, 5.0, &[(Outcome::Denied, 3)]),
        );
        let mut window = Window::default();
        for family in REQUIRED_GATEWAY_HISTOGRAMS {
            window.gateway.insert(
                family.to_owned(),
                ServerQuantiles {
                    n: 1.0,
                    mean_ms: 1.0,
                    p50_ms: 1.0,
                    p95_ms: 1.0,
                    p99_ms: 1.0,
                },
            );
        }
        window
            .gauges
            .insert(format!("{REQUIRED_GAUGE}{{state=\"idle\"}}"), 1.0);
        report.phases.push(PhaseReport {
            name: "steady".to_owned(),
            completed: true,
            windows: vec![window],
            measured,
            ..PhaseReport::default()
        });
        report.status = Status::Completed;
        report
    }

    fn ledger() -> Ledger {
        Ledger {
            schema: LEDGER_SCHEMA.to_owned(),
            baselines: Vec::new(),
        }
    }

    #[test]
    fn a_complete_run_passes_its_budgets_and_the_check() {
        let report = complete_report();
        let assessment = assess(&report);
        assert!(
            assessment.iter().all(|check| check.passed),
            "{assessment:?}"
        );
        assert_eq!(
            check(&report, &ledger(), &BTreeMap::new()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn missing_axes_breaches_and_isolation_failures_are_reported() {
        let mut report = complete_report();
        report.environment.hardware.clear();
        report.phases[0].measured.remove("knowledge_search");
        report.phases[0].windows[0]
            .gateway
            .remove("synveda_db_pool_acquire_seconds");
        report.phases[0].measured.insert(
            "isolation_breach".to_owned(),
            summary(0, 0.0, 0.0, &[(Outcome::Unexpected, 1)]),
        );
        report.phases[0]
            .measured
            .get_mut("context_run")
            .unwrap()
            .latency
            .get_mut(&Measure::Response)
            .unwrap()
            .p99_ms = 999.0;
        let problems = check(&report, &ledger(), &BTreeMap::new());
        assert!(
            problems.iter().any(|problem| problem.contains("hardware")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("knowledge_search")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("synveda_db_pool_acquire_seconds")),
            "{problems:?}"
        );
        let failed: Vec<String> = assess(&report)
            .into_iter()
            .filter(|a| !a.passed)
            .map(|a| a.check)
            .collect();
        assert!(failed.contains(&"cross_tenant_isolation_breaches".to_owned()));
        assert!(failed.contains(&"context_run_p99_ms".to_owned()));
        assert!(failed.contains(&"knowledge_search_p99_ms".to_owned()));
    }

    #[test]
    fn a_changed_profile_or_rewritten_baseline_is_refused() {
        let mut report = complete_report();
        report.profile.seed += 1;
        assert!(
            check(&report, &ledger(), &BTreeMap::new())
                .iter()
                .any(|p| p.contains("digest"))
        );
        let report = complete_report();
        let bytes = b"{\"thresholds\":{}}".to_vec();
        let mut reviewed = ledger();
        reviewed.baselines.push(LedgerEntry {
            file: "smoke.json".to_owned(),
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            profile_digest: report.profile_digest.clone(),
            reviewed_by: "owner".to_owned(),
            reviewed_on: "2026-10-28".to_owned(),
        });
        let files: BTreeMap<String, Vec<u8>> =
            [("smoke.json".to_owned(), bytes)].into_iter().collect();
        assert!(check(&report, &reviewed, &files).is_empty());
        let rewritten: BTreeMap<String, Vec<u8>> = [("smoke.json".to_owned(), b"{}".to_vec())]
            .into_iter()
            .collect();
        assert!(
            check(&report, &reviewed, &rewritten)
                .iter()
                .any(|p| p.contains("changed after review"))
        );
        let unlisted: BTreeMap<String, Vec<u8>> = [("new.json".to_owned(), b"{}".to_vec())]
            .into_iter()
            .collect();
        assert!(
            check(&report, &ledger(), &unlisted)
                .iter()
                .any(|p| p.contains("not in the reviewed ledger"))
        );
    }

    #[test]
    fn a_checkpoint_resumes_only_under_its_own_profile() {
        let dir = std::env::temp_dir().join(format!("synveda-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.json");
        let mut report = complete_report();
        report.status = Status::Running;
        report.write(&path).unwrap();
        let resumed = Report::resume(&path, &sample()).unwrap();
        assert_eq!(resumed.resumed, 1);
        let mut other = sample();
        other.phases[0].seconds += 1;
        assert!(Report::resume(&path, &other).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
