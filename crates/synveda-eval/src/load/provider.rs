//! A loopback embedding provider that speaks TEI's `POST /embed`.
//!
//! It returns deterministic unit vectors derived from each input's BLAKE3
//! stream, so the gateway and worker exercise the 1024-dimension index
//! without a model. `POST /control` sets an added delay and an HTTP 503
//! share for fault phases; `GET /stats` returns and resets the service-time
//! histogram so provider time stays separately attributable.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::schedule::Rng;
use super::stats::{Measure, Saved, Tally};

const MAX_INPUTS: usize = 64;
const MAX_INPUT_BYTES: usize = 32_768;
const MAX_DELAY_MS: u64 = 60_000;
pub const OPERATION: &str = "provider_embed";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fault {
    pub delay_ms: u64,
    pub error_percent: u32,
}

struct Shared {
    dimension: usize,
    fault: Fault,
    rng: Rng,
    tally: Tally,
    requests: u64,
    errors: u64,
}

#[derive(Deserialize)]
struct EmbedRequest {
    inputs: Vec<String>,
}

/// Window statistics returned by `GET /stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub requests: u64,
    pub errors: u64,
    pub saved: Saved,
}

/// One input's deterministic unit vector.
pub fn vector(input: &str, dimension: usize) -> Vec<f32> {
    let bytes = input.as_bytes();
    let mut reader = blake3::Hasher::new()
        .update(&bytes[..bytes.len().min(MAX_INPUT_BYTES)])
        .finalize_xof();
    let mut raw = vec![0_u8; dimension * 2];
    reader.fill(&mut raw);
    let mut values: Vec<f32> = raw
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])))
        .collect();
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        values.iter_mut().for_each(|value| *value /= norm);
    } else if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    values
}

async fn embed(
    State(shared): State<Arc<Mutex<Shared>>>,
    Json(request): Json<EmbedRequest>,
) -> Result<Json<Vec<Vec<f32>>>, StatusCode> {
    let started = Instant::now();
    if request.inputs.is_empty() || request.inputs.len() > MAX_INPUTS {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (fault, fail, dimension) = {
        let mut guard = shared.lock().await;
        guard.requests += 1;
        let fail = guard.rng.below(100) < u64::from(guard.fault.error_percent);
        (guard.fault, fail, guard.dimension)
    };
    if fault.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(fault.delay_ms)).await;
    }
    let result = if fail {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    } else {
        Ok(Json(
            request
                .inputs
                .iter()
                .map(|input| vector(input, dimension))
                .collect(),
        ))
    };
    let mut guard = shared.lock().await;
    guard.errors += u64::from(fail);
    guard
        .tally
        .latency(OPERATION, Measure::Service, started.elapsed());
    result
}

async fn control(State(shared): State<Arc<Mutex<Shared>>>, Json(fault): Json<Fault>) -> StatusCode {
    if fault.delay_ms > MAX_DELAY_MS || fault.error_percent > 100 {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    shared.lock().await.fault = fault;
    StatusCode::NO_CONTENT
}

async fn stats(State(shared): State<Arc<Mutex<Shared>>>) -> Result<Json<Stats>, StatusCode> {
    let mut guard = shared.lock().await;
    let fresh = Tally::new().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tally = std::mem::replace(&mut guard.tally, fresh);
    let stats = Stats {
        requests: std::mem::take(&mut guard.requests),
        errors: std::mem::take(&mut guard.errors),
        saved: tally.save(),
    };
    Ok(Json(stats))
}

/// Serves until the process is stopped. Only a loopback address is accepted.
pub async fn serve(listen: SocketAddr, dimension: usize, seed: u64) -> Result<(), String> {
    if !listen.ip().is_loopback() {
        return Err("the stub provider listens on loopback only".to_owned());
    }
    if !(1..=4_096).contains(&dimension) {
        return Err("stub dimension must be 1..=4096".to_owned());
    }
    let shared = Arc::new(Mutex::new(Shared {
        dimension,
        fault: Fault::default(),
        rng: Rng::new(seed),
        tally: Tally::new()?,
        requests: 0,
        errors: 0,
    }));
    let app = Router::new()
        .route("/embed", post(embed))
        .route("/control", post(control))
        .route("/stats", get(stats))
        .route("/health", get(|| async { StatusCode::OK }))
        .layer(DefaultBodyLimit::max(MAX_INPUTS * MAX_INPUT_BYTES + 4_096))
        .with_state(shared);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|err| format!("bind stub provider {listen}: {err}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|err| format!("stub provider stopped: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_deterministic_unit_length_and_distinct() {
        let first = vector("release checklist", 1024);
        assert_eq!(first, vector("release checklist", 1024));
        assert_ne!(first, vector("release checklists", 1024));
        assert_eq!(first.len(), 1024);
        let norm = first.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "{norm}");
    }
}
