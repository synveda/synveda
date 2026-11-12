//! Re-authorised expansion of named Knowledge (ADR-0163 decisions 3-5 and 9).
//!
//! No tier trusts an earlier one: a manifest entry grants nothing, and every
//! expansion repeats the Knowledge read decision under current policy.

use std::collections::HashSet;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_policy::Action;
use synveda_retrieval::TokenCounter;
use synveda_store::knowledge::{self, KnowledgeSnapshot};
use synveda_store::{rls, sessions};
use synveda_types::knowledge::KnowledgeSource;
use synveda_types::session::{ContextRun, SessionEventType};
use synveda_types::{
    ContextRunId, Error, KnowledgeItemId, KnowledgeRevisionId, Result, ScopeId, Sensitivity,
    SessionId, TenantId, TraceRetentionMode,
};
use utoipa::ToSchema;

use super::{
    KnowledgeItemView, KnowledgeSourceView, PlannedCandidate, PlannedPayload, ScoreSeed,
    current_active_at, graph, planned_score, query_scope_ids, rank_and_deduplicate, respond,
    visible_sources_with_policy,
};
use crate::app::AppState;
use crate::audit;
use crate::request::{body as request_body, commit, tenant_id};
use crate::workspaces::ApiErrorBody;

const MAX_EXPAND_IDS: usize = 32;
const MAX_EXPAND_BYTES: usize = 512 * 1024;
const MAX_REHYDRATION_EVENTS: i64 = 256;
const MAX_REHYDRATED_ITEMS: usize = 32;
const EVENT_ID_PREFIX: &str = "knowledge-expanded:";

/// Which revision an expansion is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeExpandResolve {
    /// Serve only what the named manifest's instant saw as the head.
    Pinned,
    /// Serve the current head.
    Current,
}

impl KnowledgeExpandResolve {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::Current => "current",
        }
    }
}

/// `POST /v1/sessions/{session_id}/knowledge-expand`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeExpandBody {
    /// Knowledge item ids to read, 1-32, unique.
    #[schema(value_type = Vec<String>)]
    pub ids: Vec<KnowledgeItemId>,
    /// The ContextRun whose manifest named these ids. Pass it only for ids
    /// taken from that manifest.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub manifest_id: Option<ContextRunId>,
    /// `pinned` (default with a manifest) or `current`.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub resolve: Option<KnowledgeExpandResolve>,
}

/// One requested id's outcome.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeExpandItemView {
    /// The requested id.
    #[schema(value_type = String, format = "uuid")]
    pub id: KnowledgeItemId,
    /// `served`, `not_found_at_as_of` or `over_budget`. Every refusal is the
    /// same `not_found_at_as_of`, whatever the cause.
    pub status: String,
    /// The full current item, only when served.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<KnowledgeItemView>,
    /// Independently authorised provenance, only when served.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<KnowledgeSourceView>,
    /// Tokens of the served body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
}

impl KnowledgeExpandItemView {
    fn refused(id: KnowledgeItemId) -> Self {
        Self::withheld(id, "not_found_at_as_of")
    }

    fn over_budget(id: KnowledgeItemId) -> Self {
        Self::withheld(id, "over_budget")
    }

    fn withheld(id: KnowledgeItemId, status: &str) -> Self {
        Self {
            id,
            status: status.to_owned(),
            knowledge: None,
            sources: Vec::new(),
            tokens: None,
        }
    }
}

/// Expansion result, in request order.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KnowledgeExpandView {
    /// The named manifest, when one was named.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub manifest_id: Option<ContextRunId>,
    /// `pinned` or `current`.
    pub resolve: String,
    /// Valid-time instant the current heads were checked at.
    pub as_of: DateTime<Utc>,
    /// One outcome per requested id.
    pub items: Vec<KnowledgeExpandItemView>,
    /// Tokens across every served body.
    pub tokens_returned: u32,
    /// `estimated` or `exact_encoding`.
    pub token_count_kind: String,
}

