//! `synveda mcp` — the generic MCP server (ADPT-2, ADR-0057 as amended).
//!
//! Two tools, `recall` and `remember`, spoken over newline-delimited
//! JSON-RPC on stdio to any MCP client — Claude Desktop and Cursor are the
//! two the acceptance criterion names, and neither has a hook seam.
//!
//! # This is an adapter that happens to live in the CLI binary
//!
//! Seed §7 puts the generic MCP server in the "Harness adapters (thin,
//! stateless)" row, and that row's defining property is the label on the
//! arrow beneath it: *three primitives only*. ADR-0057 decision 1 keeps the
//! layer and gives up only the language — but it puts this module inside a
//! binary that already links `synveda-store`, `synveda-identity`,
//! `synveda-policy` and `synveda-audit` for its dev-bootstrap commands, so
//! the rule has to be stated rather than assumed:
//!
//! **`synveda mcp` is a gateway client.** It reaches the product over `/v1`
//! holding a bearer, exactly as `synveda login` and the CLI's other served
//! verbs do, and it must not call a core crate — not for a shortcut, not in
//! a test. Everything below goes through [`crate::api`]. This is the one
//! property the TypeScript package would have enforced structurally, and it
//! is a review obligation here instead.
//!
//! # Dual-era, and why so little of it is written here
//!
//! ADR-0057 decision 3 asks for `2026-07-28` — `server/discover`,
//! per-request `_meta` version selection, `UnsupportedProtocolVersionError`
//! (`-32022`) carrying the supported list — *and* a legacy `initialize`
//! path for clients that open that way. That surface is the reason decision
//! 2 was amended to `rmcp`: the SDK's request dispatch validates the
//! per-request version, answers `-32022` itself, and serves a first frame
//! that is not `initialize` under the inline lifecycle. What this module
//! owes it is [`ServerHandler::supported_protocol_versions`] — stated
//! rather than inherited, because the SDK's default is "every revision this
//! build knows", which would advertise revisions on a future `rmcp` bump
//! that nobody decided to implement.
//!
//! # The write is advertised by who owns it, not by who is asking
//!
//! `--writes host` advertises `recall` only, because something in the host
//! already writes observations: a hook-driven harness (ADPT-1's `Stop`), or
//! a framework calling us through its own memory interface (ADPT-4's
//! shims). `--writes tool` advertises both, because nothing else writes and
//! the model's tool call is the only path there is.
//!
//! Getting this wrong writes the corpus twice, and the duplicate is
//! semantic rather than byte-identical — a model's composed assertion and
//! the transcript slice containing its reasoning are different payloads, so
//! ADR-0020 decision 2's buffer-level idempotency cannot see it. Hence a
//! capability flag rather than a vendor list: seed §2 principle 6 is law,
//! and a harness nobody has heard of must configure correctly without this
//! ADR being reopened.
//!
//! # Failure posture, inverted from the hooks
//!
//! A hook must never break a session, so it degrades in silence. This
//! caller is a model that asked a question and can read an answer, so a
//! failure is *reported* — as `isError` on a successful result, which is
//! how MCP puts a problem in front of the model rather than the client's
//! error handler. A gateway that is down, a login that has expired, or a
//! denied write come back as a sentence the model can act on. Protocol
//! errors are reserved for the client: MCP clients render them opaquely, so
//! anything the caller should read must not be one.
//!
//! Diagnostics go to stderr, always: stdout is the protocol.

use std::borrow::Cow;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SecondsFormat, Utc};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ResultType,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{Value, json};
use synveda_types::{ObserveKind, RecordId};

use crate::api::Api;

/// The tool names, in one place: `tools/list` advertises them, `tools/call`
/// dispatches on them, and `--writes host` removes one of them.
const RECALL: &str = "recall";
const REMEMBER: &str = "remember";

/// What the server calls itself to a client. Deliberately not the crate
/// name: the binary is `synveda` and the client's config file will say
/// `synveda`, so a third string here would be one more thing to reconcile
/// when someone is reading a client's logs.
const SERVER_NAME: &str = "synveda";

/// The per-event payload cap the observe route enforces
/// (`MAX_EVENT_PAYLOAD_BYTES`, ADR-0020). Checked here as well so an
/// oversized `remember` reads as a sentence rather than a 400 — the number
/// is the gateway's, restated, and the gateway remains the one that decides.
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// What a client should read when nothing has logged in yet. The one
/// failure a model cannot work around and a human can fix in one command.
const SIGN_IN_MESSAGE: &str =
    "Not signed in to Synveda. Run `synveda login` in a terminal, then try again.";

