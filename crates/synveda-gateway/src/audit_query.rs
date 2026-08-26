//! The audit query API (AUD-2, ADR-0045): `/v1/audit/*` behind tenant
//! resolution and one PDP action, `AuditRead`.
//!
//! Five routes. `events` is the search the AC asks for — actor, action,
//! outcome, resource, time window, denials included as an ordinary filter
//! value. `disclosures` and `knowledge` are the two questions the feature
//! exists for, one call each. `verify` is the chain check the CLI has had
//! since AUD-1, now reachable by an auditor who holds no `DATABASE_URL`;
//! `export` freezes and pages the complete canonical evidence for offline
//! verification.
//!
//! Three properties hold across all five, and none of them is a filter
//! this module could forget to apply:
//!
//! - **Tenant-complete or refused.** `AuditRead` reaches only
//!   `Resource::Tenant` — the Cedar schema leaves `Scope` out of its
//!   `appliesTo`, so a subtree-scoped audit request is not representable
//!   rather than merely rejected here (ADR-0045 decision 2). A
//!   subtree-bound auditor is denied, and the denial names what it would
//!   take.
//! - **No content.** Every disclosure response carries only stable Knowledge
//!   item ids, immutable revision ids, content hashes and planner reason codes.
//!   Resolving one to content requires an independent `KnowledgeRead` decision
//!   through the current Knowledge API (ADR-0045 decision 6; ADR-0084).
//! - **The answer states its own completeness.** Every response carries
//!   the chain head it was taken against and the seq range it covered, so
//!   a finding can be re-derived by someone who does not trust the auditor
//!   who found it, and a truncated page says so (ADR-0045 decision 9).
//!
//! Reading the chain appends to the chain: an allowed admin-plane read
//! chains its own `authz.decision` (ADR-0019 decision 4), so an audit
//! query shows up in the next audit query. That is a property — "who has
//! been reading the trail" is a question a regulator asks — and it is why
//! the pages are cursor-paginated rather than offset-paginated.

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_audit::{
    AUTHORITY_ACTIONS, AuditAction, ChainVerification, Disclosure, EXPORT_CANONICALIZATION,
    EXPORT_FORMAT, EXPORT_HASH_ALGORITHM, EventFilter, Outcome, StoredEvent,
};
use synveda_policy::{Action, Resource};
use synveda_store::{knowledge as knowledge_store, rls};
use synveda_types::{
    ArtifactFamily, ContextRunId, Error, KnowledgeItemId, KnowledgeRevisionId, Result, SessionId,
};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{commit, tenant_id};
use crate::telemetry::AUDIT_QUERY_OPERATIONS_TOTAL;

/// The page cap; `limit` above it is a 400, not a silent trim — a surface
/// that quietly shrank an audit answer would be the omission decision 2
/// refuses.
const MAX_LIMIT: i64 = 1000;
const DEFAULT_LIMIT: i64 = 100;

/// The authority window's cap. The authority half folds from the start of
/// the chain, so it is bounded separately and reports what it covered.
const AUTHORITY_LIMIT: i64 = 1000;

/// Counts the operation and renders the result — the same outcome taxonomy
/// as every governed plane.
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(AUDIT_QUERY_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// Where the chain stood when the answer was taken, and what the answer
/// covered (ADR-0045 decision 9).
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditFrame)]
pub(crate) struct Frame {
    /// The chain head's sequence number when the query ran.
    head_seq: i64,
    /// The head hash, hex — the value that makes an answer re-derivable.
    head_hash: String,
    /// The lowest seq in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    first_seq: Option<i64>,
    /// The highest seq in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seq: Option<i64>,
    /// Whether the limit cut the answer short.
    truncated: bool,
    /// The cursor to continue from, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<i64>,
}

impl<T> From<&synveda_audit::Page<T>> for Frame {
    fn from(page: &synveda_audit::Page<T>) -> Self {
        Frame {
            head_seq: page.frame.head_seq,
            head_hash: hex(&page.frame.head_hash),
            first_seq: page.first_seq,
            last_seq: page.last_seq,
            truncated: page.truncated(),
            next_cursor: page.next_cursor,
        }
    }
}

