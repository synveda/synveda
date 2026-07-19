//! The write path: redaction/secret scanning, extraction, dedup/conflict
//! detection, summarise-at-write, and embedding — feeding the `derived`
//! channel (tech plan §3).
//!
//! Redaction (MEM-2, ADR-0021) is live: [`scan`] runs in the observe ack
//! path, before persistence (seed §6). Extraction and beyond land with
//! MEM-3+.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod redaction;

pub use redaction::{Finding, FindingCategory, ScanOutcome, scan};