/// Who owns the write at this host (ADR-0057 decision 6).
///
/// Capability-shaped on purpose. The three kinds of host that qualify for
/// `Host` — hook-driven, framework-driven, and any future shape with its
/// own seam — are the same fact from this server's side, and a mode enum
/// carrying vendor names is a vocabulary every new guest has to be added
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Writes {
    /// Nothing else writes, so the tool must: advertise `recall` and
    /// `remember`. Model-driven clients — Claude Desktop, Cursor.
    Tool,
    /// The host already writes observations on its own schedule, so the
    /// tool must not: advertise `recall` only. A harness whose hooks we tap
    /// (ADPT-1), or a framework calling us through its memory interface
    /// (ADPT-4, ADPT-6, ADPT-7).
    Host,
}

/// The server: a credential profile, a session identity for whatever it
/// writes, and the tools this launch advertises.
pub struct Server {
    /// Resolved per call rather than held, so a session that outlives an
    /// access token refreshes instead of failing. An MCP server runs for as
    /// long as its client does, which is longer than any bearer lives.
    profile: String,
    /// One opaque session per server process (ADR-0020's `session_id` is
    /// "an opaque harness session identifier; groups this batch's events").
    /// A launch is the only session boundary this server can observe: it
    /// has no transcript and no harness telling it when a conversation
    /// started.
    session_id: String,
    /// What this launch advertises — decision 6's whole mechanism, decided
    /// once. `tools/list` is per-process, which is what lets `--writes` be
    /// a launch argument and keeps ADR-0027 decision 1's property that this
    /// arrives as configuration rather than restructuring; holding the list
    /// rather than recomputing it per request is that property written down
    /// in the type.
    tools: Vec<Tool>,
}

impl Server {
    /// Builds a server for `profile`, advertising the tools `writes` says
    /// this host needs.
    pub fn new(profile: String, writes: Writes) -> Result<Self, String> {
        Ok(Self {
            profile,
            session_id: format!("mcp-{}", random_token()?),
            tools: match writes {
                Writes::Tool => vec![recall_tool(), remember_tool()],
                Writes::Host => vec![recall_tool()],
            },
        })
    }
}

// ── The tools, as the model sees them ──────────────────────────────────

/// `recall`'s schema is CTX-5's, unchanged (ADR-0057 decision 5): one tool
/// with the route's own `ids` xor `query` shape rather than one tool per
/// shape, so a client that has read the plugin's server has read this one.
///
/// The description is doing real work. An agent that does not know recall
/// reaches *wider* than its session-start block will never reach for it,
/// and one that does not know `as_of` exists cannot ask what was true in
/// March.
fn recall_tool() -> Tool {
    Tool::new(
        RECALL,
        "Search or fetch governed organisational memory. Ask a question with `query` \
         to search every scope your policy lets you read — which is wider than the \
         scopes your session-start context block composes from — or pass `ids` to \
         fetch the full body of records a block named as `(recall <id>)`. \
         Use `as_of` to ask what was known at a past instant. \
         Results carry their channel (published means reviewed, derived means \
         unreviewed), provenance, and validity window, so you can weigh them. \
         What you may read is decided at call time under your own identity: \
         material you have no access to is simply absent from the answer.",
        schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The question to answer. Mutually exclusive with `ids`.",
                },
                "ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description":
                        "Record ids to fetch in full, as an inject block printed them. \
                         Mutually exclusive with `query`.",
                },
                "as_of": {
                    "type": "string",
                    "description":
                        "RFC 3339 instant. Serve bodies as the database held them then \
                         (\"what did we know on 2026-03-03\"). Rewinds the corpus, never \
                         your access.",
                },
                "valid_at": {
                    "type": "string",
                    "description":
                        "RFC 3339 instant. Which assertions were true about the world then. \
                         Defaults to `as_of`.",
                },
                "limit": {
                    "type": "number",
                    "description": "How many records a query may return (1-32, default 32).",
                },
            },
            "additionalProperties": false,
        })),
    )
    .with_title("Recall governed memory")
}

/// `remember`, not `observe` (ADR-0057 decision 7).
///
/// `observe` names the primitive from the platform's side — a batch of
/// session events into a staging buffer. The audience for a tool
/// description is a model deciding whether to call it, and `observe`
/// invites it to narrate the session, which is the hook's job and would
/// flood a personal scope with material the pipeline must score and
/// discard. So the name and the description both say *one durable fact,
/// deliberately*, and the description says plainly where it lands, because
/// a model that thinks this writes to the team will write things it should
/// not.
fn remember_tool() -> Tool {
    Tool::new(
        REMEMBER,
        "Store one durable fact in your own personal memory, so a future session can \
         recall it. Use it for something worth keeping past this conversation — a \
         decision and its reason, a preference, a procedure that worked. Do not narrate \
         the session: this is not a log, and material that is only useful right now \
         makes future recall worse. \
         It writes to your own personal scope and nowhere else — never to a team, a \
         department, or another person — and nothing reaches a shared scope without a \
         human review. Secrets are scanned for and refused or quarantined before \
         anything is stored, so do not pass credentials in the hope of storing them.",
        schema(json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description":
                        "The fact to remember, stated so it still makes sense to a reader \
                         who has none of this conversation's context.",
                },
            },
            "required": ["text"],
            "additionalProperties": false,
        })),
    )
    .with_title("Remember a fact")
}

