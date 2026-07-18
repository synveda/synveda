//! Sensitivity classification.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Sensitivity classification carried by every record and asset (seed §4.2);
/// policy decisions key on it.
///
/// Variants are ordered least → most sensitive, so `Ord` comparisons read as
/// "at least as sensitive as" (`level >= Sensitivity::Confidential`).
///
/// There is deliberately no `Default`: classification is always an explicit
/// decision. Under the `regulated-strict` policy pack every write is
/// classified (seed §6); an implicit default would hide exactly the decision
/// an auditor needs to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    /// Safe for anyone, inside or outside the organisation.
    Public,
    /// Default working tier: visible within the organisation per scope policy.
    Internal,
    /// Restricted to explicitly granted scopes.
    Confidential,
    /// Highest tier: dual approval and compliance involvement for any
    /// promotion or lapse touching it (tech plan §2.4).
    Restricted,
}

impl Sensitivity {
    /// All levels, least to most sensitive.
    pub const ALL: [Sensitivity; 4] = [
        Sensitivity::Public,
        Sensitivity::Internal,
        Sensitivity::Confidential,
        Sensitivity::Restricted,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Sensitivity::Public => "public",
            Sensitivity::Internal => "internal",
            Sensitivity::Confidential => "confidential",
            Sensitivity::Restricted => "restricted",
        }
    }
}

impl fmt::Display for Sensitivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Sensitivity {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Sensitivity::Public),
            "internal" => Ok(Sensitivity::Internal),
            "confidential" => Ok(Sensitivity::Confidential),
            "restricted" => Ok(Sensitivity::Restricted),
            other => Err(Error::Invalid {
                message: format!("unknown sensitivity level: {other:?}"),
            }),
        }
    }
}
