//! `/scim/v2/Groups` (AUTH-4, ADR-0059).
//!
//! A SCIM group is a **directory group**, never a hierarchy node. Its
//! `displayName` is what AUTH-2's mapping resolver sees — `group_mappings`
//! first, then the `synveda-{dept}-{team}` convention — and that is the
//! only thing about it the product reads (decision 6).
//!
//! Membership changes are therefore placement changes, so every mutation
//! here re-reconciles the members it touched. Nothing else does: renaming a
//! group re-resolves everybody in it, because the name *is* the mapping
//! key, and that is the one rename in this product that moves people.
//!
//! # It also projects onto the governed access model
//!
//! Since CPR-6 every mutation here mirrors the directory group onto a
//! [`synveda_types::access::Group`] with `source = 'directory'`, and its
//! membership onto `group_members` keyed by each member's **token subject**
//! (ADR-0073 decision 9). That is the whole of "directory users and groups map
//! to principals, groups, group_members and scope_grants": a principal *is* a
//! subject, a directory group *is* a group, and there is no enterprise
//! membership table beside the one a person working alone uses.
//!
//! Two things it deliberately does not do. It writes **no grants** — a
//! directory says who is in a group, never what the group may do, and a sync
//! that invented grants would be a directory writing policy. And it projects
//! nobody who has not provisioned an identity yet: a subject is what a
//! verified token carries, and a directory row that has never been through
//! reconciliation has no subject to name. Such a person joins the group on the
//! sync that follows their first login.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use synveda_store::{access, directory, identities};
use synveda_types::{DirectoryGroupId, DirectoryUser, DirectoryUserId};

use super::users::ListQuery;
use super::{ScimAuth, ScimError, ScimJson, base_url, filter, page_bounds, reconcile, wire};
use crate::app::AppState;

