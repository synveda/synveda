//! Record classification vocabulary (seed §4.2).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// How a memory record came to exist — the Shruti/Smriti split (seed §4.2).
///
/// There is no `Default`: whether a record is derived or pinned is always an
/// explicit decision by the pipeline or an author, never a fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordKind {
    /// Extracted automatically by the ingestion pipeline; clearly watermarked
    /// as unreviewed until promoted through VedaFlow.
    Derived,
    /// Authored or canonical content — cannot be shadowed or decayed.
    Pinned,
}

impl RecordKind {
    /// All kinds.
    pub const ALL: [RecordKind; 2] = [RecordKind::Derived, RecordKind::Pinned];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RecordKind::Derived => "derived",
            RecordKind::Pinned => "pinned",
        }
    }
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RecordKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "derived" => Ok(RecordKind::Derived),
            "pinned" => Ok(RecordKind::Pinned),
            other => Err(Error::Invalid {
                message: format!("unknown record kind: {other:?}"),
            }),
        }
    }
}

/// What a memory record asserts (seed §4.2). Drives extraction routing,
/// retrieval weighting, and promotion rules (e.g. FLOW-4 acts on
/// `Procedure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordClass {
    /// A statement about the world ("the staging cluster is in eu-west-1").
    Fact,
    /// A decision that was made, with its context.
    Decision,
    /// A preference of a person or team.
    Preference,
    /// A how-to: steps that accomplish something.
    Procedure,
    /// A named thing (person, system, customer) and what is known about it.
    Entity,
    /// A summarised episode: what happened in a session or event.
    Episode,
}

impl RecordClass {
    /// All classes.
    pub const ALL: [RecordClass; 6] = [
        RecordClass::Fact,
        RecordClass::Decision,
        RecordClass::Preference,
        RecordClass::Procedure,
        RecordClass::Entity,
        RecordClass::Episode,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RecordClass::Fact => "fact",
            RecordClass::Decision => "decision",
            RecordClass::Preference => "preference",
            RecordClass::Procedure => "procedure",
            RecordClass::Entity => "entity",
            RecordClass::Episode => "episode",
        }
    }
}

impl fmt::Display for RecordClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RecordClass {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fact" => Ok(RecordClass::Fact),
            "decision" => Ok(RecordClass::Decision),
            "preference" => Ok(RecordClass::Preference),
            "procedure" => Ok(RecordClass::Procedure),
            "entity" => Ok(RecordClass::Entity),
            "episode" => Ok(RecordClass::Episode),
            other => Err(Error::Invalid {
                message: format!("unknown record class: {other:?}"),
            }),
        }
    }
}
