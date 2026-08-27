//! The session ledger (CPR-10, ADR-0076): agent runtime activity as
//! first-class, tenant-bound records.
//!
//! Three types, and the relationship between them is the whole design.
//!
//! - A [`Session`] is one run of an agent in a workspace, optionally against
//!   a project — when it started, which client and model ran it, who opened
//!   it, whether it is still open.
//! - A [`SessionEvent`] is one **immutable** thing that happened inside that
//!   run: a message, a tool call, a file change, a command, a skill load, a
//!   context request, an adapter warning. Append-only, ordered, idempotent by
//!   the client's own event id.
//! - A [`ContextRun`] is one act of composing context *for* that session, with
//!   the rendered block it produced.
//!
//! Nothing else stores a transcript. A timeline is a **projection** over these
//! three (CPR-10, ADR-0076 decision 9) — merged and ordered at read time, not
//! a fourth table that has to be kept in step with the first three.
//!
//! ## The governed scope is derived, never submitted
//!
//! A session names a workspace and optionally a project; the governed scope it
//! is decided at is the project's scope when there is a project, and the
//! workspace's when there is not. That is a database fact — composite foreign
//! keys hold it (migration `0044`) — rather than a value a client sends,
//! because a client that could name the scope could name a scope the workspace
//! is not in.
//!
//! Nothing on the wire carries a tenant or an acting principal either
//! (ADR-0076 decision 8). Both come from the verified token, so a client
//! cannot open a session as somebody else or in somebody else's tenant even
//! by accident.
//!
//! There is one runtime write path: `POST /v1/sessions/{id}/events`. The
//! aggregate id is server-owned and is the root for capture, context and
//! audit evidence.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ContextCompletionStatus, ContextRunId, Error, ProjectId, RepositoryId, Result, ScopeId,
    SessionEventId, SessionId, TenantId, TraceRetentionMode, WorkspaceId,
};

/// Longest agent-client name, in characters. Bounded because it is stored,
/// listed, labelled onto metrics and rendered.
pub const MAX_CLIENT_CHARS: usize = 64;

/// Longest free-text label on a session — `client_version`, `agent_name`,
/// `model_name`, `branch`, and the harness-supplied identifiers.
pub const MAX_LABEL_CHARS: usize = 200;

/// Longest task summary.
pub const MAX_TASK_SUMMARY_CHARS: usize = 2_000;

/// Longest end reason (CPR-11, ADR-0077 decision 4).
///
/// Shorter than a task summary on purpose: a reason is a sentence about how a
/// run stopped, and a field long enough for a stack trace would become one.
pub const MAX_END_REASON_CHARS: usize = 500;

/// Largest session `metadata` object, in bytes of its compact JSON encoding.
pub const MAX_METADATA_BYTES: usize = 8_192;

/// Largest session-event `payload`, in bytes of its compact JSON encoding.
///
/// Larger than a session's metadata because this is where a message body, a
/// tool argument list or a diff summary lands, and smaller than the observe
/// plane's body limit because an event is one turn's worth of a transcript
/// rather than a document.
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 65_536;

/// Longest client-supplied event id.
pub const MAX_CLIENT_EVENT_ID_CHARS: usize = 200;

// ── Session status ───────────────────────────────────────────────────────────

/// Where a run is in its life.
///
/// Five values, and the two-phase close is the reason there are five rather
/// than three. An adapter learns that a run is over at a hook that must return
/// quickly, and it usually still has events buffered: `ending` is what it says
/// at that moment — *no new work, I am flushing* — and `ended` is what it says
/// when the flush lands. Collapsing the two would mean either an adapter that
/// blocks its host while it drains, or a session that reads as finished while
/// its last five events are still arriving.
///
/// `abandoned` and `failed` are the two ways a run stops without finishing,
/// and they are separate because they call for different things: an abandoned
/// run is one nobody closed (a killed client, a closed laptop, a headless run
/// that exited), and a failed one is a run that broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Open. The agent may still be working.
    Active,
    /// Closing. No new work; buffered events are still welcome.
    Ending,
    /// Closed, having finished.
    Ended,
    /// Closed without anybody saying so.
    Abandoned,
    /// Closed because something broke.
    Failed,
}

