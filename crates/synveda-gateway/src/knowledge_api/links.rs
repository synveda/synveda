//! Business links between Knowledge items (ADR-0164): assert, end and read.
//! Writes are governed Knowledge commands; a read shows a link only when the
//! caller may read both of its ends.

use std::collections::HashMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_policy::Resource;
use synveda_store::{entity_links, rls};
use synveda_types::knowledge::{EntityLink, EntityPredicate, KnowledgeCommand};
use synveda_types::{EntityLinkId, Error, KnowledgeItemId, KnowledgeRevisionId};
use utoipa::ToSchema;

use super::{
    KnowledgeMutationView, authorize_snapshot, execute_command, execute_command_with, read_event,
    readable_snapshot, respond, snapshot, validate_as_known_at,
};
use crate::app::AppState;
use crate::knowledge::links::link_not_found;
use crate::request::{body, commit, tenant_id};
use crate::workspaces::{ApiErrorBody, string_enum};

/// Most links one read returns; a read that would exceed it is refused.
const MAX_ITEM_LINKS: i64 = 1_000;

fn predicate_schema() -> utoipa::openapi::schema::Object {
    string_enum(EntityPredicate::ALL.iter().map(|value| value.as_str()))
}

/// `POST /v1/knowledge/{id}/links`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkKnowledgeBody {
    /// Exact current revision of the source the author inspected. It becomes
    /// the link's asserting revision.
    #[schema(value_type = String, format = "uuid")]
    pub expected_revision_id: KnowledgeRevisionId,
    /// Item the link points to.
    #[schema(value_type = String, format = "uuid")]
    pub target_item_id: KnowledgeItemId,
    /// Compact IRI of a predicate the source's scope enables.
    #[schema(schema_with = predicate_schema)]
    pub predicate: String,
    /// Start of the valid-time window. Defaults to now.
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
}

/// `POST /v1/knowledge/{id}/links/{link_id}/end`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EndLinkBody {
    /// End of the valid-time window, exclusive. Defaults to now.
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
    /// Bounded human reason.
    pub reason: String,
}

/// One visible business link. Both ends passed independent read decisions.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntityLinkView {
    /// Stable link id.
    #[schema(value_type = String, format = "uuid")]
    pub id: EntityLinkId,
    /// Item asserting the link.
    #[schema(value_type = String, format = "uuid")]
    pub source_item_id: KnowledgeItemId,
    /// Item the link points to.
    #[schema(value_type = String, format = "uuid")]
    pub target_item_id: KnowledgeItemId,
    /// Source revision current when the link was asserted.
    #[schema(value_type = String, format = "uuid")]
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Compact IRI of the predicate.
    #[schema(schema_with = predicate_schema)]
    pub predicate: String,
    /// Start of the valid-time window.
    pub valid_from: DateTime<Utc>,
    /// End of the valid-time window, exclusive, when an end is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime<Utc>>,
    /// When the link was recorded.
    pub created_at: DateTime<Utc>,
    /// When its end was recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
}

impl From<EntityLink> for EntityLinkView {
    fn from(link: EntityLink) -> Self {
        Self {
            id: link.id,
            source_item_id: link.source_item_id,
            target_item_id: link.target_item_id,
            asserting_revision_id: link.asserting_revision_id,
            predicate: link.predicate.as_str().to_owned(),
            valid_from: link.valid_from,
            valid_to: link.valid_to,
            created_at: link.created_at,
            ended_at: link.ended_at,
        }
    }
}

/// An item's visible business links.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntityLinkListView {
    /// Links into or out of the item, oldest first.
    pub links: Vec<EntityLinkView>,
}

/// The bitemporal lens over an item's links.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LinkLensParams {
    /// Valid-time instant. Defaults to now.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub as_of: Option<DateTime<Utc>>,
    /// Transaction-time instant. Omitted, the read sees everything recorded;
    /// it cannot be in the future.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "date-time")]
    pub as_known_at: Option<DateTime<Utc>>,
}

