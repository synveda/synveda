//! Public Procedure evidence, fixed qualification and review submission.
use crate::api::Api;
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use synveda_types::{CaptureCandidateId, SessionId};

#[derive(Args)]
pub struct Options {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true)]
    profile: Option<String>,
}
#[derive(Subcommand)]
enum Command {
    /// Record bounded outcome evidence; the gateway assigns its evidence class.
    Outcome {
        session: SessionId,
        #[arg(long)]
        file: PathBuf,
    },
    /// Freeze a small append from exact authenticated outcomes, without a model call.
    Improve {
        session: SessionId,
        #[arg(long)]
        file: PathBuf,
    },
    /// Freeze an exact candidate against a governed independent evaluation suite.
    Evaluate {
        candidate: CaptureCandidateId,
        #[arg(long)]
        candidate_hash: String,
        #[arg(long)]
        suite: String,
    },
    /// Report once against the frozen suite using the admitted evaluator identity.
    Results {
        attempt: uuid::Uuid,
        #[arg(long)]
        file: PathBuf,
    },
    /// Submit a qualified exact edit for the existing independent review workflow.
    Submit {
        candidate: CaptureCandidateId,
        #[arg(long)]
        candidate_hash: String,
        #[arg(long)]
        key: String,
    },
    /// Inspect the exact frozen specification and retained qualification receipt.
    Inspect { attempt: uuid::Uuid },
}
fn file_body(path: &Path) -> Result<Value, String> {
    use std::io::Read;
    let file =
        std::fs::File::open(path).map_err(|_| "Procedure JSON file is unavailable".to_owned())?;
    if !file
        .metadata()
        .map_err(|_| "Procedure file metadata is unavailable".to_owned())?
        .is_file()
    {
        return Err("Procedure input must be a regular JSON file".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Procedure JSON file is unreadable".to_owned())?;
    if bytes.len() > 65536 {
        return Err("Procedure JSON file exceeds 64 KiB".to_owned());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Procedure JSON file is invalid".to_owned())
}
pub async fn run(options: Options) -> Result<(), String> {
    let profile = crate::profile_name(options.profile)?;
    let (api, _) = Api::connect(&profile).await?;
    let receipt = match options.command {
        Command::Outcome { session, file } => {
            api.post(
                &format!("/v1/sessions/{session}/outcomes"),
                Some(file_body(&file)?),
            )
            .await?
        }
        Command::Improve { session, file } => {
            api.post(
                &format!("/v1/sessions/{session}/procedure-improvements"),
                Some(file_body(&file)?),
            )
            .await?
        }
        Command::Evaluate {
            candidate,
            candidate_hash,
            suite,
        } => {
            api.post(
                &format!("/v1/capture-candidates/{candidate}/procedure-evaluations"),
                Some(json!({"candidate_hash":candidate_hash,"suite_id":suite})),
            )
            .await?
        }
        Command::Results { attempt, file } => {
            api.post(
                &format!("/v1/procedure-evaluations/{attempt}/results"),
                Some(file_body(&file)?),
            )
            .await?
        }
        Command::Submit {
            candidate,
            candidate_hash,
            key,
        } => {
            api.post_idempotent_as::<Value>(
                &format!("/v1/capture-candidates/{candidate}/procedure-submit"),
                Some(json!({"candidate_hash":candidate_hash})),
                &key,
            )
            .await?
        }
        Command::Inspect { attempt } => {
            api.get(&format!("/v1/procedure-evaluations/{attempt}"))
                .await?
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?
    );
    Ok(())
}
