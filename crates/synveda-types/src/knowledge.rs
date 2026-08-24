//! Stable Knowledge aggregates and immutable revisions (CPR-15, ADR-0080).
//!
//! A [`KnowledgeItem`] is the stable thing a person names. A
//! [`KnowledgeRevision`] is the exact immutable content a context run,
//! verification or feedback event cites. [`KnowledgeSource`] normalises
//! provenance and has its own governed scope, because permission to read a
//! shared conclusion does not imply permission to inspect every private
//! conversation or document that supported it. [`KnowledgeRelation`] is an
//! explicit claim between stable items, asserted by one exact revision.
//!
//! This module contains no persistence or authorisation. It is the vocabulary
//! shared by those layers; the store enforces the same bounds with CHECKs and
//! the application layer decides through the PDP before reading or changing
//! an aggregate.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, KnowledgeItemId, KnowledgeRelationId, KnowledgeRevisionId, KnowledgeSourceId, ProjectId,
    Result, ScopeId, Sensitivity, SessionEventId, TenantId,
};

/// Longest Knowledge title, in characters.
pub const MAX_KNOWLEDGE_TITLE_CHARS: usize = 300;
/// Largest Markdown body, in bytes of UTF-8.
pub const MAX_KNOWLEDGE_BODY_BYTES: usize = 131_072;
/// Longest revision summary, in characters.
pub const MAX_KNOWLEDGE_SUMMARY_CHARS: usize = 2_000;
/// Maximum number of tags on one revision.
pub const MAX_KNOWLEDGE_TAGS: usize = 64;
/// Longest canonical tag, in characters.
pub const MAX_KNOWLEDGE_TAG_CHARS: usize = 64;
/// Largest verification or extension metadata object, in compact JSON bytes.
pub const MAX_KNOWLEDGE_METADATA_BYTES: usize = 16_384;
/// Longest source locator, in characters.
pub const MAX_SOURCE_LOCATOR_CHARS: usize = 2_048;
/// Longest external source revision or version label, in characters.
pub const MAX_SOURCE_REVISION_CHARS: usize = 512;
/// Longest principal or actor label stored on Knowledge provenance.
pub const MAX_KNOWLEDGE_PRINCIPAL_CHARS: usize = 255;

/// What a Knowledge item says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    /// A statement about the world.
    Fact,
    /// A choice that was made and remains in force.
    Decision,
    /// A person- or scope-sensitive preference.
    Preference,
    /// Steps that accomplish something.
    Procedure,
    /// A named thing and what is known about it.
    Entity,
    /// Something that happened, retained as history.
    Episode,
    /// A working convention a project or scope follows.
    Convention,
    /// A hazard, caveat or condition worth surfacing.
    Warning,
    /// A durable pointer to external material.
    Reference,
}

impl KnowledgeType {
    /// Every type in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Fact,
        Self::Decision,
        Self::Preference,
        Self::Procedure,
        Self::Entity,
        Self::Episode,
        Self::Convention,
        Self::Warning,
        Self::Reference,
    ];

    /// Stable wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Preference => "preference",
            Self::Procedure => "procedure",
            Self::Entity => "entity",
            Self::Episode => "episode",
            Self::Convention => "convention",
            Self::Warning => "warning",
            Self::Reference => "reference",
        }
    }
}

impl fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KnowledgeType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown Knowledge type: {value:?}"),
            })
    }
}

/// How a Knowledge item first entered the governed system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeOrigin {
    /// Selected from host-observed session activity.
    Observed,
    /// Explicitly asserted by an agent or integration.
    Asserted,
    /// Written deliberately by a person or governed authoring flow.
    Authored,
    /// Proposed by an external-format import.
    Imported,
}

impl KnowledgeOrigin {
    /// Every origin in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Observed,
        Self::Asserted,
        Self::Authored,
        Self::Imported,
    ];

    /// Stable wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Asserted => "asserted",
            Self::Authored => "authored",
            Self::Imported => "imported",
        }
    }
}