impl SessionStatus {
    /// Every status, in declaration order.
    pub const ALL: &'static [SessionStatus] = &[
        SessionStatus::Active,
        SessionStatus::Ending,
        SessionStatus::Ended,
        SessionStatus::Abandoned,
        SessionStatus::Failed,
    ];

    /// The three a session may be **closed** into — what
    /// `POST /v1/sessions/{id}/end` accepts.
    pub const TERMINAL: &'static [SessionStatus] = &[
        SessionStatus::Ended,
        SessionStatus::Abandoned,
        SessionStatus::Failed,
    ];

    /// Stable wire name, identical to the serde form and to the stored value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Ending => "ending",
            SessionStatus::Ended => "ended",
            SessionStatus::Abandoned => "abandoned",
            SessionStatus::Failed => "failed",
        }
    }

    /// Whether the session is closed — nothing more may be appended to it.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            SessionStatus::Ended | SessionStatus::Abandoned | SessionStatus::Failed
        )
    }

    /// Whether events may still be appended.
    ///
    /// `ending` accepts them and `active` accepts them; the terminal three do
    /// not. That is the whole content of the two-phase close: an adapter that
    /// has said "no new work" can still deliver the work already done.
    #[must_use]
    pub const fn accepts_events(&self) -> bool {
        matches!(self, SessionStatus::Active | SessionStatus::Ending)
    }

    /// Whether `self` may become `next`.
    ///
    /// Forward only: `active` may begin closing or close outright, `ending`
    /// may close, and a closed session never reopens or changes how it closed.
    /// Enforced here **and** by a trigger in migration `0044`, because a rule
    /// that lives only in a function holds only for callers who went through
    /// that function.
    #[must_use]
    pub const fn may_become(&self, next: SessionStatus) -> bool {
        match (self, next) {
            (SessionStatus::Active, SessionStatus::Ending) => true,
            (SessionStatus::Active | SessionStatus::Ending, next) => next.is_terminal(),
            _ => false,
        }
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SessionStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        SessionStatus::ALL
            .iter()
            .copied()
            .find(|status| status.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown session status: {s:?} (one of {})",
                    joined(SessionStatus::ALL.iter().map(SessionStatus::as_str))
                ),
            })
    }
}

// ── Session event types ──────────────────────────────────────────────────────

/// What happened. A **closed** vocabulary, because a timeline that has to
/// render an event kind it has never heard of can only render it as a shrug —
/// and because an open vocabulary makes "what did this agent do" a question
/// answered differently by every adapter.
///
/// Nine families: the run's own lifecycle, the conversation, tools, files,
/// commands, skills, context requests, what the adapter itself could not do,
/// and what a model chose to remember. The last one is not decoration: an adapter that silently dropped half a
/// run is indistinguishable from a quiet run, which is exactly the failure
/// ADPT-8 found by measuring.
///
/// Every variant is renamed to its **dotted** wire name explicitly. A
/// `rename_all` here would produce `message_user` while [`SessionEventType::as_str`]
/// — which is what the API document, the stored column and the audit payload
/// all carry — produces `message.user`, and the two would then be one API that
/// accepts one spelling and answers with another. That is not a hypothetical:
/// it is what the first cut of this type did, and every event-appending test
/// failed on it at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionEventType {
    /// The run began. Optional: opening the session already records that.
    #[serde(rename = "session.started")]
    SessionStarted,
    /// The run reached its own end, as the client saw it.
    #[serde(rename = "session.ended")]
    SessionEnded,
    /// A person said something.
    #[serde(rename = "message.user")]
    MessageUser,
    /// The agent said something.
    #[serde(rename = "message.assistant")]
    MessageAssistant,
    /// A tool was called.
    #[serde(rename = "tool.invoked")]
    ToolInvoked,
    /// A tool answered.
    #[serde(rename = "tool.result")]
    ToolResult,
    /// A file was read.
    #[serde(rename = "file.read")]
    FileRead,
    /// A file was written, created or deleted.
    #[serde(rename = "file.changed")]
    FileChanged,
    /// A shell command ran.
    #[serde(rename = "command.executed")]
    CommandExecuted,
    /// A governed skill was materialised into the client.
    #[serde(rename = "skill.loaded")]
    SkillLoaded,
    /// The agent asked for context — the client-side half of what a
    /// [`ContextRun`] records on the server.
    #[serde(rename = "context.requested")]
    ContextRequested,
    /// The adapter could not do something, and is saying so rather than
    /// dropping it silently.
    #[serde(rename = "adapter.warning")]
    AdapterWarning,
    /// A fact **a model composed and chose to store**, arriving because the
    /// model called a write tool rather than because a hook observed a run.
    ///
    /// The distinction is epistemic and cannot be recovered later (ADR-0057
    /// decision 8): a hook records what happened whether or not the model
    /// thinks to call it, while an assertion is the model's own claim, shaped
    /// by the model, for the recorder. This carried `ObserveKind::Assertion`
    /// until CPR-12 and it is here rather than deleted with the rest of that
    /// vocabulary for exactly the reason decision 8 gives: once a model's
    /// assertion and a host's observation share a name, no later feature can
    /// separate them, and the corpus can never answer "did a person say this
    /// or did a model decide it" about anything written before somebody
    /// noticed.
    ///
    /// It buys provenance, not privilege: the same route, the same
    /// `SessionWrite` decision, the same redaction scan, the same scope.
    #[serde(rename = "memory.asserted")]
    MemoryAsserted,
}

