//! Bounded Open Knowledge Format v0.2 exchange (CPR-27, ADR-0087).
//!
//! This leaf crate knows the external format and shared value types only. It
//! cannot persist, authorise, publish, fetch a URL, run Git or execute bundle
//! content. The gateway supplies already authenticated bytes and decides what
//! current Knowledge may enter an export.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod archive;
mod export;
mod format;

pub use export::{ExportBundle, ExportFile, ExportKnowledge, ExportRelation, ExportSource};
pub use format::{
    ArtifactKind, BundleEncoding, BundleInput, BundleInspection, ImportArtifact, InputEntry,
    InputEntryKind, KnowledgeFormatAdapter, OkfAdapter, ProposedConcept, ProposedLink,
    SourceDescriptor, SourceKind,
};

/// The sole format version this adapter implements.
pub const OKF_VERSION: &str = "0.2";
/// Canonical upstream repository pinned by ADR-0087.
pub const OKF_SPEC_REPOSITORY: &str =
    "https://github.com/GoogleCloudPlatform/open-knowledge-format";
/// Exact canonical specification revision verified on 2026-08-25.
pub const OKF_SPEC_COMMIT: &str = "ad30107c31c06aec8a7d5636e0d1058118604e6f";
/// Verification date for the pinned upstream revision.
pub const OKF_SPEC_VERIFIED_AT: &str = "2026-08-25";

/// Maximum encoded archive bytes accepted by the adapter.
pub const MAX_ARCHIVE_BYTES: usize = 1_500_000;
/// Maximum total expanded bytes across admitted artifacts.
pub const MAX_EXPANDED_BYTES: usize = 4_000_000;
/// Maximum bytes in one Markdown artifact.
pub const MAX_ARTIFACT_BYTES: usize = 262_144;
/// Maximum files in one bundle.
pub const MAX_ARTIFACTS: usize = 2_000;
/// Maximum YAML frontmatter bytes in one concept.
pub const MAX_FRONTMATTER_BYTES: usize = 32_768;