impl fmt::Display for KnowledgeOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KnowledgeOrigin {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown Knowledge origin: {value:?}"),
            })
    }
}

/// Where a Knowledge item is in its governed lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeLifecycleState {
    /// Current and eligible for ordinary retrieval.
    Active,
    /// Still present but due for verification.
    Stale,
    /// Replaced by an explicit superseding item or revision.
    Superseded,
    /// Intentionally removed from ordinary current use.
    Archived,
    /// A governed erasure operation is still deciding or executing.
    ErasurePending,
    /// Plaintext content has been removed; only a tombstone remains.
    Erased,
}

impl KnowledgeLifecycleState {
    /// Every lifecycle state in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Active,
        Self::Stale,
        Self::Superseded,
        Self::Archived,
        Self::ErasurePending,
        Self::Erased,
    ];

    /// Stable wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
            Self::ErasurePending => "erasure_pending",
            Self::Erased => "erased",
        }
    }

    /// Whether ordinary current retrieval may consider this item.
    #[must_use]
    pub const fn is_current(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl fmt::Display for KnowledgeLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KnowledgeLifecycleState {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown Knowledge lifecycle state: {value:?}"),
            })
    }
}

/// One explicit relationship between stable Knowledge items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRelationType {
    /// The source provides evidence for the target.
    Supports,
    /// The two items express materially the same knowledge.
    Duplicates,
    /// The source and target cannot both be current truth as stated.
    Contradicts,
    /// The source explicitly replaces the target.
    Supersedes,
    /// The source was derived from the target.
    DerivedFrom,
    /// The source cites or points to the target.
    References,
    /// A deliberately weak association.
    RelatedTo,
    /// The source is a future or state transition from the target.
    TransitionsTo,
}

impl KnowledgeRelationType {
    /// Every relation type in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Supports,
        Self::Duplicates,
        Self::Contradicts,
        Self::Supersedes,
        Self::DerivedFrom,
        Self::References,
        Self::RelatedTo,
        Self::TransitionsTo,
    ];

    /// Stable wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Duplicates => "duplicates",
            Self::Contradicts => "contradicts",
            Self::Supersedes => "supersedes",
            Self::DerivedFrom => "derived_from",
            Self::References => "references",
            Self::RelatedTo => "related_to",
            Self::TransitionsTo => "transitions_to",
        }
    }
}

impl fmt::Display for KnowledgeRelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KnowledgeRelationType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown Knowledge relation type: {value:?}"),
            })
    }
}

/// What authoritative material a Knowledge revision came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceType {
    /// One immutable event in a governed agent session.
    SessionEvent,
    /// A deliberate manual assertion with no external locator.
    Manual,
    /// A document identified by a stable logical locator.
    Document,
    /// A repository path at an explicit revision.
    Repository,
    /// A canonical URL.
    Url,
    /// An Open Knowledge Format artifact.
    Okf,
    /// A derived result whose locator names the deriving method or input set.
    SystemDerived,
}

impl KnowledgeSourceType {
    /// Every source type in stable declaration order.
    pub const ALL: &'static [Self] = &[
        Self::SessionEvent,
        Self::Manual,
        Self::Document,
        Self::Repository,
        Self::Url,
        Self::Okf,
        Self::SystemDerived,
    ];

    /// Stable wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionEvent => "session_event",
            Self::Manual => "manual",
            Self::Document => "document",
            Self::Repository => "repository",
            Self::Url => "url",
            Self::Okf => "okf",
            Self::SystemDerived => "system_derived",
        }
    }
}

impl fmt::Display for KnowledgeSourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KnowledgeSourceType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown Knowledge source type: {value:?}"),
            })
    }
}