impl SessionEventType {
    /// Every type, in declaration order.
    pub const ALL: &'static [SessionEventType] = &[
        SessionEventType::SessionStarted,
        SessionEventType::SessionEnded,
        SessionEventType::MessageUser,
        SessionEventType::MessageAssistant,
        SessionEventType::ToolInvoked,
        SessionEventType::ToolResult,
        SessionEventType::FileRead,
        SessionEventType::FileChanged,
        SessionEventType::CommandExecuted,
        SessionEventType::SkillLoaded,
        SessionEventType::ContextRequested,
        SessionEventType::AdapterWarning,
        SessionEventType::MemoryAsserted,
    ];

    /// Stable wire name, identical to the serde form and to the stored value.
    ///
    /// Dotted rather than snake_cased: `message.user` reads as a family and a
    /// member, which is what the vocabulary is. The per-variant `serde(rename)`
    /// above is the same list, and
    /// `every_event_type_spells_itself_the_same_way_everywhere` pins that they
    /// agree.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SessionEventType::SessionStarted => "session.started",
            SessionEventType::SessionEnded => "session.ended",
            SessionEventType::MessageUser => "message.user",
            SessionEventType::MessageAssistant => "message.assistant",
            SessionEventType::ToolInvoked => "tool.invoked",
            SessionEventType::ToolResult => "tool.result",
            SessionEventType::FileRead => "file.read",
            SessionEventType::FileChanged => "file.changed",
            SessionEventType::CommandExecuted => "command.executed",
            SessionEventType::SkillLoaded => "skill.loaded",
            SessionEventType::ContextRequested => "context.requested",
            SessionEventType::AdapterWarning => "adapter.warning",
            SessionEventType::MemoryAsserted => "memory.asserted",
        }
    }

    /// Whether an event of this type is eligible for durable capture.
    ///
    /// A distinction the old four-value `ObserveKind` never had to make: all
    /// four of its values carried content somebody said or a tool returned.
    /// Thirteen names include bookkeeping — a run starting, a skill being
    /// materialised, an adapter reporting that it dropped something — and
    /// running an extractor over "session started" is LLM spend that can only
    /// produce noise.
    ///
    /// A capture batch freezes only the types that answer `true` here. The
    /// others are still appended, ordered, visible on the timeline and
    /// auditable: they are part of what happened, but not candidate input.
    #[must_use]
    pub const fn capture_eligible(&self) -> bool {
        match self {
            SessionEventType::MessageUser
            | SessionEventType::MessageAssistant
            | SessionEventType::ToolInvoked
            | SessionEventType::ToolResult
            | SessionEventType::FileChanged
            | SessionEventType::CommandExecuted
            | SessionEventType::MemoryAsserted => true,
            // Structure, not durable content. `file.read` is here rather than above
            // deliberately: that a file was opened is provenance about the
            // run, and a memory saying "the agent read src/main.rs" is the
            // kind of derived statement that fills a corpus without informing
            // anything. What the agent *did* with it lands as a message or a
            // change.
            SessionEventType::SessionStarted
            | SessionEventType::SessionEnded
            | SessionEventType::FileRead
            | SessionEventType::SkillLoaded
            | SessionEventType::ContextRequested
            | SessionEventType::AdapterWarning => false,
        }
    }

    /// The family this type belongs to — what a timeline groups and filters
    /// by, and what a metric is labelled with so the cardinality is eight
    /// rather than twelve.
    #[must_use]
    pub const fn family(&self) -> &'static str {
        match self {
            SessionEventType::SessionStarted | SessionEventType::SessionEnded => "lifecycle",
            SessionEventType::MessageUser | SessionEventType::MessageAssistant => "message",
            SessionEventType::ToolInvoked | SessionEventType::ToolResult => "tool",
            SessionEventType::FileRead | SessionEventType::FileChanged => "file",
            SessionEventType::CommandExecuted => "command",
            SessionEventType::SkillLoaded => "skill",
            SessionEventType::ContextRequested => "context",
            SessionEventType::AdapterWarning => "adapter",
            SessionEventType::MemoryAsserted => "memory",
        }
    }
}

