use std::io::{Cursor, Write};

use chrono::{TimeZone, Utc};
use serde_json::json;
use synveda_okf::{
    BundleInput, ExportKnowledge, ExportRelation, ExportSource, InputEntry, InputEntryKind,
    KnowledgeFormatAdapter, OKF_SPEC_COMMIT, OKF_VERSION, OkfAdapter, SourceDescriptor, SourceKind,
};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType,
    KnowledgeType,
};
use synveda_types::{KnowledgeItemId, KnowledgeRevisionId, KnowledgeSourceId, Sensitivity};

fn source(kind: SourceKind) -> SourceDescriptor {
    SourceDescriptor {
        kind,
        locator: "fixture:pulseboard".to_owned(),
        revision: (kind == SourceKind::Git).then(|| "0f4e5d6c".to_owned()),
    }
}

fn file(path: &str, content: &str) -> InputEntry {
    InputEntry {
        logical_path: path.to_owned(),
        kind: InputEntryKind::File,
        bytes: content.as_bytes().to_vec(),
    }
}

fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap()
}

#[test]
fn v02_unknown_types_metadata_trust_and_links_survive_mapping() {
    let bundle = OkfAdapter
        .inspect(
            source(SourceKind::Directory),
            BundleInput::Entries(vec![
                file(
                    "index.md",
                    "---\nokf_version: \"0.2\"\n---\n\n# PulseBoard\n",
                ),
                file(
                    "decisions/webhooks.md",
                    r#"---
type: Vendor Runbook Matrix
title: Webhook identity
description: Provider event IDs deduplicate webhook delivery.
tags: [Webhooks, Delivery, webhooks]
generated: { by: human:alice, at: 2026-08-20T10:00:00Z }
verified: { by: human:bob, at: 2026-08-24T09:00:00Z }
status: stable
stale_after: 2027-01-01T00:00:00Z
sources:
  - id: provider-contract
    resource: https://docs.example.test/events
vendor_extension: { owner: platform, level: 7 }
---

Use the provider event ID. See [request tracing](../conventions/tracing.md).
"#,
                ),
                file(
                    "conventions/tracing.md",
                    "---\ntype: Convention\ntitle: Request tracing\n---\n\nUse traceparent.\n",
                ),
            ]),
            at(),
        )
        .unwrap();

    assert_eq!(bundle.format_version, OKF_VERSION);
    assert_eq!(bundle.specification_commit, OKF_SPEC_COMMIT);
    assert_eq!(bundle.concepts.len(), 2);
    let unknown = bundle
        .concepts
        .iter()
        .find(|concept| concept.logical_path == "decisions/webhooks.md")
        .unwrap();
    assert_eq!(unknown.okf_type, "Vendor Runbook Matrix");
    assert_eq!(unknown.knowledge_type, KnowledgeType::Reference);
    assert_eq!(unknown.content.tags, ["delivery", "webhooks"]);
    assert_eq!(
        unknown.content.verification_metadata["okf"]["trust_tier"],
        "human-reviewed"
    );
    assert_eq!(
        unknown.content.metadata["okf"]["frontmatter"]["vendor_extension"]["owner"],
        "platform"
    );
    assert_eq!(
        unknown.links[0].target_logical_path,
        "conventions/tracing.md"
    );
}

#[test]
fn hard_cut_refuses_v01_fallbacks_and_missing_type() {
    let error = OkfAdapter
        .inspect(
            source(SourceKind::Directory),
            BundleInput::Entries(vec![
                file("index.md", "---\nokf_version: \"0.1\"\n---\n"),
                file("fact.md", "---\ntype: Fact\n---\nBody\n"),
            ]),
            at(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("expected 0.2"));

    let missing = OkfAdapter
        .inspect(
            source(SourceKind::Directory),
            BundleInput::Entries(vec![file(
                "fact.md",
                "---\ntimestamp: 2026-08-20T10:00:00Z\n---\nBody\n",
            )]),
            at(),
        )
        .unwrap_err();
    assert!(missing.to_string().contains("non-empty type"));
}

#[test]
fn source_and_entry_boundaries_refuse_credentials_links_binary_and_escape() {
    let credential_source = SourceDescriptor {
        kind: SourceKind::Git,
        locator: "https://alice:secret@example.test/repo".to_owned(),
        revision: None,
    };
    assert!(
        OkfAdapter
            .inspect(
                credential_source,
                BundleInput::Entries(vec![file("a.md", "---\ntype: Fact\n---\nA\n")]),
                at(),
            )
            .is_err()
    );
    for entry in [
        InputEntry {
            logical_path: "../escape.md".to_owned(),
            kind: InputEntryKind::File,
            bytes: b"---\ntype: Fact\n---\nA\n".to_vec(),
        },
        InputEntry {
            logical_path: "link.md".to_owned(),
            kind: InputEntryKind::Symlink,
            bytes: b"target".to_vec(),
        },
        InputEntry {
            logical_path: "references/run.py".to_owned(),
            kind: InputEntryKind::File,
            bytes: b"print('never')".to_vec(),
        },
    ] {
        assert!(
            OkfAdapter
                .inspect(
                    source(SourceKind::Directory),
                    BundleInput::Entries(vec![entry]),
                    at(),
                )
                .is_err()
        );
    }
}

#[test]
fn zip_and_tar_inputs_use_the_same_validation_and_digest() {
    let markdown = b"---\ntype: Decision\ntitle: Header\n---\n\nUse traceparent.\n";
    let mut zip_bytes = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_bytes);
        zip.start_file(
            "decisions/header.md",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(markdown).unwrap();
        zip.finish().unwrap();
    }
    let zip = OkfAdapter
        .inspect(
            source(SourceKind::Zip),
            BundleInput::Zip(zip_bytes.into_inner()),
            at(),
        )
        .unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut tar = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_path("decisions/header.md").unwrap();
        header.set_size(markdown.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, markdown.as_slice()).unwrap();
        tar.finish().unwrap();
    }
    let tar = OkfAdapter
        .inspect(source(SourceKind::Tar), BundleInput::Tar(tar_bytes), at())
        .unwrap();
    assert_eq!(zip.bundle_digest, tar.bundle_digest);
    assert_eq!(zip.concepts[0].content_hash, tar.concepts[0].content_hash);
}

