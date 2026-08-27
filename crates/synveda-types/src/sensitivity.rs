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
    /// promotion or governed relaxation touching it (tech plan §2.4).
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

    /// The working tier: what a request means when it says nothing, and
    /// what the extraction pipeline floors its proposals at (ADR-0022
    /// decision 7).
    pub const WORKING: Sensitivity = Sensitivity::Internal;

    /// The highest tier the extraction pipeline may assign (AUTHZ-5,
    /// ADR-0038 decision 8).
    ///
    /// `restricted` is defined by the invariant approval floor as the tier
    /// carrying a compliance signature (ADR-0032 decision 4), and an
    /// uncalibrated, self-reported model judgement cannot manufacture one —
    /// so a model that says `restricted` gets this instead, and the top tier
    /// arrives only through a reviewed reclassification.
    pub const MAX_DERIVED: Sensitivity = Sensitivity::Confidential;

    /// Every tier at or below `self`, ascending — the set form of a
    /// ceiling.
    ///
    /// The read path pushes a *set* rather than a ceiling (ADR-0038
    /// decision 3), because a per-scope answer the PDP produced tier by tier
    /// need not be contiguous: a pack that permits `confidential` while
    /// denying `internal` has said something strange, and the honest
    /// response is to enforce exactly what it said. This is the helper for
    /// the ordinary case, where a ceiling is what somebody meant.
    #[must_use]
    pub fn at_or_below(self) -> Vec<Sensitivity> {
        Sensitivity::ALL
            .into_iter()
            .filter(|level| *level <= self)
            .collect()
    }

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

/// One `(scope, tier)` pair a caller may read — the read path's predicate
/// unit since AUTHZ-5 (ADR-0038 decision 3).
///
/// A pair rather than a scope set plus a global ceiling, because the PDP
/// answers per scope *and* per tier: one scope may admit `confidential`
/// through an explicit binding while its neighbour on the same chain admits
/// only the working tiers. A single ceiling over the whole set could only
/// express the wrong thing — the maximum (a widening) or the minimum (a
/// silent loss).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeTier {
    /// The scope.
    pub scope_id: crate::ScopeId,
    /// One tier the caller may read there.
    pub sensitivity: Sensitivity,
}

impl ScopeTier {
    /// Every pair for one scope's allowed set.
    #[must_use]
    pub fn expand(scope_id: crate::ScopeId, sensitivities: &[Sensitivity]) -> Vec<ScopeTier> {
        sensitivities
            .iter()
            .map(|sensitivity| ScopeTier {
                scope_id,
                sensitivity: *sensitivity,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ceiling_expands_to_every_tier_at_or_below_it_ascending() {
        assert_eq!(Sensitivity::Public.at_or_below(), [Sensitivity::Public]);
        assert_eq!(
            Sensitivity::Confidential.at_or_below(),
            [
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential
            ]
        );
        assert_eq!(Sensitivity::Restricted.at_or_below(), Sensitivity::ALL);
    }

    /// The two product constants, pinned rather than assumed: the tier a
    /// request means when it says nothing, and the highest one an extractor
    /// may assign (ADR-0038 decision 8).
    #[test]
    fn the_working_tier_and_the_extraction_ceiling_are_what_the_adrs_say() {
        assert_eq!(Sensitivity::WORKING, Sensitivity::Internal);
        assert_eq!(Sensitivity::MAX_DERIVED, Sensitivity::Confidential);
        assert!(
            Sensitivity::MAX_DERIVED < Sensitivity::Restricted,
            "no pipeline output may carry the tier that means compliance signed off"
        );
        assert!(Sensitivity::WORKING <= Sensitivity::MAX_DERIVED);
    }
}