/// A hand-written JSON Schema, rather than one derived from a Rust type.
///
/// Decision 5 pins `recall`'s schema to CTX-5's exactly, and a derived
/// schema would be that shape only by coincidence — the field descriptions
/// above are the same prose the plugin's server has been serving, and they
/// belong where a reviewer can diff them against it.
fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        // Unreachable: every caller is a literal object above. Stated as a
        // panic rather than a fallible signature because a malformed
        // schema is a build-time mistake, not a runtime condition.
        other => unreachable!("a tool schema must be a JSON object, got {other}"),
    }
}

// ── recall ─────────────────────────────────────────────────────────────

/// The recall wire shapes (`crates/synveda-gateway/src/recall.rs`), as much
/// of them as the rendering below reads.
#[derive(Deserialize)]
struct RecallResponse {
    entries: Vec<RecallEntry>,
    as_of: DateTime<Utc>,
    valid_at: DateTime<Utc>,
    scopes_considered: usize,
    scopes_decided: usize,
    truncated: bool,
    degraded: Vec<String>,
}

#[derive(Deserialize)]
struct RecallEntry {
    record_id: RecordId,
    scope_id: String,
    channel: String,
    class: String,
    sensitivity: String,
    content: String,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    staleness_permille: u16,
}

/// What one `recall` call asked for, after the arguments have been read.
#[derive(Default, Deserialize)]
struct RecallArgs {
    query: Option<String>,
    ids: Option<Vec<String>>,
    as_of: Option<String>,
    valid_at: Option<String>,
    limit: Option<u32>,
}

