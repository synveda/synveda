//! Directory user adapter state and provisioning credentials.
//!
//! Two things live here and they are different in kind. The **user adapter
//! row** (`scim_users`) is mutable because the directory is its author and
//! SCIM must round-trip declared attributes. Groups and memberships are the
//! shared access aggregates in [`crate::access`], not another mirror. The
//! **credential**
//! (`scim_credentials`) is append-and-stamp: issued, used, revoked, never
//! deleted, because which credential sealed which identity has to stay
//! answerable after the credential is gone.
//!
//! Everything here is tenant-scoped (forced RLS, ADR-0009): reach it inside
//! [`crate::rls::begin_tenant_tx`]. That includes the credential lookup,
//! which is why the presented token names its tenant — the caller names it,
//! the secret proves it, and the row is found under that tenant's own
//! policy or not at all (migration 0036's amendment to decision 13).
//!
//! Nothing here is governed material. Identity and access projection is the
//! gateway reconciler's seam, and all resulting authority is decided through
//! the same scope grants and PDP as manually managed access (CPR-34,
//! ADR-0093).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    DirectoryUser, DirectoryUserId, Error, IdentityId, Result, ScimCredential, ScimCredentialId,
    TenantId,
};
use uuid::Uuid;

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation: a `userName` already live, an
        // `externalId` already anchored, or a membership row already
        // there. RFC 7644 §3.3 wants 409 for the first two, which
        // `Conflict` becomes at the SCIM error seam.
        if matches!(db.code().as_deref(), Some("23505") | Some("23503")) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Visible to the crate so [`crate::directory_sync`] can select mirror rows
/// into the same shape rather than keeping a second copy of thirteen columns
/// and the conversion below — two structs for one table is how they drift.
pub(crate) struct UserRow {
    pub(crate) id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) directory_source: String,
    pub(crate) external_id: Option<String>,
    pub(crate) user_name: String,
    pub(crate) active: bool,
    pub(crate) display_name: Option<String>,
    pub(crate) given_name: Option<String>,
    pub(crate) family_name: Option<String>,
    pub(crate) work_email: Option<String>,
    pub(crate) identity_id: Option<Uuid>,
    pub(crate) version: i64,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<UserRow> for DirectoryUser {
    fn from(row: UserRow) -> Self {
        DirectoryUser {
            id: DirectoryUserId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            directory_source: row.directory_source,
            external_id: row.external_id,
            user_name: row.user_name,
            active: row.active,
            display_name: row.display_name,
            given_name: row.given_name,
            family_name: row.family_name,
            work_email: row.work_email,
            identity_id: row.identity_id.map(IdentityId::from_uuid),
            version: row.version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

struct CredentialRow {
    id: Uuid,
    tenant_id: Uuid,
    label: String,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    created_by: String,
}

impl From<CredentialRow> for ScimCredential {
    fn from(row: CredentialRow) -> Self {
        ScimCredential {
            id: ScimCredentialId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            label: row.label,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            last_used_at: row.last_used_at,
            created_at: row.created_at,
            created_by: row.created_by,
        }
    }
}

// ── Users ───────────────────────────────────────────────────────────────

/// The attributes a create or replace carries. One struct rather than nine
/// arguments, and it is deliberately *not* [`DirectoryUser`]: a client
/// supplies attributes, never a version, a timestamp or an identity link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAttributes {
    /// Stable adapter/provider key (`scim`, `entra`, `okta`, …).
    pub directory_source: String,
    /// `externalId`, when the directory sends one.
    pub external_id: Option<String>,
    /// `userName`.
    pub user_name: String,
    /// `active`.
    pub active: bool,
    /// `displayName`.
    pub display_name: Option<String>,
    /// `name.givenName`.
    pub given_name: Option<String>,
    /// `name.familyName`.
    pub family_name: Option<String>,
    /// `emails[type eq "work"].value`.
    pub work_email: Option<String>,
}

/// Inserts a mirror row for a person the directory has just created.
#[tracing::instrument(
    name = "store.directory.create_user",
    skip_all,
    fields(tenant.id = %tenant_id, directory.user = %id),
    err(Display)
)]
pub async fn create_user(
    executor: impl PgExecutor<'_>,
    id: DirectoryUserId,
    tenant_id: TenantId,
    attributes: &UserAttributes,
) -> Result<DirectoryUser> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        insert into scim_users
            (id, tenant_id, directory_source, external_id, user_name, active, display_name,
             given_name, family_name, work_email)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        returning id, tenant_id, directory_source, external_id, user_name, active, display_name,
                  given_name, family_name, work_email, identity_id, version,
                  created_at, updated_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        attributes.directory_source,
        attributes.external_id,
        attributes.user_name,
        attributes.active,
        attributes.display_name,
        attributes.given_name,
        attributes.family_name,
        attributes.work_email,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// Replaces a mirror row's attributes, bumping its ETag. Returns `None`
