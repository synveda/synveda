use std::collections::{BTreeMap, BTreeSet};

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType,
    KnowledgeType,
};
use synveda_types::{Error, KnowledgeItemId, KnowledgeRevisionId, KnowledgeSourceId, Result};

use crate::format::{object, preserved_frontmatter};
use crate::{OKF_SPEC_COMMIT, OKF_VERSION};

/// One independently authorised provenance source admitted to export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSource {
    /// Stable Synveda source id.
    pub id: KnowledgeSourceId,
    /// Source family.
    pub source_type: KnowledgeSourceType,
    /// Governed locator when independently visible.
    pub locator: Option<String>,
    /// Exact source revision when known.
    pub source_revision: Option<String>,
    /// Source content digest when known.
    pub content_hash: Option<String>,
    /// Bounded extension evidence.
    pub metadata: Value,
}

/// One independently authorised relation admitted to export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportRelation {
    /// Stable target Knowledge aggregate.
    pub target_item_id: KnowledgeItemId,
    /// Synveda relation vocabulary retained in extension metadata.
    pub relation: String,
}

/// One exact current Knowledge revision supplied by the governed gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportKnowledge {
    /// Stable aggregate id.
    pub item_id: KnowledgeItemId,
    /// Exact immutable revision.
    pub revision_id: KnowledgeRevisionId,
    /// Synveda knowledge vocabulary.
    pub knowledge_type: KnowledgeType,
    /// Creation origin.
    pub origin: KnowledgeOrigin,
    /// Current lifecycle state.
    pub lifecycle: KnowledgeLifecycleState,
    /// Exact current content.
    pub content: KnowledgeRevisionContent,
    /// Independently visible provenance only.
    pub sources: Vec<ExportSource>,
    /// Independently visible outgoing relations only.
    pub relations: Vec<ExportRelation>,
}

/// One deterministic output file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFile {
    /// Bundle-relative logical path.
    pub logical_path: String,
    /// Exact UTF-8 Markdown bytes.
    pub content: String,
    /// BLAKE3-256 of `content`.
    pub content_hash: String,
}

/// Deterministic OKF v0.2 bundle projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportBundle {
    /// Exact format version.
    pub format_version: String,
    /// Exact pinned specification revision.
    pub specification_commit: String,
    /// Stable ordered files, including generated root `index.md`.
    pub files: Vec<ExportFile>,
    /// Digest over ordered paths and hashes.
    pub bundle_digest: String,
}

pub(crate) fn render_export(items: &[ExportKnowledge]) -> Result<ExportBundle> {
    if items.len() > 2_000 {
        return Err(Error::Invalid {
            message: "an OKF export contains at most 2000 Knowledge items".to_owned(),
        });
    }
    let mut ordered = items.to_vec();
    ordered.sort_by_key(|item| item.item_id);
    let mut seen = BTreeSet::new();
    for item in &ordered {
        if !seen.insert(item.item_id) {
            return Err(Error::Invalid {
                message: format!(
                    "Knowledge item {} appears twice in OKF export",
                    item.item_id
                ),
            });
        }
        if item.lifecycle == KnowledgeLifecycleState::Erased
            || item.lifecycle == KnowledgeLifecycleState::ErasurePending
        {
            return Err(Error::Invalid {
                message: "erased or erasure-pending Knowledge cannot enter an OKF export"
                    .to_owned(),
            });
        }
    }

    let paths = export_paths(&ordered);
    let mut files = Vec::with_capacity(ordered.len() + 1);
    for item in &ordered {
        let content = render_item(item, &paths)?;
        files.push(ExportFile {
            logical_path: paths[&item.item_id].clone(),
            content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
            content,
        });
    }
    files.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    let index = render_index(&ordered, &paths);
    files.insert(
        0,
        ExportFile {
            logical_path: "index.md".to_owned(),
            content_hash: blake3::hash(index.as_bytes()).to_hex().to_string(),
            content: index,
        },
    );
    let mut digest = blake3::Hasher::new();
    digest.update(OKF_VERSION.as_bytes());
    for file in &files {
        digest.update(&(file.logical_path.len() as u64).to_be_bytes());
        digest.update(file.logical_path.as_bytes());
        digest.update(file.content_hash.as_bytes());
    }
    Ok(ExportBundle {
        format_version: OKF_VERSION.to_owned(),
        specification_commit: OKF_SPEC_COMMIT.to_owned(),
        files,
        bundle_digest: digest.finalize().to_hex().to_string(),
    })
}