impl RecallArgs {
    /// The wire body, or the sentence to hand the model instead.
    ///
    /// The xor is checked here as well as at the gateway, on CTX-5's
    /// reasoning: an agent that gets it wrong reads a sentence rather than
    /// a 400. Only what was asked for is sent, so the gateway's defaults
    /// stay the gateway's.
    fn body(self) -> Result<Value, String> {
        let ids = self.ids.unwrap_or_default();
        let query = self.query.filter(|text| !text.trim().is_empty());
        let mut body = serde_json::Map::new();
        match (&query, ids.is_empty()) {
            (Some(_), false) => return Err("Pass either `query` or `ids`, not both.".to_owned()),
            (None, true) => {
                return Err(
                    "Pass a `query` to search, or `ids` to fetch records by name.".to_owned(),
                );
            }
            (Some(query), true) => {
                body.insert("query".to_owned(), json!(query));
            }
            (None, false) => {
                body.insert("ids".to_owned(), json!(ids));
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
        Ok(Value::Object(body))
    }
}

/// `recall` — search or fetch, under the caller's own identity.
async fn recall(server: &Server, args: RecallArgs) -> CallToolResult {
    let body = match args.body() {
        Ok(body) => body,
        Err(message) => return tool_error(message),
    };
    let api = match connect(server).await {
        Ok(api) => api,
        Err(message) => return tool_error(message),
    };
    match api
        .post_as::<RecallResponse>("/v1/recall", Some(body))
        .await
    {
        Ok(response) => CallToolResult::success(vec![ContentBlock::text(render_recall(&response))]),
        Err(message) => tool_error(format!("Recall failed: {message}")),
    }
}

/// The answer as text, in the shape an inject block already uses — trust
/// markers first, then the body — so an agent that has read a block does
/// not have to learn a second format (ADR-0042 decision 15).
fn render_recall(response: &RecallResponse) -> String {
    if response.entries.is_empty() {
        return format!("No memory available to you at {}.", instant(response.as_of));
    }
    let entries: Vec<String> = response.entries.iter().map(render_entry).collect();
    let mut notes = Vec::new();
    // A bounded answer must never read as a complete one (ADR-0042
    // decision 5), so this is stated rather than left to be inferred from
    // a count nobody was given.
    if response.truncated {
        notes.push(format!(
            "Incomplete: {} scopes could have contributed, {} were searched.",
            response.scopes_considered, response.scopes_decided,
        ));
    }
    if !response.degraded.is_empty() {
        notes.push(format!(
            "Degraded ({}): ranked on the lexical leg only.",
            response.degraded.join(", "),
        ));
    }
    // The watermark, so a recall is as citable as a block: the reader can
    // say which versions of which records it was answering from.
    notes.push(format!(
        "Watermark: {} as of {} (valid {}).",
        response
            .entries
            .iter()
            .map(|entry| entry.record_id.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        instant(response.as_of),
        instant(response.valid_at),
    ));
    format!("{}\n\n{}", entries.join("\n\n"), notes.join("\n"))
}

fn render_entry(entry: &RecallEntry) -> String {
    let mut markers = vec![
        entry.class.clone(),
        if entry.channel == "published" {
            "published".to_owned()
        } else {
            "unreviewed".to_owned()
        },
    ];
    // A reader cannot know what they are holding unless they are told, and
    // that does not change because they asked for it by name (ADR-0038
    // decision 11).
    if matches!(entry.sensitivity.as_str(), "confidential" | "restricted") {
        markers.push(entry.sensitivity.clone());
    }
    let validity = match entry.valid_to {
        None => format!("valid from {}", instant(entry.valid_from)),
        Some(end) => format!("valid {}..{}", instant(entry.valid_from), instant(end)),
    };
    format!(
        "[{}] {}\n  (recall {}) scope {} · {validity} · freshness {}‰",
        markers.join("] ["),
        entry.content,
        entry.record_id,
        entry.scope_id,
        entry.staleness_permille,
    )
}

// ── remember ───────────────────────────────────────────────────────────

/// The observe wire shapes (`crates/synveda-gateway/src/observe.rs`), as
/// much of them as the disposition below reads.
#[derive(Deserialize)]
struct ObserveResponse {
    accepted: usize,
    duplicates: usize,
    quarantined: usize,
    denied: usize,
    #[serde(default)]
    events: Vec<EventOutcome>,
}

#[derive(Deserialize)]
struct EventOutcome {
    status: String,
    #[serde(default)]
    redactions: Option<Value>,
}

#[derive(Deserialize)]
struct RememberArgs {
    text: String,
}

/// `remember` — one model-composed fact into the caller's own home scope.
///
/// The route takes **no scope parameter**: the write lands at the caller's
/// own home scope and only there, gated by `MemoryWrite`, the role-free
/// own-home floor every placed principal holds (seed §2.1). A model calling
/// this cannot write into a team, a department, or another person's memory
/// *because the request has nowhere to say so* — which is the only reason
/// ADR-0057 could say yes to a model-callable write at all. What it does
/// risk is epistemic, and [`ObserveKind::Assertion`] is what records that.
async fn remember(server: &Server, args: RememberArgs) -> CallToolResult {
    let text = args.text.trim();
    if text.is_empty() {
        return tool_error("Pass the fact to remember as `text`.".to_owned());
    }
    let payload = json!({ "text": text });
    // The gateway measures the re-serialised payload and refuses over the
    // cap; measured here too so an oversized call reads as a sentence the
    // model can act on rather than as a rejected batch.
    let size = serde_json::to_vec(&payload).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_PAYLOAD_BYTES {
        return tool_error(format!(
            "That is {size} bytes and the per-event cap is {MAX_PAYLOAD_BYTES}. \
             Remember the durable fact rather than the whole passage.",
        ));
    }

    let key = match random_token() {
        Ok(key) => key,
        Err(message) => return tool_error(message),
    };
    let body = json!({
        "session_id": server.session_id,
        "events": [{
            "idempotency_key": key,
            // Decision 8. A fresh key per call rather than a content hash:
            // a model stating the same fact twice is asserting it twice,
            // not retrying, and only the sender can tell those apart.
            "kind": ObserveKind::Assertion,
            "payload": payload,
            "occurred_at": Utc::now(),
        }],
    });

    let api = match connect(server).await {
        Ok(api) => api,
        Err(message) => return tool_error(message),
    };
    match api
        .post_as::<ObserveResponse>("/v1/observe", Some(body))
        .await
    {
        Ok(response) => {
            let (text, is_error) = render_remember(&response);
            if is_error {
                tool_error(text)
            } else {
                CallToolResult::success(vec![ContentBlock::text(text)])
            }
        }
        Err(message) => tool_error(format!("Remember failed: {message}")),
    }
}

/// What became of the write, and whether the model should read it as a
/// failure.
///
/// MEM-2's scan runs between validation and the staging insert, so a clean
/// admission is one of four outcomes and the other three are the model's
/// business: a denied write stored nothing, a quarantined one is waiting on
/// a human, and a duplicate means the fact was already there. Saying
/// "stored" for any of them would be a lie the model then reasons from.
fn render_remember(response: &ObserveResponse) -> (String, bool) {
    let finding = response
        .events
        .first()
        .and_then(|event| event.redactions.as_ref())
        .map(|found| format!(" The secret scan reported: {found}."))
        .unwrap_or_default();
    if response.denied > 0 {
        return (
            format!(
                "Refused: nothing was stored.{finding} \
                 Rewrite it without the material the scan objected to.",
            ),
            true,
        );
    }
    if response.quarantined > 0 {
        return (
            format!("Held for review: it was not stored and a person has to release it.{finding}",),
            true,
        );
    }
    if response.duplicates > 0 && response.accepted == 0 {
        return (
            "Already remembered — this was recorded before.".to_owned(),
            false,
        );
    }
    if response.accepted == 0 {
        // The route acked but claimed nothing: say so rather than invent a
        // disposition the gateway did not report.
        let status = response
            .events
            .first()
            .map_or("no outcome", |event| event.status.as_str());
        return (
            format!("The gateway accepted nothing and reported `{status}`."),
            true,
        );
    }
    (
        "Remembered. It enters extraction now and becomes recallable to you shortly; \
         reaching a shared scope needs a human review."
            .to_owned(),
        false,
    )
}

// ── The protocol surface ───────────────────────────────────────────────

impl ServerHandler for Server {
    /// The revisions this server implements (ADR-0057 decision 3).
    ///
    /// Stated rather than inherited from the SDK: the default is every
    /// revision the linked `rmcp` knows about, which would silently start
    /// advertising a revision on the next bump that nobody decided to
    /// implement. `2026-07-28` is the one this server is written for;
    /// `2025-11-25` and `2025-06-18` are here because the AC's two clients
    /// decide which era is exercised and the legacy `initialize` path is
    /// answered rather than assumed away. A version outside this list is
    /// refused with `-32022` and the list itself, so the client can retry.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2025_06_18,
        ])
    }

