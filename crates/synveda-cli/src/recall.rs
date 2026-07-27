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
    mode: String,
    requested: usize,
    as_of: DateTime<Utc>,
    scopes_considered: usize,
    scopes_decided: usize,
    truncated: bool,
    degraded: Vec<String>,
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

/// What one recall asks for: named records, or a question, at an instant.
pub struct Ask<'a> {
    /// The handles, when naming records.
    pub ids: &'a [RecordId],
    /// The question, when asking one. Exclusive with `ids` — clap
    /// enforces it at the surface, the gateway enforces it again.
    pub query: Option<&'a str>,
    /// Transaction time (CTX-5, ADR-0042 decision 7).
    pub as_of: Option<DateTime<Utc>>,
    /// Valid time; defaults to `as_of` at the gateway.
    pub valid_at: Option<DateTime<Utc>>,
    /// Result cap for a question.
    pub limit: Option<usize>,
}

impl Ask<'_> {
    /// The wire body. Only what was asked for is sent, so the gateway's
    /// defaults stay the gateway's — a CLI that filled them in would be a
    /// second place the surface's contract lives.
    fn body(&self) -> Result<serde_json::Value, String> {
        let mut body = serde_json::Map::new();
        match self.query {
            Some(query) => {
                body.insert("query".to_owned(), json!(query));
            }
            None => {
                if self.ids.is_empty() {
                    return Err(
                        "name at least one record id, or ask a question with --query".to_owned(),
                    );
                }
                body.insert("ids".to_owned(), json!(self.ids));
            }
        }
        if let Some(at) = self.as_of {
            body.insert("as_of".to_owned(), json!(at));
        }
        if let Some(at) = self.valid_at {
            body.insert("valid_at".to_owned(), json!(at));
        }
        if let Some(limit) = self.limit {
            body.insert("limit".to_owned(), json!(limit));
        }
        Ok(serde_json::Value::Object(body))
    }
}

/// `synveda recall <id>... | --query <question>` — the bodies behind the
/// handles, or the answer to a question (CTX-4/CTX-5).
pub async fn recall(
    profile: &str,
    ask: Ask<'_>,
    json_out: bool,
    quiet: bool,
) -> Result<(), String> {
    let body = ask.body()?;
    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }
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
    let at = response.as_of.format("%Y-%m-%d %H:%M:%S");
    if served == 0 {
        println!("nothing available to you at {at}");
    } else if response.mode == "ids" && served < response.requested {
        println!(
            "{served} of {} available to you at {at} — the rest are not, or no longer are",
            response.requested,
        );
    } else if response.mode == "query" {
        println!(
            "{served} of {} scopes you may read at {at}",
            response.scopes_decided,
        );
    }
    // A bounded answer must never read as a complete one (ADR-0042
    // decision 5), so this is stated rather than left to be inferred from
    // a count nobody was given.
    if response.truncated {
        println!(
            "note: {} scopes could have contributed and {} were searched — \
             this answer is incomplete",
            response.scopes_considered, response.scopes_decided,
        );
    }
    if !response.degraded.is_empty() {
        println!(
            "note: degraded ({}) — ranking used the lexical leg only",
            response.degraded.join(", "),
        );
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
