//! Tamper-evident audit log: append-only, BLAKE3 hash-chained per tenant
//! (AUD-1, ADR-0019; seed §2.5), with WORM export later (AUD-3).
//!
//! One chain per tenant: event N's hash covers event N−1's hash and a
//! canonical serialisation of N's content, so mutating any historic row —
//! content, order, linkage, or the head — breaks [`verify`]. Emission is a
//! gateway/CLI seam (the layering keeps this crate beside `store` and
//! `policy`, depending only on `types`): callers open a tenant transaction
//! via `synveda_store::rls::begin_tenant_tx` and pass its connection to
//! [`append`], so the event commits atomically with the action it records.
//!
//! The schema lives in `crates/synveda-store/migrations/0011_audit_log.sql`
//! (one embedded migrator for the workspace); the chain semantics live
//! here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod canonical;
mod chain;
mod event;
mod query;

pub use chain::{
    AUDIT_APPEND_FAILURES_TOTAL, AUDIT_EVENTS_TOTAL, AUDIT_VERIFICATIONS_TOTAL, AppendedEvent,
    BreakReason, ChainVerification, StoredEvent, append, compute_hash, genesis_hash, head_seq,
    since, tail, verify,
};
pub use event::{Actor, ActorKind, AuditAction, AuditEvent, Outcome};
pub use query::{
    AUTHORITY_ACTIONS, ChainFrame, DISCLOSURE_ACTIONS, DisclosedEntry, Disclosure, EventFilter,
    Known, Page, disclosures, fold_knowledge, frame, knowledge, search,
};
