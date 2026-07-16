//! The Policy Decision Point: `authorize(subject, action, resource, context)` facade
//! over the policy engines (Cedar first; OPA/OpenFGA as adapters — tech plan §1.2),
//! plus the policy pack loader.
//!
//! Every read and write passes through this crate; no code path may bypass it
//! (seed §2.2). Implementation lands with AUTHZ-1.

use synveda_types as _;
