use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use synveda_types::knowledge::{
    KnowledgeRevisionContent, KnowledgeType, knowledge_revision_content_hash,
    normalise_knowledge_tags, validate_knowledge_revision_content,
};
use synveda_types::{Error, Result, Sensitivity};

use crate::archive;
use crate::export::{ExportBundle, ExportKnowledge, render_export};
use crate::{
    MAX_ARTIFACT_BYTES, MAX_ARTIFACTS, MAX_EXPANDED_BYTES, MAX_FRONTMATTER_BYTES, OKF_SPEC_COMMIT,
    OKF_VERSION,
};

/// Where an imported OKF bundle came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A client-selected directory tree.
    Directory,
    /// A zip archive.
    Zip,
    /// A tar or compressed tar archive.
    Tar,
    /// A checked-out Git tree with an explicitly reported revision.
    Git,
}

impl SourceKind {
    /// Stable wire/storage value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::Git => "git",
        }
    }
}

/// Bounded byte envelope supplied to the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleEncoding {
    /// Already enumerated inert files, used for a directory or checked-out Git tree.
    Entries,
    /// PKZIP bytes.
    Zip,
    /// POSIX tar bytes.
    Tar,
    /// Gzip-compressed POSIX tar bytes.
    TarGzip,
}

impl BundleEncoding {
    /// Stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Entries => "entries",
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGzip => "tar_gzip",
        }
    }
}

/// Entry kind reported by a client enumerating a local tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputEntryKind {
    /// An ordinary inert file.
    File,
    /// A directory marker. It carries no content and is ignored.
    Directory,
    /// A symbolic link. Always refused.
    Symlink,
    /// A device, socket or other special filesystem entry. Always refused.
    Special,
}

/// One client-enumerated bundle entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEntry {
    /// Bundle-relative slash-separated logical path.
    pub logical_path: String,
    /// Entry kind, retained so a client cannot silently dereference a link.
    pub kind: InputEntryKind,
    /// Exact bytes for a regular file; empty for a directory marker.
    pub bytes: Vec<u8>,
}

/// One complete bundle input. Paths are never read for remote API requests.
#[derive(Debug, Clone)]
pub enum BundleInput {
    /// Explicit inert entries supplied by a client.
    Entries(Vec<InputEntry>),
    /// Bounded zip bytes.
    Zip(Vec<u8>),
    /// Bounded tar bytes.
    Tar(Vec<u8>),
    /// Bounded gzip-compressed tar bytes.
    TarGzip(Vec<u8>),
    /// A local-only directory seam for controlled clients and fixture tests.
    Directory(PathBuf),
}

/// Source identity retained beside the immutable bundle digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
    /// Directory, archive or Git working tree.
    pub kind: SourceKind,
    /// Bounded non-secret source label; never a gateway-local path grant.
    pub locator: String,
    /// Explicit source revision when available; required for Git.
    pub revision: Option<String>,
}

/// Role one Markdown artifact plays in an OKF v0.2 bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// A concept document with required frontmatter `type`.
    Concept,
    /// Reserved progressive-disclosure index.
    Index,
    /// Reserved chronological update log.
    Log,
}

/// One immutable parsed import artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportArtifact {
    /// Canonical bundle-relative path.
    pub logical_path: String,
    /// Concept, reserved index or reserved log.
    pub kind: ArtifactKind,
    /// BLAKE3-256 of exact admitted bytes.
    pub content_hash: String,
    /// Parsed frontmatter, or an empty object for reserved files without it.
    pub frontmatter: Value,
    /// Exact Markdown body after frontmatter.
    pub body_markdown: String,
}

/// One internal OKF link proposed as a Knowledge relation after publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedLink {
    /// Target concept's canonical logical path.
    pub target_logical_path: String,
    /// Initial Synveda relation vocabulary. OKF links are untyped.
    pub relation: String,
}

/// One concept mapped into complete proposed Knowledge content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedConcept {
    /// Index into [`BundleInspection::artifacts`].
    pub artifact_index: usize,
    /// Stable source path used by imports and deterministic round trips.
    pub logical_path: String,
    /// Exact producer-defined OKF type, including unknown values.
    pub okf_type: String,
    /// Synveda's proposed Knowledge vocabulary.
    pub knowledge_type: KnowledgeType,
    /// Complete immutable revision content proposal.
    pub content: KnowledgeRevisionContent,
    /// Canonical semantic content hash.
    pub content_hash: String,
    /// Proposed links to other concepts in this bundle.
    pub links: Vec<ProposedLink>,
    /// Whether the external lifecycle permits an active candidate.
    pub materializable: bool,
}

