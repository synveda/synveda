use chrono::{TimeZone, Utc};
use serde_json::json;
use synveda_okf::{
    BundleInput, BundleInspection, ExportKnowledge, InputEntry, InputEntryKind,
    KnowledgeFormatAdapter, MAX_FRONTMATTER_BYTES, OkfAdapter, SourceDescriptor, SourceKind,
    validate_export_bundle,
};
use synveda_types::knowledge::{KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeType};

const SCALARS: &str = include_str!("fixtures/scalar-types.md");

fn inspect(markdown: &str) -> synveda_types::Result<BundleInspection> {
    OkfAdapter.inspect(
        SourceDescriptor {
            kind: SourceKind::Directory,
            locator: "fixture:yaml-boundary".to_owned(),
            revision: None,
        },
        BundleInput::Entries(vec![InputEntry {
            logical_path: "facts/scalars.md".to_owned(),
            kind: InputEntryKind::File,
            bytes: markdown.as_bytes().to_vec(),
        }]),
        Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap(),
    )
}

#[test]
fn scalar_types_and_inert_extension_text_survive_mapping() {
    let bundle = inspect(SCALARS).unwrap();
    let extension = &bundle.artifacts[0].frontmatter["vendor_extension"];
    assert_eq!(
        extension,
        &json!({
            "true_values": [true, true, true],
            "false_values": [false, false, false],
            "legacy_words": ["yes", "no", "on", "off", "y", "n"],
            "opaque_id": "0123", "decimal_separator": "1_000",
            "binary": 3, "octal": 15, "hex": 255,
            "unsigned_max": u64::MAX, "signed_min": i64::MIN,
            "exponent": 1200.0, "negative_zero": -0.0,
            "timestamp": "2026-08-20T10:00:00Z", "forced_string": "true",
            "string_nan": ".nan", "float_overflow": "1e999", "unicode": "नमस्ते",
            "quoted": "foo *baz << !include ${HOME}",
            "literal": "Keep literal text.\n!include /private/synthetic.yaml\n",
            "folded": "Preserve words.",
        })
    );
    assert!(
        extension["negative_zero"]
            .as_f64()
            .unwrap()
            .is_sign_negative()
    );
    assert_eq!(
        bundle.concepts[0].content.metadata["okf"]["frontmatter"],
        bundle.artifacts[0].frontmatter
    );
}

#[test]
fn literal_yaml_punctuation_does_not_create_expansion_or_inclusion() {
    let markdown = SCALARS
        .replace("foo *baz", "foo &bar *baz")
        .replace("Keep literal text.", "Keep &literal and *literal inert.");
    let bundle = inspect(&markdown).unwrap();
    let extension = &bundle.artifacts[0].frontmatter["vendor_extension"];
    assert_eq!(extension["quoted"], "foo &bar *baz << !include ${HOME}");
    assert_eq!(
        extension["literal"],
        "Keep &literal and *literal inert.\n!include /private/synthetic.yaml\n"
    );
}

#[test]
fn hostile_syntax_is_refused_without_echoing_source_text() {
    for yaml in [
        "type: Fact\ntype: Decision\n",
        "type: Fact\nvendor: {key: one, key: two}\n",
        "type: Fact\nvendor: &anchor value\n",
        "type: Fact\nvendor: [&anchor value, *anchor]\n",
        "type: Fact\nvendor: {nested: &anchor {key: value}}\n",
        "type: Fact\nvendor: *missing\n",
        "type: Fact\nvendor: {<<: {key: value}}\n",
        "type: Fact\nvendor: !include /private/synthetic.yaml\n",
        "type: Fact\nvendor: !custom value\n",
        "type: Fact\nvendor: !!binary SGVsbG8=\n",
        "type: Fact\nvendor: !!seq {key: value}\n",
        "type: Fact\nvendor: !!map [value]\n",
        "type: Fact\nvendor: !!float .nan\n",
        "type: Fact\nvendor: .inf\n",
        "type: Fact\nvendor: -.Inf\n",
        "type: Fact\nvendor: 18446744073709551616\n",
        "type: Fact\n...\ntype: Decision\n",
        "[Fact, Decision]\n",
        "scalar\n",
        "type: Fact\nvendor: [unterminated\n",
    ] {
        let markdown = format!("---\n{yaml}# synthetic-secret-do-not-echo\n---\nBody.\n");
        let error = inspect(&markdown).unwrap_err().to_string();
        assert!(error.contains("facts/scalars.md"), "{error}");
        assert!(!error.contains("synthetic-secret-do-not-echo"), "{error}");
        assert!(!error.contains("/private/synthetic.yaml"), "{error}");
    }
}

