//! The Synveda gateway: axum HTTP/gRPC serving the three primitives
//! (`inject`, `recall`, `observe`) behind AuthN → tenant resolution → PDP →
//! rate limits → audit (seed §7). The ONLY binary that speaks to the outside.
//!
//! The three primitives land in Phase 1 (CTX-3, MEM-1). What exists today is
//! the FND-5 observability baseline: OTel tracing wired through
//! gateway→core→store, a Prometheus `/metrics` endpoint (including the
//! `synveda_tokens_per_inject` histogram the composition engine will record
//! into), and the ops-plane probes `/healthz` and `/readyz` (ADR-0007).
//!
//! This is a library crate only so integration tests can build the router;
//! nothing outside the workspace consumes it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod app;
pub mod telemetry;