/// The content-bearing part of a new immutable revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeRevisionContent {
    /// Human-readable title.
    pub title: String,
    /// Canonical Markdown body.
    pub body_markdown: String,
    /// Short retrieval and listing summary.
    pub summary: String,
    /// Canonical lower-case, sorted, unique tags.
    pub tags: Vec<String>,
    /// Sensitivity tier used by policy decisions.
    pub sensitivity: Sensitivity,
    /// Confidence on an integer `0..=1000` scale.
    pub confidence_permille: i32,
    /// When this knowledge began to hold in the world.
    pub valid_from: DateTime<Utc>,
    /// When it stopped holding, if known.
    pub valid_to: Option<DateTime<Utc>>,
    /// When verification becomes due, if configured.
    pub stale_after: Option<DateTime<Utc>>,
    /// Verification facts, always a JSON object.
    pub verification_metadata: Value,
    /// Forward-compatible product metadata, always a JSON object.
    pub metadata: Value,
}

/// One stable Knowledge aggregate head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeItem {
    /// Stable item id.
    pub id: KnowledgeItemId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Governing scope.
    pub scope_id: ScopeId,
    /// Associated project, when the knowledge concerns one.
    pub project_id: Option<ProjectId>,
    /// Owning principal, when one person owns it.
    pub owner_principal_id: Option<String>,
    /// What the item says.
    pub knowledge_type: KnowledgeType,
    /// How the item first entered the system.
    pub origin: KnowledgeOrigin,
    /// Current governed lifecycle state.
    pub lifecycle_state: KnowledgeLifecycleState,
    /// Exact revision currently selected by the head.
    pub current_revision_id: KnowledgeRevisionId,
    /// Principal or system actor that created the item.
    pub created_by: Option<String>,
    /// Principal or system actor that last changed the head.
    pub updated_by: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last head change time.
    pub updated_at: DateTime<Utc>,
    /// Start of this current head state's transaction-time interval.
    pub transaction_from: DateTime<Utc>,
}

/// One immutable content revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeRevision {
    /// Immutable revision id.
    pub id: KnowledgeRevisionId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Stable item this revises.
    pub knowledge_item_id: KnowledgeItemId,
    /// Monotonic number within the item, starting at one.
    pub revision_number: i64,
    /// Revision content.
    pub content: KnowledgeRevisionContent,
    /// BLAKE3-256 over the canonical semantic content, lower-case hex.
    pub content_hash: String,
    /// Actor that authored this revision.
    pub created_by: Option<String>,
    /// Database-stamped transaction time.
    pub transaction_time: DateTime<Utc>,
}

/// One normalised provenance source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSource {
    /// Stable source id.
    pub id: KnowledgeSourceId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Scope at which source details are governed.
    pub scope_id: ScopeId,
    /// Source family.
    pub source_type: KnowledgeSourceType,
    /// Real event id for a `session_event` source.
    pub session_event_id: Option<SessionEventId>,
    /// Logical document, repository, URL, OKF or derivation locator.
    pub locator: Option<String>,
    /// External repository, document or artifact revision.
    pub source_revision: Option<String>,
    /// Source-content hash when one is known.
    pub content_hash: Option<String>,
    /// Forward-compatible descriptor metadata.
    pub metadata: Value,
    /// Actor that registered the descriptor.
    pub created_by: Option<String>,
    /// Registration time.
    pub created_at: DateTime<Utc>,
}

/// One append-only relationship claim between stable Knowledge items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeRelation {
    /// Stable relation id.
    pub id: KnowledgeRelationId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Item making the claim.
    pub source_item_id: KnowledgeItemId,
    /// Item the claim is about.
    pub target_item_id: KnowledgeItemId,
    /// Exact source-item revision that asserted the relation.
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Relationship vocabulary.
    pub relation_type: KnowledgeRelationType,
    /// Forward-compatible relation metadata.
    pub metadata: Value,
    /// Actor that registered the relation.
    pub created_by: Option<String>,
    /// Registration time.
    pub created_at: DateTime<Utc>,
}

