//! Bounded Open Knowledge Format v0.2 exchange (CPR-27, ADR-0087).
//!
//! This leaf crate knows the external format and shared value types only. It
//! cannot persist, authorise, publish, fetch a URL, run Git or execute bundle
//! content. The gateway supplies already authenticated bytes and decides what
//! current Knowledge may enter an export.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use synveda_types::Result;

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

/// Enumerate one local directory into the same inert entry envelope accepted
/// by the public API.
///
/// This is the controlled-client side of ADR-0087 decision 8: a CLI may read
/// the path its user selected, while a gateway receives only validated bytes.
/// Symlinks and special files remain entries the adapter refuses; `.git`
/// administration data is never admitted.
///
/// # Errors
///
/// Returns [`synveda_types::Error::Invalid`] for an unreadable/non-directory
/// root, a link or special entry, non-UTF-8/path-escaping names, or a bundle
/// beyond the adapter's bounds.
pub fn read_local_directory(root: &Path) -> Result<Vec<InputEntry>> {
    archive::directory_entries(root)
}

/// Validate and canonicalise a bundle-relative slash-separated logical path.
///
/// Export clients repeat this check before joining a server-returned path to
/// a local output directory. The gateway is authoritative, but its answer is
/// still network input to the filesystem-owning client.
///
/// # Errors
///
/// Returns [`synveda_types::Error::Invalid`] for absolute paths, parent
/// traversal, empty/dot components, platform separators or overlong paths.
pub fn normalise_logical_path(path: &str) -> Result<String> {
    archive::normalise_path(path)
}

/// Verify that a value received from the public export API is the exact,
/// deterministic bundle this pinned adapter defines.
///
/// A filesystem-owning client calls this before creating any output path.
/// It rechecks version/spec pins, stable path ordering, per-file hashes and
/// the aggregate digest rather than treating a network response as trusted
/// filesystem instructions.
///
/// # Errors
///
/// Returns [`synveda_types::Error::Invalid`] when any path, hash, ordering,
/// format pin or aggregate digest is inconsistent.
pub fn validate_export_bundle(bundle: &ExportBundle) -> Result<()> {
    export::validate_bundle(bundle)
}
