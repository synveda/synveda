//! FLOW-8: product export uses the public API; offline Git verification has
//! no database or bearer dependency.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use synveda_types::ScopeId;

use crate::api::Api;

#[derive(Subcommand)]
pub enum Command {
    /// Export or resume one channel to a governed local or private GitHub target.
    Export {
        scope: ScopeId,
        #[arg(long)]
        target: String,
        /// prompt/published, prompt/staged, context-pack/published or context-pack/staged.
        #[arg(long)]
        channel: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Independently verify an exported bare repository or ordinary clone.
    Verify {
        repository: PathBuf,
        #[arg(long)]
        channel: String,
        /// JSON object mapping key IDs to independently trusted 32-byte arrays.
        #[arg(long)]
        keys: Option<PathBuf>,
    },
}

fn selector(channel: &str) -> Result<(&str, &str), String> {
    match channel.split_once('/') {
        Some((asset @ ("prompt" | "context-pack"), channel @ ("published" | "staged"))) => Ok((asset, channel)),
        _ => Err("--channel must name prompt/published, prompt/staged, context-pack/published or context-pack/staged".into()),
    }
}

pub async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Export {
            scope,
            target,
            channel,
            profile,
        } => {
            synveda_git::validate_target(&target).map_err(|error| error.to_string())?;
            let (asset, channel) = selector(&channel)?;
            let profile = crate::profile_name(profile)?;
            let (api, _) = Api::connect(&profile).await?;
            let response = api
                .post(
                    &format!("/v1/channels/{scope}/git-export"),
                    Some(json!({ "target": target, "asset": asset, "channel": channel })),
                )
                .await?;
            println!("{response}");
            Ok(())
        }
        Command::Verify {
            repository,
            channel,
            keys,
        } => {
            let (asset, channel) = selector(&channel)?;
            let mut trusted: BTreeMap<String, [u8; 32]> = BTreeMap::new();
            if let Some(path) = keys {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(|_| "cannot open trusted public-key file")?
                    .take(65_537)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "cannot read trusted public-key file")?;
                if bytes.len() > 65_536 {
                    return Err("trusted public-key file exceeds its bound".into());
                }
                trusted = serde_json::from_slice(&bytes)
                    .map_err(|_| "trusted key file must map key IDs to 32-byte arrays")?;
            }
            let manifest = synveda_git::verify_repository(
                &repository,
                &format!("refs/heads/synveda/{asset}/{channel}"),
                &trusted,
            )
            .await
            .map_err(|error| error.to_string())?;
            println!(
                "{}",
                json!({ "valid": true, "format": manifest.format, "source_head": manifest.snapshot.head, "mapping_digest": manifest.mapping_digest, "commits": manifest.commits.len(), "objects": manifest.objects.len() })
            );
            Ok(())
        }
    }
}
