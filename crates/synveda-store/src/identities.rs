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
use synveda_types::{
    Error, Identity, IdentityId, IdentityKind, IdentityStatus, Result, ScopeId, TenantId,
};
use uuid::Uuid;

/// The reserved slug of the quarantine scope: the org root's child that
/// unmapped users' personal scopes are created under (ADR-0013 decision 4).
pub const QUARANTINE_SLUG: &str = "quarantine";

struct IdentityRow {
    id: Uuid,
    tenant_id: Uuid,
    subject: Option<String>,
    kind: String,
    email: Option<String>,
    display_name: Option<String>,
    scope_id: Uuid,
    quarantined: bool,
    status: String,
    departed_at: Option<DateTime<Utc>>,
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
        let status: IdentityStatus = row.status.parse().map_err(|_| Error::Internal {
            message: format!("identity {} has unknown status {:?}", row.id, row.status),
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
            status,
            departed_at: row.departed_at,
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
               i.scope_id, i.status, i.departed_at, i.created_at,
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
               i.scope_id, i.status, i.departed_at, i.created_at,
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
               i.scope_id, i.status, i.departed_at, i.created_at,
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
///
/// `subject` is `None` for an identity a directory created before its
/// person ever logged in; [`bind_subject`] binds it at that first login
/// (AUTH-4, ADR-0059 decision 5).
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
    subject: Option<&str>,
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
    // duplicating the join logic here. Keyed by id rather than subject:
    // a directory-created identity has no subject yet (AUTH-4, ADR-0059
    // decision 5).
    by_id(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("identity {id} vanished mid-provision"),
        })
}

/// Binds `subject` to an identity a directory created before its person
/// ever logged in (AUTH-4, ADR-0059 decision 5) — the login path's half of
/// the correspondence rule.
///
/// A subject belongs to at most one identity, and a **departed** row
/// yields it: without that, one rehired person could never log in again,
/// because their former self would still be holding their `sub` under the
/// subject-unique constraint. A live row never yields — the conflict
/// surfaces and the caller adopts the winner, exactly as the JIT
/// first-login race does (ADR-0013 decision 2).
///
/// AUD-1 wiring point: the gateway chains `identity.provisioned` for the
/// binding, since this is the moment the subject becomes a caller.
#[tracing::instrument(
    name = "store.identities.bind_subject",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id),
    err(Display)
)]
pub async fn bind_subject(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: IdentityId,
    subject: &str,
) -> Result<Identity> {
    sqlx::query!(
        r#"
        update identities set subject = null
        where tenant_id = $1 and subject = $2 and status = 'departed'
        "#,
        tenant_id.as_uuid(),
        subject,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    let updated = sqlx::query!(
        r#"
        update identities set subject = $3
        where tenant_id = $1 and id = $2 and subject is null and status = 'active'
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        subject,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if updated.rows_affected() == 0 {
        return Err(Error::Conflict {
            message: format!("identity {id} is not an unbound active identity"),
        });
    }
    by_id(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("identity {id} vanished mid-bind"),
        })
}

/// Seals an identity: the directory says this person is gone (AUTH-4,
/// ADR-0059 decisions 7 and 8). Returns the sealed identity, or `None`
/// when no active row matched — a second `active: false` for somebody
/// already departed is a no-op rather than an error, because a
/// provisioning agent retries and RFC 7644 has no vocabulary for "already
/// done".
///
/// The subject is deliberately **kept**. A departed row that had released
/// its subject would let the very next login re-provision the person
/// through the JIT door with a fresh personal scope and normal access —
/// the seal undone by the person it sealed. What releases it is one
/// directory-anchored successor, in [`bind_subject`].
///
/// AUD-1 wiring point: the gateway chains `identity.sealed`.
#[tracing::instrument(
    name = "store.identities.depart",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id),
    err(Display)
)]
pub async fn depart(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: IdentityId,
) -> Result<Option<Identity>> {
    let updated = sqlx::query!(
        r#"
        update identities set status = 'departed', departed_at = now()
        where tenant_id = $1 and id = $2 and status = 'active'
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    by_id(&mut *conn, tenant_id, id).await
}

/// Re-points a live identity at a freshly created personal scope — the
/// second half of a sealing move (AUTH-4, ADR-0059 decision 10).
///
/// Called with [`seal_scope_as_former_self`] in one transaction, and in
/// this order: the live row must let go of the old node before a tombstone
/// can take it, or the one-personal-scope-per-node constraint refuses.
#[tracing::instrument(
    name = "store.identities.rescope",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id, scope.id = %scope_id),
    err(Display)
)]
pub async fn rescope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: IdentityId,
    scope_id: ScopeId,
) -> Result<Identity> {
    let updated = sqlx::query!(
        r#"
        update identities set scope_id = $3
        where tenant_id = $1 and id = $2 and status = 'active'
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        scope_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if updated.rows_affected() == 0 {
        return Err(Error::Conflict {
            message: format!("identity {id} is not an active identity"),
        });
    }
    by_id(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("identity {id} vanished mid-rescope"),
        })
}