/// Returns canonical lower-case, sorted and de-duplicated tags.
///
/// # Errors
///
/// [`Error::Invalid`] when there are too many tags or a tag is blank or too
/// long after trimming.
pub fn normalise_knowledge_tags(tags: &[String]) -> Result<Vec<String>> {
    if tags.len() > MAX_KNOWLEDGE_TAGS {
        return Err(Error::Invalid {
            message: format!(
                "a Knowledge revision has at most {MAX_KNOWLEDGE_TAGS} tags, got {}",
                tags.len()
            ),
        });
    }
    let mut canonical = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim().to_lowercase();
        let len = tag.chars().count();
        if len == 0 || len > MAX_KNOWLEDGE_TAG_CHARS {
            return Err(Error::Invalid {
                message: format!(
                    "a Knowledge tag is 1..={MAX_KNOWLEDGE_TAG_CHARS} characters after trimming"
                ),
            });
        }
        canonical.push(tag);
    }
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

/// Validates the bounded and temporal shape of revision content.
///
/// # Errors
///
/// [`Error::Invalid`] for blank/oversized content, confidence outside
/// `0..=1000`, an invalid valid-time or stale-time interval, or metadata that
/// is not a bounded JSON object.
pub fn validate_knowledge_revision_content(content: &KnowledgeRevisionContent) -> Result<()> {
    validate_non_blank_chars("Knowledge title", &content.title, MAX_KNOWLEDGE_TITLE_CHARS)?;
    if content.body_markdown.trim().is_empty() {
        return Err(Error::Invalid {
            message: "a Knowledge Markdown body cannot be blank".to_owned(),
        });
    }
    if content.body_markdown.len() > MAX_KNOWLEDGE_BODY_BYTES {
        return Err(Error::Invalid {
            message: format!(
                "a Knowledge Markdown body is at most {MAX_KNOWLEDGE_BODY_BYTES} bytes"
            ),
        });
    }
    validate_non_blank_chars(
        "Knowledge summary",
        &content.summary,
        MAX_KNOWLEDGE_SUMMARY_CHARS,
    )?;
    let canonical_tags = normalise_knowledge_tags(&content.tags)?;
    if canonical_tags != content.tags {
        return Err(Error::Invalid {
            message: "Knowledge tags must be lower-case, sorted and unique".to_owned(),
        });
    }
    if !(0..=1000).contains(&content.confidence_permille) {
        return Err(Error::Invalid {
            message: format!(
                "Knowledge confidence is {} per mille; the range is 0..=1000",
                content.confidence_permille
            ),
        });
    }
    if content
        .valid_to
        .is_some_and(|end| end <= content.valid_from)
    {
        return Err(Error::Invalid {
            message: "Knowledge valid_to must be later than valid_from".to_owned(),
        });
    }
    if let Some(stale_after) = content.stale_after {
        if stale_after <= content.valid_from {
            return Err(Error::Invalid {
                message: "Knowledge stale_after must be later than valid_from".to_owned(),
            });
        }
        if content.valid_to.is_some_and(|end| stale_after > end) {
            return Err(Error::Invalid {
                message: "Knowledge stale_after cannot be later than valid_to".to_owned(),
            });
        }
    }
    validate_metadata("verification metadata", &content.verification_metadata)?;
    validate_metadata("Knowledge metadata", &content.metadata)
}