fn export_paths(items: &[ExportKnowledge]) -> BTreeMap<KnowledgeItemId, String> {
    let desired: Vec<(KnowledgeItemId, Option<String>)> = items
        .iter()
        .map(|item| {
            let path = item
                .content
                .metadata
                .get("okf")
                .and_then(|okf| okf.get("logical_path"))
                .and_then(Value::as_str)
                .filter(|path| {
                    crate::archive::normalise_path(path).as_deref() == Ok(*path)
                        && path.to_ascii_lowercase().ends_with(".md")
                        && !matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
                })
                .map(ToOwned::to_owned);
            (item.item_id, path)
        })
        .collect();
    let mut counts = BTreeMap::<String, usize>::new();
    for path in desired.iter().filter_map(|(_, path)| path.as_ref()) {
        *counts.entry(path.clone()).or_default() += 1;
    }
    desired
        .into_iter()
        .map(|(id, desired)| {
            let path = desired
                .filter(|path| counts.get(path) == Some(&1))
                .unwrap_or_else(|| format!("knowledge/{id}.md"));
            (id, path)
        })
        .collect()
}

fn render_item(
    item: &ExportKnowledge,
    paths: &BTreeMap<KnowledgeItemId, String>,
) -> Result<String> {
    let mut frontmatter = preserved_frontmatter(&item.content.metadata);
    // v0.1 fallback fields are intentionally not carried by this hard-cut
    // adapter, even when they arrived as otherwise unknown metadata.
    for key in [
        "timestamp",
        "type",
        "title",
        "description",
        "tags",
        "status",
        "generated",
        "verified",
        "stale_after",
        "sources",
    ] {
        frontmatter.remove(key);
    }
    let okf = item.content.metadata.get("okf");
    let okf_type = okf
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| export_type(item.knowledge_type).to_owned());
    frontmatter.insert("type".to_owned(), Value::String(okf_type));
    frontmatter.insert(
        "title".to_owned(),
        Value::String(item.content.title.clone()),
    );
    frontmatter.insert(
        "description".to_owned(),
        Value::String(item.content.summary.clone()),
    );
    if !item.content.tags.is_empty() {
        frontmatter.insert(
            "tags".to_owned(),
            Value::Array(
                item.content
                    .tags
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    frontmatter.insert(
        "status".to_owned(),
        Value::String(
            match item.lifecycle {
                KnowledgeLifecycleState::Active | KnowledgeLifecycleState::Stale => "stable",
                KnowledgeLifecycleState::Superseded | KnowledgeLifecycleState::Archived => {
                    "deprecated"
                }
                KnowledgeLifecycleState::ErasurePending | KnowledgeLifecycleState::Erased => {
                    unreachable!("refused above")
                }
            }
            .to_owned(),
        ),
    );
    let generated = item
        .content
        .verification_metadata
        .get("okf")
        .and_then(|value| value.get("generated"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "by": "synveda/0.2.0",
                "at": item.content.valid_from.to_rfc3339_opts(SecondsFormat::Secs, true),
            })
        });
    frontmatter.insert("generated".to_owned(), generated);
    if let Some(verified) = item
        .content
        .verification_metadata
        .get("okf")
        .and_then(|value| value.get("verified"))
        .filter(|value| !value.is_null())
    {
        frontmatter.insert("verified".to_owned(), verified.clone());
    }
    if let Some(stale_after) = item.content.stale_after {
        frontmatter.insert(
            "stale_after".to_owned(),
            Value::String(stale_after.to_rfc3339_opts(SecondsFormat::Secs, true)),
        );
    }
    let sources = export_sources(&item.sources);
    if !sources.is_empty() {
        frontmatter.insert("sources".to_owned(), Value::Array(sources));
    }
    let relations = item
        .relations
        .iter()
        .filter_map(|relation| {
            paths.get(&relation.target_item_id).map(|path| {
                json!({
                    "type": relation.relation,
                    "target": format!("/{path}"),
                    "target_item_id": relation.target_item_id,
                })
            })
        })
        .collect::<Vec<_>>();
    frontmatter.insert(
        "synveda".to_owned(),
        json!({
            "item_id": item.item_id,
            "revision_id": item.revision_id,
            "knowledge_type": item.knowledge_type.as_str(),
            "origin": item.origin.as_str(),
            "lifecycle": item.lifecycle.as_str(),
            "specification_commit": OKF_SPEC_COMMIT,
            "relations": relations,
        }),
    );
    let yaml = serde_yaml::to_string(&object(frontmatter)).map_err(|_| Error::Internal {
        message: "deterministic OKF frontmatter serialization failed".to_owned(),
    })?;
    let original_empty = item
        .content
        .metadata
        .get("okf")
        .and_then(|okf| okf.get("original_body_empty"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let synthetic_empty = format!("# {}\n", item.content.title);
    let mut body = if original_empty && item.content.body_markdown == synthetic_empty {
        String::new()
    } else {
        item.content.body_markdown.trim_end().to_owned()
    };
    let visible_relations: Vec<_> = item
        .relations
        .iter()
        .filter_map(|relation| {
            paths
                .get(&relation.target_item_id)
                .map(|path| (&relation.relation, path))
        })
        .collect();
    if !visible_relations.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("## Related knowledge\n\n");
        for (relation, path) in visible_relations {
            body.push_str(&format!("- [{relation}] (/{path})\n").replace("] (", "]("));
        }
    }
    Ok(format!(
        "---\n{}---\n\n{}{}",
        yaml,
        body,
        if body.is_empty() { "" } else { "\n" }
    ))
}

fn export_sources(sources: &[ExportSource]) -> Vec<Value> {
    let mut sources = sources.to_vec();
    sources.sort_by_key(|source| source.id);
    sources
        .into_iter()
        .map(|source| {
            let mut value = source
                .metadata
                .get("okf")
                .and_then(|okf| okf.get("source"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            value
                .entry("id".to_owned())
                .or_insert_with(|| Value::String(format!("synveda-{}", source.id)));
            value.entry("resource".to_owned()).or_insert_with(|| {
                Value::String(
                    source
                        .locator
                        .unwrap_or_else(|| format!("synveda:source:{}", source.id)),
                )
            });
            value.insert(
                "synveda_source_type".to_owned(),
                Value::String(source.source_type.as_str().to_owned()),
            );
            if let Some(revision) = source.source_revision {
                value.insert("source_revision".to_owned(), Value::String(revision));
            }
            if let Some(hash) = source.content_hash {
                value.insert("content_hash".to_owned(), Value::String(hash));
            }
            Value::Object(value)
        })
        .collect()
}

fn render_index(items: &[ExportKnowledge], paths: &BTreeMap<KnowledgeItemId, String>) -> String {
    let mut body = String::from("---\nokf_version: \"0.2\"\n---\n\n# Synveda Knowledge export\n\n");
    for item in items {
        body.push_str(&format!(
            "- [{}]({}) - {}\n",
            item.content.title, paths[&item.item_id], item.content.summary
        ));
    }
    body
}

fn export_type(kind: KnowledgeType) -> &'static str {
    match kind {
        KnowledgeType::Fact => "Fact",
        KnowledgeType::Decision => "Decision",
        KnowledgeType::Preference => "Preference",
        KnowledgeType::Procedure => "Procedure",
        KnowledgeType::Entity => "Entity",
        KnowledgeType::Episode => "Episode",
        KnowledgeType::Convention => "Convention",
        KnowledgeType::Warning => "Warning",
        KnowledgeType::Reference => "Reference",
    }
}