/// Validated, deterministic inspection of one bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleInspection {
    /// Exact implemented format version.
    pub format_version: String,
    /// Exact canonical specification revision.
    pub specification_commit: String,
    /// Source identity supplied by the client.
    pub source: SourceDescriptor,
    /// Canonical digest over ordered logical paths and exact artifact hashes.
    pub bundle_digest: String,
    /// Every admitted immutable Markdown artifact.
    pub artifacts: Vec<ImportArtifact>,
    /// Every concept's proposed Knowledge mapping.
    pub concepts: Vec<ProposedConcept>,
    /// Content-free validation notices such as tolerated broken links.
    pub notices: Vec<String>,
}

/// Versioned boundary implemented by an external knowledge format.
pub trait KnowledgeFormatAdapter: Send + Sync {
    /// Stable external version identifier.
    fn version(&self) -> &'static str;

    /// Validate, inspect and map one bounded bundle without persistence.
    fn inspect(
        &self,
        source: SourceDescriptor,
        input: BundleInput,
        imported_at: DateTime<Utc>,
    ) -> Result<BundleInspection>;

    /// Render freshly authorised current Knowledge deterministically.
    fn export(&self, items: &[ExportKnowledge]) -> Result<ExportBundle>;
}

/// The repository-pinned OKF v0.2 implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct OkfAdapter;

impl KnowledgeFormatAdapter for OkfAdapter {
    fn version(&self) -> &'static str {
        OKF_VERSION
    }

    fn inspect(
        &self,
        source: SourceDescriptor,
        input: BundleInput,
        imported_at: DateTime<Utc>,
    ) -> Result<BundleInspection> {
        validate_source(&source)?;
        let entries = archive::entries(input)?;
        inspect_entries(source, entries, imported_at)
    }

    fn export(&self, items: &[ExportKnowledge]) -> Result<ExportBundle> {
        render_export(items)
    }
}

fn validate_source(source: &SourceDescriptor) -> Result<()> {
    let locator = source.locator.trim();
    if locator.is_empty() || locator.chars().count() > 1_000 || locator.contains('\0') {
        return Err(Error::Invalid {
            message: "OKF source locator must be non-blank and at most 1000 characters".to_owned(),
        });
    }
    if locator
        .split_once("://")
        .is_some_and(|(_, authority)| authority.split('/').next().is_some_and(|v| v.contains('@')))
    {
        return Err(Error::Invalid {
            message: "OKF source locators must not contain credentials".to_owned(),
        });
    }
    if source.kind == SourceKind::Git && source.revision.as_deref().is_none_or(str::is_empty) {
        return Err(Error::Invalid {
            message: "an OKF Git source requires an explicit reported revision".to_owned(),
        });
    }
    if source.revision.as_deref().is_some_and(|revision| {
        revision.trim().is_empty()
            || revision.chars().count() > 255
            || revision.contains(['\n', '\r', '\0'])
    }) {
        return Err(Error::Invalid {
            message: "OKF source revision must be non-blank and at most 255 characters".to_owned(),
        });
    }
    Ok(())
}