/// Validates a normalised source descriptor's type-specific shape.
///
/// # Errors
///
/// [`Error::Invalid`] when required event/locator fields are missing or
/// contradictory, a label/hash is malformed, or metadata is not a bounded
/// object.
pub fn validate_knowledge_source(
    source_type: KnowledgeSourceType,
    session_event_id: Option<SessionEventId>,
    locator: Option<&str>,
    source_revision: Option<&str>,
    content_hash: Option<&str>,
    metadata: &Value,
) -> Result<()> {
    match source_type {
        KnowledgeSourceType::SessionEvent => {
            if session_event_id.is_none() || locator.is_some() {
                return Err(Error::Invalid {
                    message: "a session_event source requires an event id and no locator"
                        .to_owned(),
                });
            }
        }
        KnowledgeSourceType::Manual => {
            if session_event_id.is_some() || locator.is_some() || source_revision.is_some() {
                return Err(Error::Invalid {
                    message: "a manual source has no event, locator or external revision"
                        .to_owned(),
                });
            }
        }
        KnowledgeSourceType::Document
        | KnowledgeSourceType::Repository
        | KnowledgeSourceType::Url
        | KnowledgeSourceType::Okf
        | KnowledgeSourceType::SystemDerived => {
            if session_event_id.is_some() || locator.is_none() {
                return Err(Error::Invalid {
                    message: format!(
                        "a {source_type} source requires a locator and no session event id"
                    ),
                });
            }
        }
    }
    if let Some(locator) = locator {
        validate_non_blank_chars("source locator", locator, MAX_SOURCE_LOCATOR_CHARS)?;
    }
    if let Some(revision) = source_revision {
        validate_non_blank_chars("source revision", revision, MAX_SOURCE_REVISION_CHARS)?;
    }
    if let Some(hash) = content_hash {
        validate_content_hash(hash)?;
    }
    validate_metadata("source metadata", metadata)
}

/// Validates a lower-case BLAKE3-256 hex digest.
///
/// # Errors
///
/// [`Error::Invalid`] unless `hash` is exactly 64 lower-case hexadecimal
/// characters.
pub fn validate_content_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Error::Invalid {
            message: "a content hash must be 64 lower-case hexadecimal characters".to_owned(),
        });
    }
    Ok(())
}

/// Validates a principal/actor label stored as creation or ownership
/// provenance.
///
/// # Errors
///
/// [`Error::Invalid`] for a blank or overlong present label.
pub fn validate_knowledge_principal(value: Option<&str>, field: &str) -> Result<()> {
    if let Some(value) = value {
        validate_non_blank_chars(field, value, MAX_KNOWLEDGE_PRINCIPAL_CHARS)?;
    }
    Ok(())
}

/// Validates relation metadata and distinct endpoints.
///
/// # Errors
///
/// [`Error::Invalid`] for a self-relation or malformed metadata.
pub fn validate_knowledge_relation(
    source: KnowledgeItemId,
    target: KnowledgeItemId,
    metadata: &Value,
) -> Result<()> {
    if source == target {
        return Err(Error::Invalid {
            message: "a Knowledge relation must connect two distinct items".to_owned(),
        });
    }
    validate_metadata("relation metadata", metadata)
}

fn validate_non_blank_chars(field: &str, value: &str, max: usize) -> Result<()> {
    let len = value.chars().count();
    if value.trim().is_empty() || len > max {
        return Err(Error::Invalid {
            message: format!("{field} must be non-blank and at most {max} characters"),
        });
    }
    Ok(())
}

