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
//! Graph linking (GRPH-2, ADR-0044) is live: [`linking`] resolves the
//! extractor's entity mentions against the vertices that already exist
//! and writes the `entity` and `episode` graphs — as a step of the same
//! write transaction, so a record and every claim about it commit
//! together. The `provenance` graph is projected from
//! `record_supersessions` rather than written, so one claim keeps one
//! system of record.
//! Retention (MEM-6, ADR-0040) is live: [`retention`] is the third
//! background loop — expiry out of the live corpus, destruction of closed
//! versions past a second horizon, and disposal of the observe staging
//! plane. It enforces nothing: the read path already refused this material
//! in the query that asked, and every horizon is read from the pack at the
//! moment the pass runs rather than stamped on a record.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod dedup;
pub mod embedding;
pub mod extraction;
pub mod linking;
pub mod promotion;
mod redaction;
pub mod retention;
mod skillrubric;
mod skillscan;
pub mod worker;

pub use redaction::{Finding, FindingCategory, ScanOutcome, scan};
pub use skillrubric::{
    CheckResult, MANIFEST_BUDGET_CHARS, MAX_SCORE, MIN_DESCRIPTION_CHARS, RUBRIC_VERSION,
    RubricScore, score_bundle,
};
pub use skillscan::{
    BundleScan, FileScan, SKILL_RULESET_VERSION, SkillFinding, scan_bundle, scan_file,
};