impl fmt::Display for SessionEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SessionEventType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        SessionEventType::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown session event type: {s:?} (one of {})",
                    joined(SessionEventType::ALL.iter().map(SessionEventType::as_str))
                ),
            })
    }
}

/// The event-payload schema version a client declares.
///
/// Every event carries one, and it is the client's statement about the shape
/// of `payload` rather than the server's about the row. This product has
/// exactly one shape today; the field exists because an adapter that ships
/// separately from the gateway will one day have two, and a timeline that
/// cannot tell them apart has to guess.
pub const CURRENT_EVENT_SCHEMA_VERSION: i32 = 1;

// ── The rows ─────────────────────────────────────────────────────────────────

/// One run of an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Stable for the session's whole life, and what every event, context run
    /// and audit entry names.
    pub id: SessionId,
    /// Owning tenant. Resolved from the token, never submitted.
    pub tenant_id: TenantId,
    /// The workspace the run happened in.
    pub workspace_id: WorkspaceId,
    /// The project, when the run was against one.
    pub project_id: Option<ProjectId>,
    /// The governed scope this session is decided at: the project's when there
    /// is a project, the workspace's when there is not.
    ///
    /// **Derived, never submitted.** Composite foreign keys in migration
    /// `0044` hold it equal to the owning subtype's own `scope_id`, so a
    /// session cannot be decided against a scope its workspace is not in.
    pub scope_id: ScopeId,
    /// The token subject that opened it. Text rather than an
    /// [`crate::IdentityId`], for the reason `scope_grants.principal_id` is
    /// (ADR-0072): the PDP's principal is `(tenant, subject)`, and an identity
    /// row is not required for a subject to act.
    pub principal_id: String,
    /// The agent client, as it names itself — `claude-code`, `cursor`, `mcp`.
    ///
    /// **Not a closed vocabulary**, deliberately (seed §2 principle 6: the
    /// harness is a guest, and supporting a new one must never require
    /// touching the core). A CHECK on the *grammar* rather than on the list
    /// keeps it a label instead of free prose.
    pub client_name: String,
    /// The client's own version, when it says one.
    pub client_version: Option<String>,
    /// A stable id for *this installation* of that client, when it has one:
    /// what distinguishes two machines running the same client as the same
    /// person. Opaque to this product.
    pub client_installation_id: Option<String>,
    /// The harness's own identifier for this run.
    ///
    /// Never an identity here and nothing joins on it — it exists so a
    /// stateless hook holding only the harness's id can find the session it
    /// already opened instead of minting a second one. Unique per
    /// `(tenant, principal, client_name)` when present.
    pub external_session_id: Option<String>,
    /// Which agent ran, when the client distinguishes several.
    pub agent_name: Option<String>,
    /// The model, as the client names it.
    pub model_name: Option<String>,
    /// The repository the run was against, when the project has one attached.
    pub repository_id: Option<RepositoryId>,
    /// The branch the run was on.
    pub branch: Option<String>,
    /// What the run is about, in the client's words.
    pub task_summary: Option<String>,
    /// Where the run is in its life.
    pub status: SessionStatus,
    /// When the run began.
    pub started_at: DateTime<Utc>,
    /// When it closed. `Some` exactly when [`SessionStatus::is_terminal`] — a
    /// database CHECK, not a convention.
    pub ended_at: Option<DateTime<Utc>>,
    /// Why it stopped, in the client's words (CPR-11, ADR-0077 decision 4).
    ///
    /// Distinct from [`Session::task_summary`], which is what the run was
    /// *about*: `status` says a run failed, this says the hook timed out. Only
    /// ever set as part of a close — a database CHECK forbids one on an
    /// `active` row — and free text, because the vocabulary belongs to the
    /// harness.
    pub end_reason: Option<String>,
    /// The `occurred_at` of the newest event appended to it, or `None` while
    /// nothing has been. What a listing sorts "recently active" by, and the
    /// one column an event append writes.
    pub last_observed_at: Option<DateTime<Utc>>,
    /// The client's own labelling bag: a JSON object, at most
    /// [`MAX_METADATA_BYTES`] encoded.
    ///
    /// It is **never** copied into an audit payload — the chain records that
    /// metadata was present and how large it was, and nothing else. An agent's
    /// environment is where credentials live, and this is the field a harness
    /// would put an environment in.
    pub metadata: serde_json::Value,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

