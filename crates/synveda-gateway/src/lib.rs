//! The Synveda gateway: axum HTTP/gRPC serving the three primitives
//! (`inject`, `recall`, `observe`) behind AuthN → tenant resolution → PDP →
//! rate limits → audit (seed §7). The ONLY binary that speaks to the outside.
//!
//! The three primitives land later in Phase 1 (CTX-3, MEM-1). What exists
//! today: the FND-5 observability baseline (OTel tracing gateway→core→store,
//! Prometheus `/metrics`, ops probes — ADR-0007) and the TEN-1 tenant plane
//! (bearer-token → tenant resolution middleware guarding `/v1`, task-local
//! context, `tenant.id` on every request trace — ADR-0008).
//!
//! Since AUTHZ-1 the PDP stage is real: the embedded Cedar engine
//! (ADR-0002/ADR-0012) decides every hierarchy admin operation, and the
//! pack refresher hot-swaps per-tenant policy packs. Since AUTH-2 a first
//! login JIT-provisions the subject into the hierarchy (ADR-0013), and
//! every PDP decision carries the identity's quarantine status.
//!
//! This is a library crate only so integration tests can build the router;
//! nothing outside the workspace consumes it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod app;
pub mod auth;
pub mod authz;
pub mod error;
pub mod hierarchy;
pub mod policy;
mod provision;
pub mod roles;
pub mod telemetry;
pub mod tenant;
