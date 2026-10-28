//! `synveda-eval load`: the mixed public-API workload behind the capacity
//! envelope.
//!
//! Agents in a measured tenant and a noisy tenant follow seeded open-loop
//! schedules through ordered phases. Each phase runs in checkpoint windows;
//! after every window the harness scrapes gateway and worker metrics, reads
//! the stub provider's service times, samples process and database growth,
//! and atomically rewrites a resumable, content-free report.

pub mod observe;
pub mod profile;
pub mod provider;
pub mod report;
pub mod schedule;
pub mod stats;
pub mod workload;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use profile::{DatabaseFault, Phase, Profile};
use report::{Environment, PhaseReport, ProviderWindow, Report, Status, Window};
use schedule::{Op, arrivals, window_seed};
use stats::{Outcome, Tally};
use workload::{Actor, Agent, KNOWN_OPS, Shared};

const MAX_ARRIVALS_PER_WINDOW: usize = 100_000;
const NOISY_AGENT_OFFSET: u32 = 1_000_000;

/// Everything the load command needs from its caller.
pub struct Args {
    pub env: PathBuf,
    pub profile: PathBuf,
    pub report: PathBuf,
    pub resume: bool,
    pub environment: PathBuf,
    pub worker_url: Option<String>,
    pub provider_url: Option<String>,
    pub fault_hook: Option<PathBuf>,
    pub gateway_pid: Option<u32>,
    pub worker_pid: Option<u32>,
    pub database_samples: Option<PathBuf>,
    pub gate: bool,
}