/// One immutable thing that happened inside a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// The event's identity.
    pub id: SessionEventId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The session it belongs to.
    pub session_id: SessionId,
    /// What happened.
    pub event_type: SessionEventType,
    /// The `payload` shape the client declared.
    pub event_schema_version: i32,
    /// The client's own id for this event. **The idempotency key**: unique per
    /// session, so a redelivered batch appends nothing twice.
    pub client_event_id: String,
    /// Position in the session, assigned by the server on append.
    ///
    /// Server-assigned rather than client-supplied because it is what a
    /// timeline orders by when two events share a millisecond, and a client
    /// that could choose it could interleave itself into somebody else's
    /// ordering. Gapless per session, and monotonic by construction.
    pub sequence: i64,
    /// When the client says it happened.
    pub occurred_at: DateTime<Utc>,
    /// When the gateway received it. The two differ, and both matter: a
    /// buffered adapter delivers an hour late, and only one of these is a
    /// clock this deployment controls.
    pub received_at: DateTime<Utc>,
    /// The event's content, shaped by `event_type` and
    /// `event_schema_version`. A JSON object, at most
    /// [`MAX_EVENT_PAYLOAD_BYTES`] encoded.
    pub payload: serde_json::Value,
    /// BLAKE3-256 of the canonical payload, hex-encoded.
    ///
    /// The server's, computed on append and never the client's to assert:
    /// what it is for is telling two events with one `client_event_id` apart,
    /// and a digest a client supplied could not do that.
    pub payload_hash: String,
    /// The admission scan's finding summary — `[{rule, category, count}]`,
    /// never matched text (CPR-12, ADR-0078 decision 1). `None` when the
    /// payload was clean.
    ///
    /// Immutable provenance rather than state: it is decided once, at
    /// admission, and stays true of this payload forever. Which is why it
    /// lives on an append-only row and the quarantine *review* does not.
    pub redactions: Option<serde_json::Value>,
}

