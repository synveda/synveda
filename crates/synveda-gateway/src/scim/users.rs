//! `/scim/v2/Users` (AUTH-4, ADR-0059).
//!
//! Every mutation here writes the mirror and then calls the reconciler,
//! in that order and never the other way round: the directory's statement
//! is the record, and what the product makes of it is derived. A
//! reconciliation that failed would leave the mirror ahead of the
//! projection, which the next PATCH or the AUTH-5 sweep converges — where
//! the reverse would leave a person placed by a statement nobody made.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use synveda_store::directory::{self, UserAttributes};
use synveda_types::{DirectoryUser, DirectoryUserId};

use super::{ScimAuth, ScimError, ScimJson, base_url, filter, page_bounds, reconcile, wire};
use crate::app::AppState;

/// `?filter=`, `?startIndex=`, `?count=` — RFC 7644 §3.4.2.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    filter: Option<String>,
    #[serde(rename = "startIndex", default)]
    start_index: Option<i64>,
    #[serde(default)]
    count: Option<i64>,
}

impl ListQuery {
    /// The `?filter=` text, if the caller sent one.
    pub(super) fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// The caller's 1-based `startIndex`, unclamped.
    pub(super) fn start_index(&self) -> Option<i64> {
        self.start_index
    }

    /// The caller's requested page size, unclamped.
    pub(super) fn count(&self) -> Option<i64> {
        self.count
    }
}