fn inspect_entries(
    source: SourceDescriptor,
    mut entries: Vec<InputEntry>,
    imported_at: DateTime<Utc>,
) -> Result<BundleInspection> {
    if entries.len() > MAX_ARTIFACTS {
        return Err(Error::Invalid {
            message: format!("OKF bundle exceeds {MAX_ARTIFACTS} entries"),
        });
    }
    entries.retain(|entry| entry.kind != InputEntryKind::Directory);
    if entries
        .iter()
        .any(|entry| entry.kind != InputEntryKind::File)
    {
        return Err(Error::Invalid {
            message: "OKF bundles must not contain symlinks or special entries".to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    let mut total = 0usize;
    for entry in &mut entries {
        entry.logical_path = archive::normalise_path(&entry.logical_path)?;
        if !seen.insert(entry.logical_path.clone()) {
            return Err(Error::Invalid {
                message: format!("duplicate OKF logical path: {}", entry.logical_path),
            });
        }
        if entry.bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF artifact exceeds byte limit: {}", entry.logical_path),
            });
        }
        total = total
            .checked_add(entry.bytes.len())
            .ok_or_else(|| Error::Invalid {
                message: "OKF expanded byte total overflowed".to_owned(),
            })?;
        if total > MAX_EXPANDED_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF bundle exceeds {MAX_EXPANDED_BYTES} expanded bytes"),
            });
        }
    }
    entries.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    if entries.is_empty() {
        return Err(Error::Invalid {
            message: "OKF bundle contains no Markdown artifacts".to_owned(),
        });
    }

    let mut artifacts = Vec::with_capacity(entries.len());
    for entry in entries {
        let lower = entry.logical_path.to_ascii_lowercase();
        if !lower.ends_with(".md") {
            return Err(Error::Invalid {
                message: format!(
                    "unsupported executable or binary OKF artifact: {}",
                    entry.logical_path
                ),
            });
        }
        let text = String::from_utf8(entry.bytes.clone()).map_err(|_| Error::Invalid {
            message: format!("OKF artifact is not UTF-8 Markdown: {}", entry.logical_path),
        })?;
        if text.contains('\0') {
            return Err(Error::Invalid {
                message: format!("OKF artifact contains a NUL byte: {}", entry.logical_path),
            });
        }
        let name = entry.logical_path.rsplit('/').next().unwrap_or_default();
        let (kind, frontmatter, body) = match name {
            "index.md" => {
                let root = !entry.logical_path.contains('/');
                let (frontmatter, body) = parse_reserved_index(&text, root, &entry.logical_path)?;
                if root
                    && let Some(version) = frontmatter.get("okf_version")
                    && version.as_str() != Some(OKF_VERSION)
                {
                    return Err(Error::Invalid {
                        message: format!(
                            "unsupported OKF version in index.md: expected {OKF_VERSION}"
                        ),
                    });
                }
                (ArtifactKind::Index, frontmatter, body)
            }
            "log.md" => {
                validate_log(&text, &entry.logical_path)?;
                (ArtifactKind::Log, json!({}), text)
            }
            _ => {
                let (frontmatter, body) = parse_concept(&text, &entry.logical_path)?;
                (ArtifactKind::Concept, frontmatter, body)
            }
        };
        artifacts.push(ImportArtifact {
            logical_path: entry.logical_path,
            kind,
            content_hash: blake3::hash(&entry.bytes).to_hex().to_string(),
            frontmatter,
            body_markdown: body,
        });
    }

    // The adapter selection is explicit, so a missing declaration still means
    // v0.2. What is forbidden is a declaration of v0.1, not a conformant bundle
    // exercising the spec's optional version key.
    let concept_paths: BTreeSet<&str> = artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Concept)
        .map(|artifact| artifact.logical_path.as_str())
        .collect();
    let mut notices = Vec::new();
    let mut concepts = Vec::new();
    for (artifact_index, artifact) in artifacts.iter().enumerate() {
        if artifact.kind != ArtifactKind::Concept {
            continue;
        }
        let mut concept = map_concept(artifact_index, artifact, imported_at)?;
        let mut links = Vec::new();
        for target in markdown_links(&artifact.logical_path, &artifact.body_markdown)? {
            if concept_paths.contains(target.as_str()) {
                links.push(ProposedLink {
                    target_logical_path: target,
                    relation: "references".to_owned(),
                });
            } else {
                notices.push(format!("broken_link:{}", artifact.logical_path));
            }
        }
        links.sort_by(|left, right| left.target_logical_path.cmp(&right.target_logical_path));
        links.dedup_by(|left, right| left.target_logical_path == right.target_logical_path);
        concept.links = links;
        concepts.push(concept);
    }
    if concepts.is_empty() {
        return Err(Error::Invalid {
            message: "OKF bundle contains no concept documents".to_owned(),
        });
    }
    notices.sort();
    notices.dedup();
    let mut digest = blake3::Hasher::new();
    digest.update(OKF_VERSION.as_bytes());
    for artifact in &artifacts {
        digest.update(&(artifact.logical_path.len() as u64).to_be_bytes());
        digest.update(artifact.logical_path.as_bytes());
        digest.update(artifact.content_hash.as_bytes());
    }
    Ok(BundleInspection {
        format_version: OKF_VERSION.to_owned(),
        specification_commit: OKF_SPEC_COMMIT.to_owned(),
        source,
        bundle_digest: digest.finalize().to_hex().to_string(),
        artifacts,
        concepts,
        notices,
    })
}

