//! The write path: redaction/secret scanning, extraction, dedup/conflict
//! detection, summarise-at-write, and embedding — feeding the `derived`
//! channel (tech plan §3).
//!
//! Redaction (MEM-2, ADR-0021) is live: [`scan`] runs in the observe ack
//! path, before persistence (seed §6). Extraction (MEM-3, ADR-0022) is
//! live: [`extraction`] is the Extractor seam and [`worker`] is the
//! observe queue's consumer, spawned by the gateway. Embedding (MEM-4,
//! ADR-0023) is live: [`embedding`] is the Embedder seam, and the worker
//! commits every record atomically with its vector — embed-or-fail.
//! Auto-promotion (FLOW-4, ADR-0033) is live: [`promotion`] sweeps
//! recall evidence out of the audit chain and opens FLOW-3 proposals
//! under the material owner's authority — a second background loop
//! beside the extraction worker, spawned by the same binary.
//! Dedup and conflict detection (MEM-5, ADR-0039) are live: [`dedup`] is
//! the judge, and the worker runs it inside the same write transaction
//! as the insert — so a restatement merges into what it restates and a
//! contradiction closes the valid window of the record it replaces,
//! atomically with the record that caused it. The store is no longer
//! ADD-only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod dedup;
pub mod embedding;
pub mod extraction;
pub mod promotion;
mod redaction;
pub mod worker;

pub use redaction::{Finding, FindingCategory, ScanOutcome, scan};
