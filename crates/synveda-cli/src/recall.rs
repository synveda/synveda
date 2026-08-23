//! `synveda recall` — compose governed context from a terminal (CPR-12,
//! ADR-0078 decision 5).
//!
//! # What this was, and what it is now
//!
//! It was the other half of tiered injection (CTX-4, ADR-0041 decision 13):
//! an inject block's index tier ended its lines with `(recall <id>)` and this
//! turned a name into a body. It also carried CTX-5's bitemporal read —
//! `--as-of` to ask what was known at a past instant.
//!
//! `/v1/recall` is deleted, and **both of those go with it**. The endpoint
//! that replaced it composes a block for a question and takes neither handles
//! nor an instant. That is a real capability loss and it is written here
//! rather than smoothed over: Prompt 18 re-cuts recall over the new model and
//! is where the handle tier and the bitemporal read come back.
//!
//! # Why a terminal command opens a session
//!
//! Every composition names the run it belongs to. So this opens an ephemeral
//! `cli` run, composes into it, and leaves it closed — which means "who asked
//! this deployment for context, and what did they get" is answerable about a
//! person at a terminal exactly as it is about an agent. It costs one extra
//! round trip and buys the property the whole programme is for.
//!
//! HTTP-only, on FLOW-6's precedent: a composition is a governed read whose
//! decisions the PDP takes per scope and whose audit event the gateway chains
//! under the caller's own identity. A CLI that read the records itself would
//! leave no decision in the trail, so this module opens no database connection
//! and the verb takes no `--database-url`.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/sessions.rs`) ─────────

/// As much of `GET /v1/me` as choosing a workspace needs.
#[derive(Deserialize)]
struct MeResponse {
    workspaces: Vec<MeWorkspace>,
}

#[derive(Deserialize)]
struct MeWorkspace {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct OpenedSession {
    id: String,
}

#[derive(Deserialize)]
struct ContextRunResponse {
    rendered: String,
    block_hash: String,
    tokens: i32,
    budget_tokens: i32,
    entry_count: i32,
    #[serde(default)]
    degraded: Vec<String>,
    created_at: DateTime<Utc>,
}

/// What one composition asks for.
pub struct Ask<'a> {
    /// The question. Required — there is no shape that omits it now.
    pub query: &'a str,
    /// The workspace to compose in, when the caller can see more than one.
    pub workspace: Option<&'a str>,
    /// Narrow the block's token budget. Narrowing only: the pack's budget is
    /// the ceiling.
    pub budget_tokens: Option<u32>,
}

/// `synveda recall --query <question>` — compose a block and print it.
pub async fn recall(
    profile: &str,
    ask: Ask<'_>,
    json_out: bool,
    quiet: bool,
) -> Result<(), String> {
    if ask.query.trim().is_empty() {
        return Err("ask a question with --query".to_owned());
    }
    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }

    let workspace = resolve_workspace(&api, ask.workspace).await?;
    let session = open_run(&api, &workspace).await?;

    let mut body = serde_json::Map::new();
    body.insert("query".to_owned(), json!(ask.query));
    if let Some(budget) = ask.budget_tokens {
        body.insert("budget_tokens".to_owned(), json!(budget));
    }
    let path = format!("/v1/sessions/{session}/context-runs");
    // A fresh key per invocation: two identical questions a minute apart are
    // two compositions over a corpus that may have moved, not a retry.
    let key = format!("cli-recall-{}", uuid_like()?);

    if json_out {
        let value: serde_json::Value = api
            .post_idempotent_as(&path, Some(serde_json::Value::Object(body)), &key)
            .await?;
        println!("{value}");
        return Ok(());
    }

    let response: ContextRunResponse = api
        .post_idempotent_as(&path, Some(serde_json::Value::Object(body)), &key)
        .await?;

    let at = response.created_at.format("%Y-%m-%d %H:%M:%S");
    if response.entry_count == 0 || response.rendered.trim().is_empty() {
        println!("nothing available to you at {at}");
        return Ok(());
    }
    println!("{}", response.rendered.trim_end());
    println!();
    println!(
        "{} record(s), {} of {} tokens, block {}",
        response.entry_count,
        response.tokens,
        response.budget_tokens,
        short(&response.block_hash),
    );
    // A degraded answer must never read as a complete one (ADR-0042
    // decision 5), so this is stated rather than left to be inferred.
    if !response.degraded.is_empty() {
        println!(
            "note: degraded ({}) — ranking used the lexical leg only",
            response.degraded.join(", "),
        );
    }
    Ok(())
}

/// The workspace to compose in.
///
/// One is taken; more than one is a question rather than a guess, for the
/// reason `synveda mcp` gives: composing in whichever workspace sorted first
/// would quietly answer from the wrong team's memory.
async fn resolve_workspace(api: &Api, named: Option<&str>) -> Result<String, String> {
    if let Some(id) = named {
        return Ok(id.to_owned());
    }
    let me: MeResponse = api.get_as("/v1/me").await?;
    match me.workspaces.len() {
        0 => Err("you have no workspace yet — create one in the console first".to_owned()),
        1 => Ok(me.workspaces[0].id.clone()),
        _ => {
            let names: Vec<String> = me
                .workspaces
                .iter()
                .map(|workspace| format!("{} ({})", workspace.name, workspace.id))
                .collect();
            Err(format!(
                "you can see {} workspaces — name one with --workspace: {}",
                me.workspaces.len(),
                names.join(", "),
            ))
        }
    }
}

/// Opens the ephemeral run this composition belongs to.
async fn open_run(api: &Api, workspace: &str) -> Result<String, String> {
    let external = uuid_like()?;
    let opened: OpenedSession = api
        .post_idempotent_as(
            "/v1/sessions",
            Some(json!({
                "workspace_id": workspace,
                "client_name": "cli",
                "client_version": env!("CARGO_PKG_VERSION"),
                "external_session_id": external,
                "task_summary": "synveda recall",
            })),
            &format!("cli-recall-open-{external}"),
        )
        .await?;
    Ok(opened.id)
}

/// A random identifier for one invocation.
///
/// Hex from the same CSPRNG `synveda login` uses for its CSRF state, rather
/// than a `uuid` dependency this crate does not otherwise need.
fn uuid_like() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| format!("read random bytes: {err}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
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
