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
//!
//! Environment (EVAL-3, ADR-0061): `SYNVEDA_JUDGE` (`lexical` [default] |
//! `claude`) selects the judge the way `SYNVEDA_EXTRACTOR` selects an
//! extractor, with `ANTHROPIC_API_KEY`, `SYNVEDA_JUDGE_MODEL` and
//! `SYNVEDA_ANTHROPIC_BASE_URL` as its companions. The default reaches no
//! network; only `claude` costs money, and only per graded pair.

#![forbid(unsafe_code)]

mod agreement;
mod client;
mod extraction;
mod fixtures;
mod judge;
mod qa;
mod qa_runner;
mod report;
mod runner;
mod scenario;
mod security;
mod security_runner;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::client::Client;
use crate::judge::{Judge, Tally};
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
        /// Directory of `*.json` Q&A corpora (EVAL-4). A third suite, and
        /// for a measured reason: a Q&A corpus is seeded once and asked
        /// many times, which the scenario format cannot say (ADR-0047
        /// decision 4).
        #[arg(long, default_value = "evals/fixtures/qa")]
        qa: PathBuf,
        /// Directory of `*.json` security corpora (EVAL-5). A fourth
        /// suite: a boundary declaration and a Q&A question are half
        /// inert in each other (ADR-0048 decision 12).
        #[arg(long, default_value = "evals/fixtures/security")]
        security: PathBuf,
        /// How many distinct generated variants the security suite asks.
        /// The nightly's full budget, or the deterministic slice the
        /// pull-request job runs (ADR-0048 decision 13) — and a gated
        /// floor either way, because a one-sided gate whose denominator
        /// the run chooses passes by measuring less.
        #[arg(long, default_value_t = security_runner::DEFAULT_VARIANTS)]
        security_variants: usize,
        /// Whether this run's embedder ranks by meaning. The harness is a
        /// client and cannot see the gateway's configuration, so whoever
        /// brought TEI up says so — and `semantic` questions are skipped
        /// and counted without it (ADR-0047 decision 5).
        #[arg(long)]
        dense_retrieval: bool,
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
        #[arg(long, default_value = "evals/fixtures/qa")]
        qa: PathBuf,
        #[arg(long, default_value = "evals/fixtures/security")]
        security: PathBuf,
        /// Directory of `*.json` labelled sets the judge is measured
        /// against (EVAL-3, ADR-0061 decision 4). Parsed here for the
        /// same reason every other corpus is: a set labelled all one way
        /// scores perfect agreement for a judge that read nothing.
        #[arg(long, default_value = "evals/fixtures/judge")]
        labels: PathBuf,
        #[arg(long, default_value = "evals/baseline.json")]
        baseline: PathBuf,
    },
    /// Measure the configured judge against a labelled set — the thing
    /// ADR-0046 option 6 said had to happen before a judge is allowed to
    /// decide whether the product regressed (ADR-0061 decision 4).
    ///
    /// Needs no gateway and no database: the labels are a file, and only
    /// `SYNVEDA_JUDGE=claude` reaches a network. It gates nothing
    /// (decision 5) — it reports the rate and, more usefully, the rows
    /// the judge got wrong.
    Judge {
        #[arg(long, default_value = "evals/fixtures/judge")]
        labels: PathBuf,
        /// Where to write the JSON report. Defaults to stdout.
        #[arg(long)]
        report: Option<PathBuf>,
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
            qa: qa_dir,
            security: security_dir,
            labels: labels_dir,
            baseline,
        } => {
            let scenarios = scenario::load_suite(&suite)?;
            // Corpus validation runs here too, with no database and no
            // gateway: a mislabelled fixture would move a gated number
            // forever and silently, and in both of the corpus's readers
            // (ADR-0046 decision 7).
            let groups = fixtures::load_corpus(&fixtures_dir)?;
            // The Q&A guards run here too, and two of them are the reason
            // this command exists: a question that declared the wrong
            // retrieval leg would fail on the wrong path forever, for a
            // corpus reason rather than a product one (ADR-0047
            // decision 5).
            let corpora = qa::load_corpora(&qa_dir)?;
            // The exhaustiveness guard is the reason this command matters
            // most for the security corpus: an undeclared (record, reader)
            // pair is a boundary nothing asserts, and it would still
            // report zero leaks (ADR-0048 decision 5).
            let boundaries = security::load_corpora(&security_dir)?;
            // The judge's labelled sets, and the judge itself. Both are
            // checked here for the same reason the corpora are: a set
            // labelled all one way reports perfect agreement for a judge
            // that read nothing, and a misconfigured `SYNVEDA_JUDGE`
            // should fail before a run that costs money per call, not
            // partway through one (the gateway's startup discipline).
            let labels = agreement::load_sets(&labels_dir)?;
            let judge = judge::from_env()?;
            let baseline = Baseline::load(&baseline)?;
            eprintln!(
                "synveda-eval: {} scenario(s), {} fixture(s) across {} group(s), {} \
                 question(s) across {} corpus/corpora, and {} record(s) with {} declared \
                 boundary/boundaries across {} security corpus/corpora parse; the baseline \
                 bounds {}",
                scenarios.len(),
                groups
                    .iter()
                    .map(|group| group.fixtures.len())
                    .sum::<usize>(),
                groups.len(),
                corpora
                    .iter()
                    .map(|corpus| corpus.questions.len())
                    .sum::<usize>(),
                corpora.len(),
                boundaries
                    .iter()
                    .map(|corpus| corpus.material.len())
                    .sum::<usize>(),
                boundaries
                    .iter()
                    .flat_map(|corpus| corpus.material.iter())
                    .map(|record| record.readable_by.len() + record.forbidden_to.len())
                    .sum::<usize>(),
                boundaries.len(),
                baseline
                    .metrics
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            eprintln!(
                "synveda-eval: {} labelled judge pair(s) across {} set(s) parse; the configured \
                 judge is `{}`",
                labels.iter().map(|set| set.pairs.len()).sum::<usize>(),
                labels.len(),
                judge.method()
            );
            Ok(true)
        }
        Command::Judge {
            labels: labels_dir,
            report: report_path,
        } => {
            let sets = agreement::load_sets(&labels_dir)?;
            let judge = judge::from_env()?;
            let started_at = chrono::Utc::now().to_rfc3339();
            let mut tally = Tally::default();
            let mut measured = Vec::with_capacity(sets.len());
            for set in &sets {
                eprintln!("synveda-eval: judge/{}", set.set);
                measured.push(agreement::measure(&judge, set, &mut tally).await);
            }

            let report = agreement::JudgeReport {
                method: judge.method().to_owned(),
                started_at,
                metrics: agreement::metrics(&measured),
                sets: measured,
                tally,
            };
            let json = serde_json::to_string_pretty(&report)
                .map_err(|err| format!("serialise the report: {err}"))?;
            match &report_path {
                Some(path) => std::fs::write(path, format!("{json}\n"))
                    .map_err(|err| format!("write {}: {err}", path.display()))?,
                None => println!("{json}"),
            }
            eprint!("{}", agreement::summarise(&report));

            // The only failure is having measured nothing. A low
            // agreement rate is a finding about the judge (reversal
            // trigger (a)), not a broken run, and decision 5 keeps this
            // tier off every gate — a judge measurement that failed a
            // build would be that gate through a side door.
            let graded: usize = report.sets.iter().map(|set| set.graded).sum();
            if graded == 0 {
                return Err(format!(
                    "the judge graded none of {} pair(s); an agreement rate over nothing is not a \
                     measurement",
                    report.sets.iter().map(|set| set.pairs).sum::<usize>()
                ));
            }
            Ok(true)
        }
        Command::Run {
            env,
            suite,
            fixtures: fixtures_dir,
            qa: qa_dir,
            security: security_dir,
            security_variants,
            dense_retrieval,
            baseline,
            report: report_path,
            update_baseline,
            seed_timeout_secs,
        } => {
            let environment = Environment::load(&env)?;
            let scenarios = scenario::load_suite(&suite)?;
            let groups = fixtures::load_corpus(&fixtures_dir)?;
            let corpora = qa::load_corpora(&qa_dir)?;
            let boundaries = security::load_corpora(&security_dir)?;
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

            let qa_options = qa_runner::Options {
                seed_timeout,
                dense_retrieval,
            };
            let mut qa_outcomes = Vec::with_capacity(corpora.len());
            for corpus in &corpora {
                eprintln!("synveda-eval: qa/{}", corpus.corpus);
                qa_outcomes
                    .push(qa_runner::run_corpus(&client, &environment, corpus, &qa_options).await?);
            }

            let security_options = security_runner::Options {
                seed_timeout,
                variants: security_variants,
            };
            let mut security_outcomes = Vec::with_capacity(boundaries.len());
            for corpus in &boundaries {
                eprintln!("synveda-eval: security/{}", corpus.corpus);
                security_outcomes.push(
                    security_runner::run_corpus(&client, &environment, corpus, &security_options)
                        .await?,
                );
            }

            // One metrics map and one gate over all four suites (ADR-0046
            // decision 10, ADR-0047 decision 4, ADR-0048 decision 12): the
            // formats differ because they measure different things, the
            // gate vocabulary does not.
            let mut metrics = report::metrics(&outcomes);
            metrics.extend(extraction::metrics(&extraction_outcomes));
            metrics.extend(qa_runner::metrics(&qa_outcomes));
            metrics.extend(security_runner::metrics(&security_outcomes));
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
                qa: qa_outcomes,
                security: security_outcomes,
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