/// One chain row as the API renders it.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditEventView)]
pub(crate) struct EventView {
    seq: i64,
    occurred_at: DateTime<Utc>,
    /// How the actor was established (`subject`/`break_glass`/`system`).
    actor_kind: String,
    actor_subject: String,
    action: String,
    resource: String,
    outcome: String,
    payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
    /// This row's hash, hex.
    hash: String,
}

impl From<StoredEvent> for EventView {
    fn from(event: StoredEvent) -> Self {
        EventView {
            seq: event.seq,
            occurred_at: event.occurred_at,
            actor_kind: event.actor_kind,
            actor_subject: event.actor_subject,
            action: event.action,
            resource: event.resource,
            outcome: event.outcome,
            payload: event.payload,
            trace_id: event.trace_id,
            hash: hex(&event.hash),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct EventsParams {
    /// Restrict to one acting subject.
    actor: Option<String>,
    /// Restrict to one dotted action name. Unknown names are a 400 rather
    /// than an empty answer: "no events" and "you spelled it wrong" are
    /// different facts, and only one of them is an audit finding.
    action: Option<String>,
    /// Restrict to one outcome — `allow`, `deny`, `success`, `failure`.
    outcome: Option<String>,
    /// Exact match on the recorded resource string (`"scope <uuid>"`,
    /// `"tenant <uuid>"`). Not parsed: the column is a display string by
    /// AUD-1's specification, and a parse that worked for some actions and
    /// failed for others would silently omit rows.
    resource: Option<String>,
    /// Inclusive lower bound on `occurred_at`.
    from: Option<DateTime<Utc>>,
    /// Exclusive upper bound on `occurred_at`.
    until: Option<DateTime<Utc>>,
    /// Closed governed artifact family carried by `artifact_references`.
    artifact_family: Option<String>,
    /// Stable artifact or binding id. Requires `artifact_family`.
    artifact_id: Option<String>,
    /// Exact immutable version/digest. Requires `artifact_family`.
    artifact_version: Option<String>,
    /// Exact session recorded in an event payload.
    session_id: Option<SessionId>,
    /// Exact context run recorded in an event payload.
    context_run_id: Option<ContextRunId>,
    /// Return events after this seq — the cursor from a previous page.
    after: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditEventsResponse)]
pub(crate) struct EventsResponse {
    events: Vec<EventView>,
    #[serde(flatten)]
    frame: Frame,
}

/// `GET /v1/audit/events` — the search (ADR-0045 decision 3).
#[utoipa::path(
    get,
    path = "/v1/audit/events",
    operation_id = "list_audit_events",
    tag = "audit",
    params(
        ("actor" = Option<String>, Query),
        ("action" = Option<String>, Query),
        ("outcome" = Option<String>, Query),
        ("resource" = Option<String>, Query),
        ("from" = Option<DateTime<Utc>>, Query),
        ("until" = Option<DateTime<Utc>>, Query),
        ("artifact_family" = Option<String>, Query),
        ("artifact_id" = Option<String>, Query),
        ("artifact_version" = Option<String>, Query),
        ("session_id" = Option<String>, Query, format = "uuid"),
        ("context_run_id" = Option<String>, Query, format = "uuid"),
        ("after" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "A cursor page from the tenant audit chain", body = EventsResponse),
        (status = 400, description = "A filter or page bound is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Audit read is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "audit.events", skip_all)]
pub(crate) async fn events(
    State(state): State<AppState>,
    Query(params): Query<EventsParams>,
) -> Response {
    let result = async {
        let limit = limit_of(params.limit)?;
        let after = non_negative_cursor(params.after, "after")?;
        let payload_contains = payload_filter(&params)?;
        let filter = EventFilter {
            actor_subject: params.actor,
            actions: match params.action.as_deref() {
                Some(name) => vec![action_named(name)?],
                None => Vec::new(),
            },
            outcome: match params.outcome.as_deref() {
                Some(name) => Some(outcome_named(name)?),
                None => None,
            },
            resource: params.resource,
            from: params.from,
            until: params.until,
            payload_contains,
        };

        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = gate(&state, &mut tx).await?;
        let page = synveda_audit::search(&mut tx, tenant_id, &filter, after, limit).await?;
        let frame = Frame::from(&page);
        chain_the_read(
            &mut tx,
            "events",
            &authorized,
            json!({
                "count": page.items.len(),
                "after": after,
                "artifact_family": params.artifact_family,
                "artifact_id": params.artifact_id,
                "artifact_version": params.artifact_version,
                "session_id": params.session_id,
                "context_run_id": params.context_run_id,
            }),
        )
        .await?;
        commit(tx).await?;

        Ok(Json(EventsResponse {
            events: page.items.into_iter().map(Into::into).collect(),
            frame,
        }))
    }
    .await;
    respond(&state, "events", result).await
}

/// One disclosure as the API renders it — the shape of what was served,
/// never the substance.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditDisclosureView)]
pub(crate) struct DisclosureView {
    seq: i64,
    occurred_at: DateTime<Utc>,
    actor_kind: String,
    /// Who was served.
    actor_subject: String,
    /// The delivery act that put this revision in a session context.
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    reason_codes: Vec<String>,
}

impl From<Disclosure> for DisclosureView {
    fn from(disclosure: Disclosure) -> Self {
        DisclosureView {
            seq: disclosure.seq,
            occurred_at: disclosure.occurred_at,
            actor_kind: disclosure.actor_kind,
            actor_subject: disclosure.actor_subject,
            action: disclosure.action,
            session_id: disclosure.session_id,
            knowledge_item_id: disclosure.entry.knowledge_item_id,
            knowledge_revision_id: disclosure.entry.knowledge_revision_id,
            content_hash: disclosure.entry.content_hash,
            reason_codes: disclosure.entry.reason_codes,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct DisclosuresParams {
    /// The stable Knowledge item asked about.
    knowledge_item: KnowledgeItemId,
    /// The window's inclusive start. With `until` absent this is a day:
    /// "on date D" is the question, so the default window is 24 hours.
    from: DateTime<Utc>,
    /// The window's exclusive end; defaults to `from` plus a day.
    until: Option<DateTime<Utc>>,
    after: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditDisclosuresResponse)]
pub(crate) struct DisclosuresResponse {
    /// Who the chain records the Knowledge item being **served** to in the window,
    /// with what they got. This is evidence.
    disclosed: Vec<DisclosureView>,
    /// The events that opened and closed authority over the window — role
    /// grants, pack assignments, relaxations, publications, classifications.
    /// These are *inputs*, not a set of principals.
    authority: Vec<EventView>,
    /// Whether the authority half hit its own cap, which is separate from
    /// the disclosure page's.
    authority_truncated: bool,
    /// Why the two lists are not one, in the response rather than only in
    /// the ADR: merging them means deciding, and deciding over
    /// reconstructed inputs is a replay of authority rather than a record
    /// of it (ADR-0045 decision 4).
    note: &'static str,
    #[serde(flatten)]
    frame: Frame,
}

/// The sentence that keeps `disclosed` from being read as `authority` and
/// neither from being read as "everyone who could have seen it".
const DISCLOSURE_NOTE: &str = "`disclosed` is who the chain records being served this Knowledge item in \
     the window. `authority` is what governed its scope over the same \
     window. They are not merged: deciding who *could* have seen it from \
     reconstructed inputs would be a replay of authority rather than the \
     record of it (ADR-0045 decision 4).";

/// `GET /v1/audit/disclosures` — "who could see X on date D", as two lists
/// (ADR-0045 decision 4).
#[utoipa::path(
    get,
    path = "/v1/audit/disclosures",
    operation_id = "list_audit_disclosures",
    tag = "audit",
    params(
        ("knowledge_item" = String, Query, format = "uuid"),
        ("from" = DateTime<Utc>, Query),
        ("until" = Option<DateTime<Utc>>, Query),
        ("after" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "Knowledge disclosure and authority evidence", body = DisclosuresResponse),
        (status = 400, description = "A filter or page bound is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Audit read is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "audit.disclosures", skip_all)]
pub(crate) async fn disclosures(
    State(state): State<AppState>,
    Query(params): Query<DisclosuresParams>,
) -> Response {
    let result = async {
        let limit = limit_of(params.limit)?;
        let until = params
            .until
            .unwrap_or(params.from + chrono::Duration::days(1));
        if until <= params.from {
            return Err(Error::Invalid {
                message: "until must be after from".to_owned(),
            });
        }

        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = gate(&state, &mut tx).await?;

        let page = synveda_audit::disclosures(
            &mut tx,
            tenant_id,
            params.knowledge_item,
            params.from,
            until,
            params.after.unwrap_or(0),
            limit,
        )
        .await?;

        // The authority half: the events that opened and closed authority
        // up to the end of the window. No lower bound — a binding made in
        // January still stood in March, and the chain is the only place
        // that fact lives.
        let authority = synveda_audit::search(
            &mut tx,
            tenant_id,
            &EventFilter {
                actions: AUTHORITY_ACTIONS.to_vec(),
                until: Some(until),
                ..EventFilter::default()
            },
            0,
            AUTHORITY_LIMIT,
        )
        .await?;

        let frame = Frame::from(&page);
        let authority_truncated = authority.truncated();
        chain_the_read(
            &mut tx,
            "disclosures",
            &authorized,
            json!({
                "knowledge_item_id": params.knowledge_item.to_string(),
                "from": params.from,
                "until": until,
                "disclosed": page.items.len(),
                "authority": authority.items.len(),
            }),
        )
        .await?;
        commit(tx).await?;

        Ok(Json(DisclosuresResponse {
            disclosed: page.items.into_iter().map(Into::into).collect(),
            authority: authority.items.into_iter().map(Into::into).collect(),
            authority_truncated,
            note: DISCLOSURE_NOTE,
            frame,
        }))
    }
    .await;
    respond(&state, "disclosures", result).await
}

/// One Knowledge item a subject was last served, with what they got.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditKnownView)]
pub(crate) struct KnownView {
    /// Stable aggregate address when retained. Hashes-only traces deliberately
    /// omit it and remain in `unresolved` as content-free evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    reason_codes: Vec<String>,
    /// The chain position of the last delivery — the evidence.
    seq: i64,
    /// When it was last delivered.
    occurred_at: DateTime<Utc>,
    /// How it arrived that last time.
    action: String,
    /// How many times it was served in the window read.
    occasions: usize,
    /// Immutable revision valid-time start, when retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_from: Option<DateTime<Utc>>,
    /// Immutable revision valid-time end, when bounded and retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_to: Option<DateTime<Utc>>,
    /// Immutable revision transaction time, when retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction_time: Option<DateTime<Utc>>,
    /// `valid`, `outside_valid_time`, `not_known_at` or `unresolved`.
    temporal_status: String,
}

#[derive(Deserialize)]
pub(crate) struct KnowledgeParams {
    /// The subject asked about — a user or a service identity.
    subject: String,
    /// Semantic valid-time instant. Defaults to `as_known_at`.
    valid_at: Option<DateTime<Utc>>,
    /// Transaction-time/delivery cutoff. Defaults to now.
    as_known_at: Option<DateTime<Utc>>,
    /// Resume before this sequence cursor when walking older disclosures.
    before: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditKnowledgeResponse)]
pub(crate) struct KnowledgeResponse {
    subject: String,
    valid_at: DateTime<Utc>,
    as_known_at: DateTime<Utc>,
    /// One row per item, carrying the revision *last* delivered at or
    /// before `as_known_at` and valid at `valid_at`.
    known: Vec<KnownView>,
    /// Delivered revisions that are retained but outside the requested
    /// valid/transaction-time pair. They are evidence, not part of `known`.
    outside_time: Vec<KnownView>,
    /// Hashes-only or erased delivery evidence whose temporal interval can no
    /// longer be resolved. It is not silently counted as known.
    unresolved: Vec<KnownView>,
    /// What this answer is, stated in it: what A was served, not what A
    /// could have asked for (ADR-0045 decision 5).
    note: &'static str,
    #[serde(flatten)]
    frame: Frame,
}

const KNOWLEDGE_NOTE: &str = "What the chain records this subject being served at or before the \
     as-known instant and whose retained immutable revision covers the valid-time \
     instant — not what they were permitted to ask for. `outside_time` and \
     `unresolved` preserve evidence the bitemporal claim cannot include. Content \
     remains behind an independent KnowledgeRead decision (ADR-0084).";

/// `GET /v1/audit/knowledge` — "what did agent A know at time T" (ADR-0045
/// decision 5).
#[utoipa::path(
    get,
    path = "/v1/audit/knowledge",
    operation_id = "get_audit_knowledge",
    tag = "audit",
    params(
        ("subject" = String, Query),
        ("valid_at" = Option<DateTime<Utc>>, Query),
        ("as_known_at" = Option<DateTime<Utc>>, Query),
        ("before" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "Knowledge revisions delivered to a subject", body = KnowledgeResponse),
        (status = 400, description = "The subject or page bound is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Audit read is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "audit.knowledge", skip_all)]
pub(crate) async fn knowledge(
    State(state): State<AppState>,
    Query(params): Query<KnowledgeParams>,
) -> Response {
    let result = async {
        let limit = limit_of(params.limit)?;
        if params.subject.is_empty() {
            return Err(Error::Invalid {
                message: "subject must not be empty".to_owned(),
            });
        }
        let as_known_at = params.as_known_at.unwrap_or_else(Utc::now);
        let valid_at = params.valid_at.unwrap_or(as_known_at);
        let before = params.before.unwrap_or(i64::MAX);
        if before <= 0 {
            return Err(Error::Invalid {
                message: "before must be a positive sequence cursor".to_owned(),
            });
        }

        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = gate(&state, &mut tx).await?;
        let page = synveda_audit::knowledge(
            &mut tx,
            tenant_id,
            &params.subject,
            as_known_at,
            before,
            limit,
        )
        .await?;
        let frame = Frame::from(&page);
        let folded = synveda_audit::fold_knowledge(&page.items);
        let revision_ids: Vec<KnowledgeRevisionId> = folded
            .iter()
            .filter_map(|item| item.entry.knowledge_revision_id.as_deref())
            .filter_map(|id| id.parse().ok())
            .collect();
        let evidence =
            knowledge_store::revision_temporal_evidence(&mut *tx, tenant_id, &revision_ids).await?;
        let mut known = Vec::new();
        let mut outside_time = Vec::new();
        let mut unresolved = Vec::new();
        for item in folded {
            let resolved = item
                .entry
                .knowledge_revision_id
                .as_deref()
                .and_then(|id| id.parse::<KnowledgeRevisionId>().ok())
                .and_then(|id| evidence.iter().find(|entry| entry.revision_id == id));
            let destination = match resolved {
                Some(entry)
                    if item
                        .entry
                        .knowledge_item_id
                        .as_deref()
                        .and_then(|id| id.parse::<KnowledgeItemId>().ok())
                        != Some(entry.knowledge_item_id)
                        || item.entry.content_hash.as_deref()
                            != Some(entry.content_hash.as_str()) =>
                {
                    &mut unresolved
                }
                Some(entry)
                    if entry.transaction_time <= as_known_at
                        && entry.valid_from <= valid_at
                        && entry.valid_to.is_none_or(|until| valid_at < until) =>
                {
                    &mut known
                }
                Some(_) => &mut outside_time,
                None => &mut unresolved,
            };
            destination.push(render_known(item, resolved, as_known_at, valid_at));
        }

        chain_the_read(
            &mut tx,
            "knowledge",
            &authorized,
            json!({
                "subject": params.subject,
                "valid_at": valid_at,
                "as_known_at": as_known_at,
                "before": before,
                "knowledge_items": known.len(),
                "outside_time": outside_time.len(),
                "unresolved": unresolved.len(),
                "disclosures": page.items.len(),
            }),
        )
        .await?;
        commit(tx).await?;

        Ok(Json(KnowledgeResponse {
            subject: params.subject,
            valid_at,
            as_known_at,
            known,
            outside_time,
            unresolved,
            note: KNOWLEDGE_NOTE,
            frame,
        }))
    }
    .await;
    respond(&state, "knowledge", result).await
}

fn render_known(
    item: synveda_audit::Known,
    evidence: Option<&knowledge_store::RevisionTemporalEvidence>,
    as_known_at: DateTime<Utc>,
    valid_at: DateTime<Utc>,
) -> KnownView {
    let temporal_status = match evidence {
        None => "unresolved",
        Some(entry)
            if item
                .entry
                .knowledge_item_id
                .as_deref()
                .and_then(|id| id.parse::<KnowledgeItemId>().ok())
                != Some(entry.knowledge_item_id)
                || item.entry.content_hash.as_deref() != Some(entry.content_hash.as_str()) =>
        {
            "unresolved"
        }
        Some(entry) if entry.transaction_time > as_known_at => "not_known_at",
        Some(entry)
            if entry.valid_from > valid_at
                || entry.valid_to.is_some_and(|until| valid_at >= until) =>
        {
            "outside_valid_time"
        }
        Some(_) => "valid",
    };
    KnownView {
        knowledge_item_id: item.entry.knowledge_item_id,
        knowledge_revision_id: item.entry.knowledge_revision_id,
        content_hash: item.entry.content_hash,
        reason_codes: item.entry.reason_codes,
        seq: item.seq,
        occurred_at: item.occurred_at,
        action: item.action,
        occasions: item.occasions,
        valid_from: evidence.map(|entry| entry.valid_from),
        valid_to: evidence.and_then(|entry| entry.valid_to),
        transaction_time: evidence.map(|entry| entry.transaction_time),
        temporal_status: temporal_status.to_owned(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ExportParams {
    /// Resume after this sequence. The first request uses zero.
    after: Option<i64>,
    /// Frozen snapshot head returned by the first page.
    through: Option<i64>,
    /// Page size, 1..=1000.
    limit: Option<i64>,
}

/// Every canonical event input needed by an offline verifier.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditExportEvent)]
pub(crate) struct ExportEventView {
    seq: i64,
    occurred_at: DateTime<Utc>,
    actor_kind: String,
    actor_subject: String,
    action: String,
    resource: String,
    outcome: String,
    payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
    prev_hash: String,
    hash: String,
}

impl From<StoredEvent> for ExportEventView {
    fn from(event: StoredEvent) -> Self {
        Self {
            seq: event.seq,
            occurred_at: event.occurred_at,
            actor_kind: event.actor_kind,
            actor_subject: event.actor_subject,
            action: event.action,
            resource: event.resource,
            outcome: event.outcome,
            payload: event.payload,
            trace_id: event.trace_id,
            prev_hash: hex(&event.prev_hash),
            hash: hex(&event.hash),
        }
    }
}

/// One page from a frozen deterministic audit-chain prefix.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditExportPage)]
pub(crate) struct ExportResponse {
    format: &'static str,
    hash_algorithm: &'static str,
    canonicalization: &'static str,
    #[schema(value_type = String, format = "uuid")]
    tenant_id: synveda_types::TenantId,
    genesis_hash: String,
    snapshot_seq: i64,
    snapshot_hash: String,
    events: Vec<ExportEventView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seq: Option<i64>,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<i64>,
}

/// `GET /v1/audit/export` — a cursor page from one frozen, offline-verifiable
/// chain prefix (CPR-33, ADR-0092 decisions 4 and 5).
#[utoipa::path(
    get,
    path = "/v1/audit/export",
    operation_id = "export_audit_chain",
    tag = "audit",
    params(
        ("after" = Option<i64>, Query),
        ("through" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "A frozen deterministic chain-export page", body = ExportResponse),
        (status = 400, description = "The cursor, frozen head or page bound is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Audit read is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "audit.export", skip_all)]
pub(crate) async fn export(
    State(state): State<AppState>,
    Query(params): Query<ExportParams>,
) -> Response {
    let result = async {
        let limit = limit_of(params.limit)?;
        let after = non_negative_cursor(params.after, "after")?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = gate(&state, &mut tx).await?;
        let page =
            synveda_audit::export_page(&mut tx, tenant_id, after, params.through, limit).await?;
        let snapshot_seq = page.frame.head_seq;
        let snapshot_hash = hex(&page.frame.head_hash);
        let first_seq = page.first_seq;
        let last_seq = page.last_seq;
        let truncated = page.truncated();
        let next_cursor = page.next_cursor;
        let count = page.items.len();
        let events = page.items.into_iter().map(Into::into).collect();
        chain_the_read(
            &mut tx,
            "export",
            &authorized,
            json!({
                "after": after,
                "through": snapshot_seq,
                "count": count,
                "snapshot_hash": snapshot_hash,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ExportResponse {
            format: EXPORT_FORMAT,
            hash_algorithm: EXPORT_HASH_ALGORITHM,
            canonicalization: EXPORT_CANONICALIZATION,
            tenant_id,
            genesis_hash: hex(&synveda_audit::genesis_hash(tenant_id)),
            snapshot_seq,
            snapshot_hash,
            events,
            first_seq,
            last_seq,
            truncated,
            next_cursor,
        }))
    }
    .await;
    respond(&state, "export", result).await
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = AuditVerifyResponse)]
pub(crate) struct VerifyResponse {
    /// Whether every row recomputes to its stored hash and the head
    /// matches.
    valid: bool,
    /// The number of events checked.
    events: i64,
    /// The chain head after verification.
    head_seq: i64,
    head_hash: String,
    /// The first divergence, when there is one: the seq and why. A broken
    /// chain is a 200 with `valid: false`, not an error — the verification
    /// succeeded; it is the chain that did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    broken_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// `GET /v1/audit/verify` — the chain check, under the same `AuditRead`
/// (ADR-0045 decision 1): it returns a verdict and a sequence number and
/// no event content, so a principal who may read the chain may check it.
#[utoipa::path(
    get,
    path = "/v1/audit/verify",
    operation_id = "verify_audit_chain",
    tag = "audit",
    responses(
        (status = 200, description = "Audit chain verification result", body = VerifyResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Audit read is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "audit.verify", skip_all)]
pub(crate) async fn verify(State(state): State<AppState>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let authorized = gate(&state, &mut tx).await?;
        let report = synveda_audit::verify_report(&mut tx, tenant_id).await?;

        let response = match report.verification {
            ChainVerification::Valid { events } => VerifyResponse {
                valid: true,
                events,
                head_seq: report.head_seq,
                head_hash: hex(&report.head_hash),
                broken_at: None,
                reason: None,
            },
            ChainVerification::Broken { seq, reason } => VerifyResponse {
                valid: false,
                events: report.head_seq,
                head_seq: report.head_seq,
                head_hash: hex(&report.head_hash),
                broken_at: Some(seq),
                reason: Some(reason.to_string()),
            },
        };
        chain_the_read(
            &mut tx,
            "verify",
            &authorized,
            json!({"valid": response.valid, "events": response.events}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(response))
    }
    .await;
    respond(&state, "verify", result).await
}

/// The one decision every route in this module takes: `AuditRead` at the
/// tenant. There is no scope-resource variant — the schema has none — so a
/// subtree-bound auditor is denied here with the requirement named rather
/// than served a partial chain (ADR-0045 decision 2).
async fn gate(state: &AppState, tx: &mut sqlx::PgConnection) -> Result<authz::Authorized> {
    let tenant = tenant_id()?;
    authz::require(state, tx, Action::AuditRead, Resource::Tenant(tenant), None).await
}

/// An allowed admin-plane read chains its decision (ADR-0019 decision 4) —
/// including this one, which is why an audit query appears in the next
/// audit query's results.
async fn chain_the_read(
    tx: &mut sqlx::PgConnection,
    op: &'static str,
    authorized: &authz::Authorized,
    mut detail: Value,
) -> Result<()> {
    let tenant = tenant_id()?;
    if let Some(object) = detail.as_object_mut() {
        object.insert("op".to_owned(), json!(op));
        object.insert(
            "authz".to_owned(),
            audit::decision_context(Action::AuditRead, authorized),
        );
    }
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        Resource::Tenant(tenant).to_string(),
        Outcome::Allow,
        detail,
    )
    .await
    .map(|_| ())
}

/// Validate a page limit. Over the cap is a 400: an audit surface must not
/// quietly return less than it was asked for.
fn limit_of(limit: Option<i64>) -> Result<i64> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be 1..={MAX_LIMIT}"),
        });
    }
    Ok(limit)
}

fn non_negative_cursor(cursor: Option<i64>, name: &str) -> Result<i64> {
    let cursor = cursor.unwrap_or(0);
    if cursor < 0 {
        return Err(Error::Invalid {
            message: format!("{name} must be non-negative"),
        });
    }
    Ok(cursor)
}

/// Build one exact JSON-containment predicate from validated structured
/// filters. Artifact fields occupy one object in the reference array, which
/// prevents a family on one reference and an id on another from satisfying
/// the same query.
fn payload_filter(params: &EventsParams) -> Result<Option<Value>> {
    if params.artifact_family.is_none()
        && (params.artifact_id.is_some() || params.artifact_version.is_some())
    {
        return Err(Error::Invalid {
            message: "artifact_id and artifact_version require artifact_family".to_owned(),
        });
    }
    let mut payload = serde_json::Map::new();
    if let Some(name) = params.artifact_family.as_deref() {
        let family: ArtifactFamily = name.parse()?;
        let mut reference = serde_json::Map::new();
        reference.insert("family".to_owned(), json!(family.as_str()));
        if let Some(id) = params.artifact_id.as_deref() {
            bounded_filter("artifact_id", id, 1_024)?;
            reference.insert("artifact_id".to_owned(), json!(id));
        }
        if let Some(version) = params.artifact_version.as_deref() {
            bounded_filter("artifact_version", version, 512)?;
            reference.insert("version".to_owned(), json!(version));
        }
        payload.insert(
            "artifact_references".to_owned(),
            Value::Array(vec![Value::Object(reference)]),
        );
    }
    if let Some(session_id) = params.session_id {
        payload.insert("session_id".to_owned(), json!(session_id));
    }
    if let Some(context_run_id) = params.context_run_id {
        payload.insert("context_run_id".to_owned(), json!(context_run_id));
    }
    Ok((!payload.is_empty()).then_some(Value::Object(payload)))
}

fn bounded_filter(name: &str, value: &str, max: usize) -> Result<()> {
    let length = value.chars().count();
    if length == 0 || length > max || value.chars().any(char::is_control) {
        return Err(Error::Invalid {
            message: format!("{name} must contain 1..={max} non-control characters"),
        });
    }
    Ok(())
}

/// Resolve a dotted action name against the closed in-process vocabulary.
///
/// An unknown name is a 400 rather than an empty result set: the `action`
/// column is open text so later features can add actions without schema
/// churn, but a *query* for one that does not exist is a typo, and
/// answering it with "no events" would let a misspelling read as a finding.
fn action_named(name: &str) -> Result<AuditAction> {
    AuditAction::ALL
        .iter()
        .copied()
        .find(|action| action.as_str() == name)
        .ok_or_else(|| Error::Invalid {
            message: format!("unknown action: {name:?}"),
        })
}

fn outcome_named(name: &str) -> Result<Outcome> {
    match name {
        "allow" => Ok(Outcome::Allow),
        "deny" => Ok(Outcome::Deny),
        "success" => Ok(Outcome::Success),
        "failure" => Ok(Outcome::Failure),
        other => Err(Error::Invalid {
            message: format!("unknown outcome: {other:?} (allow|deny|success|failure)"),
        }),
    }
}

/// Lowercase hex, matching the CLI's rendering of the same bytes.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_name_resolves_and_round_trips() {
        for action in AuditAction::ALL {
            let resolved = action_named(action.as_str()).expect("the name resolves");
            assert_eq!(resolved.as_str(), action.as_str());
        }
    }

    #[test]
    fn the_action_list_has_no_duplicates() {
        // A duplicate would mean the enum grew and the paste landed twice,
        // which the length check alone would not catch.
        let mut names: Vec<&str> = AuditAction::ALL.iter().map(|a| a.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            total,
            "duplicate action name in AuditAction::ALL"
        );
    }

    #[test]
    fn an_unknown_action_or_outcome_is_rejected_rather_than_answered_emptily() {
        assert!(action_named("memory.exfiltrated").is_err());
        assert!(action_named("").is_err());
        assert!(outcome_named("allowed").is_err());
        assert!(outcome_named("deny").is_ok());
    }

    #[test]
    fn a_limit_outside_the_range_is_refused_rather_than_trimmed() {
        assert_eq!(limit_of(None).expect("default"), DEFAULT_LIMIT);
        assert_eq!(limit_of(Some(1)).expect("one"), 1);
        assert_eq!(limit_of(Some(MAX_LIMIT)).expect("cap"), MAX_LIMIT);
        assert!(limit_of(Some(0)).is_err());
        assert!(limit_of(Some(-1)).is_err());
        assert!(
            limit_of(Some(MAX_LIMIT + 1)).is_err(),
            "over the cap must be a 400, never a silent trim"
        );
    }
}
