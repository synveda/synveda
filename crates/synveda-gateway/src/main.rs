//! The Synveda gateway: axum HTTP/gRPC serving the three primitives
//! (`inject`, `recall`, `observe`) behind AuthN → tenant resolution → PDP →
//! rate limits → audit (seed §7). The ONLY binary that speaks to the outside.
//!
//! Implementation lands in Phase 1 (CTX-3, MEM-1).

use synveda_types as _;

fn main() {}
