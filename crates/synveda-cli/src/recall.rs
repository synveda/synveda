//! `synveda recall` — query current governed Knowledge from a terminal
//! (ADR-0084).
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
//! HTTP-only, on the precedent of `synveda proposal`: a query is a governed read whose
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
    #[serde(default)]
    summaries: Vec<KnowledgeSummary>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct KnowledgeSummary {
    id: String,
    knowledge_type: String,
    title: String,
    summary: String,
    sensitivity: String,
    expand_cost_tokens: u32,
    #[serde(default)]
    sources: Vec<KnowledgeSource>,
}

#[derive(Deserialize)]
pub(crate) struct KnowledgeExpandResponse {
    as_of: DateTime<Utc>,
    items: Vec<KnowledgeExpandItem>,
    tokens_returned: u32,
}

#[derive(Deserialize)]
struct KnowledgeExpandItem {
    id: String,
    status: String,
    #[serde(default)]
    knowledge: Option<KnowledgeItem>,
    #[serde(default)]
    sources: Vec<KnowledgeSource>,
    #[serde(default)]
    revised_since_offer: bool,
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
    knowledge_type: String,
    current_revision: KnowledgeRevision,
}

#[derive(Deserialize)]
struct KnowledgeRevision {
    title: String,
    body_markdown: String,
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
    /// `summary` or `full`. Omitted leaves the gateway's default.
    pub depth: Option<&'a str>,
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
    if let Some(depth) = ask.depth {
        body.insert("depth".to_owned(), json!(depth));
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
    if response.items.is_empty() && response.summaries.is_empty() {
        return format!("No current Knowledge available to you at {at}.");
    }
    let mut lines = vec![format!("# Synveda Knowledge (as of {at})")];
    for hit in &response.summaries {
        lines.push(format!("\n## {}", hit.title));
        lines.push(hit.summary.trim().to_owned());
        lines.push(format!(
            "_id {} · {} · {} · full text about {} tokens · source: {}_",
            hit.id,
            hit.knowledge_type,
            hit.sensitivity,
            hit.expand_cost_tokens,
            source_summary(&hit.sources),
        ));
    }
    for item in &response.items {
        render_full_item(&mut lines, &item.knowledge, &item.sources, false);
    }
    lines.push(format!(
        "\n{} item(s); retrieval={}",
        response.items.len() + response.summaries.len(),
        response.retrieval_mode,
    ));
    if response.truncated {
        lines.push("Results stopped at the response size limit; narrow the query.".to_owned());
    }
    if let Some(degradation) = &response.degradation {
        lines.push(format!(
            "Degraded ({degradation}): semantic ranking was unavailable."
        ));
    }
    lines.join("\n")
}

fn source_summary(sources: &[KnowledgeSource]) -> String {
    if sources.is_empty() {
        return "source evidence withheld or unavailable".to_owned();
    }
    sources
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
}

fn render_full_item(
    lines: &mut Vec<String>,
    item: &KnowledgeItem,
    sources: &[KnowledgeSource],
    revised: bool,
) {
    let revision = &item.current_revision;
    lines.push(format!("\n## {}", revision.title));
    lines.push(revision.body_markdown.trim().to_owned());
    let note = if revised {
        " · revised since the manifest offered it"
    } else {
        ""
    };
    lines.push(format!(
        "_id {} · {}{note} · source: {}_",
        item.id,
        item.knowledge_type,
        source_summary(sources),
    ));
}

/// Printed after every `not_found_at_as_of` id alike, so it says nothing about
/// any one of them.
const UNAVAILABLE_NEXT_STEP: &str = "These may have been removed or replaced, or may be \
     outside your access. Continue without them, or use recall to search current Knowledge.";

/// Render an expansion for the generic MCP adapter. A refusal reads the same
/// whatever its cause.
pub(crate) fn render_knowledge_expand(response: &KnowledgeExpandResponse) -> String {
    let at = response.as_of.format("%Y-%m-%d %H:%M:%S");
    let mut lines = vec![format!("# Synveda Knowledge, expanded (as of {at})")];
    for item in &response.items {
        if let Some(knowledge) = item.knowledge.as_ref().filter(|_| item.status == "served") {
            render_full_item(
                &mut lines,
                knowledge,
                &item.sources,
                item.revised_since_offer,
            );
        }
    }
    let with_status = |status: &str| {
        response
            .items
            .iter()
            .filter(|item| item.status == status)
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let unavailable = with_status("not_found_at_as_of");
    if !unavailable.is_empty() {
        lines.push(format!(
            "\nNot available to you now: {unavailable}. {UNAVAILABLE_NEXT_STEP}"
        ));
    }
    let over = with_status("over_budget");
    if !over.is_empty() {
        lines.push(format!(
            "\nOver the response size limit; expand these in a separate call: {over}."
        ));
    }
    lines.push(format!("\n{} tokens returned.", response.tokens_returned));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn response(items: Vec<KnowledgeQueryItem>) -> KnowledgeQueryResponse {
        KnowledgeQueryResponse {
            summaries: Vec::new(),
            truncated: false,
            items,
            as_of: "2026-08-25T00:00:00Z".parse().expect("an instant"),
            retrieval_mode: "lexical".to_owned(),
            degradation: None,
        }
    }

    fn item(id: &str, title: &str, body: &str) -> KnowledgeItem {
        KnowledgeItem {
            id: id.to_owned(),
            knowledge_type: "decision".to_owned(),
            current_revision: KnowledgeRevision {
                title: title.to_owned(),
                body_markdown: body.to_owned(),
            },
        }
    }

    #[test]
    fn recall_renders_the_expandable_id_and_provenance() {
        let rendered = render_knowledge_query(&response(vec![KnowledgeQueryItem {
            knowledge: item("item-1", "Database", "Use Postgres."),
            sources: vec![KnowledgeSource {
                source_type: "repository".to_owned(),
                session_event_id: None,
                locator: Some("docs/architecture.md".to_owned()),
                source_revision: Some("abc123".to_owned()),
            }],
        }]));
        assert!(rendered.contains("Use Postgres."), "{rendered}");
        assert!(rendered.contains("_id item-1 · decision"), "{rendered}");
        assert!(
            rendered.contains("repository:docs/architecture.md@abc123"),
            "{rendered}"
        );
    }

    #[test]
    fn a_summary_hit_names_only_the_id_expand_accepts() {
        let mut summary = response(Vec::new());
        summary.summaries.push(KnowledgeSummary {
            id: "item-7".to_owned(),
            knowledge_type: "convention".to_owned(),
            title: "Retries".to_owned(),
            summary: "Retries reuse the provider event id.".to_owned(),
            sensitivity: "internal".to_owned(),
            expand_cost_tokens: 410,
            sources: Vec::new(),
        });
        let rendered = render_knowledge_query(&summary);
        assert!(
            rendered.contains("_id item-7 · convention · internal · full text about 410 tokens"),
            "{rendered}"
        );
        assert!(!rendered.contains("revision"), "{rendered}");
    }

    #[test]
    fn recall_states_empty_and_degraded_results_honestly() {
        let empty = render_knowledge_query(&response(Vec::new()));
        assert!(empty.contains("No current Knowledge"), "{empty}");

        let mut degraded = response(vec![KnowledgeQueryItem {
            knowledge: item("item-1", "Window", "Thursday."),
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

    fn expanded(items: Vec<KnowledgeExpandItem>) -> KnowledgeExpandResponse {
        KnowledgeExpandResponse {
            as_of: "2026-11-16T00:00:00Z".parse().expect("an instant"),
            items,
            tokens_returned: 12,
        }
    }

    fn served(id: &str, revised: bool) -> KnowledgeExpandItem {
        KnowledgeExpandItem {
            id: id.to_owned(),
            status: "served".to_owned(),
            knowledge: Some(item(id, "Retries", "Reuse the provider event id.")),
            sources: Vec::new(),
            revised_since_offer: revised,
        }
    }

    fn refused(id: &str, status: &str) -> KnowledgeExpandItem {
        KnowledgeExpandItem {
            id: id.to_owned(),
            status: status.to_owned(),
            knowledge: None,
            sources: Vec::new(),
            revised_since_offer: false,
        }
    }

    #[test]
    fn an_expansion_marks_a_revised_item_and_only_that_one() {
        let rendered = render_knowledge_expand(&expanded(vec![
            served("item-1", true),
            served("item-2", false),
        ]));
        assert_eq!(
            rendered
                .matches("revised since the manifest offered it")
                .count(),
            1,
            "{rendered}"
        );
        assert!(
            rendered.contains("_id item-1 · decision · revised since the manifest offered it"),
            "{rendered}"
        );
    }

    #[test]
    fn every_miss_reads_alike_and_names_a_next_step() {
        let one = render_knowledge_expand(&expanded(vec![refused("a", "not_found_at_as_of")]));
        let two = render_knowledge_expand(&expanded(vec![
            refused("a", "not_found_at_as_of"),
            refused("b", "not_found_at_as_of"),
        ]));
        for rendered in [&one, &two] {
            assert!(rendered.contains(UNAVAILABLE_NEXT_STEP), "{rendered}");
            assert_eq!(
                rendered.matches(UNAVAILABLE_NEXT_STEP).count(),
                1,
                "{rendered}"
            );
        }
        assert!(two.contains("Not available to you now: a, b."), "{two}");

        let over = render_knowledge_expand(&expanded(vec![refused("c", "over_budget")]));
        assert!(
            over.contains("expand these in a separate call: c."),
            "{over}"
        );
        assert!(!over.contains(UNAVAILABLE_NEXT_STEP), "{over}");
    }
}