    /// Identity and capabilities, for both eras: `initialize` returns this
    /// with the negotiated version substituted, and `server/discover`
    /// derives its answer from it.
    ///
    /// Modern-preferred (decision 3): the version here is the one a client
    /// falls back to, so it names `2026-07-28` rather than whatever the SDK
    /// currently calls `LATEST` — which, at 3.1.0, is still the older one.
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Synveda is governed organisational memory. `recall` answers from every \
                 scope this identity's policy permits, which is wider than what a \
                 session-start context block composes from — reach for it when the answer \
                 might already be known here rather than reasoning from scratch. Results \
                 say whether they were reviewed and how fresh they are; weigh them on that."
                    .to_owned(),
            )
    }

    /// What this launch advertises — decision 6. `remember` is absent under
    /// `--writes host`, so a model on a hook-driven harness cannot store
    /// the same turn its host is already observing.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: self.tools.clone(),
            next_cursor: None,
            meta: None,
            ttl_ms: None,
            cache_scope: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    /// Dispatch, and the second half of decision 6: a `remember` call under
    /// `--writes host` is refused rather than served, because a tool that
    /// is absent from `tools/list` but answers `tools/call` is not absent.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let advertised = self.tools.iter().any(|tool| tool.name == request.name);
        if !advertised {
            // A protocol error, not a tool error: an unknown tool is the
            // client's mistake to handle, not something a model can retry
            // its way out of.
            return Err(McpError::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        }
        let result = match request.name.as_ref() {
            RECALL => match serde_json::from_value(arguments) {
                Ok(args) => recall(self, args).await,
                Err(error) => tool_error(format!("Could not read the arguments: {error}")),
            },
            REMEMBER => match serde_json::from_value(arguments) {
                Ok(args) => remember(self, args).await,
                Err(error) => tool_error(format!("Could not read the arguments: {error}")),
            },
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown tool `{other}`"),
                    None,
                ));
            }
        };
        Ok(result.into())
    }
}

/// `synveda mcp` — serve the two primitives on stdio until the client
/// closes it (ADR-0057 decision 9).
///
/// stdio only, and taking an SDK does not oblige us to serve a second
/// transport: both AC clients launch a subprocess, an HTTP listener holding
/// a live credential on a developer laptop is a new exposure, and the
/// hosted story is ADPT-3's — versioned API, API keys for service
/// identities — rather than something to improvise here.
pub async fn serve(profile: String, writes: Writes) -> Result<(), String> {
    let server = Server::new(profile, writes)?;
    // stderr, because stdout is the protocol. It says which tools this
    // launch advertises, which is the one thing about `--writes` that is
    // invisible from the client side when it is wrong.
    eprintln!(
        "synveda mcp: serving {} on stdio",
        server
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>()
            .join(" and "),
    );
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|err| format!("serve MCP on stdio: {err}"))?;
    running
        .waiting()
        .await
        .map_err(|err| format!("the MCP service ended abnormally: {err}"))?;
    Ok(())
}

