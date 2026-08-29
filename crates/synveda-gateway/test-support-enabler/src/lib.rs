//! Dev-dependency feature unifier for Synveda gateway integration tests.
//!
//! Cargo integration tests compile the library as a normal dependency, so
//! `cfg(test)` alone cannot expose a crate-internal router. This package is a
//! dev dependency which enables the non-default `test-support` feature only
//! while gateway test targets are built.

#![forbid(unsafe_code)]
