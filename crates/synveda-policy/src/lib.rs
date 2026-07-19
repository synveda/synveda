//! The Policy Decision Point: `authorize(subject, action, resource, context)`
//! facade over the policy engines (Cedar embedded first; OPA/OpenFGA as
//! adapters — ADR-0002, AUTHZ-6), plus the policy packs (AUTHZ-2,
//! ADR-0014): the embedded product bundles and the loader for stored
//! custom packs.
//!
//! Every read and write passes through this crate; no code path may bypass
//! it (seed §2.2). The facade takes and returns domain types only — Cedar
//! never crosses the crate boundary (ADR-0012 decision 1) — and this crate
//! never touches storage: callers supply the rows entities are materialised
//! from and assignments the effective pack resolves from (seed §2.4), and
//! the gateway pushes stored packs in through [`Pdp::install_source`]
//! (hot reload, ADR-0012 decision 5).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod pdp;
mod request;

pub use pdp::{
    EMBEDDED_PACKS, EffectivePack, OPEN_COLLABORATION, PackOrigin, Pdp, REGULATED_STRICT, STANDARD,
    is_reserved,
};
pub use request::{Action, AuthzContext, AuthzDecision, Principal, Resource};

/// Authorization decisions, labelled by `action`, `decision`
/// (`allow`/`deny`), and `pack` (name only — versions are unbounded and go
/// to the decision log, ADR-0012 decision 6). Emitted here through the
/// `metrics` facade; described where the recorder lives (ADR-0007).
pub const AUTHZ_DECISIONS_TOTAL: &str = "synveda_authz_decisions_total";

/// Times an assigned pack name resolved to no compiled pack and the
/// decision fell back to the embedded default (ADR-0014 decision 7).
/// Nonzero means assignment data and pack sources have drifted —
/// out-of-band writes, since the store refuses referenced deletions.
pub const POLICY_PACK_FALLBACKS_TOTAL: &str = "synveda_policy_pack_fallbacks_total";
