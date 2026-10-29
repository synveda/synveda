//! A load profile: who acts, how often, and the ordered phases a run walks
//! through. Every rate, count and duration is bounded here so a typo cannot
//! become an unbounded workload.

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "synveda.load-profile.v1";
const MAX_AGENTS: u32 = 2_000;
const MAX_PHASES: usize = 64;
const MAX_PHASE_SECS: u64 = 172_800;
const MAX_RATE_PER_MINUTE: f64 = 600.0;
const MAX_MULTIPLIER: f64 = 100.0;
const MAX_PROVIDER_DELAY_MS: u64 = 60_000;
const MAX_EVENT_BYTES: u32 = 32_768;
const MAX_IN_FLIGHT: u32 = 4_096;

/// Per-agent mean rates, in operations per minute at multiplier 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    pub append_per_minute: f64,
    pub context_run_per_minute: f64,
    pub knowledge_search_per_minute: f64,
    pub knowledge_create_per_minute: f64,
    pub stale_edit_per_minute: f64,
    pub denied_probe_per_minute: f64,
    /// Appends after which an agent ends its Session, freezes it for
    /// Capture and opens a new one.
    pub events_per_session: u32,
}

/// Event payload sizes: most are small, a few are tool output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventSizes {
    pub small_bytes: u32,
    pub large_bytes: u32,
    /// Share of appends that carry the large payload, in percent.
    pub large_percent: u32,
}

/// What the generator and the stub provider do during one phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub name: String,
    pub seconds: u64,
    /// Scales every measured agent's rates.
    pub multiplier: f64,
    /// Scales the noisy tenant's agents; zero leaves them idle.
    pub noisy_multiplier: f64,
    /// Added before every stub embedding response.
    pub provider_delay_ms: u64,
    /// Share of stub embedding requests answered with HTTP 503, in percent.
    pub provider_error_percent: u32,
    /// Asks the operator wrapper to slow the database for this phase.
    pub database_fault: DatabaseFault,
    /// Excluded from steady-state budgets: startup and recovery transients.
    pub warmup: bool,
    /// Whether the profile's budgets judge this phase. Only a steady phase
    /// at the declared load without an injected fault may be assessed.
    pub assess: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseFault {
    None,
    CpuContention,
}

/// Proposed budgets checked against non-warmup phases with multiplier 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub append_p95_ms: f64,
    pub append_p99_ms: f64,
    pub context_run_p95_ms: f64,
    pub context_run_p99_ms: f64,
    pub knowledge_search_p99_ms: f64,
    /// Unexpected outcomes (5xx, transport errors, timeouts) per thousand.
    pub unexpected_per_thousand: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: String,
    pub name: String,
    pub seed: u64,
    /// Agents in the measured tenant and in the noisy tenant.
    pub agents: u32,
    pub noisy_agents: u32,
    pub mix: Mix,
    pub event_sizes: EventSizes,
    /// Generator-wide ceiling on requests waiting or in flight.
    pub max_in_flight: u32,
    pub request_timeout_ms: u64,
    /// Seconds between checkpoint writes of the resumable report.
    pub checkpoint_seconds: u64,
    pub phases: Vec<Phase>,
    pub budgets: Budgets,
}

impl Profile {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read profile {}: {err}", path.display()))?;
        let profile: Self = serde_json::from_str(&raw)
            .map_err(|err| format!("parse profile {}: {err}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("profile schema must be {SCHEMA}"));
        }
        if self.name.is_empty() || self.name.len() > 64 {
            return Err("profile name must be 1..=64 characters".to_owned());
        }
        if self.agents == 0 || self.agents > MAX_AGENTS || self.noisy_agents > MAX_AGENTS {
            return Err(format!(
                "agents must be 1..={MAX_AGENTS}, noisy agents at most {MAX_AGENTS}"
            ));
        }
        if self.max_in_flight == 0 || self.max_in_flight > MAX_IN_FLIGHT {
            return Err(format!("max_in_flight must be 1..={MAX_IN_FLIGHT}"));
        }
        if !(1_000..=300_000).contains(&self.request_timeout_ms) {
            return Err("request_timeout_ms must be 1000..=300000".to_owned());
        }
        if !(10..=3_600).contains(&self.checkpoint_seconds) {
            return Err("checkpoint_seconds must be 10..=3600".to_owned());
        }
        self.mix.validate()?;
        self.event_sizes.validate()?;
        if self.phases.is_empty() || self.phases.len() > MAX_PHASES {
            return Err(format!("a profile needs 1..={MAX_PHASES} phases"));
        }
        self.phases.iter().try_for_each(Phase::validate)
    }
}