// ── Shared plumbing ────────────────────────────────────────────────────

/// A gateway client for this call, with a currently-valid bearer.
///
/// Per call rather than held: this process outlives its access token, and
/// [`Api::connect`] is what refreshes one. The message on failure is the
/// one a human can act on — every other error the model reads is about the
/// request, and this one is about the machine.
async fn connect(server: &Server) -> Result<Api, String> {
    match Api::connect(&server.profile).await {
        Ok((api, _origin)) => Ok(api),
        Err(error) => {
            eprintln!("synveda mcp: {error}");
            Err(SIGN_IN_MESSAGE.to_owned())
        }
    }
}

/// A tool-level failure: `isError` on a successful result, which is how MCP
/// puts a problem in front of the *model* rather than the client's error
/// handler. An agent can read this and try something else.
fn tool_error(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

/// RFC 3339, seconds precision — the format the schema documents for
/// `as_of` and `valid_at`, so what a recall answers with is what a
/// follow-up can ask with.
fn instant(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 128 bits from the system CSPRNG, URL-safe. Session ids and idempotency
/// keys both fit the route's 200-character cap comfortably.
fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| format!("system CSPRNG unavailable: {err}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(writes: Writes) -> Server {
        Server::new("default".to_owned(), writes).expect("a server")
    }

    fn advertised(writes: Writes) -> Vec<String> {
        server(writes)
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    #[test]
    fn writes_host_advertises_recall_only() {
        assert_eq!(
            advertised(Writes::Host),
            [RECALL],
            "a host that already writes must not be offered a write tool"
        );
    }

    #[test]
    fn writes_tool_advertises_both() {
        assert_eq!(advertised(Writes::Tool), [RECALL, REMEMBER]);
    }

    /// ADR-0057 decision 1's one unenforced property, enforced.
    ///
    /// The amendment gave up the TypeScript package, and with it the only
    /// thing that made "three primitives only, over `/v1`" *structural*:
    /// this module now sits in a binary that links every core crate, so
    /// nothing but a reviewer's attention stops a future edit from reaching
    /// past the gateway to the store. That is a review obligation the ADR
    /// records and this test discharges — a shortcut here would leave no
    /// PDP decision in the trail and no audit event under anyone's
    /// identity, which is exactly what seed §2.2 forbids and exactly what
    /// nobody would notice in a diff that otherwise looked like a
    /// performance fix.
    ///
    /// The needles are assembled rather than written, so the guard does not
    /// match its own source. `synveda-types` is deliberately absent from
    /// the list: it is the shared wire vocabulary at the base of the
    /// layering rule, not a service, and naming `ObserveKind::Assertion`
    /// through it is what keeps decision 8's variant from being re-spelled
    /// as a string literal here.
    #[test]
    fn this_adapter_reaches_the_product_only_through_the_gateway() {
        let source = include_str!("mcp.rs");
        for crate_name in [
            concat!("synveda", "_store"),
            concat!("synveda", "_identity"),
            concat!("synveda", "_policy"),
            concat!("synveda", "_audit"),
            concat!("synveda", "_ingest"),
            concat!("synveda", "_retrieval"),
            concat!("sq", "lx"),
        ] {
            assert!(
                !source.contains(crate_name),
                "`synveda mcp` is a gateway client (ADR-0057 decision 1): it must reach \
                 the product over /v1 holding a bearer, never through `{crate_name}` — \
                 not for a shortcut, not in a test",
            );
        }
    }

    /// The half of decision 6 that a `tools/list` test cannot reach: an
    /// unadvertised tool must also be unreachable, or `--writes host`
    /// merely hides the double-write from the listing.
    #[test]
    fn a_tool_this_launch_does_not_advertise_is_not_dispatchable() {
        assert!(server(Writes::Host).get_tool(REMEMBER).is_none());
        assert!(server(Writes::Host).get_tool(RECALL).is_some());
        assert!(server(Writes::Tool).get_tool(REMEMBER).is_some());
    }

    /// Decision 3: the modern revision is implemented and preferred, and
    /// the legacy ones are answered rather than assumed away. Pinned
    /// because an `rmcp` bump changing `KNOWN_VERSIONS` must not change
    /// what this server claims.
    #[test]
    fn both_eras_are_advertised_and_the_modern_one_is_preferred() {
        let server = server(Writes::Tool);
        let supported = server.supported_protocol_versions();
        assert!(supported.contains(&ProtocolVersion::V_2026_07_28));
        assert!(supported.contains(&ProtocolVersion::V_2025_11_25));
        assert!(supported.contains(&ProtocolVersion::V_2025_06_18));
        assert_eq!(
            server.get_info().protocol_version,
            ProtocolVersion::V_2026_07_28,
            "the fallback a client lands on must be the revision this server is written for",
        );
    }

    #[test]
    fn recall_refuses_both_and_neither_before_the_gateway_sees_them() {
        let both = RecallArgs {
            query: Some("what did we decide".to_owned()),
            ids: Some(vec!["0198f000-0000-7000-8000-000000000000".to_owned()]),
            ..RecallArgs::default()
        };
        assert!(both.body().unwrap_err().contains("not both"));
        assert!(
            RecallArgs::default()
                .body()
                .unwrap_err()
                .contains("`ids` to fetch")
        );
    }

    #[test]
    fn recall_sends_only_what_was_asked_for() {
        let body = RecallArgs {
            query: Some("payments".to_owned()),
            ..RecallArgs::default()
        }
        .body()
        .expect("a query is enough");
        let object = body.as_object().expect("an object");
        assert_eq!(
            object.len(),
            1,
            "the gateway's defaults must stay the gateway's: {object:?}"
        );
        assert_eq!(object["query"], json!("payments"));

        let body = RecallArgs {
            ids: Some(vec!["0198f000-0000-7000-8000-000000000000".to_owned()]),
            as_of: Some("2026-03-03T00:00:00Z".to_owned()),
            limit: Some(4),
            ..RecallArgs::default()
        }
        .body()
        .expect("ids are enough");
        assert_eq!(body["ids"], json!(["0198f000-0000-7000-8000-000000000000"]));
        assert_eq!(body["as_of"], json!("2026-03-03T00:00:00Z"));
        assert_eq!(body["limit"], json!(4));
        assert!(body.get("query").is_none());
    }

    /// A blank `query` is not a query. Without this the xor reads it as one
    /// and the gateway refuses a request the model could have been told
    /// about here.
    #[test]
    fn a_blank_query_is_not_a_query() {
        let args = RecallArgs {
            query: Some("   ".to_owned()),
            ..RecallArgs::default()
        };
        assert!(args.body().unwrap_err().contains("`ids` to fetch"));
    }

    /// The four dispositions MEM-2 can produce, and the three of them a
    /// model must not read as "stored".
    #[test]
    fn every_disposition_says_what_actually_happened() {
        let outcome = |accepted, duplicates, quarantined, denied| ObserveResponse {
            accepted,
            duplicates,
            quarantined,
            denied,
            events: vec![EventOutcome {
                status: "admitted".to_owned(),
                redactions: None,
            }],
        };

        let (text, is_error) = render_remember(&outcome(1, 0, 0, 0));
        assert!(!is_error, "{text}");
        assert!(text.starts_with("Remembered"), "{text}");

        let (text, is_error) = render_remember(&outcome(0, 0, 0, 1));
        assert!(is_error, "a denied write stored nothing and must say so");
        assert!(text.contains("nothing was stored"), "{text}");

        let (text, is_error) = render_remember(&outcome(0, 0, 1, 0));
        assert!(is_error, "a quarantined write is not stored yet");
        assert!(text.contains("a person has to release it"), "{text}");

        let (text, is_error) = render_remember(&outcome(0, 1, 0, 0));
        assert!(!is_error, "a duplicate is not a failure");
        assert!(text.contains("Already remembered"), "{text}");

        let (text, is_error) = render_remember(&outcome(0, 0, 0, 0));
        assert!(
            is_error,
            "an ack that claimed nothing must not read as success"
        );
        assert!(text.contains("accepted nothing"), "{text}");
    }

    /// The scan's finding summary is the reason a write was refused, and it
    /// carries rule and category and never matched text (ADR-0021) — so it
    /// is safe to show the model and useless to withhold.
    #[test]
    fn a_refusal_carries_the_scans_reason() {
        let response = ObserveResponse {
            accepted: 0,
            duplicates: 0,
            quarantined: 0,
            denied: 1,
            events: vec![EventOutcome {
                status: "denied".to_owned(),
                redactions: Some(
                    json!([{"rule": "aws-access-key", "category": "secret", "count": 1}]),
                ),
            }],
        };
        let (text, _) = render_remember(&response);
        assert!(text.contains("aws-access-key"), "{text}");
    }

    #[test]
    fn recall_renders_trust_markers_before_the_body() {
        let response = RecallResponse {
            entries: vec![RecallEntry {
                record_id: "0198f000-0000-7000-8000-000000000000"
                    .parse()
                    .expect("a record id"),
                scope_id: "0198f000-0000-7000-8000-000000000001".to_owned(),
                channel: "derived".to_owned(),
                class: "decision".to_owned(),
                sensitivity: "confidential".to_owned(),
                content: "we chose Postgres".to_owned(),
                valid_from: "2026-03-03T00:00:00Z".parse().expect("an instant"),
                valid_to: None,
                staleness_permille: 120,
            }],
            as_of: "2026-08-05T00:00:00Z".parse().expect("an instant"),
            valid_at: "2026-08-05T00:00:00Z".parse().expect("an instant"),
            scopes_considered: 3,
            scopes_decided: 3,
            truncated: false,
            degraded: Vec::new(),
        };
        let rendered = render_recall(&response);
        assert!(
            rendered.starts_with("[decision] [unreviewed] [confidential] we chose Postgres"),
            "{rendered}",
        );
        assert!(
            rendered.contains("(recall 0198f000-0000-7000-8000-000000000000)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Watermark:"),
            "a recall must be as citable as a block"
        );
    }

    /// A bounded answer must never read as a complete one (ADR-0042
    /// decision 5) — including when the boundary is the search's, not the
    /// caller's.
    #[test]
    fn a_truncated_answer_says_it_is_incomplete() {
        let response = RecallResponse {
            entries: Vec::new(),
            as_of: "2026-08-05T00:00:00Z".parse().expect("an instant"),
            valid_at: "2026-08-05T00:00:00Z".parse().expect("an instant"),
            scopes_considered: 40,
            scopes_decided: 8,
            truncated: true,
            degraded: vec!["vector".to_owned()],
        };
        // No entries at all is its own sentence, and it is not an error.
        assert!(render_recall(&response).contains("No memory available to you"));

        let response = RecallResponse {
            entries: vec![RecallEntry {
                record_id: "0198f000-0000-7000-8000-000000000000"
                    .parse()
                    .expect("a record id"),
                scope_id: "0198f000-0000-7000-8000-000000000001".to_owned(),
                channel: "published".to_owned(),
                class: "fact".to_owned(),
                sensitivity: "internal".to_owned(),
                content: "the deploy window is Thursday".to_owned(),
                valid_from: "2026-03-03T00:00:00Z".parse().expect("an instant"),
                valid_to: None,
                staleness_permille: 0,
            }],
            ..response
        };
        let rendered = render_recall(&response);
        assert!(
            rendered.contains("Incomplete: 40 scopes could have contributed"),
            "{rendered}"
        );
        assert!(rendered.contains("Degraded (vector)"), "{rendered}");
    }

    /// Decision 8, at the one point this module decides it: the write tool
    /// writes assertions and nothing else, because a `remember` that
    /// reported `decision` would be indistinguishable from a hook's
    /// observation for the rest of the corpus's life.
    #[test]
    fn the_write_tool_writes_assertions() {
        assert!(ObserveKind::Assertion.is_model_asserted());
        assert_eq!(json!(ObserveKind::Assertion), json!("assertion"));
    }

    /// Two servers must not share a session id, and the id must fit the
    /// route's text-field cap.
    #[test]
    fn each_launch_is_its_own_session() {
        let first = server(Writes::Tool).session_id;
        let second = server(Writes::Tool).session_id;
        assert_ne!(first, second);
        assert!(first.starts_with("mcp-"));
        assert!(first.chars().count() <= 200, "{first}");
    }

    /// Decision 5: the schema a client reads here is the one CTX-5's server
    /// has been serving. The xor lives in the descriptions rather than in
    /// the schema, which is why it is also checked in Rust above.
    #[test]
    fn the_recall_schema_is_the_shipped_one() {
        let tool = recall_tool();
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("properties");
        let mut names: Vec<&String> = properties.keys().collect();
        names.sort();
        assert_eq!(names, ["as_of", "ids", "limit", "query", "valid_at"]);
        assert_eq!(tool.input_schema["additionalProperties"], json!(false));
        assert!(
            tool.input_schema.get("required").is_none(),
            "every field is optional"
        );
    }

    #[test]
    fn the_write_tool_asks_for_exactly_one_thing() {
        let tool = remember_tool();
        assert_eq!(tool.input_schema["required"], json!(["text"]));
        assert_eq!(tool.input_schema["additionalProperties"], json!(false));
        // No scope parameter, and there must never be one: placement
        // decides where a write lands (ADR-0020 decision 4), and a field
        // here would be a second answer to a question the route does not
        // ask.
        assert!(tool.input_schema["properties"].get("scope").is_none());
    }
}
