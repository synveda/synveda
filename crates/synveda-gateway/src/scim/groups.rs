//! `/scim/v2/Groups` projected directly onto the shared access graph.
//!
//! A SCIM group is the same [`synveda_types::access::Group`] a manual access
//! grant names. The protocol-specific resource id and optional `externalId`
//! are provenance on that aggregate; there is no SCIM group/member mirror.
//! Membership is keyed by stable identities, so provisioning before first
//! login is complete rather than deferred until a token subject exists.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_store::{access, directory};
use synveda_types::access::Group;
use synveda_types::workspace::LifecycleStatus;
use synveda_types::{DirectoryUser, DirectoryUserId, GroupId, IdentityId, TenantId};

use super::users::ListQuery;
use super::{ScimAuth, ScimError, ScimJson, base_url, filter, page_bounds, wire};
use crate::app::AppState;

const DIRECTORY_SOURCE: &str = "scim";

/// `GET /Groups`.
pub async fn list(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ScimError> {
    let base = base_url(&state);
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let mut groups: Vec<Group> = access::directory_groups(&mut *tx, tenant_id, DIRECTORY_SOURCE)
        .await?
        .into_iter()
        .filter(|group| group.status == LifecycleStatus::Active)
        .collect();

    if let Some(text) = query.filter() {
        let parsed =
            filter::parse(text, filter::GROUP_FILTERABLE).map_err(super::users::refuse_filter)?;
        groups.retain(|group| match parsed.attribute.as_str() {
            "displayname" => group.display_name == parsed.value,
            "id" => parsed
                .value
                .parse::<GroupId>()
                .is_ok_and(|id| group.id == id),
            "externalid" => group.directory_external_id.as_deref() == Some(parsed.value.as_str()),
            _ => false,
        });
    }

    let total = i64::try_from(groups.len()).unwrap_or(i64::MAX);
    let (start, count) = page_bounds(query.start_index(), query.count());
    let offset = usize::try_from(start.saturating_sub(1)).unwrap_or(usize::MAX);
    let limit = usize::try_from(count).unwrap_or(0);
    let page: Vec<Group> = groups.into_iter().skip(offset).take(limit).collect();

    let mut resources = Vec::with_capacity(page.len());
    for group in &page {
        let members = scim_members(&mut tx, tenant_id, group.id).await?;
        resources.push(wire::GroupResource::of(group, &members, &base));
    }
    Ok(ScimJson(
        StatusCode::OK,
        wire::ListResponse::new(resources, total, start),
    )
    .into_response())
}

/// `GET /Groups/{id}`.
pub async fn get(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let group = scim_group(&mut tx, tenant_id, id).await?;
    let members = scim_members(&mut tx, tenant_id, id).await?;
    Ok(ScimJson(
        StatusCode::OK,
        wire::GroupResource::of(&group, &members, &base_url(&state)),
    )
    .into_response())
}

/// `POST /Groups`.
pub async fn create(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Json(body): Json<wire::GroupResource>,
) -> Result<Response, ScimError> {
    let display_name = required_display_name(body.display_name.as_deref())?;
    let member_ids = member_ids(&body.members)?;
    let tenant_id = auth.tenant.id;
    let id = GroupId::new();
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let identities = member_identity_ids(&mut tx, tenant_id, &member_ids).await?;
    let group = access::sync_directory_group(
        &mut tx,
        id,
        tenant_id,
        DIRECTORY_SOURCE,
        &id.to_string(),
        body.external_id.as_deref(),
        &directory_slug(id),
        display_name,
        &identities,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?;
    audit_group(
        &mut tx,
        &auth,
        AuditAction::GroupCreated,
        "create",
        &group,
        identities.len(),
    )
    .await?;
    tx.commit().await.map_err(commit_error)?;
    respond(&state, &auth, id, StatusCode::CREATED).await
}

/// `PUT /Groups/{id}` — complete attribute and membership replacement.
pub async fn replace(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::GroupResource>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let member_ids = member_ids(&body.members)?;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = scim_group(&mut tx, tenant_id, id).await?;
    let identities = member_identity_ids(&mut tx, tenant_id, &member_ids).await?;
    let display_name = body
        .display_name
        .as_deref()
        .unwrap_or(current.display_name.as_str());
    let group = access::sync_directory_group(
        &mut tx,
        current.id,
        tenant_id,
        DIRECTORY_SOURCE,
        current_directory_resource(&current)?,
        body.external_id.as_deref(),
        &current.slug,
        display_name,
        &identities,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?;
    audit_group(
        &mut tx,
        &auth,
        AuditAction::GroupUpdated,
        "replace",
        &group,
        identities.len(),
    )
    .await?;
    tx.commit().await.map_err(commit_error)?;
    respond(&state, &auth, id, StatusCode::OK).await
}

/// `PATCH /Groups/{id}` — the incremental shapes Entra and Okta send.
pub async fn patch(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::PatchRequest>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = scim_group(&mut tx, tenant_id, id).await?;
    let current_members = scim_members(&mut tx, tenant_id, id).await?;
    let mut members: BTreeSet<DirectoryUserId> =
        current_members.into_iter().map(|user| user.id).collect();
    let mut display_name = current.display_name.clone();

    for operation in &body.operations {
        let op = operation.op.to_ascii_lowercase();
        let path = operation.path.clone().unwrap_or_default();
        let lowered = path.to_ascii_lowercase();

        if op == "remove"
            && let Some(value) = filter::member_value_path(&path)
        {
            members.remove(&parse_member_id(&value)?);
            continue;
        }

        match lowered.as_str() {
            "members" => {
                let changed = member_ids(&parse_members(operation.value.as_ref())?)?;
                match op.as_str() {
                    "add" => members.extend(changed),
                    "remove" if changed.is_empty() => members.clear(),
                    "remove" => {
                        for member in changed {
                            members.remove(&member);
                        }
                    }
                    "replace" => members = changed.into_iter().collect(),
                    _ => return Err(invalid_operation(&op)),
                }
            }
            "displayname" => {
                if !matches!(op.as_str(), "add" | "replace") {
                    return Err(invalid_operation(&op));
                }
                display_name = required_display_name(
                    operation.value.as_ref().and_then(serde_json::Value::as_str),
                )?
                .to_owned();
            }
            other => {
                return Err(ScimError::typed(
                    StatusCode::BAD_REQUEST,
                    "invalidPath",
                    format!("`{other}` is not a patchable path on Group"),
                ));
            }
        }
    }

    let member_ids: Vec<DirectoryUserId> = members.into_iter().collect();
    let identities = member_identity_ids(&mut tx, tenant_id, &member_ids).await?;
    let group = access::sync_directory_group(
        &mut tx,
        current.id,
        tenant_id,
        DIRECTORY_SOURCE,
        current_directory_resource(&current)?,
        current.directory_external_id.as_deref(),
        &current.slug,
        &display_name,
        &identities,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?;
    audit_group(
        &mut tx,
        &auth,
        AuditAction::GroupUpdated,
        "patch",
        &group,
        identities.len(),
    )
    .await?;
    tx.commit().await.map_err(commit_error)?;
    respond(&state, &auth, id, StatusCode::OK).await
}

/// `DELETE /Groups/{id}` archives the shared aggregate. Archived groups
/// resolve to nobody while grants and provenance remain reviewable.
pub async fn delete(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = scim_group(&mut tx, tenant_id, id).await?;
    let resource_id = current_directory_resource(&current)?.to_owned();
    let group = access::retire_directory_group(&mut tx, tenant_id, DIRECTORY_SOURCE, &resource_id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    audit_group(
        &mut tx,
        &auth,
        AuditAction::GroupUpdated,
        "archive",
        &group,
        0,
    )
    .await?;
    tx.commit().await.map_err(commit_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn scim_group(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: GroupId,
) -> Result<Group, ScimError> {
    access::get_group(&mut *tx, tenant_id, id)
        .await?
        .filter(|group| {
            group.directory_source.as_deref() == Some(DIRECTORY_SOURCE)
                && group.status == LifecycleStatus::Active
        })
        .ok_or_else(ScimError::not_found)
}

async fn scim_members(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    group_id: GroupId,
) -> Result<Vec<DirectoryUser>, ScimError> {
    let memberships = access::group_members(&mut *tx, tenant_id, group_id).await?;
    let mut users = Vec::with_capacity(memberships.len());
    for member in memberships {
        if let Some(user) = directory::user_for_identity(&mut *tx, tenant_id, member.identity_id)
            .await?
            .filter(|user| user.directory_source == DIRECTORY_SOURCE)
        {
            users.push(user);
        }
    }
    users.sort_by_key(|user| user.id);
    Ok(users)
}

async fn member_identity_ids(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    members: &[DirectoryUserId],
) -> Result<Vec<IdentityId>, ScimError> {
    let mut identities = Vec::with_capacity(members.len());
    for member in members {
        let user = directory::user(&mut *tx, tenant_id, DIRECTORY_SOURCE, *member)
            .await?
            .ok_or_else(ScimError::not_found)?;
        let identity_id = user.identity_id.ok_or_else(|| {
            ScimError::typed(
                StatusCode::CONFLICT,
                "mutability",
                format!("member {member} has not completed identity reconciliation"),
            )
        })?;
        identities.push(identity_id);
    }
    identities.sort_unstable();
    identities.dedup();
    Ok(identities)
}

async fn respond(
    state: &AppState,
    auth: &ScimAuth,
    id: GroupId,
    status: StatusCode,
) -> Result<Response, ScimError> {
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let group = scim_group(&mut tx, tenant_id, id).await?;
    let members = scim_members(&mut tx, tenant_id, id).await?;
    Ok(ScimJson(
        status,
        wire::GroupResource::of(&group, &members, &base_url(state)),
    )
    .into_response())
}

async fn audit_group(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    auth: &ScimAuth,
    action: AuditAction,
    operation: &'static str,
    group: &Group,
    member_count: usize,
) -> Result<(), ScimError> {
    crate::audit::record_as(
        tx,
        auth.tenant.id,
        Actor::system("scim"),
        action,
        format!("group {}", group.id),
        Outcome::Success,
        serde_json::json!({
            "source": DIRECTORY_SOURCE,
            "credential_id": auth.credential.id,
            "operation": operation,
            "group_id": group.id,
            "directory_resource_id": group.directory_resource_id,
            "directory_external_id": group.directory_external_id,
            "revision": group.revision,
            "status": group.status.as_str(),
            "member_count": member_count,
        }),
    )
    .await
    .map(|_| ())
    .map_err(ScimError::from)
}

fn current_directory_resource(group: &Group) -> Result<&str, ScimError> {
    group.directory_resource_id.as_deref().ok_or_else(|| {
        ScimError::from_taxonomy(&synveda_types::Error::Internal {
            message: format!("directory group {} has no resource id", group.id),
        })
    })
}

fn required_display_name(value: Option<&str>) -> Result<&str, ScimError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ScimError::typed(
                StatusCode::BAD_REQUEST,
                "invalidValue",
                "displayName is required",
            )
        })
}

fn directory_slug(id: GroupId) -> String {
    format!("dir-{id}")
}

fn member_ids(members: &[wire::MultiValue]) -> Result<Vec<DirectoryUserId>, ScimError> {
    members
        .iter()
        .filter_map(|member| member.value.as_deref())
        .map(parse_member_id)
        .collect()
}

fn parse_member_id(value: &str) -> Result<DirectoryUserId, ScimError> {
    value.parse::<DirectoryUserId>().map_err(|_| {
        ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidValue",
            format!("`{value}` is not a member id this server issued"),
        )
    })
}

fn parse_members(value: Option<&serde_json::Value>) -> Result<Vec<wire::MultiValue>, ScimError> {
    let invalid = || {
        ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidValue",
            "members must be an array of `{\"value\": \"<id>\"}` objects",
        )
    };
    match value {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(value @ serde_json::Value::Array(_)) => {
            serde_json::from_value(value.clone()).map_err(|_| invalid())
        }
        Some(value @ serde_json::Value::Object(_)) => serde_json::from_value(value.clone())
            .map(|member| vec![member])
            .map_err(|_| invalid()),
        Some(_) => Err(invalid()),
    }
}

fn invalid_operation(operation: &str) -> ScimError {
    ScimError::typed(
        StatusCode::BAD_REQUEST,
        "invalidValue",
        format!("`{operation}` is not a supported Group patch operation"),
    )
}

fn parse_id(id: &str) -> Result<GroupId, ScimError> {
    id.parse::<GroupId>().map_err(|_| ScimError::not_found())
}

fn commit_error(err: sqlx::Error) -> ScimError {
    ScimError::from_taxonomy(&synveda_types::Error::Storage {
        message: format!("commit group write: {err}"),
    })
}
