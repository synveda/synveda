//! VedaFlow: git-style governance for knowledge assets — BLAKE3 content-addressed
//! objects/trees/commits/refs, the derived/staged/published channels, and the
//! proposal/approval lifecycle, implemented natively in Postgres (tech plan §2).
//!
//! Crate added by tech plan §5; sits in the middle tier of the dependency graph
//! (placement to be recorded in ADR-0003, FND-6). Implementation lands with FLOW-1.

use synveda_types as _;
