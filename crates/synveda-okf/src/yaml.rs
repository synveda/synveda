use noyalib::policy::{DenyAnchors, Policy, PolicyEvent, PolicyEventKind};
use serde_json::Value;
use synveda_types::{Error, Result};

use crate::MAX_FRONTMATTER_BYTES;

pub(crate) fn parse(input: &str, path: &str) -> Result<Value> {
    // Preserve scalar types while bounding actual parser structure, including
    // compact flow syntax. Policies force the checked AST path before Serde.
    let config = noyalib::ParserConfig::serde_yaml_compat()
        .max_depth(32)
        .max_document_length(MAX_FRONTMATTER_BYTES)
        .max_stream_bytes(MAX_FRONTMATTER_BYTES)
        .max_documents(1)
        .max_nodes(MAX_FRONTMATTER_BYTES)
        .max_events(MAX_FRONTMATTER_BYTES * 2)
        .max_total_scalar_bytes(MAX_FRONTMATTER_BYTES)
        .max_mapping_keys(MAX_FRONTMATTER_BYTES)
        .max_sequence_length(MAX_FRONTMATTER_BYTES)
        .max_alias_expansions(0)
        .max_merge_keys(0)
        .duplicate_key_policy(noyalib::DuplicateKeyPolicy::Error)
        .merge_key_policy(noyalib::MergeKeyPolicy::Error)
        .with_policy(DenyAnchors)
        .with_policy(JsonValues);
    let value: Value = noyalib::from_str_with_config(input, &config).map_err(|_| invalid(path))?;
    if !value.is_object() {
        return Err(invalid(path));
    }
    Ok(value)
}

#[derive(Debug)]
struct JsonValues;

impl Policy for JsonValues {
    fn check_event(&self, event: PolicyEvent<'_>) -> noyalib::Result<()> {
        if let Some(tag) = event.tag {
            let name = tag
                .strip_prefix("!!")
                .or_else(|| tag.strip_prefix("tag:yaml.org,2002:"));
            let allowed = matches!(
                (name, event.kind),
                (
                    Some("str" | "int" | "float" | "bool" | "null"),
                    PolicyEventKind::Scalar
                ) | (Some("seq"), PolicyEventKind::SequenceStart)
                    | (Some("map"), PolicyEventKind::MappingStart)
            );
            if !allowed {
                return Err(noyalib::Error::Deserialize(
                    "unsupported OKF YAML tag".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn check_value(&self, value: &noyalib::Value) -> noyalib::Result<()> {
        // Run before JSON's visitor can silently turn a non-finite number into
        // null. The library also refuses unrepresented tagged values.
        noyalib::validate_yaml_json_schema(value)
    }
}

fn invalid(path: &str) -> Error {
    Error::Invalid {
        message: format!("OKF frontmatter is not a bounded YAML mapping: {path}"),
    }
}

pub(crate) fn emit(value: &Value) -> Result<String> {
    // Quote every string: the emitter's numeric heuristics do not cover all
    // binary forms the parser recognizes. Frontmatter owns its delimiters.
    let config = noyalib::SerializerConfig::new()
        .indent(2)
        .document_start(false)
        .document_end(false)
        .compact_list_indent(true)
        .quote_all(true)
        .prefer_single_quotes(true)
        .max_depth(32);
    let mut output =
        noyalib::to_string_with_config(value, &config).map_err(|_| Error::Internal {
            message: "deterministic OKF frontmatter serialization failed".to_owned(),
        })?;
    // The emitter need not end a document with LF when markers are disabled;
    // the Markdown closing delimiter must begin on its own line.
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}
