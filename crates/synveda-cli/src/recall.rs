//! `synveda recall` — the other half of tiered injection (CTX-4,
//! ADR-0041 decision 13).
//!
//! An inject block's index tier ends its lines with `(recall <id>)`. This
//! is what that instruction means: the command an agent or a human runs
//! to turn a name into a body. It is the navigation path the acceptance
//! criterion asks for, available in a live session with nothing to
//! register — CTX-5's MCP tool joins the ADPT-1 plugin manifest later
//! (ADR-0027's own reversal trigger) and calls this same route.
//!
//! HTTP-only, on FLOW-6's precedent: a recall is a governed read whose
//! `MemoryRead` decisions the PDP takes per scope and whose
//! `context.recalled` event the gateway chains under the caller's own
//! identity. A CLI that read the records itself would leave no decision in
//! the trail, so this module opens no database connection and the verb
//! takes no `--database-url`.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use synveda_types::{RecordClass, RecordId, RecordKind, ScopeId, Sensitivity};

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/recall.rs`) ───────────

#[derive(Deserialize)]
struct RecallResponse {
    entries: Vec<RecallEntry>,
    requested: usize,
    as_of: DateTime<Utc>,
}

#[derive(Deserialize)]
struct RecallEntry {
    record_id: RecordId,
    scope_id: ScopeId,
    channel: String,
    kind: RecordKind,
    class: RecordClass,
    sensitivity: Sensitivity,
    content: String,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    object_hash: String,
    staleness_permille: u16,
}

/// `synveda recall <id>...` — the bodies behind the handles.
pub async fn recall(
    profile: &str,
    ids: &[RecordId],
    json_out: bool,
    quiet: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }
    let body = json!({ "ids": ids });
    if json_out {
        println!("{}", api.post("/v1/recall", Some(body)).await?);
        return Ok(());
    }

    let response: RecallResponse = api.post_as("/v1/recall", Some(body)).await?;
    for entry in &response.entries {
        // The trust labels first, then the body — the order an agent
        // should weigh them in, and the same labels the block carried.
        println!(
            "── {} [{}] {} {}{}",
            entry.record_id,
            entry.class,
            entry.channel,
            entry.kind,
            match entry.sensitivity {
                // Marked for the reason ADR-0038 decision 11 gives about
                // the block: a reader cannot know what they are holding
                // unless they are told, and that does not change because
                // they asked for it by name.
                tier if tier > Sensitivity::WORKING => format!(" [{tier}]"),
                _ => String::new(),
            },
        );
        println!(
            "   scope {}  valid {}{}  freshness {}‰  {}",
            entry.scope_id,
            entry.valid_from.format("%Y-%m-%d"),
            entry
                .valid_to
                .map_or_else(String::new, |end| format!("..{}", end.format("%Y-%m-%d"))),
            entry.staleness_permille,
            short(&entry.object_hash),
        );
        println!();
        println!("{}", entry.content);
        println!();
    }

    // The gap is not an error and is not hidden. A handle is a name rather
    // than a capability (ADR-0041 decision 5), so a block composed before
    // a role changed can name records this caller may no longer read —
    // and the honest thing is to say how many, without saying which, since
    // the surface itself answers uniformly (decision 6).
    let served = response.entries.len();
    if served < response.requested {
        println!(
            "{} of {} available to you at {} — the rest are not, or no longer are",
            served,
            response.requested,
            response.as_of.format("%Y-%m-%d %H:%M:%S"),
        );
    } else if served == 0 {
        println!("nothing available to you at {}", response.as_of);
    }
    Ok(())
}

/// Which identity is reading — the `synveda proposal` discipline
/// (ADR-0035): never leave a caller guessing whose access answered.
fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(name) => eprintln!("reading as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("reading as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}
