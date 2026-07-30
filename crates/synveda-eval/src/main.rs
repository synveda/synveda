//! `synveda-eval` — the eval harness (EVAL-1, ADR-0028).
//!
//! Runs a scenario suite against a live stack, reports the five axes
//! (accuracy, latency, tokens, recall, abstention), and fails against a
//! committed baseline. It holds no Synveda crate dependency and reaches
//! the stack only through `/v1` with an actor's own bearer, so every
//! number it reports is a number the product actually produces for a
//! caller — PDP included.
//!
//! The privileged half — admitting a tenant, building a hierarchy,
//! registering the actors — is `evals/lib.sh`, in the same shell idiom
//! every demo uses. This binary is a client and nothing more.

#![forbid(unsafe_code)]

mod client;
mod extraction;
mod fixtures;
mod report;
mod runner;
mod scenario;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::client::Client;
use crate::report::{Baseline, Report};
use crate::runner::Options;
use crate::scenario::Environment;

#[derive(Parser)]
#[command(
    name = "synveda-eval",
    about = "Synveda eval harness (EVAL-1)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a suite against the live stack an environment file describes.
    Run {
        /// The environment `evals/lib.sh` printed: gateway URL, tenant,
        /// and one bearer per actor.
        #[arg(long)]
        env: PathBuf,
        /// Directory of `*.json` scenarios.
        #[arg(long, default_value = "evals/scenarios")]
        suite: PathBuf,
        /// Directory of `*.json` extraction fixture groups (EVAL-2). A
        /// second suite rather than a stretched scenario format: the two
        /// shapes measure different things and would each be worse for
        /// carrying the other's fields (ADR-0046 decision 10). Both reduce
        /// into one metrics map and gate against one baseline.
        #[arg(long, default_value = "evals/fixtures/extraction")]
        fixtures: PathBuf,
        /// The committed gate.
        #[arg(long, default_value = "evals/baseline.json")]
        baseline: PathBuf,
        /// Where to write the JSON report. Defaults to stdout.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Rewrite the baseline from this run instead of gating against
        /// it. Deliberate, and a diff someone has to review.
        #[arg(long)]
        update_baseline: bool,
        /// How long seeded material gets to become composable.
        #[arg(long, default_value_t = runner::DEFAULT_SEED_TIMEOUT.as_secs())]
        seed_timeout_secs: u64,
    },
    /// Parse and validate a suite without touching a gateway. This is
    /// what CI can run: no database, no stack, still catches a scenario
    /// that would have silently measured nothing.
    Check {
        #[arg(long, default_value = "evals/scenarios")]
        suite: PathBuf,
        #[arg(long, default_value = "evals/fixtures/extraction")]
        fixtures: PathBuf,
        #[arg(long, default_value = "evals/baseline.json")]
        baseline: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        // The gate failed: the run worked and the answer is "worse".
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("synveda-eval: {message}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<bool, String> {
    match cli.command {
        Command::Check {
            suite,
            fixtures: fixtures_dir,
            baseline,
        } => {
            let scenarios = scenario::load_suite(&suite)?;
            // Corpus validation runs here too, with no database and no
            // gateway: a mislabelled fixture would move a gated number
            // forever and silently, and in both of the corpus's readers
            // (ADR-0046 decision 7).
            let groups = fixtures::load_corpus(&fixtures_dir)?;
            let baseline = Baseline::load(&baseline)?;
            eprintln!(
                "synveda-eval: {} scenario(s) and {} fixture(s) across {} group(s) parse; the \
                 baseline bounds {}",
                scenarios.len(),
                groups
                    .iter()
                    .map(|group| group.fixtures.len())
                    .sum::<usize>(),
                groups.len(),
                baseline
                    .metrics
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(true)
        }
        Command::Run {
            env,
            suite,
            fixtures: fixtures_dir,
            baseline,
            report: report_path,
            update_baseline,
            seed_timeout_secs,
        } => {
            let environment = Environment::load(&env)?;
            let scenarios = scenario::load_suite(&suite)?;
            let groups = fixtures::load_corpus(&fixtures_dir)?;
            let baseline_file = baseline;
            let baseline = Baseline::load(&baseline_file)?;
            let client = Client::new(&environment.gateway_url)?;
            let seed_timeout = Duration::from_secs(seed_timeout_secs);
            let options = Options { seed_timeout };

            let started_at = chrono::Utc::now().to_rfc3339();
            let mut outcomes = Vec::with_capacity(scenarios.len());
            for scenario in &scenarios {
                eprintln!("synveda-eval: {}", scenario.name);
                outcomes
                    .push(runner::run_scenario(&client, &environment, scenario, &options).await?);
            }

            let extraction_options = extraction::Options { seed_timeout };
            let mut extraction_outcomes = Vec::with_capacity(groups.len());
            for group in &groups {
                eprintln!("synveda-eval: extraction/{}", group.group);
                extraction_outcomes.push(
                    extraction::run_group(&client, &environment, group, &extraction_options)
                        .await?,
                );
            }

            // One metrics map and one gate over both suites (ADR-0046
            // decision 10): the formats differ because they measure
            // different things, the gate vocabulary does not.
            let mut metrics = report::metrics(&outcomes);
            metrics.extend(extraction::metrics(&extraction_outcomes));
            let gate = report::gate(&baseline, &metrics);
            let report = Report {
                suite: suite.display().to_string(),
                tenant_id: environment.tenant_id.clone(),
                gateway_url: environment.gateway_url.clone(),
                started_at,
                actors: environment
                    .actors
                    .iter()
                    .map(|(name, actor)| {
                        (
                            name.clone(),
                            actor.scope.clone().unwrap_or_else(|| "unstated".to_owned()),
                        )
                    })
                    .collect(),
                scenarios: outcomes,
                extraction: extraction_outcomes,
                metrics,
                gate,
            };

            let json = serde_json::to_string_pretty(&report)
                .map_err(|err| format!("serialise the report: {err}"))?;
            match &report_path {
                Some(path) => std::fs::write(path, format!("{json}\n"))
                    .map_err(|err| format!("write {}: {err}", path.display()))?,
                None => println!("{json}"),
            }
            eprint!("{}", report::summarise(&report));

            if update_baseline {
                let updated = baseline.updated(&report.metrics);
                let body = serde_json::to_string_pretty(&updated)
                    .map_err(|err| format!("serialise the baseline: {err}"))?;
                std::fs::write(&baseline_file, format!("{body}\n"))
                    .map_err(|err| format!("write {}: {err}", baseline_file.display()))?;
                eprintln!(
                    "\n  baseline rewritten at {} — review the diff",
                    baseline_file.display()
                );
                return Ok(true);
            }
            Ok(report.gate.passed)
        }
    }
}