fn parse_concept(text: &str, path: &str) -> Result<(Value, String)> {
    let (frontmatter, body) = split_frontmatter(text, path)?.ok_or_else(|| Error::Invalid {
        message: format!("OKF concept requires YAML frontmatter: {path}"),
    })?;
    let kind = frontmatter
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Invalid {
            message: format!("OKF concept requires a non-empty type: {path}"),
        })?;
    if kind.chars().count() > 200 {
        return Err(Error::Invalid {
            message: format!("OKF concept type exceeds 200 characters: {path}"),
        });
    }
    validate_known_frontmatter(&frontmatter, path)?;
    Ok((frontmatter, body.to_owned()))
}

fn parse_reserved_index(text: &str, root: bool, path: &str) -> Result<(Value, String)> {
    match split_frontmatter(text, path)? {
        None => Ok((json!({}), text.to_owned())),
        Some((frontmatter, body)) if root => {
            if frontmatter
                .as_object()
                .is_some_and(|map| map.keys().any(|key| key != "okf_version"))
            {
                return Err(Error::Invalid {
                    message: "root index.md frontmatter may contain only okf_version".to_owned(),
                });
            }
            Ok((frontmatter, body.to_owned()))
        }
        Some(_) => Err(Error::Invalid {
            message: format!("nested OKF index must not contain frontmatter: {path}"),
        }),
    }
}

fn split_frontmatter<'a>(text: &'a str, path: &str) -> Result<Option<(Value, &'a str)>> {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok(None);
    };
    let mut offset = 0usize;
    let mut closing = None;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            closing = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let Some((frontmatter_end, body_start)) = closing else {
        return Err(Error::Invalid {
            message: format!("unterminated OKF frontmatter: {path}"),
        });
    };
    if frontmatter_end > MAX_FRONTMATTER_BYTES {
        return Err(Error::Invalid {
            message: format!("OKF frontmatter exceeds byte limit: {path}"),
        });
    }
    let yaml = &rest[..frontmatter_end];
    reject_yaml_expansion(yaml, path)?;
    let frontmatter: Value = serde_yaml::from_str(yaml).map_err(|_| Error::Invalid {
        message: format!("OKF frontmatter is not a YAML mapping: {path}"),
    })?;
    if !frontmatter.is_object() {
        return Err(Error::Invalid {
            message: format!("OKF frontmatter is not a YAML mapping: {path}"),
        });
    }
    Ok(Some((frontmatter, &rest[body_start..])))
}

fn reject_yaml_expansion(yaml: &str, path: &str) -> Result<()> {
    let mut previous_indent = 0usize;
    for line in yaml.lines() {
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        if indent > 40 || indent.saturating_sub(previous_indent) > 20 {
            return Err(Error::Invalid {
                message: format!("OKF frontmatter nesting exceeds limit: {path}"),
            });
        }
        previous_indent = indent;
        let trimmed = line.trim_start();
        if trimmed.starts_with("<<:")
            || trimmed.starts_with('*')
            || trimmed.split_whitespace().any(|part| part.starts_with('&'))
        {
            return Err(Error::Invalid {
                message: format!("OKF frontmatter aliases are not supported: {path}"),
            });
        }
    }
    Ok(())
}