/// `GET /Groups`.
pub async fn list(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ScimError> {
    let base = base_url(&state);
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;

    let (groups, total) = match query.filter() {
        Some(text) => {
            let parsed = filter::parse(text, filter::GROUP_FILTERABLE)
                .map_err(super::users::refuse_filter)?;
            let found = match parsed.attribute.as_str() {
                "displayname" => {
                    directory::group_by_display_name(&mut *tx, tenant_id, &parsed.value).await?
                }
                "id" => match parsed.value.parse::<DirectoryGroupId>() {
                    Ok(id) => directory::group(&mut *tx, tenant_id, id).await?,
                    Err(_) => None,
                },
                // `externalId` is stored but not indexed for lookup: a
                // filter on it is well-formed and unimplemented, which is
                // the 501 case rather than a wrong empty list.
                _ => {
                    return Err(super::users::refuse_filter(
                        filter::FilterError::Unsupported(
                            "filtering groups by externalId is not supported".to_owned(),
                        ),
                    ));
                }
            };
            let total = i64::from(found.is_some());
            (found.into_iter().collect::<Vec<_>>(), total)
        }
        None => {
            let (start, count) = page_bounds(query.start_index(), query.count());
            let total = directory::count_groups(&mut *tx, tenant_id).await?;
            (
                directory::groups(&mut *tx, tenant_id, start - 1, count).await?,
                total,
            )
        }
    };

    let mut resources = Vec::with_capacity(groups.len());
    for group in &groups {
        let members = directory::members_of(&mut *tx, tenant_id, group.id).await?;
        resources.push(wire::GroupResource::of(group, &members, &base));
    }
    let start = page_bounds(query.start_index(), query.count()).0;
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
    let group = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    let members = directory::members_of(&mut *tx, tenant_id, id).await?;
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
    let display_name = body.display_name.clone().ok_or_else(|| {
        ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidValue",
            "displayName is required",
        )
    })?;
    let tenant_id = auth.tenant.id;
    let members = member_ids(&body.members)?;

    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let group = directory::create_group(
        &mut *tx,
        DirectoryGroupId::new(),
        tenant_id,
        body.external_id.as_deref(),
        &display_name,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?;
    directory::replace_members(&mut tx, tenant_id, group.id, &members).await?;
    project(&mut tx, tenant_id, &group).await?;
    tx.commit().await.map_err(commit_error)?;

    reconcile_members(&state, &auth, &members).await?;
    respond(&state, &auth, group.id, StatusCode::CREATED).await
}

/// `PUT /Groups/{id}` — Okta's membership replace.
pub async fn replace(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::GroupResource>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let members = member_ids(&body.members)?;

    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    let display_name = body.display_name.clone().unwrap_or(current.display_name);
    directory::rename_group(
        &mut *tx,
        tenant_id,
        id,
        body.external_id.as_deref(),
        &display_name,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?
    .ok_or_else(ScimError::not_found)?;
    // Everybody who was in the group before is re-reconciled too: a
    // replace that dropped somebody changed their placement as surely as
    // one that added somebody.
    let previously = directory::members_of(&mut *tx, tenant_id, id).await?;
    directory::replace_members(&mut tx, tenant_id, id, &members).await?;
    let group = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    project(&mut tx, tenant_id, &group).await?;
    tx.commit().await.map_err(commit_error)?;

    let mut touched = members.clone();
    touched.extend(previously.iter().map(|user| user.id));
    touched.sort_unstable();
    touched.dedup();
    reconcile_members(&state, &auth, &touched).await?;
    respond(&state, &auth, id, StatusCode::OK).await
}

/// `PATCH /Groups/{id}` — the membership shape both clients send.
pub async fn patch(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::PatchRequest>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut touched: Vec<DirectoryUserId> = Vec::new();

    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;

    for operation in &body.operations {
        let op = operation.op.to_ascii_lowercase();
        let path = operation.path.clone().unwrap_or_default();
        let lowered = path.to_ascii_lowercase();

        // Entra's removal shape: the member id is in the path filter
        // rather than in the value. It is the one place complex-attribute
        // filtering appears in a request this server must honour.
        if op == "remove"
            && let Some(value) = filter::member_value_path(&path)
        {
            let member = value
                .parse::<DirectoryUserId>()
                .map_err(|_| ScimError::not_found())?;
            directory::remove_member(&mut *tx, tenant_id, id, member).await?;
            touched.push(member);
            continue;
        }

        match lowered.as_str() {
            "members" => {
                let members = member_ids(&parse_members(operation.value.as_ref())?)?;
                touched.extend(members.iter().copied());
                match op.as_str() {
                    "add" => {
                        for member in &members {
                            directory::add_member(&mut *tx, tenant_id, id, *member).await?;
                        }
                    }
                    "remove" if members.is_empty() => {
                        // `remove` with no value clears the membership.
                        let previously = directory::members_of(&mut *tx, tenant_id, id).await?;
                        touched.extend(previously.iter().map(|user| user.id));
                        directory::replace_members(&mut tx, tenant_id, id, &[]).await?;
                    }
                    "remove" => {
                        for member in &members {
                            directory::remove_member(&mut *tx, tenant_id, id, *member).await?;
                        }
                    }
                    _ => {
                        let previously = directory::members_of(&mut *tx, tenant_id, id).await?;
                        touched.extend(previously.iter().map(|user| user.id));
                        directory::replace_members(&mut tx, tenant_id, id, &members).await?;
                    }
                }
            }
            "displayname" => {
                let name = operation
                    .value
                    .as_ref()
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        ScimError::typed(
                            StatusCode::BAD_REQUEST,
                            "invalidValue",
                            "displayName must be a string",
                        )
                    })?;
                directory::rename_group(
                    &mut *tx,
                    tenant_id,
                    id,
                    current.external_id.as_deref(),
                    name,
                )
                .await
                .map_err(|error| ScimError::from_taxonomy(&error))?;
                // The name is the mapping key, so renaming a group
                // re-resolves placement for everybody in it.
                let members = directory::members_of(&mut *tx, tenant_id, id).await?;
                touched.extend(members.iter().map(|user| user.id));
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
    // Once, after every operation in the request: the projection is a
    // replacement, so applying it per operation would be the same answer
    // several times over.
    let patched = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    project(&mut tx, tenant_id, &patched).await?;
    tx.commit().await.map_err(commit_error)?;

    touched.sort_unstable();
    touched.dedup();
    reconcile_members(&state, &auth, &touched).await?;
    respond(&state, &auth, id, StatusCode::OK).await
}

/// `DELETE /Groups/{id}`.
///
/// A group really is deleted, unlike a person: it carries no governed
/// material and the directory is its only author (ADR-0059 decision 2).
/// Its members are re-reconciled, because losing a group is losing a
/// mapping — which is quarantine, not departure (decision 11).
pub async fn delete(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let members = directory::members_of(&mut *tx, tenant_id, id).await?;
    if !directory::delete_group(&mut *tx, tenant_id, id).await? {
        return Err(ScimError::not_found());
    }
    // The governed group is **archived**, not deleted: a grant may name it,
    // and a grant naming a row that stopped existing is one nobody can review.
    // An archived group resolves to nobody, so the access goes on the very
    // next request (ADR-0073 decision 9).
    access::retire_directory_group(&mut tx, tenant_id, &id.to_string()).await?;
    tx.commit().await.map_err(commit_error)?;

    let touched: Vec<DirectoryUserId> = members.iter().map(|user| user.id).collect();
    reconcile_members(&state, &auth, &touched).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Mirrors one directory group onto the governed access model.
///
/// The slug is derived from the directory's own id rather than from the
/// display name, and that is deliberate: a slug is immutable here
/// (`groups_slug_unique`), a display name is not, and a directory that renames
/// a group must rename it rather than orphan it and mint a second. The
/// **display name** is what carries the human-readable name, and it is exactly
/// what the AUTH-2 mapping resolver already reads.
///
/// Members are the subjects of the identities the directory's users have
/// reconciled into. A directory user with no identity yet contributes nothing —
/// see the module docs.
async fn project(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: synveda_types::TenantId,
    group: &synveda_types::DirectoryGroup,
) -> Result<(), ScimError> {
    let users = directory::members_of(&mut **tx, tenant_id, group.id).await?;
    let mut subjects: Vec<String> = Vec::with_capacity(users.len());
    for user in &users {
        let Some(identity_id) = user.identity_id else {
            continue;
        };
        // A provisioned identity may still carry no subject: a directory can
        // create somebody who has never logged in, and a principal *is* a
        // verified token subject. They join the group on the sync after their
        // first login rather than being invented one here.
        if let Some(identity) = identities::by_id(&mut **tx, tenant_id, identity_id).await?
            && let Some(subject) = identity.subject
        {
            subjects.push(subject);
        }
    }
    subjects.sort_unstable();
    subjects.dedup();
    access::sync_directory_group(
        tx,
        tenant_id,
        &group.id.to_string(),
        &directory_slug(group.id),
        &group.display_name,
        &subjects,
    )
    .await?;
    Ok(())
}

/// A stable, typeable slug for a directory group: `dir-` plus its id.
///
/// Not the display name, for [`project`]'s reason, and not a digest, because a
/// group is a thing an administrator reads in a listing and a directory id is
/// what they will search their IdP by.
fn directory_slug(id: DirectoryGroupId) -> String {
    format!("dir-{id}")
}

/// Re-projects every member a membership change touched.
async fn reconcile_members(
    state: &AppState,
    auth: &ScimAuth,
    members: &[DirectoryUserId],
) -> Result<(), ScimError> {
    for member in members {
        let Some(user) = directory::user(&state.pool, auth.tenant.id, *member).await? else {
            continue;
        };
        reconcile::reconcile(
            state,
            &auth.tenant,
            reconcile::DirectorySource::Scim(auth.credential.id),
            &user,
        )
        .await?;
    }
    Ok(())
}

/// Re-reads a group and renders it.
async fn respond(
    state: &AppState,
    auth: &ScimAuth,
    id: DirectoryGroupId,
    status: StatusCode,
) -> Result<Response, ScimError> {
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let group = directory::group(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    let members = directory::members_of(&mut *tx, tenant_id, id).await?;
    Ok(ScimJson(
        status,
        wire::GroupResource::of(&group, &members, &base_url(state)),
    )
    .into_response())
}

/// The member ids in a `members` array, refusing one that is not an id
/// this server minted.
fn member_ids(members: &[wire::MultiValue]) -> Result<Vec<DirectoryUserId>, ScimError> {
    members
        .iter()
        .filter_map(|member| member.value.as_deref())
        .map(|value| {
            value.parse::<DirectoryUserId>().map_err(|_| {
                ScimError::typed(
                    StatusCode::BAD_REQUEST,
                    "invalidValue",
                    format!("`{value}` is not a member id this server issued"),
                )
            })
        })
        .collect()
}

/// A patch value that is a `members` array, in either of the two shapes
/// clients send it: the array itself, or a single member object.
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

fn parse_id(id: &str) -> Result<DirectoryGroupId, ScimError> {
    id.parse::<DirectoryGroupId>()
        .map_err(|_| ScimError::not_found())
}

fn commit_error(err: sqlx::Error) -> ScimError {
    ScimError::from_taxonomy(&synveda_types::Error::Storage {
        message: format!("commit group write: {err}"),
    })
}

/// Unused import guard: `DirectoryUser` is the type `members_of` returns
/// and is named in this module's signatures through inference only.
#[allow(dead_code)]
fn _member_type(user: &DirectoryUser) -> DirectoryUserId {
    user.id
}
