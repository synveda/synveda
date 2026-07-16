//! Storage traits and their Postgres/pgvector/AGE implementations, including the
//! bitemporal record tables and the `VectorIndex` trait that isolates pgvector
//! from the Qdrant scale-out path (tech plan §1.1).
//!
//! Implementation lands with FND-4.

use synveda_types as _;