fn validate_known_frontmatter(frontmatter: &Value, path: &str) -> Result<()> {
    let object = frontmatter.as_object().expect("frontmatter checked above");
    for key in ["title", "description", "resource"] {
        if object.get(key).is_some_and(|value| !value.is_string()) {
            return Err(Error::Invalid {
                message: format!("OKF {key} must be a string: {path}"),
            });
        }
    }
    if object.get("tags").is_some_and(|value| {
        value
            .as_array()
            .is_none_or(|tags| tags.iter().any(|tag| !tag.is_string()))
    }) {
        return Err(Error::Invalid {
            message: format!("OKF tags must be a list of strings: {path}"),
        });
    }
    if let Some(status) = object.get("status")
        && !matches!(status.as_str(), Some("draft" | "stable" | "deprecated"))
    {
        return Err(Error::Invalid {
            message: format!("OKF status must be draft, stable or deprecated: {path}"),
        });
    }
    if let Some(generated) = object.get("generated") {
        let generated = generated.as_object().ok_or_else(|| Error::Invalid {
            message: format!("OKF generated must be a mapping: {path}"),
        })?;
        if generated.get("by").and_then(Value::as_str).is_none() {
            return Err(Error::Invalid {
                message: format!("OKF generated.by is required when generated is present: {path}"),
            });
        }
        parse_optional_time(generated.get("at"), "generated.at", path)?;
    }
    if let Some(verified) = object.get("verified") {
        let events: Vec<&Value> = if let Some(values) = verified.as_array() {
            values.iter().collect()
        } else {
            vec![verified]
        };
        for event in events {
            let event = event.as_object().ok_or_else(|| Error::Invalid {
                message: format!("OKF verified entries must be mappings: {path}"),
            })?;
            if event.get("by").and_then(Value::as_str).is_none() {
                return Err(Error::Invalid {
                    message: format!("OKF verified.by is required: {path}"),
                });
            }
            parse_optional_time(event.get("at"), "verified.at", path)?;
        }
    }
    if let Some(sources) = object.get("sources") {
        let sources = sources.as_array().ok_or_else(|| Error::Invalid {
            message: format!("OKF sources must be a list: {path}"),
        })?;
        if sources.len() > 199 {
            return Err(Error::Invalid {
                message: format!("OKF concept has more than 199 declared sources: {path}"),
            });
        }
        for source in sources {
            let resource = source
                .as_object()
                .and_then(|source| source.get("resource"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|resource| {
                    !resource.is_empty()
                        && resource.chars().count() <= 1_000
                        && !resource.contains('\0')
                });
            if resource.is_none() {
                return Err(Error::Invalid {
                    message: format!(
                        "OKF sources[].resource must be non-empty and at most 1000 characters: {path}"
                    ),
                });
            }
        }
    }
    parse_optional_time(object.get("stale_after"), "stale_after", path)?;
    Ok(())
}

fn parse_optional_time(
    value: Option<&Value>,
    field: &str,
    path: &str,
) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value.as_str().ok_or_else(|| Error::Invalid {
                message: format!("OKF {field} must be an ISO 8601 string: {path}"),
            })?)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| Error::Invalid {
                message: format!("OKF {field} must carry an explicit UTC offset: {path}"),
            })
        })
        .transpose()
}

fn map_concept(
    artifact_index: usize,
    artifact: &ImportArtifact,
    imported_at: DateTime<Utc>,
) -> Result<ProposedConcept> {
    let frontmatter = artifact
        .frontmatter
        .as_object()
        .expect("concept frontmatter object");
    let okf_type = frontmatter["type"]
        .as_str()
        .expect("validated type")
        .trim()
        .to_owned();
    let title = frontmatter
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| derived_title(&artifact.logical_path));
    let original_body_empty = artifact.body_markdown.trim().is_empty();
    let body_markdown = if original_body_empty {
        format!("# {title}\n")
    } else {
        artifact.body_markdown.clone()
    };
    let summary = frontmatter
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| summary_from(&body_markdown, &title));
    let tags = frontmatter
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags = normalise_knowledge_tags(&tags)?;
    let generated_at = frontmatter
        .get("generated")
        .and_then(Value::as_object)
        .and_then(|generated| generated.get("at"));
    let valid_from = parse_optional_time(generated_at, "generated.at", &artifact.logical_path)?
        .unwrap_or(imported_at);
    let stale_after = parse_optional_time(
        frontmatter.get("stale_after"),
        "stale_after",
        &artifact.logical_path,
    )?;
    let sensitivity = frontmatter
        .get("sensitivity")
        .and_then(Value::as_str)
        .map(str::parse)
        .transpose()?
        .unwrap_or(Sensitivity::Internal);
    let status = frontmatter
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("stable");
    let trust_tier = trust_tier(frontmatter.get("verified"));
    let verification_metadata = json!({
        "okf": {
            "generated": frontmatter.get("generated").cloned().unwrap_or(Value::Null),
            "verified": frontmatter.get("verified").cloned().unwrap_or(Value::Null),
            "trust_tier": trust_tier,
        }
    });
    let metadata = json!({
        "okf": {
            "version": OKF_VERSION,
            "specification_commit": OKF_SPEC_COMMIT,
            "logical_path": artifact.logical_path,
            "type": okf_type,
            "status": status,
            "frontmatter": Value::Object(frontmatter.clone()),
            "original_body_empty": original_body_empty,
        }
    });
    let content = KnowledgeRevisionContent {
        title,
        body_markdown,
        summary,
        tags,
        sensitivity,
        confidence_permille: 500,
        valid_from,
        valid_to: None,
        stale_after,
        verification_metadata,
        metadata,
    };
    validate_knowledge_revision_content(&content)?;
    Ok(ProposedConcept {
        artifact_index,
        logical_path: artifact.logical_path.clone(),
        okf_type: okf_type.clone(),
        knowledge_type: map_type(&okf_type),
        content_hash: knowledge_revision_content_hash(&content),
        content,
        links: Vec::new(),
        materializable: status != "deprecated",
    })
}