impl Mix {
    fn validate(&self) -> Result<(), String> {
        let rates = [
            self.append_per_minute,
            self.context_run_per_minute,
            self.knowledge_search_per_minute,
            self.knowledge_create_per_minute,
            self.stale_edit_per_minute,
            self.denied_probe_per_minute,
        ];
        if rates
            .iter()
            .any(|rate| !rate.is_finite() || *rate < 0.0 || *rate > MAX_RATE_PER_MINUTE)
        {
            return Err(format!(
                "every per-agent rate must be 0..={MAX_RATE_PER_MINUTE} per minute"
            ));
        }
        if !(1..=10_000).contains(&self.events_per_session) {
            return Err("events_per_session must be 1..=10000".to_owned());
        }
        Ok(())
    }
}

impl EventSizes {
    fn validate(&self) -> Result<(), String> {
        if self.small_bytes == 0
            || self.small_bytes > self.large_bytes
            || self.large_bytes > MAX_EVENT_BYTES
            || self.large_percent > 100
        {
            return Err(format!(
                "event sizes need 0 < small <= large <= {MAX_EVENT_BYTES} and a percent share"
            ));
        }
        Ok(())
    }
}

impl Phase {
    fn validate(&self) -> Result<(), String> {
        let valid_name = !self.name.is_empty()
            && self.name.len() <= 48
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !valid_name {
            return Err("a phase name must be 1..=48 lower-case ASCII, digits or '_'".to_owned());
        }
        if self.seconds == 0 || self.seconds > MAX_PHASE_SECS {
            return Err(format!(
                "phase {} must last 1..={MAX_PHASE_SECS} seconds",
                self.name
            ));
        }
        for multiplier in [self.multiplier, self.noisy_multiplier] {
            if !multiplier.is_finite() || !(0.0..=MAX_MULTIPLIER).contains(&multiplier) {
                return Err(format!(
                    "phase {} multipliers must be 0..={MAX_MULTIPLIER}",
                    self.name
                ));
            }
        }
        if self.provider_delay_ms > MAX_PROVIDER_DELAY_MS || self.provider_error_percent > 100 {
            return Err(format!(
                "phase {} provider fault is out of bounds",
                self.name
            ));
        }
        let faulted = self.provider_delay_ms > 0
            || self.provider_error_percent > 0
            || self.database_fault != DatabaseFault::None;
        if self.assess && (self.warmup || faulted) {
            return Err(format!(
                "phase {} cannot be assessed while warming up or faulted",
                self.name
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn sample() -> Profile {
    Profile {
        schema: SCHEMA.to_owned(),
        name: "unit".to_owned(),
        seed: 7,
        agents: 2,
        noisy_agents: 1,
        mix: Mix {
            append_per_minute: 12.0,
            context_run_per_minute: 1.0,
            knowledge_search_per_minute: 0.5,
            knowledge_create_per_minute: 0.2,
            stale_edit_per_minute: 0.1,
            denied_probe_per_minute: 0.1,
            events_per_session: 30,
        },
        event_sizes: EventSizes {
            small_bytes: 600,
            large_bytes: 6_000,
            large_percent: 5,
        },
        max_in_flight: 64,
        request_timeout_ms: 30_000,
        checkpoint_seconds: 30,
        phases: vec![Phase {
            name: "steady".to_owned(),
            seconds: 60,
            multiplier: 1.0,
            noisy_multiplier: 0.0,
            provider_delay_ms: 0,
            provider_error_percent: 0,
            database_fault: DatabaseFault::None,
            warmup: false,
            assess: true,
        }],
        budgets: Budgets {
            append_p95_ms: 50.0,
            append_p99_ms: 100.0,
            context_run_p95_ms: 150.0,
            context_run_p99_ms: 300.0,
            knowledge_search_p99_ms: 1_000.0,
            unexpected_per_thousand: 1.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reviewed_profile_validates() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn out_of_bound_values_are_refused() {
        let mut profile = sample();
        profile.agents = 0;
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.mix.append_per_minute = f64::NAN;
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.phases[0].name = "Bad Name".to_owned();
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.phases[0].seconds = MAX_PHASE_SECS + 1;
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.event_sizes.large_bytes = MAX_EVENT_BYTES + 1;
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.schema = "other".to_owned();
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.phases[0].provider_delay_ms = 50;
        assert!(
            profile.validate().is_err(),
            "a faulted phase cannot be assessed"
        );
    }

    #[test]
    fn every_committed_profile_validates() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/load/profiles");
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let profile = Profile::load(&path).unwrap_or_else(|err| panic!("{err}"));
            names.push(profile.name);
        }
        names.sort();
        assert_eq!(
            names,
            ["first-offer-10x", "first-offer-1x", "smoke", "soak-24h"]
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value["surprise"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Profile>(value).is_err());
    }
}