/// for an unknown row — the uniform 404 every resource route answers with.
#[tracing::instrument(
    name = "store.directory.replace_user",
    skip_all,
    fields(tenant.id = %tenant_id, directory.user = %id),
    err(Display)
)]
pub async fn replace_user(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: DirectoryUserId,
    attributes: &UserAttributes,
) -> Result<Option<DirectoryUser>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        update scim_users
        set external_id = $4, user_name = $5, active = $6, display_name = $7,
            given_name = $8, family_name = $9, work_email = $10,
            version = version + 1, updated_at = now()
        where tenant_id = $1 and id = $2 and directory_source = $3
        returning id, tenant_id, directory_source, external_id, user_name, active, display_name,
                  given_name, family_name, work_email, identity_id, version,
                  created_at, updated_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        attributes.directory_source,
        attributes.external_id,
        attributes.user_name,
        attributes.active,
        attributes.display_name,
        attributes.given_name,
        attributes.family_name,
        attributes.work_email,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Points a mirror row at the identity it projected onto. Written once per
/// row, by the reconciler and by nothing else.
#[tracing::instrument(
    name = "store.directory.link_identity",
    skip_all,
    fields(tenant.id = %tenant_id, directory.user = %id, identity.id = %identity_id),
    err(Display)
)]
pub async fn link_identity(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    id: DirectoryUserId,
    identity_id: IdentityId,
) -> Result<()> {
    sqlx::query!(
        r#"
        update scim_users set identity_id = $3, updated_at = now()
        where tenant_id = $1 and id = $2 and directory_source = $4
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        identity_id.as_uuid(),
        directory_source,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// One mirror row by id.
#[tracing::instrument(
    name = "store.directory.user",
    skip_all,
    fields(tenant.id = %tenant_id, directory.user = %id),
    err(Display)
)]
pub async fn user(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    id: DirectoryUserId,
) -> Result<Option<DirectoryUser>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users where tenant_id = $1 and directory_source = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        directory_source,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// The mirror row that projected onto `identity_id`, if any — the login
/// path's join, and the reconciler's way back from a person to their
/// directory row.
#[tracing::instrument(
    name = "store.directory.user_for_identity",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %identity_id),
    err(Display)
)]
pub async fn user_for_identity(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    identity_id: IdentityId,
) -> Result<Option<DirectoryUser>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users where tenant_id = $1 and identity_id = $2
        "#,
        tenant_id.as_uuid(),
        identity_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// The live mirror row anchored at `external_id` — reconciliation's first
/// match, and the login path's first (ADR-0059 decision 4).
#[tracing::instrument(
    name = "store.directory.user_by_external_id",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn user_by_external_id(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    external_id: &str,
) -> Result<Option<DirectoryUser>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users
        where tenant_id = $1 and directory_source = $2 and external_id = $3 and active
        "#,
        tenant_id.as_uuid(),
        directory_source,
        external_id,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Finds an active external id only when it identifies exactly one provider
/// row in the tenant. Login claims do not name the provisioning adapter; an
/// ambiguous anchor must therefore fail closed instead of adopting whichever
/// provider happens to sort first.
pub async fn unique_user_by_external_id(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    external_id: &str,
) -> Result<Option<DirectoryUser>> {
    let mut rows = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users
        where tenant_id = $1 and external_id = $2 and active
        order by directory_source
        limit 2
        "#,
        tenant_id.as_uuid(),
        external_id,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    if rows.len() == 1 {
        Ok(rows.pop().map(Into::into))
    } else {
        Ok(None)
    }
}

/// The live mirror row with this `userName`, case-insensitively — the
/// filter both AC clients send, and reconciliation's last match.
#[tracing::instrument(
    name = "store.directory.user_by_user_name",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn user_by_user_name(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    user_name: &str,
) -> Result<Option<DirectoryUser>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users
        where tenant_id = $1 and directory_source = $2
          and lower(user_name) = lower($3) and active
        "#,
        tenant_id.as_uuid(),
        directory_source,
        user_name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Finds an active case-insensitive user name only when exactly one provider
/// owns it in the tenant. This is the login correspondence fallback.
pub async fn unique_user_by_user_name(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    user_name: &str,
) -> Result<Option<DirectoryUser>> {
    let mut rows = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users
        where tenant_id = $1 and lower(user_name) = lower($2) and active
        order by directory_source
        limit 2
        "#,
        tenant_id.as_uuid(),
        user_name,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    if rows.len() == 1 {
        Ok(rows.pop().map(Into::into))
    } else {
        Ok(None)
    }
}

