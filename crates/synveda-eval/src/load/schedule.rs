//! Deterministic open-loop arrival times. Each agent's operations follow
//! independent Poisson processes whose seed is derived from the profile seed,
//! the agent, the phase and the window, so a resumed run schedules exactly
//! what an uninterrupted one would have from the same window onward.

use std::time::Duration;

use super::profile::Mix;

/// One scheduled public-API operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Append,
    ContextRun,
    KnowledgeSearch,
    KnowledgeCreate,
    StaleEdit,
    DeniedProbe,
}

impl Op {
    pub const ALL: [Self; 6] = [
        Self::Append,
        Self::ContextRun,
        Self::KnowledgeSearch,
        Self::KnowledgeCreate,
        Self::StaleEdit,
        Self::DeniedProbe,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::ContextRun => "context_run",
            Self::KnowledgeSearch => "knowledge_search",
            Self::KnowledgeCreate => "knowledge_create",
            Self::StaleEdit => "stale_edit",
            Self::DeniedProbe => "denied_probe",
        }
    }

    fn per_minute(self, mix: &Mix) -> f64 {
        match self {
            Self::Append => mix.append_per_minute,
            Self::ContextRun => mix.context_run_per_minute,
            Self::KnowledgeSearch => mix.knowledge_search_per_minute,
            Self::KnowledgeCreate => mix.knowledge_create_per_minute,
            Self::StaleEdit => mix.stale_edit_per_minute,
            Self::DeniedProbe => mix.denied_probe_per_minute,
        }
    }
}

/// SplitMix64: reproducible, dependency-free and good enough for arrivals.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in (0, 1).
    pub fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound.max(1)
    }
}

/// The seed for one agent's arrivals in one window of one phase.
pub fn window_seed(profile_seed: u64, agent: u32, phase: usize, window: u64) -> u64 {
    let mut rng = Rng::new(profile_seed);
    let mut mixed = rng.next_u64() ^ u64::from(agent).wrapping_mul(0xA24B_AED4_963E_E407);
    mixed ^= (phase as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25);
    mixed ^ window.wrapping_mul(0xD6E8_FEB8_6659_FD93)
}

/// Every arrival in `[0, length)` for one agent, sorted by offset.
///
/// `cap` bounds the allocation; arrivals beyond it are dropped and counted
/// so the report shows the shortfall instead of hiding it.
pub fn arrivals(
    mix: &Mix,
    multiplier: f64,
    length: Duration,
    seed: u64,
    cap: usize,
) -> (Vec<(Duration, Op)>, u64) {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    let mut dropped = 0_u64;
    let horizon = length.as_secs_f64();
    for op in Op::ALL {
        let rate = op.per_minute(mix) * multiplier / 60.0;
        if rate <= 0.0 {
            continue;
        }
        let mut at = 0.0_f64;
        loop {
            at += -rng.unit().ln() / rate;
            if at >= horizon {
                break;
            }
            if out.len() >= cap {
                dropped += 1;
                continue;
            }
            out.push((Duration::from_secs_f64(at), op));
        }
    }
    out.sort();
    (out, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::profile::sample;

    #[test]
    fn arrivals_are_deterministic_sorted_and_bounded() {
        let mix = sample().mix;
        let seed = window_seed(7, 3, 1, 9);
        let (first, dropped) = arrivals(&mix, 1.0, Duration::from_secs(600), seed, 10_000);
        let (second, _) = arrivals(&mix, 1.0, Duration::from_secs(600), seed, 10_000);
        assert_eq!(first, second);
        assert_eq!(dropped, 0);
        assert!(first.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert!(first.iter().all(|(at, _)| *at < Duration::from_secs(600)));
        let appends = first.iter().filter(|(_, op)| *op == Op::Append).count();
        // Twelve a minute over ten minutes: Poisson mean 120.
        assert!((80..=160).contains(&appends), "{appends}");
    }

    #[test]
    fn a_cap_drops_and_counts_instead_of_growing() {
        let mix = sample().mix;
        let (kept, dropped) = arrivals(&mix, 10.0, Duration::from_secs(600), 1, 50);
        assert_eq!(kept.len(), 50);
        assert!(dropped > 0);
    }

    #[test]
    fn windows_and_agents_get_distinct_streams() {
        assert_ne!(window_seed(1, 0, 0, 0), window_seed(1, 1, 0, 0));
        assert_ne!(window_seed(1, 0, 0, 0), window_seed(1, 0, 0, 1));
        assert_ne!(window_seed(1, 0, 0, 0), window_seed(1, 0, 1, 0));
        let mix = sample().mix;
        let (none, _) = arrivals(&mix, 0.0, Duration::from_secs(600), 1, 100);
        assert!(none.is_empty());
    }
}