/// One act of composing context for a session.
///
/// Minimal by intent (ADR-0076 decision 7): an identity, what was asked, and
/// the block that came back. Prompt 18 adds the explainability — which scopes
/// were considered, which were denied, why each entry made the cut — **without
/// changing this endpoint**, which is why the endpoint's shape is decided now
/// and its depth later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRun {
    /// The run's identity.
    pub id: ContextRunId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The session it was composed for.
    pub session_id: SessionId,
    /// Workspace derived from the session.
    pub workspace_id: WorkspaceId,
    /// Project derived from the session, when the run is workspace-wide.
    pub project_id: Option<ProjectId>,
    /// The governed scope it was anchored at — the session's.
    pub scope_id: ScopeId,
    /// The token subject that asked.
    pub principal_id: String,
    /// Exact immutable governed configuration, absent only for the built-in
    /// fail-safe used before a tenant publishes its first binding.
    pub configuration_version_id: Option<crate::ConfigurationVersionId>,
    /// Canonical digest of the exact runtime document, including the
    /// built-in fail-safe.
    pub configuration_hash: String,
    /// The task the caller named, when it named one. `None` is the
    /// session-start shape: everything pinned, nothing ranked.
    pub query: Option<String>,
    /// Content-free BLAKE3 digest of the query, when present.
    pub query_hash: Option<String>,
    /// The rendered block, watermark line included. Empty when nothing
    /// composed — which is a result, not an error.
    pub rendered: String,
    /// BLAKE3 over the composed entries, hex-encoded: the block's identity,
    /// the same value the rendered watermark line carries.
    pub block_hash: String,
    /// Estimated tokens of `rendered`.
    pub tokens: i32,
    /// The budget it was composed under.
    pub budget_tokens: i32,
    /// Caller-requested budget before the governed ceiling narrowed it.
    pub requested_budget_tokens: Option<i32>,
    /// How many records composed.
    pub entry_count: i32,
    /// Visible candidates retained for this run.
    pub candidate_count: i32,
    /// Knowledge revisions selected for this run.
    pub selection_count: i32,
    /// The skills this block advertised, as it advertised them (ADR-0054
    /// decision 8): name, scope, commit and object address, so an adapter can
    /// materialise exactly what was named without asking twice.
    ///
    /// Stored rather than recomputed because this endpoint is idempotent — a
    /// replay must serve the body the original served, and a channel that has
    /// moved since would otherwise change the answer.
    pub skills: serde_json::Value,
    /// Which legs degraded, if any — `embedder`, `retrieval`. Empty is the
    /// ordinary answer.
    pub degraded: Vec<String>,
    /// Valid-time instant at which current Knowledge was planned.
    pub as_of: DateTime<Utc>,
    /// Planner implementation version.
    pub retrieval_version: String,
    /// Configured semantic model, absent for lexical-only planning.
    pub embedding_model: Option<String>,
    /// Knowledge index schema/version.
    pub index_version: String,
    /// Graph implementation version, absent until graph expansion runs.
    pub graph_version: Option<String>,
    /// Governed trace-retention mode.
    pub trace_retention: TraceRetentionMode,
    /// Whether planning completed or failed.
    pub completion_status: ContextCompletionStatus,
    /// At least one candidate was filtered by policy. No count is retained.
    pub policy_exclusion: bool,
    /// When it was composed.
    pub created_at: DateTime<Utc>,
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Checks an agent-client name: non-blank, bounded, and made of the characters
/// a label is made of.
///
/// The grammar is the slug grammar one character wider — dots are allowed, so
/// `claude-code`, `zed`, `mcp` and `com.example.agent` are all sayable — and it
/// is a grammar rather than a list because seed §2 principle 6 forbids a core
/// change per harness. What it rules out is a client name carrying whitespace,
/// control characters or a whole sentence into a metric label and an audit
/// payload.
///
/// # Errors
///
/// [`Error::Invalid`] when the name is blank, too long, or outside the
/// grammar.
pub fn validate_client_name(client: &str) -> Result<()> {
    if client.is_empty() {
        return Err(Error::Invalid {
            message: "a session names the agent client that opened it; `client_name` is empty"
                .to_owned(),
        });
    }
    let len = client.chars().count();
    if len > MAX_CLIENT_CHARS {
        return Err(Error::Invalid {
            message: format!("`client_name` is at most {MAX_CLIENT_CHARS} characters, got {len}"),
        });
    }
    let shaped = client
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && client
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    if !shaped {
        return Err(Error::Invalid {
            message: format!(
                "`client_name` is a lowercase label — letters, digits, `-` and `.`, \
                 starting with a letter or digit — got {client:?}"
            ),
        });
    }
    Ok(())
}

/// Checks one optional label: absent, or present, non-blank and bounded.
///
/// Blank is refused rather than normalised to absent, for
/// [`crate::workspace::validate_description`]'s reason: "I sent an empty
/// branch" and "I sent no branch" are different requests, and silently making
/// them the same hides a client bug.
///
/// # Errors
///
/// [`Error::Invalid`] when present and blank, or over `max`.
pub fn validate_label(field: &str, value: Option<&str>, max: usize) -> Result<()> {
    let Some(value) = value else { return Ok(()) };
    if value.trim().is_empty() {
        return Err(Error::Invalid {
            message: format!("`{field}` cannot be blank; omit it instead"),
        });
    }
    let len = value.chars().count();
    if len > max {
        return Err(Error::Invalid {
            message: format!("`{field}` is at most {max} characters, got {len}"),
        });
    }
    Ok(())
}

/// Checks a JSON bag: an object, and no larger than `max` bytes encoded.
///
/// An object rather than any JSON value, because every consumer of these bags
/// reads them by key — a bare array or scalar would be a shape nothing can
/// merge, filter or render, arriving through a field whose whole purpose is
/// being merged, filtered and rendered.
///
/// # Errors
///
/// [`Error::Invalid`] when the value is not an object, or is too large.
pub fn validate_json_object(field: &str, value: &serde_json::Value, max: usize) -> Result<()> {
    if !value.is_object() {
        return Err(Error::Invalid {
            message: format!("`{field}` is a JSON object"),
        });
    }
    let encoded = value.to_string();
    if encoded.len() > max {
        return Err(Error::Invalid {
            message: format!(
                "`{field}` is at most {max} bytes encoded, got {}",
                encoded.len()
            ),
        });
    }
    Ok(())
}