/// A page of mirror rows in stable id order — the unfiltered list, whose
/// bound is the caller's `count` clamped by the route.
#[tracing::instrument(
    name = "store.directory.users",
    skip_all,
    fields(tenant.id = %tenant_id, page.offset = offset, page.limit = limit),
    err(Display)
)]
pub async fn users(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    offset: i64,
    limit: i64,
) -> Result<Vec<DirectoryUser>> {
    let rows = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users where tenant_id = $1 and directory_source = $2
        order by id
        offset $3 limit $4
        "#,
        tenant_id.as_uuid(),
        directory_source,
        offset,
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// How many mirror rows the tenant has — `totalResults` on an unfiltered
/// list, which RFC 7644 §3.4.2 requires to be the whole count rather than
/// the page's.
#[tracing::instrument(
    name = "store.directory.count_users",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn count_users(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
) -> Result<i64> {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from scim_users where tenant_id = $1 and directory_source = $2"#,
        tenant_id.as_uuid(),
        directory_source,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}

// ── Credentials ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────

/// Issues a provisioning credential. The caller mints and shows the secret
/// exactly once; only its SHA-256 arrives here.
#[tracing::instrument(
    name = "store.directory.issue_credential",
    skip_all,
    fields(tenant.id = %tenant_id, scim.credential = %id),
    err(Display)
)]
pub async fn issue_credential(
    executor: impl PgExecutor<'_>,
    id: ScimCredentialId,
    tenant_id: TenantId,
    token_hash: &[u8],
    label: &str,
    expires_at: DateTime<Utc>,
    created_by: &str,
) -> Result<ScimCredential> {
    let row = sqlx::query_as!(
        CredentialRow,
        r#"
        insert into scim_credentials
            (id, tenant_id, token_hash, label, expires_at, created_by)
        values ($1, $2, $3, $4, $5, $6)
        returning id, tenant_id, label, expires_at, revoked_at, last_used_at,
                  created_at, created_by
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        token_hash,
        label,
        expires_at,
        created_by,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// The credential a presented token hashes to, inside the tenant the token
/// named. Returns the row whether or not it is usable — expiry and
/// revocation are the caller's to check through
/// [`ScimCredential::usable_at`], so that a refusal can say which it was.
#[tracing::instrument(
    name = "store.directory.credential_by_hash",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn credential_by_hash(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    token_hash: &[u8],
) -> Result<Option<ScimCredential>> {
    let row = sqlx::query_as!(
        CredentialRow,
        r#"
        select id, tenant_id, label, expires_at, revoked_at, last_used_at,
               created_at, created_by
        from scim_credentials where tenant_id = $1 and token_hash = $2
        "#,
        tenant_id.as_uuid(),
        token_hash,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Stamps a credential as used, on a coarse cadence: only when the
/// recorded instant is more than `stale_secs` old. A provisioning agent
/// polls, and a row written on every poll turns the directory plane's read
/// path into a write path (migration 0034's rule, applied again).
#[tracing::instrument(
    name = "store.directory.touch_credential",
    skip_all,
    fields(tenant.id = %tenant_id, scim.credential = %id),
    err(Display)
)]
pub async fn touch_credential(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScimCredentialId,
    stale_secs: i32,
) -> Result<()> {
    sqlx::query!(
        r#"
        update scim_credentials set last_used_at = now()
        where tenant_id = $1 and id = $2
          and (last_used_at is null
               or last_used_at < now() - make_interval(secs => $3::int))
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        stale_secs,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Revokes a credential. Returns whether a live one was revoked, so a
/// second revoke reports honestly rather than claiming work it did not do.
#[tracing::instrument(
    name = "store.directory.revoke_credential",
    skip_all,
    fields(tenant.id = %tenant_id, scim.credential = %id),
    err(Display)
)]
pub async fn revoke_credential(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScimCredentialId,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"
        update scim_credentials set revoked_at = now()
        where tenant_id = $1 and id = $2 and revoked_at is null
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// The tenant's credentials, newest first — what `synveda scim token list`
/// renders, revoked and expired ones included, because rotation is a
/// decision about a history rather than about a current state.
#[tracing::instrument(
    name = "store.directory.credentials",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn credentials(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Vec<ScimCredential>> {
    let rows = sqlx::query_as!(
        CredentialRow,
        r#"
        select id, tenant_id, label, expires_at, revoked_at, last_used_at,
               created_at, created_by
        from scim_credentials where tenant_id = $1
        order by created_at desc
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}
