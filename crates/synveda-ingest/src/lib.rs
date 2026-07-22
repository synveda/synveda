//! The write path: redaction/secret scanning, extraction, dedup/conflict
//! detection, summarise-at-write, and embedding — feeding the `derived`
//! channel (tech plan §3).
//!
//! Redaction (MEM-2, ADR-0021) is live: [`scan`] runs in the observe ack
//! path, before persistence (seed §6). Extraction (MEM-3, ADR-0022) is
//! live: [`extraction`] is the Extractor seam and [`worker`] is the
//! observe queue's consumer, spawned by the gateway. Dedup (MEM-5) and
//! embedding (MEM-4) land next on the same seams.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod extraction;
mod redaction;
pub mod worker;

pub use redaction::{Finding, FindingCategory, ScanOutcome, scan};