/// `POST /v1/knowledge/{id}/links` — assert a business link from this item.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/links",
    operation_id = "link_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable id of the source Knowledge item"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = LinkKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body, predicate or endpoint, or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such source or visible target", body = ApiErrorBody),
        (status = 409, description = "Idempotency, revision or lifecycle conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.link", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn link(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<LinkKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let predicate: EntityPredicate = body.predicate.parse()?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/links",
            "knowledge_item_id": id,
            "body": body,
        });
        execute_command(&state, &headers, "knowledge.link", canonical, || {
            Ok(KnowledgeCommand::Link {
                link_id: EntityLinkId::new(),
                source_item_id: id,
                expected_revision_id: body.expected_revision_id,
                target_item_id: body.target_item_id,
                predicate,
                valid_from: body.valid_from.unwrap_or_else(Utc::now),
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.link", result).await
}

/// `POST /v1/knowledge/{id}/links/{link_id}/end` — end a business link.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/links/{link_id}/end",
    operation_id = "end_knowledge_link",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable id of the link's source Knowledge item"),
        ("link_id" = String, Path, description = "Stable business link id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = EndLinkBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such visible link", body = ApiErrorBody),
        (status = 409, description = "Idempotency conflict, or the link has already ended", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.end_link", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn end_link(
    State(state): State<AppState>,
    Path((id, link_id)): Path<(KnowledgeItemId, EntityLinkId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<EndLinkBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/links/{link_id}/end",
            "knowledge_item_id": id,
            "entity_link_id": link_id,
            "body": body,
        });
        let state = &state;
        execute_command_with(
            state,
            &headers,
            "knowledge.end_link",
            canonical,
            || async move {
                // Checked before the link is read, so a refusal of the reason
                // cannot depend on whether the link exists.
                crate::knowledge::validate_reason(&body.reason)?;
                // The command names both ends so erasing either clears it. The
                // command layer authorises both before revealing the link.
                let tenant_id = tenant_id()?;
                let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
                let target_item_id = entity_links::get(&mut *tx, tenant_id, link_id)
                    .await?
                    .filter(|link| link.source_item_id == id)
                    .ok_or_else(|| link_not_found(link_id))?
                    .target_item_id;
                drop(tx);
                Ok(KnowledgeCommand::EndLink {
                    link_id,
                    source_item_id: id,
                    target_item_id,
                    valid_to: body.valid_to.unwrap_or_else(Utc::now),
                    reason: body.reason,
                })
            },
        )
        .await
    }
    .await;
    respond(&state, "knowledge.end_link", result).await
}

/// `GET /v1/knowledge/{id}/links` — visible business links into or out of
/// an item, valid at `as_of` as recorded by `as_known_at`.
#[utoipa::path(
    get,
    path = "/v1/knowledge/{id}/links",
    operation_id = "list_knowledge_links",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        LinkLensParams,
    ),
    responses(
        (status = 200, description = "Visible business links", body = EntityLinkListView),
        (status = 400, description = "as_known_at is in the future, or more than 1,000 links touch the item", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge reading", body = ApiErrorBody),
        (status = 404, description = "No such visible item", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.links", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn links(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    Query(params): Query<LinkLensParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let now = Utc::now();
        validate_as_known_at(params.as_known_at, now)?;
        let as_of = params.as_of.unwrap_or(now);
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let item = snapshot(&mut tx, tenant_id, id).await?;
        let allowed = authorize_snapshot(&state, &mut tx, tenant_id, &item).await?;
        let known_at = params.as_known_at;
        let bound = MAX_ITEM_LINKS + 1;
        let stored =
            entity_links::for_item(&mut *tx, tenant_id, id, as_of, known_at, bound).await?;
        if stored.len() as i64 > MAX_ITEM_LINKS {
            return Err(Error::Invalid {
                message: format!(
                    "more than {MAX_ITEM_LINKS} business links touch this item; read them as of \
                     an earlier instant"
                ),
            });
        }
        let mut readable = HashMap::new();
        let mut links = Vec::new();
        for link in stored {
            let other = if link.source_item_id == id {
                link.target_item_id
            } else {
                link.source_item_id
            };
            let visible = match readable.get(&other) {
                Some(visible) => *visible,
                None => {
                    let visible = readable_snapshot(&state, &mut tx, tenant_id, other)
                        .await?
                        .is_some();
                    readable.insert(other, visible);
                    visible
                }
            };
            if visible {
                links.push(EntityLinkView::from(link));
            }
        }
        read_event(
            &mut tx,
            tenant_id,
            "knowledge.links",
            &allowed,
            Resource::KnowledgeItem(id),
            json!({
                "knowledge_item_id": id,
                "as_of": as_of,
                "as_known_at": params.as_known_at,
                "visible_links": links.len(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(EntityLinkListView { links }))
    }
    .await;
    respond(&state, "knowledge.links", result).await
}
