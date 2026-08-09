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
mod anthropic;
mod client;
mod extraction;
mod fixtures;
mod judge;
mod longmemeval;
mod longmemeval_runner;
mod qa;
mod qa_runner;
mod reader;
mod reading;
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
use crate::judge::Judge;
use crate::reader::Reader;
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
        /// Directory of `*.json` reader probes (EVAL-3, ADR-0061
        /// decision 6). Parsed here because its support guard is the one
        /// that stops a probe nothing could answer from reading as a
        /// reader that cannot read.
        #[arg(long, default_value = "evals/fixtures/reader")]
        probes: PathBuf,
        /// The LongMemEval corpus, in upstream's own format (ADR-0061
        /// decision 2). Fetched rather than committed, so an absent file
        /// is reported and skipped rather than failing — which is what CI
        /// does, every time.
        #[arg(long, default_value = longmemeval::DEFAULT_PATH)]
        longmemeval: PathBuf,
        /// How many instances a run would measure, so `check` can say what
        /// the slice covers before anyone spends a nightly finding out
        /// (decision 7).
        #[arg(long, default_value_t = longmemeval::DEFAULT_INSTANCES)]
        instances: usize,
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
    /// LongMemEval's deterministic retrieval tier against a live stack
    /// (ADR-0061 decision 5).
    ///
    /// The half of the benchmark this product is responsible for, and the
    /// half that is reproducible from bytes: did the block bind the
    /// evidence sessions the instance names? It reaches no model, costs
    /// nothing per run, and gates against its own baseline. The QA
    /// accuracy LongMemEval is better known for is the other tier —
    /// published, gated by nothing, and dependent on two external models.
    Longmemeval {
        /// The environment `evals/lib.sh` printed. Needs one `lme-*` actor
        /// per instance (decision 8) and the `auditor`.
        #[arg(long)]
        env: PathBuf,
        /// The corpus, in upstream's own format. Fetched rather than
        /// committed — see evals/fixtures/longmemeval/NOTICE.md.
        #[arg(long, default_value = longmemeval::DEFAULT_PATH)]
        corpus: PathBuf,
        /// How many instances to measure. The declared slice (decision 7):
        /// every report states it, because a suite that bounds its
        /// coverage says what it bounded.
        #[arg(long, default_value_t = longmemeval::DEFAULT_INSTANCES)]
        instances: usize,
        /// The caller-side budget. Absent is the pack's own default, which
        /// is the honest shape for a published number — a budget tuned
        /// until the score improved would be a benchmark measuring its own
        /// tuning.
        #[arg(long)]
        budget_tokens: Option<u32>,
        /// Also read each block with the configured reader and grade the
        /// answer with the configured judge — the model-judged tier
        /// (decision 5). This is the published figure and it gates
        /// nothing; it costs money per instance, and the reader's prompt
        /// is a whole block, which is the expensive half.
        #[arg(long)]
        judged: bool,
        /// The labelled sets the judge is measured against, on the judged
        /// tier (decision 4). Run inside the judged run rather than beside
        /// it, so publishing a score without its judge's agreement rate is
        /// structurally impossible.
        #[arg(long, default_value = "evals/fixtures/judge")]
        labels: PathBuf,
        /// The committed gate. Defaults to whichever file belongs to the
        /// tier this run measures, so a judged run cannot be gated against
        /// the deterministic tier's numbers by forgetting a flag.
        #[arg(long)]
        baseline: Option<PathBuf>,
        /// Where to write the JSON report. Defaults to stdout.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Rewrite the baseline from this run instead of gating against
        /// it. Deliberate, and a diff someone has to review.
        #[arg(long)]
        update_baseline: bool,
        /// How long a seeded instance gets to become rankable.
        #[arg(long, default_value_t = runner::DEFAULT_SEED_TIMEOUT.as_secs())]
        seed_timeout_secs: u64,
    },
    /// Rewrite a baseline from a report a run already produced, without
    /// measuring again.
    ///
    /// `--update-baseline` can only rewrite during a run, which is fine
    /// for a suite that takes seconds and free for a suite that reaches no
    /// model. LongMemEval is neither: a slice is twenty-five minutes of
    /// seeding and the judged tier bills per instance, so re-measuring
    /// purely to write floors would pay twice for one measurement.
    ///
    /// The floors are a function of a report's metrics and nothing else,
    /// so this applies exactly what `--update-baseline` would have — same
    /// `Baseline::updated`, same slack, same ceiling headroom — to a
    /// report on disk. It is not a shortcut around measuring; it is the
    /// same arithmetic over a measurement that already happened, and the
    /// report it came from is named in the diff.
    Rebaseline {
        /// The run report to take the measurements from.
        #[arg(long)]
        report: PathBuf,
        /// The baseline to rewrite.
        #[arg(long)]
        baseline: PathBuf,
    },
    /// Measure the configured reader against its probes, graded by the
    /// configured judge (ADR-0061 decision 6).
    ///
    /// The blocks come from a file rather than from `/v1/inject`, so this
    /// measures the reader and the judge and **not** Synveda — the axes
    /// are named `probe_*` rather than `qa_*` for exactly that reason.
    /// Needs no gateway and no database; only `=claude` reaches a
    /// network, and it gates nothing.
    Read {
        #[arg(long, default_value = "evals/fixtures/reader")]
        probes: PathBuf,
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
            probes: probes_dir,
            longmemeval: longmemeval_path,
            instances,
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
            let probes = reading::load_sets(&probes_dir)?;
            let reader = reader::from_env()?;
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
                "synveda-eval: {} labelled judge pair(s) across {} set(s) and {} reader probe(s) \
                 across {} set(s) parse; the configured reader is `{}` and judge is `{}`",
                labels.iter().map(|set| set.pairs.len()).sum::<usize>(),
                labels.len(),
                probes.iter().map(|set| set.probes.len()).sum::<usize>(),
                probes.len(),
                reader.method(),
                judge.method()
            );
            if let Some(note) = reader::independence_note(&reader, &judge) {
                eprintln!("synveda-eval: {note}");
            }
            // The benchmark corpus is fetched rather than committed, so
            // its absence is the ordinary case here and in CI. It is
            // stated rather than skipped quietly: decision 7's rule is
            // that a run says what it did not cover, and a `check` that
            // validated no benchmark while printing nothing reads as a
            // `check` that validated one.
            match longmemeval::load_if_present(&longmemeval_path)? {
                Some(corpus) => {
                    let (_, slice) = longmemeval::slice(&corpus, instances);
                    eprintln!("synveda-eval: longmemeval {}", slice.describe());
                }
                None => eprintln!(
                    "synveda-eval: no LongMemEval corpus at {} — nothing was validated against \
                     it; see evals/fixtures/longmemeval/NOTICE.md for how to fetch it",
                    longmemeval_path.display()
                ),
            }
            Ok(true)
        }
        Command::Judge {
            labels: labels_dir,
            report: report_path,
        } => {
            let sets = agreement::load_sets(&labels_dir)?;
            let judge = judge::from_env()?;
            let started_at = chrono::Utc::now().to_rfc3339();
            let mut tally = judge::Tally::default();
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
        Command::Longmemeval {
            env,
            corpus: corpus_path,
            instances,
            budget_tokens,
            judged,
            labels: labels_dir,
            baseline,
            report: report_path,
            update_baseline,
            seed_timeout_secs,
        } => {
            let environment = Environment::load(&env)?;
            let corpus = longmemeval::load(&corpus_path)?;
            let (picked, slice) = longmemeval::slice(&corpus, instances);
            let baseline_file = baseline.unwrap_or_else(|| {
                PathBuf::from(if judged {
                    longmemeval_runner::JUDGED_BASELINE
                } else {
                    longmemeval_runner::RETRIEVAL_BASELINE
                })
            });
            let baseline = Baseline::load(&baseline_file)?;
            let client = Client::new(&environment.gateway_url)?;
            let options = longmemeval_runner::Options {
                seed_timeout: Duration::from_secs(seed_timeout_secs),
                budget_tokens,
            };

            // The seams, resolved before anything is seeded. A
            // misconfigured `SYNVEDA_READER` should fail here rather than
            // forty sessions into a run that costs money — the gateway's
            // own startup discipline, applied to a client.
            let seams = judged
                .then(|| Ok::<_, String>((reader::from_env()?, judge::from_env()?)))
                .transpose()?;
            let suite = longmemeval_runner::Suite {
                client: &client,
                environment: &environment,
                options: &options,
                graders: seams
                    .as_ref()
                    .map(|(reader, judge)| longmemeval_runner::Graders { reader, judge }),
            };

            // One actor per instance, and the pool is the environment's
            // rather than this binary's (decision 8). Refused by name
            // rather than wrapped around: two instances on one identity
            // would put one haystack inside the other, and the run would
            // measure retrieval over a corpus twice the size the benchmark
            // specifies while reporting the benchmark's name.
            let pool = longmemeval_runner::actors(&environment);
            if pool.len() < picked.len() {
                return Err(format!(
                    "this run measures {} instance(s) and the environment registers {} `{}` \
                     actor(s); one actor per instance is what keeps the haystacks apart, so \
                     register more in evals/lib.sh (EVAL_LONGMEMEVAL_ACTORS) or ask for fewer \
                     instances",
                    picked.len(),
                    pool.len(),
                    longmemeval_runner::ACTOR_PREFIX
                ));
            }
            eprintln!("synveda-eval: longmemeval {}", slice.describe());
            let independence = suite
                .graders
                .as_ref()
                .and_then(|graders| reader::independence_note(graders.reader, graders.judge));
            if let Some(note) = &independence {
                eprintln!("synveda-eval: {note}");
            }

            let started_at = chrono::Utc::now().to_rfc3339();
            let mut tallies = longmemeval_runner::Tallies::default();

            // The judge, measured before it measures — and *inside* the run
            // that uses it, which is decision 4 made structural rather than
            // procedural. It runs first because a judge whose agreement
            // cannot be established is one whose score should not be paid
            // for, and because the labelled sets are ten pairs against N
            // instances of a 115k-token block.
            let judge_agreement = match &suite.graders {
                Some(graders) => {
                    let sets = agreement::load_sets(&labels_dir)?;
                    let mut measured = Vec::with_capacity(sets.len());
                    for set in &sets {
                        eprintln!("synveda-eval: longmemeval/judge/{}", set.set);
                        measured
                            .push(agreement::measure(graders.judge, set, &mut tallies.judge).await);
                    }
                    Some(agreement::JudgeReport {
                        method: graders.judge.method().to_owned(),
                        started_at: started_at.clone(),
                        metrics: agreement::metrics(&measured),
                        sets: measured,
                        tally: std::mem::take(&mut tallies.judge),
                    })
                }
                None => None,
            };
            // The agreement pass owns the judge tally it produced; the
            // instances start their own, so a per-instance cost is not the
            // agreement pass's cost added to it.
            // Seed everything, wait once, then probe — never a loop that
            // interleaves the three. The first run against the real corpus
            // did interleave them and measured its own extraction queue:
            // instances 1-3 seeded in seconds and graded fine, and from
            // instance 4 on every wait burned its whole timeout against
            // the backlog the first three had built, six blocks came back
            // empty, and the run reported a retrieval recall of 0.214 that
            // was about throughput. EVAL-2 found this and EVAL-4 paid for
            // it; the Q&A suite seeds once, waits for all of it, and only
            // then probes.
            let seeding = std::time::Instant::now();
            let mut seeded = Vec::with_capacity(picked.len());
            for (index, instance) in picked.iter().enumerate() {
                eprintln!(
                    "synveda-eval: longmemeval/{} seed ({} of {}, {} session(s), {} turn(s) as {})",
                    instance.question_id,
                    index + 1,
                    picked.len(),
                    instance.haystack_session_ids.len(),
                    instance.turns(),
                    pool[index]
                );
                seeded
                    .push(longmemeval_runner::seed_instance(&suite, instance, &pool[index]).await?);
            }
            eprintln!(
                "synveda-eval: longmemeval waiting for the pipeline to finish with {} turn(s)",
                seeded.iter().map(|entry| entry.events.len()).sum::<usize>()
            );
            longmemeval_runner::wait_for_all(&suite, &picked, &mut seeded, seeding).await?;

            let mut outcomes = Vec::with_capacity(picked.len());
            for ((index, instance), mut entry) in picked.iter().enumerate().zip(seeded) {
                eprintln!(
                    "synveda-eval: longmemeval/{} measure ({} of {})",
                    instance.question_id,
                    index + 1,
                    picked.len()
                );
                longmemeval_runner::measure_instance(
                    &suite,
                    instance,
                    &pool[index],
                    &mut entry.outcome,
                    &mut tallies,
                )
                .await?;
                outcomes.push(entry.outcome);
            }

            let mut metrics = longmemeval_runner::metrics(&outcomes);
            if let Some(agreement) = &judge_agreement {
                // The judge's own axes ride beside the product's, under
                // their own names rather than a `longmemeval_` prefix: they
                // are a property of the judge, and decision 4 says no claim
                // this feature publishes may be tighter than they are.
                metrics.extend(agreement.metrics.clone());
            }
            let models = longmemeval_runner::served_models(&outcomes);
            let gate = report::gate(&baseline, &metrics);
            // A judged run measures every retrieval axis on its way to the
            // QA one — same seeding, same blocks — so running the two
            // tiers as two runs seeds ~5,000 turns twice for one
            // measurement. It does not any more: the judged run also
            // checks the deterministic tier's committed floors, and *that*
            // is the gate its exit status honours. Decision 5 stands
            // exactly as written — the judged half still gates nothing;
            // the half that always gated still does.
            let retrieval_gate = judged
                .then(|| {
                    Baseline::load(std::path::Path::new(longmemeval_runner::RETRIEVAL_BASELINE))
                        .map(|retrieval| report::gate(&retrieval, &metrics))
                })
                .transpose()?;
            let run = longmemeval_runner::Report {
                started_at,
                gateway_url: environment.gateway_url.clone(),
                tenant_id: environment.tenant_id.clone(),
                slice,
                tier: if judged { "judged" } else { "retrieval" }.to_owned(),
                retrieval_gate,
                model_drift: baseline.model_drift(&models),
                models,
                independence,
                judge_agreement,
                instances: outcomes,
                tallies,
                metrics,
                gate,
            };

            let json = serde_json::to_string_pretty(&run)
                .map_err(|err| format!("serialise the report: {err}"))?;
            match &report_path {
                Some(path) => std::fs::write(path, format!("{json}\n"))
                    .map_err(|err| format!("write {}: {err}", path.display()))?,
                None => println!("{json}"),
            }
            eprint!("{}", longmemeval_runner::summarise(&run));

            if update_baseline {
                let mut updated = baseline.updated(&run.metrics);
                // Keyed to what the API served, never to the alias asked
                // for (decision 6). Writing the floors without this would
                // commit numbers whose provenance is a guess.
                updated.models = run.models.clone();
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
            // The judged tier's own bounds gate nothing (decision 5): a
            // gate that fails when a model changes rather than when the
            // code changes is an alarm nobody keeps, so its breaches print
            // and do not fail. The retrieval floors it also measured are a
            // different matter — those are the ones that have always
            // gated, and a judged run is not a way to avoid them.
            Ok(match &run.retrieval_gate {
                Some(retrieval) => retrieval.passed,
                None => run.gate.passed,
            })
        }
        Command::Rebaseline {
            report: report_path,
            baseline: baseline_file,
        } => {
            let raw = std::fs::read_to_string(&report_path)
                .map_err(|err| format!("read {}: {err}", report_path.display()))?;
            let run: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|err| format!("{} is not a report: {err}", report_path.display()))?;
            let metrics: std::collections::BTreeMap<String, f64> = serde_json::from_value(
                run.get("metrics").cloned().unwrap_or_default(),
            )
            .map_err(|err| format!("{} carries no metrics map: {err}", report_path.display()))?;
            if metrics.is_empty() {
                return Err(format!(
                    "{} measured nothing, so there are no floors to write from it",
                    report_path.display()
                ));
            }
            // A gate that breached is a run that says the product is worse
            // than the committed floor. Writing *that* in as the new floor
            // is how a regression becomes the baseline, so it takes a
            // second look rather than a flag nobody re-reads.
            if run.pointer("/gate/passed") == Some(&serde_json::Value::Bool(false)) {
                eprintln!(
                    "synveda-eval: warning — {} breached its gate, so these floors are being \
                     written from a run the committed baseline called a regression",
                    report_path.display()
                );
            }

            let baseline = Baseline::load(&baseline_file)?;
            let mut updated = baseline.updated(&metrics);
            // Keyed to what the API served on that run (ADR-0061
            // decision 6), taken from the report rather than assumed.
            if let Some(models) = run.get("models") {
                updated.models = serde_json::from_value(models.clone()).unwrap_or_default();
            }
            let body = serde_json::to_string_pretty(&updated)
                .map_err(|err| format!("serialise the baseline: {err}"))?;
            std::fs::write(&baseline_file, format!("{body}\n"))
                .map_err(|err| format!("write {}: {err}", baseline_file.display()))?;
            eprintln!(
                "synveda-eval: rewrote {} from {} ({} bounded metric(s), {} model(s) keyed) — \
                 review the diff",
                baseline_file.display(),
                report_path.display(),
                updated.metrics.len(),
                updated.models.len()
            );
            Ok(true)
        }
        Command::Read {
            probes: probes_dir,
            report: report_path,
        } => {
            let sets = reading::load_sets(&probes_dir)?;
            let reader = reader::from_env()?;
            let judge = judge::from_env()?;
            let started_at = chrono::Utc::now().to_rfc3339();
            let mut reader_tally = reader::Tally::default();
            let mut judge_tally = judge::Tally::default();
            let mut measured = Vec::with_capacity(sets.len());
            for set in &sets {
                eprintln!("synveda-eval: read/{}", set.set);
                measured.push(
                    reading::measure(&reader, &judge, set, &mut reader_tally, &mut judge_tally)
                        .await,
                );
            }

            let report = reading::ReadingReport {
                reader: reader.method().to_owned(),
                judge: judge.method().to_owned(),
                independence: reader::independence_note(&reader, &judge),
                started_at,
                metrics: reading::metrics(&measured),
                sets: measured,
                reader_tally,
                judge_tally,
            };
            let json = serde_json::to_string_pretty(&report)
                .map_err(|err| format!("serialise the report: {err}"))?;
            match &report_path {
                Some(path) => std::fs::write(path, format!("{json}\n"))
                    .map_err(|err| format!("write {}: {err}", path.display()))?,
                None => println!("{json}"),
            }
            eprint!("{}", reading::summarise(&report));

            // The only failure is having measured nothing. A reader that
            // read badly is a finding; a reader that never ran is a
            // broken harness, and the two must not exit the same way.
            let read: usize = report
                .sets
                .iter()
                .map(|set| set.probes - set.unread.len())
                .sum();
            if read == 0 {
                return Err(format!(
                    "the reader answered none of {} probe(s); a rate over nothing is not a \
                     measurement",
                    report.sets.iter().map(|set| set.probes).sum::<usize>()
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
