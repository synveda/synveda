//! The identity store (AUTH-2, ADR-0013; AUTH-3, ADR-0018): one row per
//! provisioned subject, bound to its personal user-kind scope node. Users
//! arrive through JIT provisioning; service identities through explicit
//! registration (ADR-0018 decision 2) — same table, same placement shape.
//!
//! `quarantined` is derived from placement in every read — the identity's
//! node sits directly under the tenant's reserved `quarantine` scope (the
//! org root's child with that slug) — never stored (ADR-0013 decision 4).
//! `identities` is tenant-scoped (forced RLS, ADR-0009): reach it inside
//! [`crate::rls::begin_tenant_tx`].
//!
//! AUD-1 wiring point: identity creation (`identity.provisioned`) and
//! service-identity registration/removal are audit emission points; until
//! the hash-chained log lands they are visible in the gateway's
//! `identity.provision` / `service_identity.*` spans and their counters.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{Error, Identity, IdentityId, IdentityKind, Result, ScopeId, TenantId};
use uuid::Uuid;

/// The reserved slug of the quarantine scope: the org root's child that
/// unmapped users' personal scopes are created under (ADR-0013 decision 4).
pub const QUARANTINE_SLUG: &str = "quarantine";

struct IdentityRow {
    id: Uuid,
    tenant_id: Uuid,
    subject: String,
    kind: String,
    email: Option<String>,
    display_name: Option<String>,
    scope_id: Uuid,
    quarantined: bool,
    created_at: DateTime<Utc>,
}

impl TryFrom<IdentityRow> for Identity {
    type Error = Error;

    fn try_from(row: IdentityRow) -> Result<Self> {
        // The check constraint pins the column to the vocabulary; a value
        // outside it means out-of-band writes and surfaces as Internal.
        let kind: IdentityKind = row.kind.parse().map_err(|_| Error::Internal {
            message: format!("identity {} has unknown kind {:?}", row.id, row.kind),
        })?;
        Ok(Identity {
            id: IdentityId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            subject: row.subject,
            kind,
            email: row.email,
            display_name: row.display_name,
            scope_id: ScopeId::from_uuid(row.scope_id),
            quarantined: row.quarantined,
            created_at: row.created_at,
        })
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation (subject or scope already bound — the JIT
        // first-login race), 23503 foreign_key_violation (scope vanished
        // under a concurrent delete): conflicts with concurrent state,
        // retryable by the caller.
        if matches!(db.code().as_deref(), Some("23505") | Some("23503")) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 23514 check_violation: subject outside the length bound.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Fetches the identity provisioned for `subject` in `tenant_id`, if any.
/// Quarantine derives from placement: the identity's node's parent is the
/// org root's child (depth 1) with the reserved slug (ADR-0013 decision 4).
#[tracing::instrument(
    name = "store.identities.by_subject",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn by_subject(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
) -> Result<Option<Identity>> {
    let row = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.created_at,
               coalesce(p.slug = 'quarantine' and p.depth = 1, false)
                   as "quarantined!"
        from identities i
        join hierarchy_nodes n on n.id = i.scope_id
        left join hierarchy_nodes p on p.id = n.parent_id
        where i.tenant_id = $1 and i.subject = $2
        "#,
        tenant_id.as_uuid(),
        subject,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetches an identity by id — the service-identity routes' uniform-404
/// lookup (AUTH-3, ADR-0018 decision 3).
#[tracing::instrument(
    name = "store.identities.by_id",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id),
    err(Display)
)]
pub async fn by_id(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: IdentityId,
) -> Result<Option<Identity>> {
    let row = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.created_at,
               coalesce(p.slug = 'quarantine' and p.depth = 1, false)
                   as "quarantined!"
        from identities i
        join hierarchy_nodes n on n.id = i.scope_id
        left join hierarchy_nodes p on p.id = n.parent_id
        where i.tenant_id = $1 and i.id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists the tenant's service identities, subject-ordered for a stable
/// surface (AUTH-3, ADR-0018 decision 3).
#[tracing::instrument(
    name = "store.identities.services",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn services(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<Identity>> {
    let rows = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.created_at,
               coalesce(p.slug = 'quarantine' and p.depth = 1, false)
                   as "quarantined!"
        from identities i
        join hierarchy_nodes n on n.id = i.scope_id
        left join hierarchy_nodes p on p.id = n.parent_id
        where i.tenant_id = $1 and i.kind = 'service'
        order by i.subject
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Deletes a service identity row. Deliberately keyed on
/// `kind = 'service'`: user rows have no delete path until AUTH-4/5 own
/// leavers (migration 0007's note). Returns whether a row was deleted.
/// The caller deletes the personal node in the same transaction
/// (ADR-0018 decision 2).
///
/// AUD-1 wiring point: service-identity removal is an audit emission
/// point.
#[tracing::instrument(
    name = "store.identities.delete_service",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id),
    err(Display)
)]
pub async fn delete_service(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: IdentityId,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"
        delete from identities
        where tenant_id = $1 and id = $2 and kind = 'service'
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// Provisions an identity bound to `scope_id` (the already-created personal
/// user node), returning it with its placement-derived quarantine status.
/// Fails with [`Error::Conflict`] when the subject or the node is already
/// bound — the JIT first-login race (users), or a subject collision
/// (service registration; ADR-0018 decision 3). The JIT caller retries and
/// adopts the winner's row (ADR-0013 decision 2).
///
/// Takes a connection (insert + derive-read); callers wrap it in the same
/// transaction that created the node.
#[tracing::instrument(
    name = "store.identities.create",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id, scope.id = %scope_id, identity.kind = %kind),
    err(Display)
)]
#[allow(clippy::too_many_arguments)] // the row's own columns, nothing more
pub async fn create(
    conn: &mut PgConnection,
    id: IdentityId,
    tenant_id: TenantId,
    subject: &str,
    kind: IdentityKind,
    email: Option<&str>,
    display_name: Option<&str>,
    scope_id: ScopeId,
) -> Result<Identity> {
    sqlx::query!(
        r#"
        insert into identities (id, tenant_id, subject, kind, email, display_name, scope_id)
        values ($1, $2, $3, $4, $5, $6, $7)
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        subject,
        kind.as_str(),
        email,
        display_name,
        scope_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    // Read back through the one quarantine-derivation query rather than
    // duplicating the join logic here.
    by_subject(&mut *conn, tenant_id, subject)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("identity {id} vanished mid-provision"),
        })
}