fn token(env: &Value, actor: &str) -> Result<String, String> {
    env["actors"][actor]["token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("environment has no {actor} reviewer"))
}

fn actors(env: &Value, prefix: &str, wanted: u32) -> Result<Vec<Actor>, String> {
    let mut found: Vec<(String, Actor)> = env["actors"]
        .as_object()
        .ok_or("environment has no actors")?
        .iter()
        .filter(|(name, _)| {
            name.strip_prefix(prefix).is_some_and(|rest| {
                !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .filter_map(|(name, actor)| {
            Some((
                name.clone(),
                Actor {
                    token: actor["token"].as_str()?.to_owned(),
                    workspace_id: actor["workspace_id"].as_str()?.to_owned(),
                    project_id: actor["project_id"].as_str()?.to_owned(),
                },
            ))
        })
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    if found.len() < wanted as usize {
        return Err(format!(
            "the profile needs {wanted} {prefix}* actors and the environment has {}",
            found.len()
        ));
    }
    Ok(found
        .into_iter()
        .take(wanted as usize)
        .map(|(_, actor)| actor)
        .collect())
}

struct Run {
    args: Args,
    profile: Profile,
    shared: Arc<Shared>,
    agents: Vec<Agent>,
    /// Run-long per-tenant tallies, drained at each window boundary, so a
    /// review flow still in flight is counted in the window it ends in.
    tallies: [Arc<Mutex<Tally>>; 2],
    http: reqwest::Client,
    gateway: String,
}

pub async fn run(args: Args) -> Result<bool, String> {
    let profile = Profile::load(&args.profile)?;
    let env: Value = serde_json::from_str(
        &std::fs::read_to_string(&args.env).map_err(|err| format!("read env: {err}"))?,
    )
    .map_err(|err| format!("parse env: {err}"))?;
    let environment: Environment = serde_json::from_str(
        &std::fs::read_to_string(&args.environment)
            .map_err(|err| format!("read environment facts: {err}"))?,
    )
    .map_err(|err| format!("parse environment facts: {err}"))?;
    let gateway = env["gateway_url"]
        .as_str()
        .ok_or("env has no gateway_url")?
        .to_owned();
    let reviewers = [
        workload::Reviewers {
            approvers: [token(&env, "qa-curator")?, token(&env, "qa-steward")?],
            publisher: token(&env, "qa-publisher")?,
        },
        workload::Reviewers {
            approvers: [token(&env, "xt-curator")?, token(&env, "xt-steward")?],
            publisher: token(&env, "xt-publisher")?,
        },
    ];
    let measured = actors(&env, "load-", profile.agents)?;
    let noisy = actors(&env, "load-b-", profile.noisy_agents)?;
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(4_096)
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("build HTTP client: {err}"))?;
    let shared = Arc::new(Shared {
        http: http.clone(),
        gateway: gateway.clone(),
        in_flight: Semaphore::new(profile.max_in_flight as usize),
        timeout: Duration::from_millis(profile.request_timeout_ms),
        mix: profile.mix.clone(),
        sizes: profile.event_sizes.clone(),
        foreign_sessions: [Mutex::new(Vec::new()), Mutex::new(Vec::new())],
        reviewers,
    });
    let mut agents = Vec::new();
    for (index, actor) in measured.into_iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| "too many agents")?;
        agents.push(Agent::new(
            index,
            actor,
            0,
            window_seed(profile.seed, index, usize::MAX, 0),
        ));
    }
    for (index, actor) in noisy.into_iter().enumerate() {
        let index = NOISY_AGENT_OFFSET + u32::try_from(index).map_err(|_| "too many agents")?;
        agents.push(Agent::new(
            index,
            actor,
            1,
            window_seed(profile.seed, index, usize::MAX, 0),
        ));
    }
    let mut report = if args.resume {
        Report::resume(&args.report, &profile)?
    } else {
        Report::new(&profile, environment)?
    };
    let tallies = [
        Arc::new(Mutex::new(Tally::new()?)),
        Arc::new(Mutex::new(Tally::new()?)),
    ];
    let mut run = Run {
        args,
        profile,
        shared,
        agents,
        tallies,
        http,
        gateway,
    };
    for (index, phase) in run.profile.phases.clone().iter().enumerate() {
        if report.phases.get(index).is_some_and(|done| done.completed) {
            continue;
        }
        run.phase(&mut report, index, phase).await?;
    }
    report.status = Status::Completed;
    report.assessment = report::assess(&report);
    report.write(&run.args.report)?;
    let failed: Vec<&report::Assessment> = report
        .assessment
        .iter()
        .filter(|check| !check.passed)
        .collect();
    for check in &failed {
        eprintln!(
            "synveda-eval load: {} {} observed {:.3} budget {:.3} (n={})",
            check.phase, check.check, check.observed, check.budget, check.n
        );
    }
    Ok(!run.args.gate || failed.is_empty())
}

impl Run {
    async fn phase(
        &mut self,
        report: &mut Report,
        index: usize,
        phase: &Phase,
    ) -> Result<(), String> {
        if report.phases.len() == index {
            report.phases.push(PhaseReport {
                name: phase.name.clone(),
                ..PhaseReport::default()
            });
        }
        let mut measured = Tally::restore(&report.phases[index].saved_measured, &KNOWN_OPS)?;
        let mut noisy = Tally::restore(&report.phases[index].saved_noisy, &KNOWN_OPS)?;
        self.fault(phase).await?;
        let window_seconds = self.profile.checkpoint_seconds;
        let windows = phase.seconds.div_ceil(window_seconds);
        let mut window = report.phases[index].windows.len() as u64;
        while window < windows {
            let offset = window * window_seconds;
            let length = Duration::from_secs(window_seconds.min(phase.seconds - offset));
            let observed = self.window(index, phase, window, length).await?;
            measured.merge(&observed.0)?;
            noisy.merge(&observed.1)?;
            let mut summary = observed.2;
            summary.index = window;
            summary.offset_seconds = offset as f64;
            let entry = &mut report.phases[index];
            entry.windows.push(summary);
            entry.measured = measured.summary();
            entry.noisy = noisy.summary();
            entry.saved_measured = measured.save();
            entry.saved_noisy = noisy.save();
            report.assessment = report::assess(report);
            report.write(&self.args.report)?;
            eprintln!(
                "synveda-eval load: {} window {} of {windows}",
                phase.name,
                window + 1
            );
            window += 1;
        }
        report.phases[index].completed = true;
        self.clear_fault(phase).await?;
        report.write(&self.args.report)
    }

    async fn fault(&self, phase: &Phase) -> Result<(), String> {
        if let Some(url) = &self.args.provider_url {
            let fault = provider::Fault {
                delay_ms: phase.provider_delay_ms,
                error_percent: phase.provider_error_percent,
            };
            let response = self
                .http
                .post(format!("{url}/control"))
                .json(&fault)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|err| format!("set stub provider fault: {err}"))?;
            if !response.status().is_success() {
                return Err(format!(
                    "stub provider refused its fault: {}",
                    response.status()
                ));
            }
        } else if phase.provider_delay_ms > 0 || phase.provider_error_percent > 0 {
            return Err(format!("phase {} needs --provider-url", phase.name));
        }
        if phase.database_fault == DatabaseFault::CpuContention {
            self.hook("start").await?;
        }
        Ok(())
    }

    async fn clear_fault(&self, phase: &Phase) -> Result<(), String> {
        if phase.database_fault == DatabaseFault::CpuContention {
            self.hook("stop").await?;
        }
        Ok(())
    }

    async fn hook(&self, action: &str) -> Result<(), String> {
        let hook = self
            .args
            .fault_hook
            .as_ref()
            .ok_or("a database-fault phase needs --fault-hook")?;
        let status = tokio::time::timeout(
            Duration::from_secs(60),
            tokio::process::Command::new(hook)
                .arg(action)
                .arg("cpu_contention")
                .status(),
        )
        .await
        .map_err(|_| format!("fault hook {action} timed out"))?
        .map_err(|err| format!("run fault hook: {err}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("fault hook {action} failed: {status}"))
        }
    }

    async fn window(
        &mut self,
        phase_index: usize,
        phase: &Phase,
        window: u64,
        length: Duration,
    ) -> Result<(Tally, Tally, Window), String> {
        let before = observe::scrape(&self.http, &self.gateway).await?;
        let worker_before = match &self.args.worker_url {
            Some(url) => Some(observe::scrape(&self.http, url).await?),
            None => None,
        };
        let _discard = self.provider_stats().await;
        let tallies = self.tallies.clone();
        let started = Instant::now();
        let end = started + length;
        let mut tasks = Vec::with_capacity(self.agents.len());
        for agent in std::mem::take(&mut self.agents) {
            let multiplier = if agent.tenant == 0 {
                phase.multiplier
            } else {
                phase.noisy_multiplier
            };
            let seed = window_seed(self.profile.seed, agent.index, phase_index, window);
            let (scheduled, dropped) = arrivals(
                &self.profile.mix,
                multiplier,
                length,
                seed,
                MAX_ARRIVALS_PER_WINDOW,
            );
            let (shared, tally) = (Arc::clone(&self.shared), Arc::clone(&tallies[agent.tenant]));
            tasks.push(tokio::spawn(drive(
                agent, shared, tally, scheduled, started, end, dropped,
            )));
        }
        let mut unsent = 0;
        for task in tasks {
            let (agent, missed) = task.await.map_err(|err| format!("agent task: {err}"))?;
            unsent += missed;
            self.agents.push(agent);
        }
        self.agents.sort_by_key(|agent| agent.index);
        let seconds = started.elapsed().as_secs_f64();
        let after = observe::scrape(&self.http, &self.gateway).await?;
        let worker = match (&self.args.worker_url, worker_before) {
            (Some(url), Some(before)) => {
                observe::delta(&before, &observe::scrape(&self.http, url).await?)
            }
            _ => Default::default(),
        };
        let provider = self.provider_stats().await;
        let mut measured = std::mem::replace(&mut *tallies[0].lock().await, Tally::new()?);
        let noisy = std::mem::replace(&mut *tallies[1].lock().await, Tally::new()?);
        if let Some(stats) = &provider {
            measured.merge(&Tally::restore(&stats.saved, &KNOWN_OPS)?)?;
        }
        let summary = Window {
            seconds,
            measured: measured.summary(),
            noisy: noisy.summary(),
            gateway: observe::delta(&before, &after),
            worker,
            gauges: after.gauges,
            provider: provider
                .map(|stats| ProviderWindow {
                    requests: stats.requests,
                    errors: stats.errors,
                })
                .unwrap_or_default(),
            processes: self.processes().await,
            database: self
                .args
                .database_samples
                .as_deref()
                .and_then(observe::latest_database_sample),
            unsent,
            ..Window::default()
        };
        Ok((measured, noisy, summary))
    }

    async fn provider_stats(&self) -> Option<provider::Stats> {
        let url = self.args.provider_url.as_ref()?;
        let response = self
            .http
            .get(format!("{url}/stats"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        response.json().await.ok()
    }

    async fn processes(&self) -> std::collections::BTreeMap<String, observe::ProcessSample> {
        let mut out = std::collections::BTreeMap::new();
        for (name, pid) in [
            ("gateway", self.args.gateway_pid),
            ("worker", self.args.worker_pid),
        ] {
            if let Some(pid) = pid
                && let Some(sample) = observe::process(pid).await
            {
                out.insert(name.to_owned(), sample);
            }
        }
        out
    }
}

/// One agent's window: wait for each scheduled time, act, and count what
/// the window closed on before it could start.
async fn drive(
    mut agent: Agent,
    shared: Arc<Shared>,
    tally: Arc<Mutex<Tally>>,
    scheduled: Vec<(Duration, Op)>,
    started: Instant,
    end: Instant,
    dropped: u64,
) -> (Agent, u64) {
    let mut unsent = dropped;
    for (position, (offset, op)) in scheduled.iter().enumerate() {
        let intended = started + *offset;
        if Instant::now() >= end {
            let missed = (scheduled.len() - position) as u64;
            unsent += missed;
            let mut tally = tally.lock().await;
            for (_, op) in &scheduled[position..] {
                tally.outcome(op.name(), Outcome::Unsent);
            }
            break;
        }
        tokio::time::sleep_until(intended.into()).await;
        agent.perform(&shared, &tally, *op, intended).await;
    }
    (agent, unsent)
}