#[test]
fn deterministic_export_round_trips_extensions_without_v01_residue() {
    let item = KnowledgeItemId::new();
    let target = KnowledgeItemId::new();
    let content = KnowledgeRevisionContent {
        title: "Webhook identity".to_owned(),
        body_markdown: "Use provider event IDs.".to_owned(),
        summary: "Provider event IDs deduplicate delivery.".to_owned(),
        tags: vec!["webhooks".to_owned()],
        sensitivity: Sensitivity::Internal,
        confidence_permille: 800,
        valid_from: at(),
        valid_to: None,
        stale_after: Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()),
        verification_metadata: json!({
            "okf": {
                "generated": {"by": "human:alice", "at": "2026-08-25T12:00:00Z"},
                "verified": {"by": "human:bob", "at": "2026-08-25T13:00:00Z"}
            }
        }),
        metadata: json!({
            "okf": {
                "logical_path": "decisions/webhooks.md",
                "type": "Vendor Decision",
                "frontmatter": {
                    "type": "Vendor Decision",
                    "timestamp": "2020-01-01T00:00:00Z",
                    "vendor_extension": {"kept": true}
                }
            }
        }),
    };
    let input = vec![
        ExportKnowledge {
            item_id: item,
            revision_id: KnowledgeRevisionId::new(),
            knowledge_type: KnowledgeType::Decision,
            origin: KnowledgeOrigin::Imported,
            lifecycle: KnowledgeLifecycleState::Active,
            content,
            sources: vec![ExportSource {
                id: KnowledgeSourceId::new(),
                source_type: KnowledgeSourceType::Okf,
                locator: Some("bundle:decisions/webhooks.md".to_owned()),
                source_revision: Some("0f4e5d6c".to_owned()),
                content_hash: Some("a".repeat(64)),
                metadata: json!({}),
            }],
            relations: vec![ExportRelation {
                target_item_id: target,
                relation: "references".to_owned(),
            }],
        },
        ExportKnowledge {
            item_id: target,
            revision_id: KnowledgeRevisionId::new(),
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Authored,
            lifecycle: KnowledgeLifecycleState::Active,
            content: KnowledgeRevisionContent {
                title: "Trace header".to_owned(),
                body_markdown: "Use traceparent.".to_owned(),
                summary: "Trace header convention.".to_owned(),
                tags: vec![],
                sensitivity: Sensitivity::Internal,
                confidence_permille: 900,
                valid_from: at(),
                valid_to: None,
                stale_after: None,
                verification_metadata: json!({}),
                metadata: json!({}),
            },
            sources: vec![],
            relations: vec![],
        },
    ];
    let first = OkfAdapter.export(&input).unwrap();
    let second = OkfAdapter.export(&input).unwrap();
    assert_eq!(first, second);
    let source_file = first
        .files
        .iter()
        .find(|file| file.logical_path == "decisions/webhooks.md")
        .unwrap();
    assert!(source_file.content.contains("vendor_extension:"));
    assert!(!source_file.content.contains("timestamp:"));
    assert!(source_file.content.contains("/knowledge/"));

    let round_trip = OkfAdapter
        .inspect(
            source(SourceKind::Directory),
            BundleInput::Entries(
                first
                    .files
                    .iter()
                    .map(|file| InputEntry {
                        logical_path: file.logical_path.clone(),
                        kind: InputEntryKind::File,
                        bytes: file.content.as_bytes().to_vec(),
                    })
                    .collect(),
            ),
            at(),
        )
        .unwrap();
    assert_eq!(round_trip.concepts.len(), 2);
    assert!(round_trip.concepts.iter().any(|concept| {
        concept.content.metadata["okf"]["frontmatter"]["vendor_extension"]["kept"] == true
    }));
}