#[test]
fn structural_and_byte_limits_apply_to_flow_and_block_yaml() {
    let flow = format!(
        "type: Fact\nvendor: {}value{}\n",
        "[".repeat(40),
        "]".repeat(40)
    );
    assert!(inspect(&format!("---\n{flow}---\nBody.\n")).is_err());
    let block = (0..40)
        .map(|depth| format!("{}key:\n", " ".repeat(depth)))
        .collect::<String>();
    assert!(inspect(&format!("---\ntype: Fact\n{block}---\nBody.\n")).is_err());
    let prefix = "type: Fact\n#";
    let allowed = format!(
        "{prefix}{}\n",
        "a".repeat(MAX_FRONTMATTER_BYTES - prefix.len() - 1)
    );
    assert_eq!(allowed.len(), MAX_FRONTMATTER_BYTES);
    inspect(&format!("---\n{allowed}---\nBody.\n")).unwrap();
    assert!(inspect(&format!("---\n{allowed}\n---\nBody.\n")).is_err());
}

#[test]
fn export_has_deterministic_delimiters_and_lossless_reimport() {
    let original = inspect(SCALARS).unwrap();
    let item = ExportKnowledge {
        item_id: "00000000-0000-7000-8000-000000000001".parse().unwrap(),
        revision_id: "00000000-0000-7000-8000-000000000002".parse().unwrap(),
        knowledge_type: KnowledgeType::Fact,
        origin: KnowledgeOrigin::Imported,
        lifecycle: KnowledgeLifecycleState::Active,
        content: original.concepts[0].content.clone(),
        sources: vec![],
        relations: vec![],
    };
    let bundle = OkfAdapter.export(std::slice::from_ref(&item)).unwrap();
    assert_eq!(bundle, OkfAdapter.export(&[item]).unwrap());
    validate_export_bundle(&bundle).unwrap();
    let content = &bundle
        .files
        .iter()
        .find(|file| file.logical_path == "facts/scalars.md")
        .unwrap()
        .content;
    assert_eq!(content.lines().filter(|line| *line == "---").count(), 2);
    assert!(!content.contains("%YAML"));
    let round_trip = inspect(content).unwrap();
    assert_eq!(
        round_trip.artifacts[0].frontmatter["vendor_extension"],
        original.artifacts[0].frontmatter["vendor_extension"]
    );
    assert_eq!(round_trip.artifacts[0].body_markdown, "\nBody.\n");
    assert_eq!(
        blake3::hash(content.as_bytes()).to_hex().as_str(),
        "49ce538566b56863b736c93b6ea785df3033ffceffb2cdb2207d1e14e0daa084"
    );
}

#[test]
fn unchanged_source_bytes_keep_artifact_and_knowledge_identities() {
    let first = inspect(SCALARS).unwrap();
    let repeated = inspect(SCALARS).unwrap();
    assert_eq!(first, repeated);
    // Measured with the previous YAML parser at this fixed import time. These
    // identities participate in retained import and suppression contracts.
    assert_eq!(
        first.artifacts[0].content_hash,
        "472bec66bf6118ef20fd4a4bf229e8c9143284773c9587dc67d3539ed2588d63"
    );
    assert_eq!(
        first.bundle_digest,
        "16770026529739d8e09d2dd8b73818cf43780249c5487adc7b4df919f9aaaa82"
    );
    assert_eq!(
        first.concepts[0].content_hash,
        "eea636af8b32481e9e6db10e45ac8cddee9eb9108697ca9057d1416119ed79f9"
    );
}

#[test]
fn opaque_scalar_strings_remain_strings_after_export() {
    let original = inspect(SCALARS).unwrap();
    let mut content = original.concepts[0].content.clone();
    let strings = json!([
        "0b11", "0B1_0", "0012", "1_000", ".nan", "yes", "null", "TRUE"
    ]);
    content.title = "0b11".to_owned();
    content.metadata["okf"]["type"] = json!("0B1_0");
    content.metadata["okf"]["frontmatter"]["opaque_strings"] = strings.clone();
    let item = ExportKnowledge {
        item_id: "00000000-0000-7000-8000-000000000001".parse().unwrap(),
        revision_id: "00000000-0000-7000-8000-000000000002".parse().unwrap(),
        knowledge_type: KnowledgeType::Fact,
        origin: KnowledgeOrigin::Imported,
        lifecycle: KnowledgeLifecycleState::Active,
        content,
        sources: vec![],
        relations: vec![],
    };
    let bundle = OkfAdapter.export(&[item]).unwrap();
    let rendered = &bundle
        .files
        .iter()
        .find(|file| file.logical_path == "facts/scalars.md")
        .unwrap()
        .content;
    let imported = inspect(rendered).unwrap();
    assert_eq!(imported.concepts[0].content.title, "0b11");
    assert_eq!(imported.concepts[0].okf_type, "0B1_0");
    assert_eq!(imported.artifacts[0].frontmatter["opaque_strings"], strings);
}
