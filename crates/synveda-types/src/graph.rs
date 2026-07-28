//! The knowledge graph's vocabulary (GRPH-1, ADR-0043): the named graphs
//! ADR-0004 chose and the traversal depths ADR-0029's discipline allows.
//!
//! Both types exist to make an undisciplined traversal unrepresentable
//! rather than reviewable. [`Graph`] has no `Default` and the store's
//! traversal API takes it by value, so a query that does not name its
//! semantic domain does not compile (ADR-0043 decision 2 — the discipline
//! ADR-0024 decision 1 applied to tenancy, applied here to meaning).
//! [`Depth`] is an enum rather than an integer, so unbounded depth arrives
//! as a compile error and then as an ADR, never as a slow afternoon in
//! production (decision 9).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// One of the three named graphs (ADR-0004, MAGMA-informed; the engine it
/// chose was overturned by ADR-0043, the partition it chose was not).
///
/// There is no `Default`: which graph a vertex or a claim belongs to is
/// always an explicit statement about meaning, and a default would be the
/// leak-by-omission ADR-0004 option 2 objected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Graph {
    /// Things and the relations between them — people, systems, customers.
    Entity,
    /// What happened, and when: episodes and their participants.
    Episode,
    /// Where a claim came from: the evidence graph.
    Provenance,
}

impl Graph {
    /// All graphs. Identical to `graph_vertices_graph_check`'s vocabulary
    /// in migration 0026; the completeness of this list is what the
    /// adversarial suite checks against the schema.
    pub const ALL: [Graph; 3] = [Graph::Entity, Graph::Episode, Graph::Provenance];

    /// Stable wire name, identical to the serde form and to the stored
    /// `graph` column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Graph::Entity => "entity",
            Graph::Episode => "episode",
            Graph::Provenance => "provenance",
        }
    }
}

impl fmt::Display for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Graph {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "entity" => Ok(Graph::Entity),
            "episode" => Ok(Graph::Episode),
            "provenance" => Ok(Graph::Provenance),
            other => Err(Error::Invalid {
                message: format!("unknown graph: {other:?}"),
            }),
        }
    }
}

/// How far a traversal may walk (ADR-0043 decision 9).
///
/// An enum rather than an integer, and deliberately not extensible by
/// arithmetic: GRPH-4 measured 1- and 2-hop expansion as the shapes the
/// product actually issues, and ADR-0043's reversal trigger for anything
/// deeper is a fresh ADR weighing a second engine against bounding the
/// requirement. A caller that wants three hops cannot express it, which is
/// the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    /// Direct neighbours of the seed set.
    One,
    /// Direct neighbours, and theirs.
    Two,
}

impl Depth {
    /// Both depths.
    pub const ALL: [Depth; 2] = [Depth::One, Depth::Two];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Depth::One => "one",
            Depth::Two => "two",
        }
    }

    /// The depth as a hop count, for arithmetic that needs one (span
    /// fields, result attribution). Deliberately one-way: there is no
    /// `from_hops`, because that would be the integer this type exists to
    /// refuse.
    #[must_use]
    pub const fn hops(&self) -> u8 {
        match self {
            Depth::One => 1,
            Depth::Two => 2,
        }
    }
}

impl fmt::Display for Depth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Depth {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "one" => Ok(Depth::One),
            "two" => Ok(Depth::Two),
            other => Err(Error::Invalid {
                message: format!("unknown traversal depth: {other:?}"),
            }),
        }
    }
}
