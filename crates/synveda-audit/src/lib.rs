//! Tamper-evident audit log: append-only, BLAKE3 hash-chained per tenant, with
//! WORM export (seed §2.5, AUD epic).
//!
//! Implementation lands with AUD-1.

use synveda_types as _;