fn validate_metadata(field: &str, value: &Value) -> Result<()> {
    if !value.is_object() {
        return Err(Error::Invalid {
            message: format!("{field} must be a JSON object"),
        });
    }
    let bytes = serde_json::to_vec(value).map_err(|err| Error::Invalid {
        message: format!("{field} cannot be encoded: {err}"),
    })?;
    if bytes.len() > MAX_KNOWLEDGE_METADATA_BYTES {
        return Err(Error::Invalid {
            message: format!("{field} is at most {MAX_KNOWLEDGE_METADATA_BYTES} bytes"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> KnowledgeRevisionContent {
        KnowledgeRevisionContent {
            title: "Request correlation".to_owned(),
            body_markdown: "Use `traceparent` on public requests.".to_owned(),
            summary: "Public requests use traceparent.".to_owned(),
            tags: vec!["http".to_owned(), "observability".to_owned()],
            sensitivity: Sensitivity::Internal,
            confidence_permille: 940,
            valid_from: Utc::now(),
            valid_to: None,
            stale_after: None,
            verification_metadata: serde_json::json!({"method": "reviewed"}),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn every_vocabulary_round_trips_and_unknown_values_fail() {
        for item in KnowledgeType::ALL {
            assert_eq!(item.as_str().parse::<KnowledgeType>().unwrap(), *item);
        }
        for item in KnowledgeOrigin::ALL {
            assert_eq!(item.as_str().parse::<KnowledgeOrigin>().unwrap(), *item);
        }
        for item in KnowledgeLifecycleState::ALL {
            assert_eq!(
                item.as_str().parse::<KnowledgeLifecycleState>().unwrap(),
                *item
            );
        }
        for item in KnowledgeRelationType::ALL {
            assert_eq!(
                item.as_str().parse::<KnowledgeRelationType>().unwrap(),
                *item
            );
        }
        for item in KnowledgeSourceType::ALL {
            assert_eq!(item.as_str().parse::<KnowledgeSourceType>().unwrap(), *item);
        }
        assert!("memory".parse::<KnowledgeType>().is_err());
        assert!("deleted".parse::<KnowledgeLifecycleState>().is_err());
    }

    #[test]
    fn tags_normalise_once_and_revision_validation_requires_that_form() {
        let tags = normalise_knowledge_tags(&[
            " Observability ".to_owned(),
            "HTTP".to_owned(),
            "http".to_owned(),
        ])
        .unwrap();
        assert_eq!(tags, ["http", "observability"]);

        let mut revision = content();
        validate_knowledge_revision_content(&revision).unwrap();
        revision.tags.reverse();
        assert!(validate_knowledge_revision_content(&revision).is_err());
    }

    #[test]
    fn revision_time_confidence_and_metadata_bounds_are_named() {
        let mut revision = content();
        revision.confidence_permille = 1001;
        assert!(validate_knowledge_revision_content(&revision).is_err());

        let mut revision = content();
        revision.valid_to = Some(revision.valid_from);
        assert!(validate_knowledge_revision_content(&revision).is_err());

        let mut revision = content();
        revision.metadata = serde_json::json!([]);
        assert!(validate_knowledge_revision_content(&revision).is_err());
    }

    #[test]
    fn source_shapes_distinguish_events_manual_and_located_material() {
        let event = SessionEventId::new();
        validate_knowledge_source(
            KnowledgeSourceType::SessionEvent,
            Some(event),
            None,
            None,
            None,
            &serde_json::json!({}),
        )
        .unwrap();
        validate_knowledge_source(
            KnowledgeSourceType::Manual,
            None,
            None,
            None,
            None,
            &serde_json::json!({}),
        )
        .unwrap();
        validate_knowledge_source(
            KnowledgeSourceType::Repository,
            None,
            Some("https://example.test/acme/pulseboard:src/http.rs"),
            Some("abc123"),
            Some(&"a".repeat(64)),
            &serde_json::json!({}),
        )
        .unwrap();

        assert!(
            validate_knowledge_source(
                KnowledgeSourceType::SessionEvent,
                None,
                None,
                None,
                None,
                &serde_json::json!({}),
            )
            .is_err()
        );
        assert!(
            validate_knowledge_source(
                KnowledgeSourceType::Url,
                None,
                None,
                None,
                None,
                &serde_json::json!({}),
            )
            .is_err()
        );
    }

    #[test]
    fn current_means_active_only_and_erasure_is_not_a_delete_alias() {
        assert!(KnowledgeLifecycleState::Active.is_current());
        for state in &KnowledgeLifecycleState::ALL[1..] {
            assert!(!state.is_current(), "{state} is not ordinary current truth");
        }
        assert!("deleted".parse::<KnowledgeLifecycleState>().is_err());
    }
}