/// `GET /Users` — a filtered lookup or a page.
pub async fn list(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ScimError> {
    let base = base_url(&state);
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;

    let (users, total) = match query.filter() {
        Some(text) => {
            let parsed = filter::parse(text, filter::USER_FILTERABLE).map_err(refuse_filter)?;
            let found = match parsed.attribute.as_str() {
                "username" => {
                    directory::user_by_user_name(&mut *tx, tenant_id, &parsed.value).await?
                }
                "externalid" => {
                    directory::user_by_external_id(&mut *tx, tenant_id, &parsed.value).await?
                }
                // An id that is not a uuid is not a resource this server
                // ever minted, so it matches nothing — which is a correct
                // empty list rather than a 400.
                _ => match parsed.value.parse::<DirectoryUserId>() {
                    Ok(id) => directory::user(&mut *tx, tenant_id, id).await?,
                    Err(_) => None,
                },
            };
            let total = i64::from(found.is_some());
            (found.into_iter().collect::<Vec<_>>(), total)
        }
        None => {
            let (start, count) = page_bounds(query.start_index(), query.count());
            let total = directory::count_users(&mut *tx, tenant_id).await?;
            (
                directory::users(&mut *tx, tenant_id, start - 1, count).await?,
                total,
            )
        }
    };

    let start = page_bounds(query.start_index(), query.count()).0;
    let resources: Vec<wire::UserResource> = users
        .iter()
        .map(|user| wire::UserResource::of(user, &base))
        .collect();
    Ok(ScimJson(
        StatusCode::OK,
        wire::ListResponse::new(resources, total, start),
    )
    .into_response())
}

/// `GET /Users/{id}`.
pub async fn get(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, auth.tenant.id).await?;
    let user = directory::user(&mut *tx, auth.tenant.id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    Ok(ScimJson(
        StatusCode::OK,
        wire::UserResource::of(&user, &base_url(&state)),
    )
    .into_response())
}

/// `POST /Users` — the joiner.
pub async fn create(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Json(body): Json<wire::UserResource>,
) -> Result<Response, ScimError> {
    let attributes = attributes_of(&body)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let created = directory::create_user(&mut *tx, DirectoryUserId::new(), tenant_id, &attributes)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?;
    tx.commit().await.map_err(|err| {
        ScimError::from_taxonomy(&synveda_types::Error::Storage {
            message: format!("commit user create: {err}"),
        })
    })?;

    let user = project(&state, &auth, created).await?;
    Ok(ScimJson(
        StatusCode::CREATED,
        wire::UserResource::of(&user, &base_url(&state)),
    )
    .into_response())
}

/// `PUT /Users/{id}` — a whole-resource replace (Okta's update shape).
pub async fn replace(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::UserResource>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let attributes = attributes_of(&body)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let replaced = directory::replace_user(&mut *tx, tenant_id, id, &attributes)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?
        .ok_or_else(ScimError::not_found)?;
    tx.commit().await.map_err(|err| {
        ScimError::from_taxonomy(&synveda_types::Error::Storage {
            message: format!("commit user replace: {err}"),
        })
    })?;

    let user = project(&state, &auth, replaced).await?;
    Ok(ScimJson(
        StatusCode::OK,
        wire::UserResource::of(&user, &base_url(&state)),
    )
    .into_response())
}

/// `PATCH /Users/{id}` — the shape Entra sends for everything.
pub async fn patch(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
    Json(body): Json<wire::PatchRequest>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = directory::user(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;

    let mut attributes = current_attributes(&current);
    for operation in &body.operations {
        apply_operation(&mut attributes, operation)?;
    }
    let patched = directory::replace_user(&mut *tx, tenant_id, id, &attributes)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?
        .ok_or_else(ScimError::not_found)?;
    tx.commit().await.map_err(|err| {
        ScimError::from_taxonomy(&synveda_types::Error::Storage {
            message: format!("commit user patch: {err}"),
        })
    })?;

    let user = project(&state, &auth, patched).await?;
    Ok(ScimJson(
        StatusCode::OK,
        wire::UserResource::of(&user, &base_url(&state)),
    )
    .into_response())
}

/// `DELETE /Users/{id}` — **which seals and does not delete** (ADR-0059
/// decision 11).
///
/// The response is RFC 7644's `204` and the resource then answers `404`,
/// so a conformant client sees exactly what the protocol promises. What
/// does not happen is a deletion: the mirror row is retained and marked,
/// because the personal scope behind it is under a retention hold and the
/// one operation a hold exists to prevent is this one.
pub async fn delete(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<ScimAuth>,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let id = parse_id(&id)?;
    let tenant_id = auth.tenant.id;
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let current = directory::user(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(ScimError::not_found)?;
    let mut attributes = current_attributes(&current);
    attributes.active = false;
    let deactivated = directory::replace_user(&mut *tx, tenant_id, id, &attributes)
        .await?
        .ok_or_else(ScimError::not_found)?;
    tx.commit().await.map_err(|err| {
        ScimError::from_taxonomy(&synveda_types::Error::Storage {
            message: format!("commit user delete: {err}"),
        })
    })?;
    project(&state, &auth, deactivated).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Runs the reconciler and re-reads the mirror row it may have linked.
async fn project(
    state: &AppState,
    auth: &ScimAuth,
    user: DirectoryUser,
) -> Result<DirectoryUser, ScimError> {
    reconcile::reconcile(state, &auth.tenant, auth.credential.id, &user).await?;
    Ok(directory::user(&state.pool, auth.tenant.id, user.id)
        .await
        .ok()
        .flatten()
        .unwrap_or(user))
}

/// The attributes a body carries, refusing a create with no `userName` —
/// RFC 7643 §4.1.1 makes it the one required attribute, and a mirror row
/// without one could never be matched by anything.
fn attributes_of(body: &wire::UserResource) -> Result<UserAttributes, ScimError> {
    let user_name = body.user_name.clone().ok_or_else(|| {
        ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidValue",
            "userName is required",
        )
    })?;
    Ok(UserAttributes {
        external_id: body.external_id.clone(),
        user_name,
        // Absent means active: RFC 7643 defaults it, and reading an
        // omission as `false` would seal everybody Okta ever created.
        active: body.active.unwrap_or(true),
        display_name: body.display_name.clone(),
        given_name: body.name.as_ref().and_then(|name| name.given_name.clone()),
        family_name: body.name.as_ref().and_then(|name| name.family_name.clone()),
        work_email: body.work_email(),
    })
}

/// A stored row's attributes, as the base a PATCH mutates.
fn current_attributes(user: &DirectoryUser) -> UserAttributes {
    UserAttributes {
        external_id: user.external_id.clone(),
        user_name: user.user_name.clone(),
        active: user.active,
        display_name: user.display_name.clone(),
        given_name: user.given_name.clone(),
        family_name: user.family_name.clone(),
        work_email: user.work_email.clone(),
    }
}

/// Applies one PATCH operation to a user's attributes.
///
/// The paths here are the ones Entra and Okta send, and no others. A path
/// outside the set is `400 invalidPath` rather than a silent no-op: a
/// provisioning agent that believes it deactivated somebody must not be
/// told it succeeded.
fn apply_operation(
    attributes: &mut UserAttributes,
    operation: &wire::PatchOperation,
) -> Result<(), ScimError> {
    let invalid_path = |path: &str| {
        ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidPath",
            format!("`{path}` is not a patchable path on User"),
        )
    };
    let op = operation.op.to_ascii_lowercase();
    if op != "add" && op != "replace" && op != "remove" {
        return Err(ScimError::typed(
            StatusCode::BAD_REQUEST,
            "invalidSyntax",
            format!("`{}` is not a patch operation", operation.op),
        ));
    }

    match operation.path.as_deref() {
        // Entra's "no path" form: the value is an object of attributes.
        None => {
            let object = operation
                .value
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| {
                    ScimError::typed(
                        StatusCode::BAD_REQUEST,
                        "invalidValue",
                        "a pathless patch operation needs an object value",
                    )
                })?;
            for (key, value) in object {
                assign(attributes, key, value).map_err(|_| invalid_path(key))?;
            }
            Ok(())
        }
        Some(path) => {
            let value = operation.value.clone().unwrap_or(serde_json::Value::Null);
            // `remove` on a scalar means "unset", which for `active` is
            // the same as `false` and for the rest is a null.
            let value = if op == "remove" {
                serde_json::Value::Null
            } else {
                value
            };
            assign(attributes, path, &value).map_err(|_| invalid_path(path))
        }
    }
}

/// Assigns one attribute by its SCIM path. `Err(())` means "no such
/// patchable path", which the caller renders.
fn assign(
    attributes: &mut UserAttributes,
    path: &str,
    value: &serde_json::Value,
) -> Result<(), ()> {
    let text = || value.as_str().map(str::to_owned);
    match path.to_ascii_lowercase().as_str() {
        "active" => {
            // Entra sends `false`; some clients send `"False"`. Both mean
            // the same thing to the person being deactivated.
            attributes.active = match value {
                serde_json::Value::Bool(flag) => *flag,
                serde_json::Value::String(text) => text.eq_ignore_ascii_case("true"),
                serde_json::Value::Null => false,
                _ => return Err(()),
            };
            Ok(())
        }
        "username" => {
            attributes.user_name = text().ok_or(())?;
            Ok(())
        }
        "externalid" => {
            attributes.external_id = text();
            Ok(())
        }
        "displayname" => {
            attributes.display_name = text();
            Ok(())
        }
        "name.givenname" => {
            attributes.given_name = text();
            Ok(())
        }
        "name.familyname" => {
            attributes.family_name = text();
            Ok(())
        }
        "name" => {
            let name: wire::Name = serde_json::from_value(value.clone()).map_err(|_| ())?;
            attributes.given_name = name.given_name;
            attributes.family_name = name.family_name;
            Ok(())
        }
        "emails" => {
            let emails: Vec<wire::MultiValue> =
                serde_json::from_value(value.clone()).map_err(|_| ())?;
            let resource = wire::UserResource {
                emails,
                ..wire::UserResource::default()
            };
            attributes.work_email = resource.work_email();
            Ok(())
        }
        _ => Err(()),
    }
}

/// A resource id, or the 404 an id this server never minted deserves.
fn parse_id(id: &str) -> Result<DirectoryUserId, ScimError> {
    id.parse::<DirectoryUserId>()
        .map_err(|_| ScimError::not_found())
}

/// The two statuses a refused filter takes (ADR-0059 decision 15): 400 for
/// something that is not a filter, 501 for a filter this server does not
/// implement — the status RFC 7644 §3.4.2.2 names for exactly that.
pub(super) fn refuse_filter(error: filter::FilterError) -> ScimError {
    match error {
        filter::FilterError::Malformed(detail) => {
            ScimError::typed(StatusCode::BAD_REQUEST, "invalidFilter", detail)
        }
        filter::FilterError::Unsupported(detail) => {
            ScimError::typed(StatusCode::NOT_IMPLEMENTED, "invalidFilter", detail)
        }
    }
}
