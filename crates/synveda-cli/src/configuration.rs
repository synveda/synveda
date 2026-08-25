//! Public-API client for governed runtime Configuration (CPR-30, ADR-0089).
//!
//! This module has no store authority. Templates, stable artifacts,
//! immutable versions and revisioned bindings are read or mutated through
//! the gateway, where the PDP, typed VedaFlow effect and audit chain apply.

use std::path::Path;

use serde_json::{Value, json};
use synveda_types::configuration::ConfigurationTemplate;
use synveda_types::{
    ConfigurationArtifactId, ConfigurationBindingId, ConfigurationVersionId, ScopeId,
};

use crate::api::{Api, Origin};

fn key(kind: &str) -> String {
    format!("configuration-{kind}-{}", ConfigurationArtifactId::new())
}

fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(profile) => eprintln!(
            "configuration as {} (profile {profile}) · trace {}",
            api.subject,
            api.trace_id()
        ),
        Origin::Environment => eprintln!(
            "configuration as {} (SYNVEDA_TOKEN) · trace {}",
            api.subject,
            api.trace_id()
        ),
    }
}

fn render(value: &Value, json_output: bool) -> Result<(), String> {
    if !json_output
        && let (Some(outcome), Some(change)) = (
            value.get("outcome").and_then(Value::as_str),
            value.get("change_id").and_then(Value::as_str),
        )
    {
        println!("{outcome} through VedaFlow change {change}");
        for (label, field) in [
            ("artifact", "artifact_id"),
            ("version", "version_id"),
            ("binding", "binding_id"),
            ("binding revision", "binding_revision"),
        ] {
            if let Some(value) = value.get(field).filter(|entry| !entry.is_null()) {
                println!(
                    "{label}: {}",
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), ToOwned::to_owned)
                );
            }
        }
        return Ok(());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn document(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {} as JSON: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!(
            "{} must contain one complete Configuration JSON object",
            path.display()
        ));
    }
    Ok(value)
}

async fn connect(profile: &str) -> Result<Api, String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    Ok(api)
}

pub async fn templates(profile: &str, json_output: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    render(&api.get("/v1/configuration-templates").await?, json_output)
}

pub async fn list(profile: &str, scope: Option<ScopeId>, json_output: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    let path = scope.map_or_else(
        || "/v1/configurations?limit=100".to_owned(),
        |scope| format!("/v1/configurations?governing_scope_id={scope}&limit=100"),
    );
    render(&api.get(&path).await?, json_output)
}

pub async fn show(
    profile: &str,
    id: ConfigurationArtifactId,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let artifact = api.get(&format!("/v1/configurations/{id}")).await?;
    let versions = api
        .get(&format!("/v1/configurations/{id}/versions?limit=100"))
        .await?;
    render(
        &json!({ "artifact": artifact, "versions": versions["versions"] }),
        json_output,
    )
}

pub async fn effective(profile: &str, scope: ScopeId, json_output: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    render(
        &api.get(&format!("/v1/configurations/effective?scope_id={scope}"))
            .await?,
        json_output,
    )
}

pub async fn compare(
    profile: &str,
    id: ConfigurationArtifactId,
    from: ConfigurationVersionId,
    to: ConfigurationVersionId,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    render(
        &api.get(&format!(
            "/v1/configurations/{id}/compare?from={from}&to={to}"
        ))
        .await?,
        json_output,
    )
}

pub async fn create(
    profile: &str,
    scope: ScopeId,
    name: &str,
    template: Option<ConfigurationTemplate>,
    file: Option<&Path>,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let (document, source_template) = match (template, file) {
        (Some(template), None) => {
            let listed = api.get("/v1/configuration-templates").await?;
            let document = listed["templates"]
                .as_array()
                .and_then(|templates| {
                    templates
                        .iter()
                        .find(|entry| entry["name"] == template.as_str())
                })
                .and_then(|entry| entry.get("document"))
                .cloned()
                .ok_or_else(|| format!("gateway did not offer template {template}"))?;
            (document, Some(template.as_str()))
        }
        (None, Some(path)) => (document(path)?, None),
        _ => return Err("choose exactly one of --template and --file".to_owned()),
    };
    let body = json!({
        "governing_scope_id": scope,
        "name": name,
        "document": document,
        "source_template": source_template,
    });
    let result: Value = api
        .post_idempotent_as("/v1/configurations", Some(body), &key("create"))
        .await?;
    render(&result, json_output)
}

pub async fn publish(
    profile: &str,
    id: ConfigurationArtifactId,
    expected: ConfigurationVersionId,
    file: &Path,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let result: Value = api
        .post_idempotent_as(
            &format!("/v1/configurations/{id}/versions"),
            Some(json!({
                "expected_current_version_id": expected,
                "document": document(file)?,
            })),
            &key("publish"),
        )
        .await?;
    render(&result, json_output)
}

pub async fn bindings(profile: &str, scope: ScopeId, json_output: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    render(
        &api.get(&format!(
            "/v1/configuration-bindings?scope_id={scope}&limit=100"
        ))
        .await?,
        json_output,
    )
}

pub async fn bind(
    profile: &str,
    scope: ScopeId,
    artifact: ConfigurationArtifactId,
    version: Option<ConfigurationVersionId>,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let result: Value = api
        .post_idempotent_as(
            "/v1/configuration-bindings",
            Some(json!({
                "scope_id": scope,
                "artifact_id": artifact,
                "pinned_version_id": version,
                "enabled": true,
            })),
            &key("bind"),
        )
        .await?;
    render(&result, json_output)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_binding(
    profile: &str,
    id: ConfigurationBindingId,
    expected_revision: u64,
    artifact: ConfigurationArtifactId,
    version: Option<ConfigurationVersionId>,
    enabled: bool,
    reason: &str,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let result = api
        .patch_idempotent(
            &format!("/v1/configuration-bindings/{id}"),
            json!({
                "expected_revision": expected_revision,
                "artifact_id": artifact,
                "pinned_version_id": version,
                "enabled": enabled,
                "reason": reason,
            }),
            &key("binding-update"),
        )
        .await?;
    render(&result, json_output)
}

pub async fn rollback(
    profile: &str,
    id: ConfigurationBindingId,
    expected_revision: u64,
    version: ConfigurationVersionId,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let result: Value = api
        .post_idempotent_as(
            &format!("/v1/configuration-bindings/{id}/rollback"),
            Some(json!({
                "expected_revision": expected_revision,
                "version_id": version,
            })),
            &key("rollback"),
        )
        .await?;
    render(&result, json_output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn configuration_client_has_no_store_authority() {
        let source = include_str!("configuration.rs");
        for forbidden in [
            concat!("synveda_", "store"),
            concat!("sql", "x"),
            concat!("DATABASE", "_URL"),
        ] {
            assert!(
                !source.contains(forbidden),
                "Configuration client contains {forbidden}"
            );
        }
        for path in [
            "/v1/configuration-templates",
            "/v1/configurations",
            "/v1/configuration-bindings",
        ] {
            assert!(source.contains(path), "missing public surface {path}");
        }
    }
}
