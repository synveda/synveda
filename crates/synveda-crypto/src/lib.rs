//! Envelope encryption for Synveda (TEN-4, ADR-0064).
//!
//! Two levels of key. A **key-encryption key** lives in the KMS and never
//! leaves it; per-scope **data keys** are stored only wrapped by it. Payloads
//! are sealed by a data key and bound to the scope, the column and the row
//! they belong to, so a ciphertext moved anywhere else fails to open.
//!
//! # What this covers, and what it does not
//!
//! Per-tenant keys seal the two things a per-tenant key can mean something
//! for: **artefacts that leave the database** (a tenant export) and
//! **secrets we have to read back** (an outbound directory credential, a
//! console session's tokens). They deliberately do **not** cover
//! Knowledge content, embeddings or retrieval projections —
//! application-level encryption there removes the lexical leg and the dense
//! leg, which is ADR-0024 in its entirety. Encryption at rest for the
//! retrieval substrate is the volume's; ADR-0064 decision 7 makes that
//! deployment obligation explicit rather than quietly shipping less.
//!
//! So: destroying a tenant's key makes its *sealed* data unreadable, and
//! TEN-5's erasure still has rows to delete. Crypto-shredding is not erasure
//! here, and this paragraph exists so nothing inherits that promise by
//! accident.
//!
//! # Layering
//!
//! This crate sits between `synveda-types` and the middle band —
//! `types ← crypto ← {policy, store, identity, audit, vedaflow}` — because
//! store, identity and vedaflow all need it and the rule forbids them
//! depending on each other (seed §8, ADR-0064 decision 13).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod aad;
mod envelope;
mod key;
mod kms;

pub use aad::{Purpose, RowKey};
pub use envelope::{
    ALGORITHM_XCHACHA20_POLY1305, SealingKey, envelope_is_deployment_scoped, envelope_version,
};
pub use key::{DataKey, KEY_LEN, KeyScope, KeyVersion};
pub use kms::{KeyManagement, Kms, LocalKms};