/// Checks the client's own event id: non-blank and bounded.
///
/// # Errors
///
/// [`Error::Invalid`] when blank or too long.
pub fn validate_client_event_id(id: &str) -> Result<()> {
    if id.trim().is_empty() {
        return Err(Error::Invalid {
            message: "every event carries a non-blank `client_event_id`: it is what makes a \
                      redelivered batch append nothing twice"
                .to_owned(),
        });
    }
    let len = id.chars().count();
    if len > MAX_CLIENT_EVENT_ID_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "`client_event_id` is at most {MAX_CLIENT_EVENT_ID_CHARS} characters, got {len}"
            ),
        });
    }
    Ok(())
}

/// `a`, `b` or `c` — for the "one of" clause of a vocabulary error.
fn joined<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_event_types_round_trip_through_the_wire_name() {
        for status in SessionStatus::ALL {
            assert_eq!(status.as_str().parse::<SessionStatus>().unwrap(), *status);
            assert_eq!(
                serde_json::to_string(status).unwrap(),
                format!("\"{}\"", status.as_str())
            );
        }
        assert!("closed".parse::<SessionStatus>().is_err());
        for kind in SessionEventType::ALL {
            assert_eq!(kind.as_str().parse::<SessionEventType>().unwrap(), *kind);
        }
        assert!("message.system".parse::<SessionEventType>().is_err());
    }

    /// One spelling, everywhere.
    ///
    /// `as_str` is what the OpenAPI enum, the stored column and the audit
    /// payload carry; serde is what a request body is parsed with. The first
    /// cut of this type let them diverge — a `rename_all = "snake_case"`
    /// deriving `message_user` beside an `as_str` of `message.user` — which
    /// produced an API that answered with one spelling and refused the other.
    /// Every event-appending test failed at once, which is the good version of
    /// that mistake; this is what makes it fail *here* instead.
    #[test]
    fn every_event_type_spells_itself_the_same_way_everywhere() {
        for kind in SessionEventType::ALL {
            let encoded = serde_json::to_string(kind).unwrap();
            assert_eq!(
                encoded,
                format!("\"{}\"", kind.as_str()),
                "{kind:?}: serde and as_str must agree"
            );
            assert_eq!(
                serde_json::from_str::<SessionEventType>(&encoded).unwrap(),
                *kind
            );
            // And the dotted form is the one the vocabulary is written in.
            assert!(kind.as_str().contains('.'), "{kind:?} is a dotted name");
        }
    }

    #[test]
    fn every_event_type_has_a_family_and_the_families_are_the_nine_named() {
        let mut families: Vec<&str> = SessionEventType::ALL
            .iter()
            .map(SessionEventType::family)
            .collect();
        families.sort_unstable();
        families.dedup();
        assert_eq!(
            families,
            [
                "adapter",
                "command",
                "context",
                "file",
                "lifecycle",
                "memory",
                "message",
                "skill",
                "tool"
            ]
        );
    }

    #[test]
    fn a_closed_session_never_reopens_and_never_changes_how_it_closed() {
        assert!(SessionStatus::Active.may_become(SessionStatus::Ending));
        assert!(SessionStatus::Active.may_become(SessionStatus::Ended));
        assert!(SessionStatus::Active.may_become(SessionStatus::Failed));
        assert!(SessionStatus::Ending.may_become(SessionStatus::Abandoned));
        // Backwards, sideways and to itself: all refused.
        assert!(!SessionStatus::Ending.may_become(SessionStatus::Active));
        assert!(!SessionStatus::Ending.may_become(SessionStatus::Ending));
        assert!(!SessionStatus::Active.may_become(SessionStatus::Active));
        for terminal in SessionStatus::TERMINAL {
            for next in SessionStatus::ALL {
                assert!(
                    !terminal.may_become(*next),
                    "{terminal} must not become {next}"
                );
            }
        }
    }

    #[test]
    fn ending_still_accepts_the_events_that_were_already_buffered() {
        assert!(SessionStatus::Active.accepts_events());
        assert!(SessionStatus::Ending.accepts_events());
        for terminal in SessionStatus::TERMINAL {
            assert!(!terminal.accepts_events(), "{terminal} must refuse events");
            assert!(terminal.is_terminal());
        }
        assert!(!SessionStatus::Ending.is_terminal());
    }

    #[test]
    fn a_client_name_is_a_label_and_never_a_sentence() {
        for good in [
            "claude-code",
            "cursor",
            "zed",
            "mcp",
            "com.example.agent",
            "a",
        ] {
            validate_client_name(good).unwrap_or_else(|err| panic!("{good:?}: {err}"));
        }
        for bad in [
            "",
            "Claude Code",
            "claude code",
            "-leading",
            ".leading",
            "trailing\n",
            "emoji🙂",
            "under_score",
        ] {
            assert!(
                validate_client_name(bad).is_err(),
                "{bad:?} should be refused"
            );
        }
        assert!(validate_client_name(&"a".repeat(MAX_CLIENT_CHARS)).is_ok());
        assert!(validate_client_name(&"a".repeat(MAX_CLIENT_CHARS + 1)).is_err());
    }

    #[test]
    fn a_label_is_absent_or_real() {
        validate_label("branch", None, MAX_LABEL_CHARS).unwrap();
        validate_label("branch", Some("main"), MAX_LABEL_CHARS).unwrap();
        assert!(validate_label("branch", Some(""), MAX_LABEL_CHARS).is_err());
        assert!(validate_label("branch", Some("  \n"), MAX_LABEL_CHARS).is_err());
        assert!(validate_label("branch", Some("xxxxx"), 4).is_err());
    }

    #[test]
    fn a_bag_is_an_object_and_bounded() {
        validate_json_object("metadata", &serde_json::json!({}), MAX_METADATA_BYTES).unwrap();
        validate_json_object(
            "metadata",
            &serde_json::json!({"cwd": "/w"}),
            MAX_METADATA_BYTES,
        )
        .unwrap();
        assert!(
            validate_json_object("metadata", &serde_json::json!([]), MAX_METADATA_BYTES).is_err()
        );
        assert!(
            validate_json_object("metadata", &serde_json::json!(7), MAX_METADATA_BYTES).is_err()
        );
        let big = serde_json::json!({ "k": "x".repeat(MAX_METADATA_BYTES) });
        assert!(validate_json_object("metadata", &big, MAX_METADATA_BYTES).is_err());
    }

    #[test]
    fn an_event_carries_a_client_id_or_it_is_not_idempotent() {
        validate_client_event_id("evt-1").unwrap();
        assert!(validate_client_event_id("").is_err());
        assert!(validate_client_event_id("   ").is_err());
        assert!(validate_client_event_id(&"x".repeat(MAX_CLIENT_EVENT_ID_CHARS + 1)).is_err());
    }

    #[test]
    fn a_session_round_trips_through_json() {
        let session = Session {
            id: SessionId::new(),
            tenant_id: TenantId::new(),
            workspace_id: WorkspaceId::new(),
            project_id: Some(ProjectId::new()),
            scope_id: ScopeId::new(),
            principal_id: "alice@example.com".to_owned(),
            client_name: "claude-code".to_owned(),
            client_version: Some("2.1.0".to_owned()),
            client_installation_id: Some("inst-7".to_owned()),
            external_session_id: Some("6f1e2b90".to_owned()),
            agent_name: Some("reviewer".to_owned()),
            model_name: Some("claude-opus-5".to_owned()),
            repository_id: Some(RepositoryId::new()),
            branch: Some("main".to_owned()),
            task_summary: Some("Refactor the ledger".to_owned()),
            status: SessionStatus::Ended,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            end_reason: Some("hook timed out".to_owned()),
            last_observed_at: Some(Utc::now()),
            metadata: serde_json::json!({"cwd": "/work"}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), session);
    }

    #[test]
    fn an_event_round_trips_through_json() {
        let event = SessionEvent {
            id: SessionEventId::new(),
            tenant_id: TenantId::new(),
            session_id: SessionId::new(),
            event_type: SessionEventType::ToolInvoked,
            event_schema_version: CURRENT_EVENT_SCHEMA_VERSION,
            client_event_id: "evt-9".to_owned(),
            sequence: 9,
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            payload: serde_json::json!({"tool": "grep"}),
            payload_hash: "ab".repeat(32),
            redactions: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<SessionEvent>(&json).unwrap(), event);
    }
}
