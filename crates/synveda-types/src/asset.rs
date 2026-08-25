//! The managed asset vocabulary (seed §4.3, tech plan §2.3).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// What kind of governed asset a VedaFlow object holds.
///
/// The four managed asset classes of seed §4.3 plus policy, which tech plan
/// §2.3 makes an asset in its own right. Policy packs, configuration and
/// relaxations themselves flow through VedaFlow.
///
/// This is part of a VedaFlow object's content address (FLOW-1, ADR-0030
/// decision 4), not a label beside it: identical bytes registered as a prompt
/// and as a skill are two different objects, because FLOW-3 resolves required
/// approvals from asset type × sensitivity × scope × pack, and a skill is
/// executable where a prompt is not. Content governed differently is not the
/// same content.
///
/// There is no `Default` — what an asset *is* is always an explicit decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    /// A memory record's content — derived by the pipeline or authored
    /// (seed §4.2).
    Memory,
    /// A stable Knowledge aggregate revision or governed mutation
    /// (CPR-16, ADR-0081). Distinct from the pre-cut `memory` record asset:
    /// neither vocabulary reads or writes the other's persistence model.
    Knowledge,
    /// A versioned prompt template (PRMT-1).
    Prompt,
    /// A skill definition and its bundled files (SKIL-1). Executable, and
    /// reviewed like code.
    Skill,
    /// A trusted MCP server version or exact project binding (CPR-25,
    /// ADR-0086). Declared capabilities are metadata, never authority.
    Tool,
    /// A curated bundle pinned to a scope: docs, conventions, glossaries
    /// (PRMT-2).
    ContextPack,
    /// A policy pack, governed configuration or relaxation, flowing through
    /// the same propose/review/approve path as everything else it governs.
    Policy,
    /// A complete immutable runtime-configuration document or revisioned
    /// scope binding (CPR-30, ADR-0089). Templates are source data; this is
    /// the governed artifact that runtime consumers resolve.
    Configuration,
}

impl AssetKind {
    /// All asset kinds. Kept in the same order as the `vedaflow_objects.kind`
    /// CHECK constraint (migration 0018).
    pub const ALL: [AssetKind; 8] = [
        AssetKind::Memory,
        AssetKind::Knowledge,
        AssetKind::Prompt,
        AssetKind::Skill,
        AssetKind::Tool,
        AssetKind::ContextPack,
        AssetKind::Policy,
        AssetKind::Configuration,
    ];

    /// Asset kinds represented by VedaFlow channels.
    ///
    /// Knowledge changes use VedaFlow proposals, but their current state is
    /// the aggregate projection rather than a second channel head
    /// (CPR-16, ADR-0081). Policy effects likewise write governed rows, not
    /// refs. Keeping this list separate from [`Self::ALL`] prevents either
    /// non-channelled kind from acquiring a shadow `*/published` truth.
    pub const CHANNELLED: [AssetKind; 3] =
        [AssetKind::Memory, AssetKind::Prompt, AssetKind::ContextPack];

    /// Whether this asset family has VedaFlow channel refs.
    #[must_use]
    pub const fn has_channels(self) -> bool {
        matches!(
            self,
            AssetKind::Memory | AssetKind::Prompt | AssetKind::ContextPack
        )
    }

    /// Stable wire name, identical to the serde form and to the stored
    /// column. It is hashed into every object's content address, so renaming
    /// one would re-address every object of that kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            AssetKind::Memory => "memory",
            AssetKind::Knowledge => "knowledge",
            AssetKind::Prompt => "prompt",
            AssetKind::Skill => "skill",
            AssetKind::Tool => "tool",
            AssetKind::ContextPack => "context-pack",
            AssetKind::Policy => "policy",
            AssetKind::Configuration => "configuration",
        }
    }
}

impl fmt::Display for AssetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AssetKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AssetKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown asset kind: {s:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip_through_display_and_parse() {
        for kind in AssetKind::ALL {
            assert_eq!(kind.to_string().parse::<AssetKind>().unwrap(), kind);
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
        }
    }

    #[test]
    fn unknown_names_are_invalid_not_defaulted() {
        assert!(matches!(
            "memories".parse::<AssetKind>(),
            Err(Error::Invalid { .. })
        ));
    }
}