fn trust_tier(verified: Option<&Value>) -> &'static str {
    let Some(verified) = verified else {
        return "unverified";
    };
    let values: Vec<&Value> = verified
        .as_array()
        .map_or_else(|| vec![verified], |values| values.iter().collect());
    if values.iter().any(|value| {
        value
            .get("by")
            .and_then(Value::as_str)
            .is_some_and(|actor| actor.starts_with("human:"))
    }) {
        "human-reviewed"
    } else {
        "machine-confirmed"
    }
}

fn map_type(okf_type: &str) -> KnowledgeType {
    let lower = okf_type.to_ascii_lowercase();
    if lower.contains("decision") {
        KnowledgeType::Decision
    } else if lower.contains("preference") {
        KnowledgeType::Preference
    } else if lower.contains("procedure") || lower.contains("playbook") || lower.contains("howto") {
        KnowledgeType::Procedure
    } else if lower.contains("episode") || lower.contains("incident") {
        KnowledgeType::Episode
    } else if lower.contains("convention") {
        KnowledgeType::Convention
    } else if lower.contains("warning") || lower.contains("alert") {
        KnowledgeType::Warning
    } else if lower.contains("entity") || lower.contains("table") || lower.contains("dataset") {
        KnowledgeType::Entity
    } else if lower.contains("fact") || lower.contains("metric") {
        KnowledgeType::Fact
    } else {
        KnowledgeType::Reference
    }
}

fn derived_title(path: &str) -> String {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .strip_suffix(".md")
        .unwrap_or(path);
    let value = stem.replace(['-', '_'], " ");
    let mut chars = value.chars();
    chars.next().map_or_else(
        || "Untitled".to_owned(),
        |first| first.to_uppercase().collect::<String>() + chars.as_str(),
    )
}

fn summary_from(body: &str, title: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("```"))
        .unwrap_or(title)
        .chars()
        .take(2_000)
        .collect()
}

fn validate_log(text: &str, path: &str) -> Result<()> {
    for line in text.lines().filter(|line| line.starts_with("## ")) {
        let date = line.trim_start_matches("## ").trim();
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(Error::Invalid {
                message: format!("OKF log date heading must be YYYY-MM-DD: {path}"),
            });
        }
    }
    Ok(())
}

fn markdown_links(source_path: &str, body: &str) -> Result<Vec<String>> {
    let mut links = Vec::new();
    let mut remaining = body;
    while let Some(open) = remaining.find("](") {
        remaining = &remaining[open + 2..];
        let Some(close) = remaining.find(')') else {
            break;
        };
        let raw = remaining[..close]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['<', '>']);
        remaining = &remaining[close + 1..];
        if raw.is_empty()
            || raw.starts_with('#')
            || raw.contains("://")
            || raw.starts_with("mailto:")
            || raw.starts_with("data:")
        {
            continue;
        }
        let without_fragment = raw.split('#').next().unwrap_or_default();
        if !without_fragment.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let target = if let Some(rooted) = without_fragment.strip_prefix('/') {
            archive::normalise_path(rooted)?
        } else {
            let parent = source_path.rsplit_once('/').map(|(parent, _)| parent);
            archive::resolve_relative(parent, without_fragment)?
        };
        links.push(target);
    }
    Ok(links)
}

/// Extracts the preserved OKF frontmatter object from candidate/revision metadata.
pub(crate) fn preserved_frontmatter(metadata: &Value) -> BTreeMap<String, Value> {
    metadata
        .get("okf")
        .and_then(|okf| okf.get("frontmatter"))
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Converts a sorted map to the JSON mapping shape `serde_yaml` serialises deterministically.
pub(crate) fn object(map: BTreeMap<String, Value>) -> Value {
    Value::Object(Map::from_iter(map))
}
