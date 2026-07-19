//! Domain types, identifiers, and the error taxonomy shared by every Synveda crate.
//!
//! This crate is the root of the workspace dependency graph and must never depend
//! on another `synveda-*` crate (seed §8; enforced by `scripts/check-crate-deps.mjs`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod hierarchy;
mod id;
mod identity;
mod observe;
mod policy;
mod record;
mod role;
mod sensitivity;
mod tenant;

pub use error::{Error, Result};
pub use hierarchy::{HierarchyNode, ScopeKind};
pub use id::{IdentityId, ObserveEventId, RecordId, ScopeId, TenantId};
pub use identity::{Identity, IdentityKind};
pub use observe::ObserveKind;
pub use policy::PolicyAssignment;
pub use record::{RecordClass, RecordKind};
pub use role::{Role, RoleBinding};
pub use sensitivity::Sensitivity;
pub use tenant::{Tenant, TenantStatus};
