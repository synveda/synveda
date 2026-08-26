//! Tamper-evident audit log: append-only, BLAKE3 hash-chained per tenant
//! (AUD-1, ADR-0019; seed §2.5), with deterministic offline-verifiable
//! prefix export (CPR-33, ADR-0092). External WORM retention remains AUD-3.
//!
//! One chain per tenant: event N's hash covers event N−1's hash and a
//! canonical serialisation of N's content, so mutating any historic row —
//! content, order, linkage, or the head — breaks [`verify`]. Emission is a
//! gateway/CLI seam (the layering keeps this crate beside `store` and
//! `policy`, depending only on `types`): callers open a tenant transaction
//! via `synveda_store::rls::begin_tenant_tx` and pass its connection to
//! [`append`], so the event commits atomically with the action it records.
//!
//! The `audit_log` and `audit_chain_heads` schema lives in the epoch baseline
//! at `crates/synveda-store/migrations/0001_context_platform.sql`; chain
//! semantics live here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod canonical;
mod chain;
mod event;
mod export;
mod query;

pub use chain::{
    AUDIT_APPEND_FAILURES_TOTAL, AUDIT_EVENTS_TOTAL, AUDIT_VERIFICATIONS_TOTAL, AppendedEvent,
    BreakReason, ChainVerification, StoredEvent, append, compute_hash, genesis_hash, head_seq,
    since, tail, verify,
};
pub use event::{Actor, ActorKind, AuditAction, AuditEvent, Outcome};
pub use export::{
    EXPORT_CANONICALIZATION, EXPORT_FORMAT, EXPORT_HASH_ALGORITHM, OfflineVerification,
    verify_export,
};
pub use query::{
    AUTHORITY_ACTIONS, ChainFrame, DISCLOSURE_ACTIONS, DisclosedEntry, Disclosure, EventFilter,
    Known, Page, disclosures, export_page, fold_knowledge, frame, knowledge, search,
};
