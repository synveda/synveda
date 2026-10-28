//! Server-side observation without touching the database: Prometheus
//! histograms and gauges scraped from the gateway and worker, process
//! RSS/CPU from `ps`, and the latest line of a database sample file the
//! operator wrapper maintains.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Histogram families whose window deltas a report keeps. The gateway's own
/// HTTP histogram is reduced to method and route, never status or tenant.
pub const HISTOGRAMS: [&str; 7] = [
    "synveda_http_request_duration_seconds",
    "synveda_admission_wait_seconds",
    "synveda_context_planner_stage_duration_seconds",
    "synveda_db_pool_acquire_seconds",
    "synveda_capture_extractor_seconds",
    "synveda_capture_queue_wait_seconds",
    "synveda_knowledge_index_lag_seconds",
];

/// Gauges whose latest value a report keeps.
pub const GAUGES: [&str; 4] = [
    "synveda_db_pool_connections",
    "synveda_admission_in_flight",
    "synveda_worker_heartbeat_age_seconds",
    "synveda_capture_provider_slots_held",
];

const MAX_EXPOSITION_BYTES: usize = 8 * 1024 * 1024;
const MAX_SERIES: usize = 4_096;

/// Cumulative bucket counts for one series: `(upper bound, count)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series {
    pub buckets: Vec<(f64, f64)>,
    pub sum: f64,
    pub count: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scrape {
    pub histograms: BTreeMap<String, Series>,
    pub gauges: BTreeMap<String, f64>,
}

/// `name{labels}` with `le`, `status` and `tenant` labels removed.
fn series_key(name: &str, labels: &str) -> String {
    let kept: Vec<&str> = labels
        .split(',')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or_default().trim();
            !pair.is_empty() && !matches!(key, "le" | "status" | "tenant")
        })
        .collect();
    if kept.is_empty() {
        name.to_owned()
    } else {
        format!("{name}{{{}}}", kept.join(","))
    }
}

fn label(labels: &str, wanted: &str) -> Option<f64> {
    let value = labels.split(',').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == wanted).then(|| value.trim().trim_matches('"'))
    })?;
    if value == "+Inf" {
        Some(f64::INFINITY)
    } else {
        value.parse().ok()
    }
}

fn split_sample(line: &str) -> Option<(&str, &str, f64)> {
    let (head, value) = line.rsplit_once(' ')?;
    let value: f64 = value.trim().parse().ok()?;
    match head.split_once('{') {
        Some((name, rest)) => Some((name, rest.strip_suffix('}')?, value)),
        None => Some((head, "", value)),
    }
}

/// Parses the families this harness keeps from one exposition.
pub fn parse(exposition: &str) -> Scrape {
    let mut scrape = Scrape::default();
    for line in exposition.lines().filter(|line| !line.starts_with('#')) {
        let Some((name, labels, value)) = split_sample(line) else {
            continue;
        };
        if scrape.histograms.len() + scrape.gauges.len() >= MAX_SERIES {
            break;
        }
        if let Some(family) = HISTOGRAMS.iter().find(|family| name.starts_with(**family)) {
            let series = scrape
                .histograms
                .entry(series_key(family, labels))
                .or_default();
            match &name[family.len()..] {
                "_bucket" => {
                    if let Some(bound) = label(labels, "le") {
                        merge_bucket(&mut series.buckets, bound, value);
                    }
                }
                "_sum" => series.sum += value,
                "_count" => series.count += value,
                _ => {}
            }
        } else if GAUGES.contains(&name) {
            *scrape.gauges.entry(series_key(name, labels)).or_default() += value;
        }
    }
    scrape
}

fn merge_bucket(buckets: &mut Vec<(f64, f64)>, bound: f64, value: f64) {
    match buckets.iter_mut().find(|(known, _)| *known == bound) {
        Some((_, count)) => *count += value,
        None => {
            buckets.push((bound, value));
            buckets.sort_by(|left, right| left.0.total_cmp(&right.0));
        }
    }
}

/// Window summary of one histogram series, estimated from bucket deltas the
/// way `histogram_quantile` does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerQuantiles {
    pub n: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

fn quantile(buckets: &[(f64, f64)], total: f64, q: f64) -> f64 {
    let rank = q * total;
    let mut previous = (0.0, 0.0);
    for (bound, count) in buckets {
        if *count >= rank {
            if bound.is_infinite() {
                return previous.0;
            }
            let within = (*count - previous.1).max(f64::MIN_POSITIVE);
            return previous.0 + (bound - previous.0) * ((rank - previous.1) / within);
        }
        previous = (*bound, *count);
    }
    previous.0
}

/// Per-series deltas between two scrapes; series with no new samples drop.
pub fn delta(before: &Scrape, after: &Scrape) -> BTreeMap<String, ServerQuantiles> {
    let mut out = BTreeMap::new();
    for (key, now) in &after.histograms {
        let then = before.histograms.get(key).cloned().unwrap_or_default();
        let count = now.count - then.count;
        if count <= 0.0 {
            continue;
        }
        let buckets: Vec<(f64, f64)> = now
            .buckets
            .iter()
            .map(|(bound, total)| {
                let old = then
                    .buckets
                    .iter()
                    .find(|(known, _)| known == bound)
                    .map_or(0.0, |(_, value)| *value);
                (*bound, total - old)
            })
            .collect();
        out.insert(
            key.clone(),
            ServerQuantiles {
                n: count,
                mean_ms: (now.sum - then.sum) / count * 1_000.0,
                p50_ms: quantile(&buckets, count, 0.50) * 1_000.0,
                p95_ms: quantile(&buckets, count, 0.95) * 1_000.0,
                p99_ms: quantile(&buckets, count, 0.99) * 1_000.0,
            },
        );
    }
    out
}