fn validate_ids(ids: &[KnowledgeItemId]) -> Result<()> {
    if ids.is_empty() || ids.len() > MAX_EXPAND_IDS {
        return Err(Error::Invalid {
            message: format!("`ids` holds 1-{MAX_EXPAND_IDS} Knowledge item ids"),
        });
    }
    let mut seen = HashSet::with_capacity(ids.len());
    if !ids.iter().all(|id| seen.insert(*id)) {
        return Err(Error::Invalid {
            message: "`ids` must be unique".to_owned(),
        });
    }
    Ok(())
}

fn resolve_mode(
    manifest: Option<ContextRunId>,
    requested: Option<KnowledgeExpandResolve>,
) -> Result<KnowledgeExpandResolve> {
    match (manifest, requested) {
        (None, Some(KnowledgeExpandResolve::Pinned)) => Err(Error::Invalid {
            message: "`resolve: pinned` requires `manifest_id`".to_owned(),
        }),
        (_, Some(mode)) => Ok(mode),
        (Some(_), None) => Ok(KnowledgeExpandResolve::Pinned),
        (None, None) => Ok(KnowledgeExpandResolve::Current),
    }
}

fn pinned_to_head(known: Option<KnowledgeRevisionId>, head: KnowledgeRevisionId) -> bool {
    known == Some(head)
}

fn fits_response(used: usize, next: usize) -> bool {
    used.saturating_add(next) <= MAX_EXPAND_BYTES
}

fn expansion_event_id(
    manifest: Option<ContextRunId>,
    resolve: KnowledgeExpandResolve,
    served: &[(KnowledgeItemId, KnowledgeRevisionId)],
) -> String {
    let mut addresses: Vec<String> = served
        .iter()
        .map(|(item, revision)| format!("{item}@{revision}"))
        .collect();
    addresses.sort();
    let manifest = manifest.map_or_else(|| "none".to_owned(), |id| id.to_string());
    let identity = format!("{manifest}|{}|{}", resolve.as_str(), addresses.join(","));
    format!(
        "{EVENT_ID_PREFIX}{}",
        blake3::hash(identity.as_bytes()).to_hex()
    )
}

/// The event payload a reader may see under `mode`; the stored row keeps the
/// addresses restart needs.
pub(crate) fn mask_expansion_payload(payload: Value, mode: TraceRetentionMode) -> Value {
    match mode {
        TraceRetentionMode::Full | TraceRetentionMode::Redacted => payload,
        TraceRetentionMode::HashesOnly => json!({
            "resolve": payload["resolve"],
            "tokens_returned": payload["tokens_returned"],
            "items": payload["items"].as_array().map(|items| {
                items
                    .iter()
                    .map(|item| json!({
                        "content_hash": item["content_hash"],
                        "tokens": item["tokens"],
                    }))
                    .collect::<Vec<_>>()
            }),
        }),
        TraceRetentionMode::Disabled => json!({
            "resolve": payload["resolve"],
            "tokens_returned": payload["tokens_returned"],
            "item_count": payload["items"].as_array().map_or(0, Vec::len),
        }),
    }
}

/// Newest-first distinct addresses named by stored expansion payloads, which
/// `payloads` holds newest first.
fn expanded_addresses(
    payloads: &[Value],
    limit: usize,
) -> Vec<(KnowledgeItemId, KnowledgeRevisionId)> {
    let mut seen = HashSet::new();
    let mut addresses = Vec::new();
    for item in payloads
        .iter()
        .filter_map(|payload| payload["items"].as_array())
        .flatten()
    {
        let parsed = item["knowledge_item_id"]
            .as_str()
            .and_then(|id| id.parse::<KnowledgeItemId>().ok())
            .zip(
                item["knowledge_revision_id"]
                    .as_str()
                    .and_then(|id| id.parse::<KnowledgeRevisionId>().ok()),
            );
        if let Some((item_id, revision_id)) = parsed
            && seen.insert(item_id)
        {
            addresses.push((item_id, revision_id));
            if addresses.len() == limit {
                break;
            }
        }
    }
    addresses
}

