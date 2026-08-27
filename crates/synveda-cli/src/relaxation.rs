//! Public-API client for governed policy relaxations (CPR-31, ADR-0090).
//!
//! No command has store authority: create, revision and revocation all open
//! typed VedaFlow changes through the gateway.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use synveda_types::{IdentityId, RelaxationId, RelaxationVersionId, ScopeId, Sensitivity};

use crate::api::{Api, Origin};

fn key(kind: &str) -> String {
    format!("relaxation-{kind}-{}", RelaxationId::new())
}

async fn connect(profile: &str) -> Result<Api, String> {
    let (api, origin) = Api::connect(profile).await?;
    match origin {
        Origin::Profile(name) => eprintln!(
            "policy relaxation as {} (profile {name}) · trace {}",
            api.subject,
            api.trace_id()
        ),
        Origin::Environment => eprintln!(
            "policy relaxation as {} (SYNVEDA_TOKEN) · trace {}",
            api.subject,
            api.trace_id()
        ),
    }
    Ok(api)
}

fn render(value: &Value, json_output: bool) -> Result<(), String> {
    if !json_output
        && let (Some(outcome), Some(change)) = (
            value.get("outcome").and_then(Value::as_str),
            value.get("change_id").and_then(Value::as_str),
        )
    {
        println!("{outcome} through VedaFlow change {change}");
        if let Some(id) = value.get("relaxation_id").and_then(Value::as_str) {
            println!("relaxation: {id}");
        }
        if let Some(id) = value.get("version_id").and_then(Value::as_str) {
            println!("version: {id}");
        }
        return Ok(());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn instant(value: &str, label: &str) -> Result<DateTime<Utc>, String> {
    value
        .parse::<DateTime<Utc>>()
        .map_err(|error| format!("{label} must be RFC 3339: {error}"))
}

fn sensitivity(value: &str) -> Result<Sensitivity, String> {
    value
        .parse::<Sensitivity>()
        .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
fn terms(
    subject: IdentityId,
    action: &str,
    max_sensitivity: &str,
    start: &str,
    end: &str,
    reason: &str,
) -> Result<Value, String> {
    Ok(json!({
        "subject_identity_id": subject,
        "action": action,
        "max_sensitivity": sensitivity(max_sensitivity)?,
        "requested_start_at": instant(start, "--start")?,
        "requested_end_at": instant(end, "--end")?,
        "reason": reason,
    }))
}

pub async fn list(
    profile: &str,
    scope: Option<ScopeId>,
    status: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let mut query = vec!["limit=100".to_owned()];
    if let Some(scope) = scope {
        query.push(format!("scope_id={scope}"));
    }
    if let Some(status) = status {
        query.push(format!("status={status}"));
    }
    render(
        &api.get(&format!("/v1/relaxations?{}", query.join("&")))
            .await?,
        json_output,
    )
}

pub async fn show(profile: &str, id: RelaxationId, json_output: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    let current = api.get(&format!("/v1/relaxations/{id}")).await?;
    let versions = api
        .get(&format!("/v1/relaxations/{id}/versions?limit=100"))
        .await?;
    render(
        &json!({"relaxation": current, "versions": versions["versions"]}),
        json_output,
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    profile: &str,
    scope: ScopeId,
    subject: IdentityId,
    action: &str,
    max_sensitivity: &str,
    start: &str,
    end: &str,
    reason: &str,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let mut body = terms(subject, action, max_sensitivity, start, end, reason)?;
    body["target_scope_id"] = json!(scope);
    let result: Value = api
        .post_idempotent_as("/v1/relaxations", Some(body), &key("create"))
        .await?;
    render(&result, json_output)
}

#[allow(clippy::too_many_arguments)]
pub async fn revise(
    profile: &str,
    id: RelaxationId,
    expected: RelaxationVersionId,
    subject: IdentityId,
    action: &str,
    max_sensitivity: &str,
    start: &str,
    end: &str,
    reason: &str,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let mut body = terms(subject, action, max_sensitivity, start, end, reason)?;
    body["expected_current_version_id"] = json!(expected);
    let result = api
        .patch_idempotent(&format!("/v1/relaxations/{id}"), body, &key("revise"))
        .await?;
    render(&result, json_output)
}

pub async fn revoke(
    profile: &str,
    id: RelaxationId,
    expected: RelaxationVersionId,
    reason: &str,
    json_output: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let result: Value = api
        .post_idempotent_as(
            &format!("/v1/relaxations/{id}/revoke"),
            Some(json!({
                "expected_current_version_id": expected,
                "reason": reason,
            })),
            &key("revoke"),
        )
        .await?;
    render(&result, json_output)
}
