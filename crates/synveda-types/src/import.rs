//! Durable external knowledge-format import plans (CPR-27, ADR-0087).
//!
//! These are operation records, not another Knowledge aggregate. An import
//! mapping may become a [`crate::capture::CaptureCandidate`], and only that
//! candidate's ordinary decision can enter VedaFlow and publish Knowledge.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::knowledge::{KnowledgeRevisionContent, KnowledgeType};
use crate::{
    CaptureBatchId, CaptureCandidateId, Error, ImportArtifactId, ImportJobId, ImportMappingId,
    KnowledgeItemId, KnowledgeRevisionId, ProjectId, Result, ScopeId, TenantId, WorkspaceId,
};

macro_rules! string_enum {
    ($name:ident, [$($variant:ident => $value:literal),+ $(,)?], $label:literal) => {
        #[doc = $label]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(
                #[doc = concat!("`", $value, "`.")]
                $variant,
            )+
        }

        impl $name {
            /// Every value in stable storage order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable wire and storage value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                Self::ALL.iter().copied().find(|item| item.as_str() == value)
                    .ok_or_else(|| Error::Invalid {
                        message: format!(concat!("unknown ", $label, ": {:?}"), value),
                    })
            }
        }
    };
}

string_enum!(
    ImportJobState,
    [Planned => "planned", Materialized => "materialized", Failed => "failed"],
    "import job state"
);

string_enum!(
    ImportArtifactKind,
    [Concept => "concept", Index => "index", Log => "log"],
    "import artifact kind"
);

string_enum!(
    ImportMappingClassification,
    [
        Addition => "addition",
        Update => "update",
        Duplicate => "duplicate",
        Conflict => "conflict"
    ],
    "import mapping classification"
);

/// One stable, immutable OKF inspection plan and its materialisation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJob {
    /// Stable job id.
    pub id: ImportJobId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Project the proposed Knowledge concerns.
    pub project_id: ProjectId,
    /// Project's governed scope.
    pub scope_id: ScopeId,
    /// Parent workspace.
    pub workspace_id: WorkspaceId,
    /// Authenticated actor that planned the import.
    pub principal_id: String,
    /// Versioned adapter identifier, currently `okf`.
    pub format: String,
    /// Exact external format version.
    pub format_version: String,
    /// Exact specification revision.
    pub specification_commit: String,
    /// Directory, zip, tar or Git source label.
    pub source_kind: String,
    /// Bounded source identity, never a server-local path grant.
    pub source_locator: String,
    /// Explicit source revision when present.
    pub source_revision: Option<String>,
    /// Canonical digest over admitted artifacts.
    pub bundle_digest: String,
    /// Planned, materialized or failed.
    pub state: ImportJobState,
    /// Immutable artifact count.
    pub artifact_count: i32,
    /// Immutable concept mapping count.
    pub mapping_count: i32,
    /// Reviewable candidates eventually created.
    pub candidate_count: i32,
    /// Import-sourced capture batch after materialisation.
    pub capture_batch_id: Option<CaptureBatchId>,
    /// Content-free stable failure code.
    pub error_code: Option<String>,
    /// Content-free validation notices retained from inspection.
    pub notices: Vec<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Terminal materialisation time.
    pub completed_at: Option<DateTime<Utc>>,
    /// Last state change.
    pub updated_at: DateTime<Utc>,
}

/// One immutable admitted Markdown artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportArtifact {
    /// Stable artifact id.
    pub id: ImportArtifactId,
    /// Owning job.
    pub job_id: ImportJobId,
    /// Stable bytewise path order.
    pub ordinal: i32,
    /// Safe bundle-relative logical path.
    pub logical_path: String,
    /// Concept, index or log.
    pub kind: ImportArtifactKind,
    /// Exact admitted-byte hash.
    pub content_hash: String,
    /// Parsed frontmatter, empty for reserved files without it.
    pub frontmatter: Value,
    /// Markdown body after frontmatter.
    pub body_markdown: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// One immutable concept mapping and its eventual candidate address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMapping {
    /// Stable mapping id.
    pub id: ImportMappingId,
    /// Owning job.
    pub job_id: ImportJobId,
    /// Source concept artifact.
    pub artifact_id: ImportArtifactId,
    /// Stable concept order.
    pub ordinal: i32,
    /// Exact producer-defined OKF type.
    pub okf_type: String,
    /// Proposed Synveda Knowledge type.
    pub knowledge_type: KnowledgeType,
    /// Complete proposed immutable content.
    pub content: KnowledgeRevisionContent,
    /// Canonical semantic content hash.
    pub content_hash: String,
    /// Addition, update, duplicate or conflict.
    pub classification: ImportMappingClassification,
    /// Exact visible current item compared, when any.
    pub matched_item_id: Option<KnowledgeItemId>,
    /// Exact visible current revision compared, when any.
    pub matched_revision_id: Option<KnowledgeRevisionId>,
    /// Proposed internal relation paths and kinds.
    pub proposed_relations: Value,
    /// Whether external lifecycle permits a candidate.
    pub materializable: bool,
    /// Whether derived plaintext and live Knowledge addresses were erased.
    pub content_erased: bool,
    /// Candidate created by the one materialisation, when applicable.
    pub candidate_id: Option<CaptureCandidateId>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_vocabularies_are_closed_and_round_trip() {
        for value in ImportJobState::ALL {
            assert_eq!(value.as_str().parse::<ImportJobState>().unwrap(), *value);
        }
        for value in ImportArtifactKind::ALL {
            assert_eq!(
                value.as_str().parse::<ImportArtifactKind>().unwrap(),
                *value
            );
        }
        for value in ImportMappingClassification::ALL {
            assert_eq!(
                value
                    .as_str()
                    .parse::<ImportMappingClassification>()
                    .unwrap(),
                *value
            );
        }
        assert!("published".parse::<ImportJobState>().is_err());
    }
}
