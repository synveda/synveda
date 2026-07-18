//! Domain types, identifiers, and the error taxonomy shared by every Synveda crate.
//!
//! This crate is the root of the workspace dependency graph and must never depend
//! on another `synveda-*` crate (seed §8; enforced by `scripts/check-crate-deps.mjs`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod id;
mod record;
mod sensitivity;

pub use error::{Error, Result};
pub use id::{IdentityId, RecordId, ScopeId, TenantId};
pub use record::{RecordClass, RecordKind};
pub use sensitivity::Sensitivity;