fn expansion_counter(manifest: Option<&ContextRun>) -> Result<TokenCounter> {
    match manifest {
        Some(run) if run.retrieval_version.contains("o200k") => {
            TokenCounter::o200k(run.retrieval_version.ends_with("-exact"))
        }
        _ => Ok(TokenCounter::legacy()),
    }
}

async fn readable_item(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    at: DateTime<Utc>,
    pin: Option<DateTime<Utc>>,
    universe: &[ScopeId],
) -> Result<Option<(KnowledgeSnapshot, Vec<KnowledgeSource>)>> {
    let Some(snapshot) = knowledge::current(&mut *tx, tenant_id, item_id).await? else {
        return Ok(None);
    };
    if !current_active_at(&snapshot, at) || !universe.contains(&snapshot.item.scope_id) {
        return Ok(None);
    }
    match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &snapshot).await {
        Ok(_) => {}
        Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error),
    }
    if let Some(instant) = pin {
        let known = knowledge::as_known_at(&mut *tx, tenant_id, item_id, instant).await?;
        if !pinned_to_head(known.map(|held| held.revision.id), snapshot.revision.id) {
            return Ok(None);
        }
    }
    match visible_sources_with_policy(state, tx, tenant_id, &snapshot.revision).await {
        Ok((sources, _)) => Ok(Some((snapshot, sources))),
        Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Appends the expansion event. The event's identity covers everything but
/// its instant, so a retry or a concurrent twin adopts the instant already
/// stored and converges on one row.
async fn record_expansion(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    client_event_id: String,
    at: DateTime<Utc>,
    payload: Value,
) -> Result<()> {
    let event = |occurred_at: DateTime<Utc>| sessions::NewSessionEvent {
        event_type: SessionEventType::KnowledgeExpanded,
        event_schema_version: 1,
        client_event_id: client_event_id.clone(),
        occurred_at,
        source_payload_hash: sessions::payload_hash(&payload),
        payload: payload.clone(),
        redactions: None,
        quarantine: false,
    };
    match sessions::append_events(tx, tenant_id, session_id, &[event(at)]).await {
        Ok(_) => Ok(()),
        Err(conflict @ Error::Conflict { .. }) => {
            let held =
                sessions::event_occurred_at(&mut *tx, tenant_id, session_id, &client_event_id)
                    .await?;
            match held {
                Some(instant) => {
                    sessions::append_events(tx, tenant_id, session_id, &[event(instant)])
                        .await
                        .map(|_| ())
                }
                None => Err(conflict),
            }
        }
        Err(error) => Err(error),
    }
}

/// `POST /v1/sessions/{session_id}/knowledge-expand` — read full Knowledge by id.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/knowledge-expand",
    operation_id = "expand_session_knowledge",
    tag = "context",
    params(("session_id" = String, Path, description = "Owning session")),
    request_body = KnowledgeExpandBody,
    responses(
        (status = 200, description = "One outcome per requested id", body = KnowledgeExpandView),
        (status = 400, description = "Invalid ids or resolve mode", body = ApiErrorBody),
        (status = 403, description = "The PDP denied session writing", body = ApiErrorBody),
        (status = 404, description = "No such session or manifest", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "context.knowledge_expand", skip_all, fields(session.id = %session_id))]
pub(crate) async fn knowledge_expand(
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
    payload: std::result::Result<Json<KnowledgeExpandBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = request_body(payload)?;
        validate_ids(&body.ids)?;
        let resolve = resolve_mode(body.manifest_id, body.resolve)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (session, allowed, resource, input) = crate::sessions::load_with_input(
            &state,
            &mut tx,
            tenant_id,
            session_id,
            Action::SessionWrite,
        )
        .await?;
        let manifest = match body.manifest_id {
            Some(id) => Some(
                sessions::context_run(&mut *tx, tenant_id, id)
                    .await?
                    .filter(|run| run.session_id == session_id)
                    .ok_or_else(|| crate::sessions::session_not_found(session_id))?,
            ),
            None => None,
        };
        let pin = match (resolve, &manifest) {
            (KnowledgeExpandResolve::Pinned, Some(run)) => Some(run.created_at),
            _ => None,
        };
        let counter = expansion_counter(manifest.as_ref())?;
        let at = Utc::now();
        let universe = query_scope_ids(&input);

        let mut items = Vec::with_capacity(body.ids.len());
        let mut served = Vec::new();
        let mut used = 0_usize;
        let mut tokens_returned = 0_u32;
        for item_id in body.ids.iter().copied() {
            let Some((snapshot, sources)) =
                readable_item(&state, &mut tx, tenant_id, item_id, at, pin, &universe).await?
            else {
                items.push(KnowledgeExpandItemView::refused(item_id));
                continue;
            };
            let tokens = counter.count(&snapshot.revision.content.body_markdown);
            let revision_id = snapshot.revision.id;
            let content_hash = snapshot.revision.content_hash.clone();
            let view = KnowledgeExpandItemView {
                id: item_id,
                status: "served".to_owned(),
                knowledge: Some(KnowledgeItemView::from_snapshot(snapshot, at, None)),
                sources: sources.into_iter().map(Into::into).collect(),
                tokens: Some(tokens),
            };
            let size = serde_json::to_vec(&view).map_or(usize::MAX, |bytes| bytes.len());
            if !fits_response(used, size) {
                items.push(KnowledgeExpandItemView::over_budget(item_id));
                continue;
            }
            used += size;
            tokens_returned = tokens_returned.saturating_add(tokens);
            served.push((item_id, revision_id, content_hash, tokens));
            items.push(view);
        }

        if !served.is_empty() {
            served.sort_by_key(|(item, ..)| *item);
            let addresses: Vec<_> = served
                .iter()
                .map(|(item, revision, ..)| (*item, *revision))
                .collect();
            let payload = json!({
                "manifest_id": body.manifest_id,
                "resolve": resolve.as_str(),
                "tokens_returned": tokens_returned,
                "items": served.iter().map(|(item, revision, hash, tokens)| json!({
                    "knowledge_item_id": item,
                    "knowledge_revision_id": revision,
                    "content_hash": hash,
                    "tokens": tokens,
                })).collect::<Vec<_>>(),
            });
            record_expansion(
                &mut tx,
                tenant_id,
                session_id,
                expansion_event_id(body.manifest_id, resolve, &addresses),
                at,
                payload,
            )
            .await?;
        }

        let retention =
            super::effective_trace_retention(&state, &mut tx, tenant_id, &session, &input).await?;
        let count = |status: &str| items.iter().filter(|item| item.status == status).count();
        let addresses = matches!(
            retention,
            TraceRetentionMode::Full | TraceRetentionMode::Redacted
        )
        .then(|| {
            served
                .iter()
                .map(|(item, revision, hash, _)| {
                    json!({
                        "knowledge_item_id": item,
                        "knowledge_revision_id": revision,
                        "content_hash": hash,
                    })
                })
                .collect::<Vec<_>>()
        });
        crate::sessions::read_event(
            &mut tx,
            tenant_id,
            "context.knowledge_expand",
            Action::SessionWrite,
            &allowed,
            resource,
            json!({
                "session_id": session_id,
                "manifest_id": body.manifest_id,
                "resolve": resolve.as_str(),
                "requested": items.len(),
                "served": count("served"),
                "not_found_at_as_of": count("not_found_at_as_of"),
                "over_budget": count("over_budget"),
                "tokens_returned": tokens_returned,
                "addresses": addresses,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(KnowledgeExpandView {
            manifest_id: body.manifest_id,
            resolve: resolve.as_str().to_owned(),
            as_of: at,
            items,
            tokens_returned,
            token_count_kind: counter.method().certainty().to_owned(),
        }))
    }
    .await;
    respond(&state, "context.knowledge_expand", result).await
}

/// Inputs for restoring expanded Knowledge on a restart.
pub(super) struct RehydrationInput<'a> {
    pub(super) session_id: SessionId,
    pub(super) universe: &'a [ScopeId],
    pub(super) principal_id: &'a str,
    pub(super) project_id: Option<synveda_types::ProjectId>,
    pub(super) at: DateTime<Utc>,
    pub(super) ceiling: Option<Sensitivity>,
    pub(super) mode: synveda_types::configuration::ContextOptimizationMode,
}

/// Marks what the Session already expanded for verbatim delivery, after
/// repeating the same checks a fresh expansion would make.
pub(super) async fn rehydrate_expanded(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    candidates: &mut Vec<PlannedCandidate>,
    input: RehydrationInput<'_>,
) -> Result<()> {
    let payloads = sessions::knowledge_expansions(
        &mut *tx,
        tenant_id,
        input.session_id,
        MAX_REHYDRATION_EVENTS,
    )
    .await?;
    let addresses = expanded_addresses(&payloads, MAX_REHYDRATED_ITEMS);
    if addresses.is_empty() {
        return Ok(());
    }
    for (item_id, revision_id) in addresses {
        let Some(snapshot) = knowledge::current(&mut *tx, tenant_id, item_id).await? else {
            continue;
        };
        if snapshot.revision.id != revision_id
            || !current_active_at(&snapshot, input.at)
            || !input.universe.contains(&snapshot.item.scope_id)
            || input
                .ceiling
                .is_some_and(|limit| snapshot.revision.content.sensitivity > limit)
        {
            continue;
        }
        let authorization =
            match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &snapshot).await {
                Ok(allowed) => audit::decision_context(Action::KnowledgeRead, &allowed),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
        if graph::stale_at(tx, tenant_id, &snapshot, input.at).await? {
            continue;
        }
        if let Some(candidate) = candidates
            .iter_mut()
            .find(|candidate| candidate.revision_id() == Some(revision_id))
        {
            candidate.rehydrated = true;
            candidate.exclusion = None;
            continue;
        }
        let (freshness, pin, anchor, final_score, reasons) = planned_score(
            &snapshot,
            ScoreSeed::default(),
            input.principal_id,
            input.project_id,
            input.at,
        );
        candidates.push(PlannedCandidate {
            id: synveda_types::ContextCandidateId::new(),
            payload: PlannedPayload::Knowledge(snapshot),
            sources: Vec::new(),
            keyword_micros: 0,
            semantic_micros: 0,
            anchor_micros: anchor,
            edge_weight_micros: 0,
            hop_penalty_micros: 0,
            freshness_micros: freshness,
            pin_micros: pin,
            current_state_micros: 100_000,
            final_micros: final_score,
            reasons,
            exclusion: None,
            authorization,
            selected_tokens: None,
            delivery_span: None,
            required: false,
            rehydrated: true,
            graph_path: Vec::new(),
        });
    }
    rank_and_deduplicate(candidates, input.mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_bounded_and_unique() {
        assert!(validate_ids(&[]).is_err());
        let one = KnowledgeItemId::new();
        assert!(validate_ids(&[one]).is_ok());
        assert!(validate_ids(&[one, one]).is_err());
        let many: Vec<_> = (0..33).map(|_| KnowledgeItemId::new()).collect();
        assert!(validate_ids(&many).is_err());
        assert!(validate_ids(&many[..32]).is_ok());
    }

    #[test]
    fn a_pin_needs_a_manifest_and_is_its_default() {
        use KnowledgeExpandResolve::{Current, Pinned};
        let manifest = Some(ContextRunId::new());
        assert_eq!(resolve_mode(manifest, None).unwrap(), Pinned);
        assert_eq!(resolve_mode(manifest, Some(Current)).unwrap(), Current);
        assert_eq!(resolve_mode(None, None).unwrap(), Current);
        assert!(resolve_mode(None, Some(Pinned)).is_err());
    }

    #[test]
    fn a_pin_serves_only_an_unchanged_head() {
        let head = KnowledgeRevisionId::new();
        assert!(pinned_to_head(Some(head), head));
        assert!(!pinned_to_head(Some(KnowledgeRevisionId::new()), head));
        assert!(!pinned_to_head(None, head));
    }

    #[test]
    fn the_response_cap_is_inclusive() {
        assert!(fits_response(0, MAX_EXPAND_BYTES));
        assert!(!fits_response(1, MAX_EXPAND_BYTES));
        assert!(!fits_response(usize::MAX, 1));
    }

    #[test]
    fn event_identity_ignores_request_order_and_separates_modes() {
        let manifest = Some(ContextRunId::new());
        let first = (KnowledgeItemId::new(), KnowledgeRevisionId::new());
        let second = (KnowledgeItemId::new(), KnowledgeRevisionId::new());
        let pinned = KnowledgeExpandResolve::Pinned;
        let forward = expansion_event_id(manifest, pinned, &[first, second]);
        assert_eq!(
            forward,
            expansion_event_id(manifest, pinned, &[second, first])
        );
        assert_ne!(forward, expansion_event_id(manifest, pinned, &[first]));
        assert_ne!(
            forward,
            expansion_event_id(manifest, KnowledgeExpandResolve::Current, &[first, second])
        );
        assert_ne!(forward, expansion_event_id(None, pinned, &[first, second]));
    }

    fn payload(item: KnowledgeItemId, revision: KnowledgeRevisionId) -> Value {
        json!({
            "resolve": "pinned",
            "tokens_returned": 9,
            "items": [{
                "knowledge_item_id": item,
                "knowledge_revision_id": revision,
                "content_hash": "abc",
                "tokens": 9,
            }],
        })
    }

    #[test]
    fn retention_masks_addresses_for_readers() {
        let item = KnowledgeItemId::new();
        let revision = KnowledgeRevisionId::new();
        let full = payload(item, revision);
        assert_eq!(
            mask_expansion_payload(full.clone(), TraceRetentionMode::Redacted),
            full
        );
        let hashed = mask_expansion_payload(full.clone(), TraceRetentionMode::HashesOnly);
        assert!(!hashed.to_string().contains(&item.to_string()));
        assert_eq!(hashed["items"][0]["content_hash"], "abc");
        let disabled = mask_expansion_payload(full, TraceRetentionMode::Disabled);
        assert_eq!(disabled["item_count"], 1);
        assert!(disabled.get("items").is_none());
    }

    #[test]
    fn addresses_are_newest_first_distinct_and_bounded() {
        let item = KnowledgeItemId::new();
        let newer = KnowledgeRevisionId::new();
        let older = KnowledgeRevisionId::new();
        let other = (KnowledgeItemId::new(), KnowledgeRevisionId::new());
        let payloads = vec![
            payload(item, newer),
            payload(item, older),
            payload(other.0, other.1),
            json!({"items": "not an array"}),
        ];
        assert_eq!(expanded_addresses(&payloads, 8), vec![(item, newer), other]);
        assert_eq!(expanded_addresses(&payloads, 1), vec![(item, newer)]);
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;

    fn item(seed: u128) -> KnowledgeItemId {
        KnowledgeItemId::from_uuid(Uuid::from_u128(seed))
    }

    fn revision(seed: u128) -> KnowledgeRevisionId {
        KnowledgeRevisionId::from_uuid(Uuid::from_u128(seed))
    }

    fn resolve_strategy() -> impl Strategy<Value = KnowledgeExpandResolve> {
        prop_oneof![
            Just(KnowledgeExpandResolve::Pinned),
            Just(KnowledgeExpandResolve::Current),
        ]
    }

    fn addresses() -> impl Strategy<Value = Vec<(KnowledgeItemId, KnowledgeRevisionId)>> {
        prop::collection::btree_map(1_u128..u128::MAX, any::<u128>(), 1..12).prop_map(|map| {
            map.into_iter()
                .map(|(item_seed, revision_seed)| (item(item_seed), revision(revision_seed)))
                .collect()
        })
    }

    fn payload_with(
        manifest: Option<ContextRunId>,
        served: &[(KnowledgeItemId, KnowledgeRevisionId)],
        hashes: &[String],
    ) -> Value {
        json!({
            "manifest_id": manifest,
            "resolve": "pinned",
            "tokens_returned": served.len() * 7,
            "items": served.iter().zip(hashes).map(|((item, revision), hash)| json!({
                "knowledge_item_id": item,
                "knowledge_revision_id": revision,
                "content_hash": hash,
                "tokens": 7,
            })).collect::<Vec<_>>(),
        })
    }

    proptest! {
        #[test]
        fn ids_are_accepted_exactly_when_bounded_and_unique(
            picks in prop::collection::vec(0_u128..48, 0..40)
        ) {
            let ids: Vec<_> = picks.iter().map(|seed| item(*seed)).collect();
            let unique = picks.iter().collect::<HashSet<_>>().len() == picks.len();
            let expected = !ids.is_empty() && ids.len() <= MAX_EXPAND_IDS && unique;
            prop_assert_eq!(validate_ids(&ids).is_ok(), expected);
        }

        #[test]
        fn a_pin_is_never_resolved_without_a_manifest(
            named in any::<bool>(),
            requested in prop::option::of(resolve_strategy()),
        ) {
            let manifest = named.then(ContextRunId::new);
            let resolved = resolve_mode(manifest, requested);
            match (named, requested) {
                (false, Some(KnowledgeExpandResolve::Pinned)) => prop_assert!(resolved.is_err()),
                (_, Some(mode)) => prop_assert_eq!(resolved.ok(), Some(mode)),
                (true, None) => prop_assert_eq!(resolved.ok(), Some(KnowledgeExpandResolve::Pinned)),
                (false, None) => prop_assert_eq!(resolved.ok(), Some(KnowledgeExpandResolve::Current)),
            }
        }

        #[test]
        fn a_pin_serves_only_the_revision_it_saw(
            head in 0_u128..8,
            known in prop::option::of(0_u128..8),
        ) {
            prop_assert_eq!(
                pinned_to_head(known.map(revision), revision(head)),
                known == Some(head)
            );
        }

        #[test]
        fn first_fit_never_exceeds_the_response_cap(
            sizes in prop::collection::vec(0_usize..=MAX_EXPAND_BYTES * 2, 0..40)
        ) {
            let mut used = 0_usize;
            for size in sizes {
                let admitted = fits_response(used, size);
                prop_assert_eq!(admitted, used + size <= MAX_EXPAND_BYTES);
                if admitted {
                    used += size;
                }
                prop_assert!(used <= MAX_EXPAND_BYTES);
            }
        }

        #[test]
        fn the_cap_check_cannot_overflow(used in any::<usize>(), next in any::<usize>()) {
            prop_assert_eq!(
                fits_response(used, next),
                (used as u128) + (next as u128) <= MAX_EXPAND_BYTES as u128
            );
        }

        #[test]
        fn event_identity_is_order_free_and_covers_every_address(
            served in addresses(),
            shuffled_seed in any::<u64>(),
            resolve in resolve_strategy(),
            named in any::<bool>(),
        ) {
            let manifest = named.then(ContextRunId::new);
            let id = expansion_event_id(manifest, resolve, &served);
            let digest = id.strip_prefix(EVENT_ID_PREFIX).expect("prefixed identity");
            prop_assert_eq!(digest.len(), 64);
            prop_assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));

            let mut reordered = served.clone();
            let rotation = usize::try_from(shuffled_seed).unwrap_or(0) % reordered.len();
            reordered.rotate_left(rotation);
            reordered.reverse();
            prop_assert_eq!(&expansion_event_id(manifest, resolve, &reordered), &id);

            for dropped in 0..served.len() {
                let mut fewer = served.clone();
                fewer.remove(dropped);
                prop_assert_ne!(&expansion_event_id(manifest, resolve, &fewer), &id);
                let mut changed = served.clone();
                changed[dropped].1 = KnowledgeRevisionId::new();
                prop_assert_ne!(&expansion_event_id(manifest, resolve, &changed), &id);
            }
            let other = match resolve {
                KnowledgeExpandResolve::Pinned => KnowledgeExpandResolve::Current,
                KnowledgeExpandResolve::Current => KnowledgeExpandResolve::Pinned,
            };
            prop_assert_ne!(&expansion_event_id(manifest, other, &served), &id);
            let elsewhere = Some(ContextRunId::new());
            prop_assert_ne!(&expansion_event_id(elsewhere, resolve, &served), &id);
        }

        #[test]
        fn a_masked_payload_names_no_address(
            served in addresses(),
            named in any::<bool>(),
            hash_seed in any::<u64>(),
        ) {
            let manifest = named.then(ContextRunId::new);
            let hashes: Vec<String> = (0..served.len())
                .map(|index| blake3::hash(format!("{hash_seed}:{index}").as_bytes()).to_hex().to_string())
                .collect();
            let stored = payload_with(manifest, &served, &hashes);
            for mode in [TraceRetentionMode::Full, TraceRetentionMode::Redacted] {
                prop_assert_eq!(&mask_expansion_payload(stored.clone(), mode), &stored);
            }
            for mode in [TraceRetentionMode::HashesOnly, TraceRetentionMode::Disabled] {
                let masked = mask_expansion_payload(stored.clone(), mode).to_string();
                for (item, revision) in &served {
                    prop_assert!(!masked.contains(&item.to_string()), "{mode:?}: {masked}");
                    prop_assert!(!masked.contains(&revision.to_string()), "{mode:?}: {masked}");
                }
                if let Some(manifest) = manifest {
                    prop_assert!(!masked.contains(&manifest.to_string()), "{mode:?}: {masked}");
                }
            }
            let hashed = mask_expansion_payload(stored.clone(), TraceRetentionMode::HashesOnly);
            let kept: Vec<_> = hashed["items"]
                .as_array()
                .expect("hashes_only keeps one row per item")
                .iter()
                .map(|row| row["content_hash"].as_str().unwrap_or_default().to_owned())
                .collect();
            prop_assert_eq!(kept, hashes);
            let disabled = mask_expansion_payload(stored, TraceRetentionMode::Disabled);
            prop_assert_eq!(disabled["item_count"].as_u64(), Some(served.len() as u64));
            prop_assert!(disabled.get("items").is_none());
        }

        #[test]
        fn restart_reads_each_item_once_newest_first(
            events in prop::collection::vec(
                prop::option::of(prop::collection::vec((0_u128..6, 0_u128..6, any::<bool>()), 0..6)),
                0..10,
            ),
            limit in 1_usize..8,
        ) {
            let payloads: Vec<Value> = events
                .iter()
                .map(|event| match event {
                    None => json!({"items": "not an array"}),
                    Some(rows) => json!({"items": rows.iter().map(|(item_seed, revision_seed, valid)| {
                        if *valid {
                            json!({
                                "knowledge_item_id": item(*item_seed),
                                "knowledge_revision_id": revision(*revision_seed),
                            })
                        } else {
                            json!({"knowledge_item_id": "not-an-id", "knowledge_revision_id": revision(*revision_seed)})
                        }
                    }).collect::<Vec<_>>()}),
                })
                .collect();
            let mut expected = Vec::new();
            for (item_seed, revision_seed, _) in events
                .iter()
                .flatten()
                .flatten()
                .filter(|(_, _, valid)| *valid)
            {
                if expected.len() < limit && !expected.iter().any(|(seen, _)| *seen == item(*item_seed)) {
                    expected.push((item(*item_seed), revision(*revision_seed)));
                }
            }
            prop_assert_eq!(expanded_addresses(&payloads, limit), expected);
        }

        #[test]
        fn a_withheld_item_carries_only_its_id_and_status(seed in any::<u128>()) {
            let id = item(seed);
            for (view, status) in [
                (KnowledgeExpandItemView::refused(id), "not_found_at_as_of"),
                (KnowledgeExpandItemView::over_budget(id), "over_budget"),
            ] {
                prop_assert_eq!(
                    serde_json::to_value(&view).expect("view serializes"),
                    json!({"id": id, "status": status})
                );
            }
        }
    }
}
