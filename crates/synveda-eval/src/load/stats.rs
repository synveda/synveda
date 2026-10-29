//! Outcome counts and latency histograms. Histograms keep microseconds with
//! three significant digits, so memory stays bounded however long a run is,
//! and they serialise to `(value, count)` pairs so a resumed run continues
//! the same distribution.

use std::collections::BTreeMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

const HIGHEST_MICROS: u64 = 3_600_000_000;

/// How one request ended, from the caller's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The expected 2xx.
    Ok,
    /// 401/403/404 where the workload expects a refusal.
    Denied,
    /// 409/412: a stale precondition.
    Stale,
    /// 429 from admission or a usage budget.
    Limited,
    /// Any other status, including a 403/404 the workload did not expect.
    Unexpected,
    /// The client deadline expired.
    Timeout,
    /// Connection or protocol failure.
    Transport,
    /// Scheduled but not started before its window closed.
    Unsent,
}

/// Which interval a histogram holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    /// From the scheduled send time to the response: includes any wait behind
    /// the agent's previous request, which is what removes coordinated
    /// omission from the tail.
    Response,
    /// From the actual send to the response.
    Service,
    /// From the scheduled to the actual send time.
    Lag,
}

#[derive(Debug, Clone)]
pub struct Tally {
    template: Histogram<u64>,
    histograms: BTreeMap<(&'static str, Measure), Histogram<u64>>,
    outcomes: BTreeMap<(&'static str, Outcome), u64>,
    statuses: BTreeMap<(&'static str, u16), u64>,
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros())
        .unwrap_or(u64::MAX)
        .clamp(1, HIGHEST_MICROS)
}

/// p50/p95/p99/max in milliseconds with the sample count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quantiles {
    pub n: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

fn quantiles(histogram: &Histogram<u64>) -> Quantiles {
    let ms = |micros: u64| micros as f64 / 1_000.0;
    Quantiles {
        n: histogram.len(),
        p50_ms: ms(histogram.value_at_quantile(0.50)),
        p95_ms: ms(histogram.value_at_quantile(0.95)),
        p99_ms: ms(histogram.value_at_quantile(0.99)),
        max_ms: ms(histogram.max()),
    }
}

/// One operation's summary in a report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OperationSummary {
    pub outcomes: BTreeMap<Outcome, u64>,
    pub latency: BTreeMap<Measure, Quantiles>,
    /// HTTP statuses behind `unexpected` outcomes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unexpected_statuses: BTreeMap<u16, u64>,
}

/// Serialised histogram state, `(microseconds, count)` per recorded bucket.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub histograms: Vec<SavedHistogram>,
    pub outcomes: Vec<(String, Outcome, u64)>,
    #[serde(default)]
    pub statuses: Vec<(String, u16, u64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedHistogram {
    pub op: String,
    pub measure: Measure,
    pub buckets: Vec<(u64, u64)>,
}

impl Tally {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            template: Histogram::new_with_bounds(1, HIGHEST_MICROS, 3)
                .map_err(|err| format!("latency histogram bounds: {err}"))?,
            histograms: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            statuses: BTreeMap::new(),
        })
    }

    /// Counts the HTTP status of an outcome the workload did not expect.
    pub fn unexpected_status(&mut self, op: &'static str, status: u16) {
        *self.statuses.entry((op, status)).or_default() += 1;
    }

    pub fn outcome(&mut self, op: &'static str, outcome: Outcome) {
        *self.outcomes.entry((op, outcome)).or_default() += 1;
    }

    pub fn latency(&mut self, op: &'static str, measure: Measure, duration: Duration) {
        let template = &self.template;
        let histogram = self
            .histograms
            .entry((op, measure))
            .or_insert_with(|| template.clone());
        histogram.saturating_record(micros(duration));
    }

    /// Adds another tally's samples and counts to this one.
    pub fn merge(&mut self, other: &Self) -> Result<(), String> {
        for (key, histogram) in &other.histograms {
            let template = &self.template;
            self.histograms
                .entry(*key)
                .or_insert_with(|| template.clone())
                .add(histogram)
                .map_err(|err| format!("merge histogram: {err}"))?;
        }
        for (key, count) in &other.outcomes {
            *self.outcomes.entry(*key).or_default() += count;
        }
        for (key, count) in &other.statuses {
            *self.statuses.entry(*key).or_default() += count;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn count(&self, op: &str, outcome: Outcome) -> u64 {
        self.outcomes
            .iter()
            .filter(|((name, kind), _)| *name == op && *kind == outcome)
            .map(|(_, count)| *count)
            .sum()
    }

    #[cfg(test)]
    pub fn total(&self) -> u64 {
        self.outcomes.values().sum()
    }

    pub fn summary(&self) -> BTreeMap<String, OperationSummary> {
        let mut summary: BTreeMap<String, OperationSummary> = BTreeMap::new();
        for ((op, outcome), count) in &self.outcomes {
            summary
                .entry((*op).to_owned())
                .or_default()
                .outcomes
                .insert(*outcome, *count);
        }
        for ((op, measure), histogram) in &self.histograms {
            summary
                .entry((*op).to_owned())
                .or_default()
                .latency
                .insert(*measure, quantiles(histogram));
        }
        for ((op, status), count) in &self.statuses {
            summary
                .entry((*op).to_owned())
                .or_default()
                .unexpected_statuses
                .insert(*status, *count);
        }
        summary
    }

    pub fn save(&self) -> Saved {
        let histograms = self
            .histograms
            .iter()
            .map(|((op, measure), histogram)| SavedHistogram {
                op: (*op).to_owned(),
                measure: *measure,
                buckets: histogram
                    .iter_recorded()
                    .map(|value| (value.value_iterated_to(), value.count_at_value()))
                    .collect(),
            })
            .collect();
        let outcomes = self
            .outcomes
            .iter()
            .map(|((op, outcome), count)| ((*op).to_owned(), *outcome, *count))
            .collect();
        let statuses = self
            .statuses
            .iter()
            .map(|((op, status), count)| ((*op).to_owned(), *status, *count))
            .collect();
        Saved {
            histograms,
            outcomes,
            statuses,
        }
    }

    /// Restores state saved by [`Tally::save`]; names outside `known` are
    /// refused rather than invented.
    pub fn restore(saved: &Saved, known: &[&'static str]) -> Result<Self, String> {
        let name = |raw: &str| {
            known
                .iter()
                .copied()
                .find(|candidate| *candidate == raw)
                .ok_or_else(|| format!("checkpoint names an unknown operation {raw:?}"))
        };
        let mut tally = Self::new()?;
        for saved in &saved.histograms {
            let template = tally.template.clone();
            let histogram = tally
                .histograms
                .entry((name(&saved.op)?, saved.measure))
                .or_insert(template);
            for (value, count) in &saved.buckets {
                histogram
                    .record_n(*value.min(&HIGHEST_MICROS), *count)
                    .map_err(|err| format!("restore histogram: {err}"))?;
            }
        }
        for (op, outcome, count) in &saved.outcomes {
            *tally.outcomes.entry((name(op)?, *outcome)).or_default() += count;
        }
        for (op, status, count) in &saved.statuses {
            *tally.statuses.entry((name(op)?, *status)).or_default() += count;
        }
        Ok(tally)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_and_outcomes_summarise() {
        let mut tally = Tally::new().unwrap();
        for millis in 1..=100 {
            tally.latency("append", Measure::Response, Duration::from_millis(millis));
            tally.outcome("append", Outcome::Ok);
        }
        tally.outcome("append", Outcome::Limited);
        let summary = tally.summary();
        let append = &summary["append"];
        assert_eq!(append.outcomes[&Outcome::Ok], 100);
        assert_eq!(append.outcomes[&Outcome::Limited], 1);
        let response = append.latency[&Measure::Response];
        assert_eq!(response.n, 100);
        assert!((49.0..=51.0).contains(&response.p50_ms), "{response:?}");
        assert!((98.0..=100.5).contains(&response.p99_ms), "{response:?}");
        assert_eq!(tally.total(), 101);
    }

    #[test]
    fn save_and_restore_preserve_the_distribution() {
        let mut tally = Tally::new().unwrap();
        for millis in [3, 7, 7, 250, 4_000] {
            tally.latency(
                "context_run",
                Measure::Service,
                Duration::from_millis(millis),
            );
        }
        tally.outcome("context_run", Outcome::Timeout);
        tally.unexpected_status("context_run", 503);
        let restored = Tally::restore(&tally.save(), &["context_run"]).unwrap();
        assert_eq!(restored.summary(), tally.summary());
        let mut merged = Tally::new().unwrap();
        merged.merge(&tally).unwrap();
        merged.merge(&restored).unwrap();
        assert_eq!(merged.count("context_run", Outcome::Timeout), 2);
        assert!(Tally::restore(&tally.save(), &["append"]).is_err());
    }
}
