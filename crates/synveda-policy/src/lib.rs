//! The Policy Decision Point: `authorize(subject, action, resource, context)`
//! facade over the policy engines (Cedar embedded first; OPA/OpenFGA as
//! adapters — ADR-0002, AUTHZ-6), plus the policy pack loader.
//!
//! Every read and write passes through this crate; no code path may bypass
//! it (seed §2.2). The facade takes and returns domain types only — Cedar
//! never crosses the crate boundary (ADR-0012 decision 1) — and this crate
//! never touches storage: callers supply the rows entities are materialised
//! from (seed §2.4), and the gateway pushes stored packs in through
//! [`Pdp::install_source`] (hot reload, ADR-0012 decision 5).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod pdp;
mod request;

pub use pdp::{BOOTSTRAP_PACK, BOOTSTRAP_VERSION, Pdp};
pub use request::{Action, AuthzContext, AuthzDecision, Principal, Resource};

/// Authorization decisions, labelled by `action`, `decision`
/// (`allow`/`deny`), and `pack` (name only — versions are unbounded and go
/// to the decision log, ADR-0012 decision 6). Emitted here through the
/// `metrics` facade; described where the recorder lives (ADR-0007).
pub const AUTHZ_DECISIONS_TOTAL: &str = "synveda_authz_decisions_total";
