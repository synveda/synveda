//! Session-event capture, extraction, redaction, Skill scanning and Knowledge
//! embedding seams. Extraction produces reviewable capture candidates; it
//! never publishes active Knowledge directly.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod capture_worker;
mod chain;
pub mod embedding;
pub mod extraction;
mod provider_url;
mod redaction;
mod skillrubric;
mod skillscan;

pub use redaction::{Finding, FindingCategory, ScanOutcome, scan};
pub use skillrubric::{
    CheckResult, MANIFEST_BUDGET_CHARS, MAX_SCORE, MIN_DESCRIPTION_CHARS, RUBRIC_VERSION,
    RubricScore, score_bundle,
};
pub use skillscan::{
    BundleScan, FileScan, SKILL_RULESET_VERSION, SkillFinding, scan_bundle, scan_file,
};