/// Leaves a **former self** on a scope somebody has moved out of, so that
/// the scope stays sealed after its owner has moved on (AUTH-4, ADR-0059
/// decision 10).
///
/// This is the shape a seal takes when the person is not leaving. Sealing
/// derives from the identity that owns a node, and a mover's live row has
/// gone to own a different one — so what stays behind is an identity with
/// no subject, no future, and the departed status that seals the node it
/// still holds. It is the same thing a rehire leaves behind (decision 12),
/// arriving one lifecycle event earlier: a person who moves under a
/// sealing pack has a former self, and that is what the material belongs
/// to now.
#[tracing::instrument(
    name = "store.identities.seal_scope_as_former_self",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn seal_scope_as_former_self(
    conn: &mut PgConnection,
    id: IdentityId,
    tenant_id: TenantId,
    kind: IdentityKind,
    email: Option<&str>,
    display_name: Option<&str>,
    scope_id: ScopeId,
) -> Result<Identity> {
    sqlx::query!(
        r#"
        insert into identities
            (id, tenant_id, subject, kind, email, display_name, scope_id,
             status, departed_at)
        values ($1, $2, null, $3, $4, $5, $6, 'departed', now())
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        kind.as_str(),
        email,
        display_name,
        scope_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    by_id(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("identity {id} vanished mid-seal"),
        })
}

/// The identity that owns `scope_id`, if any — the derivation's own
/// question, asked directly. One row at most:
/// `identities_scope_unique` makes a personal scope one person's.
#[tracing::instrument(
    name = "store.identities.by_scope",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn by_scope(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<Option<Identity>> {
    let row = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.status, i.departed_at, i.created_at,
               coalesce(p.slug = 'quarantine' and p.depth = 1, false)
                   as "quarantined!"
        from identities i
        join hierarchy_nodes n on n.id = i.scope_id
        left join hierarchy_nodes p on p.id = n.parent_id
        where i.tenant_id = $1 and i.scope_id = $2
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// The identity whose recorded email matches `email`, case-folded — the
/// last of the correspondence rule's three matches (AUTH-4, ADR-0059
/// decision 4), and the weakest.
///
/// It exists because `externalId` is the customer's attribute mapping
/// rather than a protocol constant: Entra's default for a custom
/// application is a mutable attribute, so a directory can send an anchor
/// that has changed for somebody the product already knows. Matching the
/// address they were provisioned with is how the two are joined instead of
/// duplicated.
///
/// Ordered by `created_at` and taking the first, so a tenant that somehow
/// holds two rows with one address resolves the same way on every call
/// rather than by whichever the planner reached first.
#[tracing::instrument(
    name = "store.identities.by_email",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn by_email(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    email: &str,
) -> Result<Option<Identity>> {
    let row = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.status, i.departed_at, i.created_at,
               coalesce(p.slug = 'quarantine' and p.depth = 1, false)
                   as "quarantined!"
        from identities i
        join hierarchy_nodes n on n.id = i.scope_id
        left join hierarchy_nodes p on p.id = n.parent_id
        where i.tenant_id = $1 and lower(i.email) = lower($2)
        order by i.created_at
        limit 1
        "#,
        tenant_id.as_uuid(),
        email,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}