pub async fn scrape(client: &reqwest::Client, url: &str) -> Result<Scrape, String> {
    let response = client
        .get(format!("{url}/metrics"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| format!("scrape {url}: {err}"))?;
    let body = response
        .text()
        .await
        .map_err(|err| format!("read scrape {url}: {err}"))?;
    if body.len() > MAX_EXPOSITION_BYTES {
        return Err(format!(
            "scrape {url} exceeded {MAX_EXPOSITION_BYTES} bytes"
        ));
    }
    Ok(parse(&body))
}

/// Resident memory and accumulated CPU of one process.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProcessSample {
    pub rss_kib: u64,
    pub cpu_seconds: f64,
}

/// Parses `ps -o time=` forms: `[[dd-]hh:]mm:ss[.ff]`.
pub fn cpu_seconds(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let (days, rest) = match raw.split_once('-') {
        Some((days, rest)) => (days.parse::<f64>().ok()?, rest),
        None => (0.0, raw),
    };
    let mut seconds = 0.0;
    for part in rest.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + seconds)
}

pub async fn process(pid: u32) -> Option<ProcessSample> {
    let output = tokio::process::Command::new("ps")
        .args(["-o", "rss=", "-o", "time=", "-p", &pid.to_string()])
        .output()
        .await
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut fields = text.split_whitespace();
    let rss_kib = fields.next()?.parse().ok()?;
    let cpu_seconds = cpu_seconds(fields.next()?)?;
    Some(ProcessSample {
        rss_kib,
        cpu_seconds,
    })
}

/// The newest complete JSON line of the wrapper's database sample file.
pub fn latest_database_sample(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = r#"# HELP synveda_admission_wait_seconds wait
synveda_admission_wait_seconds_bucket{class="context_compose",le="0.01"} 10
synveda_admission_wait_seconds_bucket{class="context_compose",le="0.1"} 10
synveda_admission_wait_seconds_bucket{class="context_compose",le="+Inf"} 10
synveda_admission_wait_seconds_sum{class="context_compose"} 0.02
synveda_admission_wait_seconds_count{class="context_compose"} 10
synveda_db_pool_connections{state="idle"} 3
synveda_unrelated_total 9
"#;

    const AFTER: &str = r#"synveda_admission_wait_seconds_bucket{class="context_compose",le="0.01"} 60
synveda_admission_wait_seconds_bucket{class="context_compose",le="0.1"} 110
synveda_admission_wait_seconds_bucket{class="context_compose",le="+Inf"} 110
synveda_admission_wait_seconds_sum{class="context_compose"} 3.02
synveda_admission_wait_seconds_count{class="context_compose"} 110
synveda_http_request_duration_seconds_bucket{method="POST",route="/v1/x",status="200",le="0.05"} 4
synveda_http_request_duration_seconds_bucket{method="POST",route="/v1/x",status="503",le="0.05"} 1
synveda_http_request_duration_seconds_bucket{method="POST",route="/v1/x",status="200",le="+Inf"} 4
synveda_http_request_duration_seconds_bucket{method="POST",route="/v1/x",status="503",le="+Inf"} 1
synveda_http_request_duration_seconds_sum{method="POST",route="/v1/x",status="200"} 0.1
synveda_http_request_duration_seconds_sum{method="POST",route="/v1/x",status="503"} 0.02
synveda_http_request_duration_seconds_count{method="POST",route="/v1/x",status="200"} 4
synveda_http_request_duration_seconds_count{method="POST",route="/v1/x",status="503"} 1
synveda_db_pool_connections{state="idle"} 1
"#;

    #[test]
    fn deltas_estimate_window_quantiles_and_drop_status_labels() {
        let deltas = delta(&parse(BEFORE), &parse(AFTER));
        let wait = &deltas[r#"synveda_admission_wait_seconds{class="context_compose"}"#];
        assert_eq!(wait.n, 100.0);
        assert!((wait.mean_ms - 30.0).abs() < 1e-9);
        assert!(wait.p50_ms <= 10.0 + 1e-9, "{wait:?}");
        assert!(wait.p95_ms > 10.0 && wait.p95_ms <= 100.0, "{wait:?}");
        let http = &deltas[r#"synveda_http_request_duration_seconds{method="POST",route="/v1/x"}"#];
        assert_eq!(http.n, 5.0);
        assert_eq!(
            parse(AFTER).gauges[r#"synveda_db_pool_connections{state="idle"}"#],
            1.0
        );
        assert!(
            !parse(BEFORE)
                .gauges
                .keys()
                .any(|key| key.contains("unrelated"))
        );
    }

    #[test]
    fn ps_cpu_times_parse() {
        assert_eq!(cpu_seconds("0:01.50"), Some(1.5));
        assert_eq!(cpu_seconds("01:02:03"), Some(3_723.0));
        assert_eq!(cpu_seconds("2-00:00:01"), Some(172_801.0));
        assert_eq!(cpu_seconds("junk"), None);
    }
}
