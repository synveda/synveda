//! FLOW-8: deterministic Git projections and bounded local/GitHub transports.
//! The caller supplies already-authorized immutable evidence; this adapter
//! has no database or policy authority. See ADR-0138 for the export format.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod destinations;
mod github;
mod projection;
mod transport;
mod wire;

pub use destinations::{Destination, ExportTransport};
pub use github::{GitHubCredential, GitHubTarget, GitHubTargets, GitHubTransport};

pub use projection::{
    CommitEvidence, EntryEvidence, MAX_BYTES, MAX_COMMITS, MAX_ENTRIES, MAX_OBJECTS, Manifest,
    ObjectEvidence, PinEvidence, Projection, Snapshot, TreeEvidence, validate_target,
};
pub use transport::{
    LocalTransport, MAX_EXPORT_STATES, MAX_VERIFICATION_BYTES, Transport, verify_repository,
};
