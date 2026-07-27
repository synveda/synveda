//! The Synveda gateway: axum HTTP/gRPC serving the three primitives
//! (`inject`, `recall`, `observe`) behind AuthN → tenant resolution → PDP →
//! rate limits → audit (seed §7). The ONLY binary that speaks to the outside.
//!
//! Since MEM-1 the first primitive is live: `POST /v1/observe` admits
//! batched session events into the RLS-staged buffer with PGMQ work
//! signals for the pipeline (ADR-0020); `inject`/`recall` land with
//! CTX-1..3. Also here: the FND-5 observability baseline (OTel tracing
//! gateway→core→store, Prometheus `/metrics`, ops probes — ADR-0007) and
//! the TEN-1 tenant plane (bearer-token → tenant resolution middleware
//! guarding `/v1`, task-local context, `tenant.id` on every request
//! trace — ADR-0008).
//!
//! Since AUTHZ-1 the PDP stage is real: the embedded Cedar engine
//! (ADR-0002/ADR-0012) decides every hierarchy admin operation, and the
//! pack refresher hot-swaps per-tenant policy packs. Since AUTH-2 a first
//! login JIT-provisions the subject into the hierarchy (ADR-0013), and
//! every PDP decision carries the identity's quarantine status.
//!
//! Since AUD-1 the audit stage is real too (ADR-0019): every admin
//! mutation, allowed read, denial, provisioning, and seam rejection chains
//! into the tenant's hash-chained audit log through the [`audit`] seams.
//!
//! Since FLOW-2 the VedaFlow trust boundary has a surface ([`channels`],
//! ADR-0031), and since FLOW-3 a review in front of it ([`proposals`] and
//! [`curators`], ADR-0032): both publish paths resolve one approval
//! matrix at [`approvals`], so what it takes to move content across the
//! boundary is answered in one place.
//!
//! This is a library crate only so integration tests can build the router;
//! nothing outside the workspace consumes it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod app;
mod approvals;
mod audit;
pub mod auth;
pub mod authz;
pub mod channels;
pub mod curators;
pub mod error;
pub mod hierarchy;
mod inject;
pub mod lapses;
pub mod observe;
pub mod policy;
pub mod proposals;
mod provision;
pub mod quarantine;
mod recall;
pub mod roles;
pub mod service_identities;
pub mod telemetry;
pub mod tenant;
