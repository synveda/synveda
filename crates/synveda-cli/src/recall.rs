//! `synveda recall` — query current governed Knowledge from a terminal
//! (CPR-20, ADR-0084).
//!
//! The command uses the session-scoped Knowledge query: current active
//! immutable revisions, independently authorised provenance and honest
//! lexical/semantic degradation. Context delivery remains a separately
//! budgeted ContextRun; a deep query never pretends to be rendered context.
//!
//! # Why a terminal command opens a session
//!
//! Every query names the run it belongs to. So this opens an ephemeral `cli`
//! run and queries through it — which means "who asked this deployment for
//! Knowledge, and what did they get" is answerable about a person at a
//! terminal exactly as it is about an agent.
//!
//! HTTP-only, on FLOW-6's precedent: a query is a governed read whose
//! decisions the PDP takes per scope and whose audit event the gateway chains
//! under the caller's own identity. A CLI that read the database itself would
//! leave no decision in the trail, so this module opens no database connection
//! and the verb takes no `--database-url`.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::api::{Api, Origin};

// ── The public query wire shapes (`context_api.rs`) ─────────────────────

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
pub(crate) struct KnowledgeQueryResponse {
    items: Vec<KnowledgeQueryItem>,
    as_of: DateTime<Utc>,
    retrieval_mode: String,
    #[serde(default)]
    degradation: Option<String>,
}

#[derive(Deserialize)]
struct KnowledgeQueryItem {
    knowledge: KnowledgeItem,
    #[serde(default)]
    sources: Vec<KnowledgeSource>,
}

#[derive(Deserialize)]
struct KnowledgeItem {
    id: String,
    scope_id: String,
    knowledge_type: String,
    current_revision: KnowledgeRevision,
}

#[derive(Deserialize)]
struct KnowledgeRevision {
    id: String,
    title: String,
    body_markdown: String,
    content_hash: String,
}

#[derive(Deserialize)]
struct KnowledgeSource {
    source_type: String,
    session_event_id: Option<String>,
    locator: Option<String>,
    source_revision: Option<String>,
}

/// What one composition asks for.
pub struct Ask<'a> {
    /// The question. Required — there is no shape that omits it now.
    pub query: &'a str,
    /// The workspace to compose in, when the caller can see more than one.
    pub workspace: Option<&'a str>,
    /// Bound the number of current Knowledge results (1–100).
    pub limit: Option<u32>,
}

/// `synveda recall --query <question>` — query current Knowledge and print it.
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
    if let Some(limit) = ask.limit {
        body.insert("limit".to_owned(), json!(limit));
    }
    let path = format!("/v1/sessions/{session}/knowledge-query");

    if json_out {
        let value: serde_json::Value = api
            .post_as(&path, Some(serde_json::Value::Object(body)))
            .await?;
        println!("{value}");
        return Ok(());
    }

    let response: KnowledgeQueryResponse = api
        .post_as(&path, Some(serde_json::Value::Object(body)))
        .await?;
    println!("{}", render_knowledge_query(&response));
    Ok(())
}

/// Render a query result for both the terminal and generic MCP adapter.
pub(crate) fn render_knowledge_query(response: &KnowledgeQueryResponse) -> String {
    let at = response.as_of.format("%Y-%m-%d %H:%M:%S");
    if response.items.is_empty() {
        return format!("No current Knowledge available to you at {at}.");
    }
    let mut lines = vec![format!("# Synveda Knowledge (as of {at})")];
    for item in &response.items {
        let revision = &item.knowledge.current_revision;
        lines.push(format!("\n## {}", revision.title));
        lines.push(revision.body_markdown.trim().to_owned());
        let sources = if item.sources.is_empty() {
            "source evidence withheld or unavailable".to_owned()
        } else {
            item.sources
                .iter()
                .map(|source| {
                    let address = source
                        .session_event_id
                        .as_deref()
                        .or(source.locator.as_deref())
                        .unwrap_or("unaddressed");
                    match source.source_revision.as_deref() {
                        Some(version) => format!("{}:{address}@{version}", source.source_type),
                        None => format!("{}:{address}", source.source_type),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(format!(
            "_Knowledge {} revision {}; type={}; scope={}; content={}; source={}_",
            item.knowledge.id,
            revision.id,
            item.knowledge.knowledge_type,
            item.knowledge.scope_id,
            short(&revision.content_hash),
            sources,
        ));
    }
    lines.push(format!(
        "\n{} item(s); retrieval={}",
        response.items.len(),
        response.retrieval_mode,
    ));
    if let Some(degradation) = &response.degradation {
        lines.push(format!(
            "Degraded ({degradation}): semantic ranking was unavailable."
        ));
    }
    lines.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn response(items: Vec<KnowledgeQueryItem>) -> KnowledgeQueryResponse {
        KnowledgeQueryResponse {
            items,
            as_of: "2026-08-25T00:00:00Z".parse().expect("an instant"),
            retrieval_mode: "lexical".to_owned(),
            degradation: None,
        }
    }

    #[test]
    fn recall_renders_exact_revision_and_provenance() {
        let rendered = render_knowledge_query(&response(vec![KnowledgeQueryItem {
            knowledge: KnowledgeItem {
                id: "item-1".to_owned(),
                scope_id: "scope-1".to_owned(),
                knowledge_type: "decision".to_owned(),
                current_revision: KnowledgeRevision {
                    id: "revision-2".to_owned(),
                    title: "Database".to_owned(),
                    body_markdown: "Use Postgres.".to_owned(),
                    content_hash: "0123456789abcdef".to_owned(),
                },
            },
            sources: vec![KnowledgeSource {
                source_type: "repository".to_owned(),
                session_event_id: None,
                locator: Some("docs/architecture.md".to_owned()),
                source_revision: Some("abc123".to_owned()),
            }],
        }]));
        assert!(rendered.contains("Use Postgres."), "{rendered}");
        assert!(
            rendered.contains("item-1 revision revision-2"),
            "{rendered}"
        );
        assert!(
            rendered.contains("repository:docs/architecture.md@abc123"),
            "{rendered}"
        );
    }

    #[test]
    fn recall_states_empty_and_degraded_results_honestly() {
        let empty = render_knowledge_query(&response(Vec::new()));
        assert!(empty.contains("No current Knowledge"), "{empty}");

        let mut degraded = response(vec![KnowledgeQueryItem {
            knowledge: KnowledgeItem {
                id: "item-1".to_owned(),
                scope_id: "scope-1".to_owned(),
                knowledge_type: "fact".to_owned(),
                current_revision: KnowledgeRevision {
                    id: "revision-1".to_owned(),
                    title: "Window".to_owned(),
                    body_markdown: "Thursday.".to_owned(),
                    content_hash: "fedcba9876543210".to_owned(),
                },
            },
            sources: Vec::new(),
        }]);
        degraded.degradation = Some("semantic_index_not_ready".to_owned());
        let rendered = render_knowledge_query(&degraded);
        assert!(rendered.contains("Thursday."), "{rendered}");
        assert!(
            rendered.contains("Degraded (semantic_index_not_ready)"),
            "{rendered}"
        );
    }
}
