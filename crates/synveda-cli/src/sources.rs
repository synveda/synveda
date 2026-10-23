//! Source status and explicit durable refresh through the public API.

use crate::api::Api;
use synveda_types::ProjectId;

/// List governed descriptors and their last runtime receipts.
pub async fn list(profile: &str, project: ProjectId) -> Result<(), String> {
    let (api, _) = Api::connect(profile).await?;
    print(api.get(&format!("/v1/projects/{project}/sources")).await?)
}

/// Inspect one exact configured source without transport work.
pub async fn status(profile: &str, project: ProjectId, source: &str) -> Result<(), String> {
    synveda_types::source::validate_source_id(source).map_err(|e| e.to_string())?;
    let (api, _) = Api::connect(profile).await?;
    print(
        api.get(&format!("/v1/projects/{project}/sources/{source}"))
            .await?,
    )
}

/// Queue a bounded worker read; retry with the same key to inspect its receipt.
pub async fn refresh(
    profile: &str,
    project: ProjectId,
    source: &str,
    key: &str,
) -> Result<(), String> {
    synveda_types::source::validate_source_id(source).map_err(|e| e.to_string())?;
    let (api, _) = Api::connect(profile).await?;
    let value: serde_json::Value = api
        .post_idempotent_as(
            &format!("/v1/projects/{project}/sources/{source}/refresh"),
            None,
            key,
        )
        .await?;
    print(value)
}

fn print(value: serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
