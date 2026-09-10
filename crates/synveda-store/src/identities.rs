//! The identity store (AUTH-2, ADR-0013; AUTH-3, ADR-0018): one row per
//! provisioned subject, bound to its personal user-kind scope node. Users
//! arrive through JIT provisioning; service identities through explicit
//! registration (ADR-0018 decision 2) — same table, same placement shape.
//!
//! An identity's scope is its own `principal`-shaped governed scope, minted
//! in the provisioning transaction (CPR-7, ADR-0074 decision 3) — no
//! placement convention, no reserved quarantine scope. `identities` is
//! tenant-scoped (forced RLS, ADR-0009): reach it inside
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

struct IdentityRow {
    id: Uuid,
    tenant_id: Uuid,
    subject: Option<String>,
    kind: String,
    email: Option<String>,
    display_name: Option<String>,
    scope_id: Uuid,
    status: String,
    departed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

/// Lifecycle rows included in a case-folded email correspondence lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailIdentityLifecycle {
    /// Only identities that may still be adopted by a live directory row.
    ActiveOnly,
    /// Active and departed identities, for exact lifecycle inspection.
    Any,
}

/// A bounded email correspondence result that never chooses one of several
/// identities by storage order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniqueIdentityMatch {
    /// No identity has this address in the requested lifecycle set.
    NoMatch,
    /// Exactly one identity has this address.
    Unique(Identity),
    /// More than one identity has this address; callers must not adopt one.
    Ambiguous,
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
               i.scope_id, i.status, i.departed_at, i.created_at
        from identities i
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
               i.scope_id, i.status, i.departed_at, i.created_at
        from identities i
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
               i.scope_id, i.status, i.departed_at, i.created_at
        from identities i
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
               i.scope_id, i.status, i.departed_at, i.created_at
        from identities i
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

/// The unique identity whose recorded email matches `email`, case-folded —
/// the last of the correspondence rule's three matches (AUTH-4, ADR-0059
/// decision 4), and the weakest.
///
/// It exists because `externalId` is the customer's attribute mapping
/// rather than a protocol constant: Entra's default for a custom
/// application is a mutable attribute, so a directory can send an anchor
/// that has changed for somebody the product already knows. Matching the
/// address they were provisioned with is how the two are joined instead of
/// duplicated.
///
/// Email is not unique in the schema: shared or recycled addresses are valid
/// facts but not valid correspondence authority. The query therefore fetches
/// at most two rows and reports ambiguity instead of choosing the oldest.
#[tracing::instrument(
    name = "store.identities.unique_by_email",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn unique_user_by_email(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    email: &str,
    lifecycle: EmailIdentityLifecycle,
) -> Result<UniqueIdentityMatch> {
    let active_only = lifecycle == EmailIdentityLifecycle::ActiveOnly;
    let rows = sqlx::query_as!(
        IdentityRow,
        r#"
        select i.id, i.tenant_id, i.subject, i.kind, i.email, i.display_name,
               i.scope_id, i.status, i.departed_at, i.created_at
        from identities i
        where i.tenant_id = $1 and lower(i.email) = lower($2)
          and i.kind = 'user'
          and (not $3::boolean or i.status = 'active')
        order by i.created_at, i.id
        limit 2
        "#,
        tenant_id.as_uuid(),
        email,
        active_only,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    let mut matches = rows
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<Identity>>>()?
        .into_iter();
    let Some(identity) = matches.next() else {
        return Ok(UniqueIdentityMatch::NoMatch);
    };
    if matches.next().is_some() {
        return Ok(UniqueIdentityMatch::Ambiguous);
    }
    Ok(UniqueIdentityMatch::Unique(identity))
}
