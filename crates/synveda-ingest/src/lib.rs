//! The write path: redaction/secret scanning, extraction, dedup/conflict
//! detection, summarise-at-write, and embedding — run as Temporal activities
//! feeding the `derived` channel (tech plan §3).
//!
//! Implementation lands with MEM-2/MEM-3.

use synveda_types as _;
