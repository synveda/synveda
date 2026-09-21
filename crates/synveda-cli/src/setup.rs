//! Public-API project selection plus a local observation choice.

use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_types::{ProjectId, WorkspaceId};

use crate::{api::Api, local_state};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Observation {
    On,
    #[default]
    Off,
}

#[derive(Args)]
pub(crate) struct Options {
    /// Verify and select this existing project; without it, list visible projects.
    #[arg(long)]
    pub project: Option<ProjectId>,
    /// Choose whether hooks may record this project's transcript/tool events.
    #[arg(long, value_enum, default_value_t = Observation::Off, requires = "project")]
    pub observation: Observation,
    #[arg(long)]
    pub profile: Option<String>,
    /// Verify the public API and show the selection without editing project configuration.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    pub profile: String,
    pub project: ProjectId,
    pub workspace: WorkspaceId,
    pub observation: Observation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    root: PathBuf,
    selection: Selection,
    previous: Value,
}

pub(crate) async fn run(options: Options) -> Result<(), String> {
    let profile = crate::profile_name(options.profile)?;
    let (api, _) = Api::connect(&profile).await?;
    let Some(project_id) = options.project else {
        let me = api.get("/v1/me").await?;
        let projects = me["projects"]
            .as_array()
            .ok_or("gateway omitted its project inventory")?;
        for project in projects {
            println!(
                "{}  {}",
                project["id"].as_str().ok_or("invalid project identity")?,
                project["display_name"].as_str().unwrap_or("")
            );
        }
        println!(
            "Select one with `synveda setup --project ID --observation off|on`. Create a workspace/project through Getting started in the console if needed."
        );
        return Ok(());
    };
    let root = crate::plugin::project_root()?;
    let config_path = config_path(&root)?;
    let before = local_state::read(&config_path)?;
    let mut config = parse_config(before.as_deref())?;
    let path = receipt_path(&root)?;
    let old: Option<Receipt> = local_state::read_json(&path)?;
    if let Some(old) = &old {
        validate_receipt(old, &root)?;
        let current = owned_fields(&config);
        if current != fields(&old.selection) && current != old.previous {
            return Err("project selection changed outside setup; preserve the config and receipt for inspection".to_owned());
        }
    }
    // Both reads pass through the gateway's normal PDP and tenant boundaries.
    // Never infer visibility from an id or from a local receipt.
    let project = api.get(&format!("/v1/projects/{project_id}")).await?;
    if project["id"] != json!(project_id) || project["status"] != "active" {
        return Err("setup requires the exact active project".to_owned());
    }
    let workspace: WorkspaceId = serde_json::from_value(project["workspace_id"].clone())
        .map_err(|_| "gateway returned no workspace identity")?;
    let parent = api.get(&format!("/v1/workspaces/{workspace}")).await?;
    if parent["id"] != json!(workspace) || parent["status"] != "active" {
        return Err("setup requires the project's active workspace".to_owned());
    }
    let selection = Selection {
        profile,
        project: project_id,
        workspace,
        observation: options.observation,
    };
    let desired = fields(&selection);
    if old.is_none()
        && [
            "workspace_id",
            "project_id",
            "observe",
            "managed_observation",
        ]
        .iter()
        .any(|key| config.get(*key).is_some_and(|v| v != &desired[*key]))
    {
        return Err("existing project configuration conflicts with this selection; review it before setup; no fields were changed".to_owned());
    }
    println!(
        "Project {} in workspace {}; observation {:?}; profile {}",
        selection.project, selection.workspace, selection.observation, selection.profile
    );
    if options.dry_run {
        println!("Would update {}", config_path.display());
        return Ok(());
    }
    let receipt = Receipt {
        version: 1,
        root: root.clone(),
        selection,
        previous: owned_fields(&config),
    };
    // Intent precedes the edit; an interrupted retry recognizes either side.
    local_state::save(&path, &receipt)?;
    for key in [
        "workspace_id",
        "project_id",
        "observe",
        "managed_observation",
    ] {
        config[key] = desired[key].clone();
    }
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    local_state::replace(&config_path, before.as_deref(), &bytes, false)?;
    println!(
        "Saved project selection. Launch hooks with SYNVEDA_PROFILE={} (and the same XDG_CONFIG_HOME if configured). Review each harness's normal trust prompts.",
        receipt.selection.profile
    );
    Ok(())
}

pub(crate) fn selection(root: &Path) -> Result<Selection, String> {
    let receipt: Receipt = local_state::read_json(&receipt_path(root)?)?
        .ok_or("run `synveda setup --project ID --observation off|on` in this repository first")?;
    validate_receipt(&receipt, root)?;
    let config = parse_config(local_state::read(&config_path(root)?)?.as_deref())?;
    if owned_fields(&config) != fields(&receipt.selection) {
        return Err("setup receipt does not match the current project configuration; rerun or repair setup before adapter registration".to_owned());
    }
    Ok(receipt.selection)
}

fn validate_receipt(receipt: &Receipt, root: &Path) -> Result<(), String> {
    if receipt.version != 1 || receipt.root != root {
        return Err("setup receipt identity differs".to_owned());
    }
    Ok(())
}

fn receipt_path(root: &Path) -> Result<PathBuf, String> {
    local_state::receipt(
        "setup",
        root.to_str().ok_or("repository path must be UTF-8")?,
    )
}

fn config_path(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join(".synveda");
    if let Ok(metadata) = std::fs::symlink_metadata(&dir)
        && (!metadata.is_dir() || metadata.is_symlink())
    {
        return Err(".synveda must be a real directory inside this repository".to_owned());
    }
    Ok(dir.join("config.json"))
}

fn parse_config(bytes: Option<&[u8]>) -> Result<Value, String> {
    let config = match bytes {
        None => json!({}),
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|_| "project config is invalid JSON; nothing was replaced")?,
    };
    if !config.is_object() {
        return Err("project config must be an object".to_owned());
    }
    Ok(config)
}

fn fields(selection: &Selection) -> Value {
    json!({"workspace_id":selection.workspace, "project_id":selection.project, "observe":selection.observation == Observation::On, "managed_observation":true})
}

fn owned_fields(config: &Value) -> Value {
    let mut fields = json!({});
    for key in [
        "workspace_id",
        "project_id",
        "observe",
        "managed_observation",
    ] {
        if let Some(value) = config.get(key) {
            fields[key] = value.clone();
        }
    }
    fields
}
