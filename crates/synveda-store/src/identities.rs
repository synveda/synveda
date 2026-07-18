//! The identity store (AUTH-2, ADR-0013): one row per provisioned subject,
//! bound to its personal user-kind scope node.
//!
//! `quarantined` is derived from placement in every read — the identity's
//! node sits directly under the tenant's reserved `quarantine` scope (the
//! org root's child with that slug) — never stored (ADR-0013 decision 4).
//! `identities` is tenant-scoped (forced RLS, ADR-0009): reach it inside
//! [`crate::rls::begin_tenant_tx`].
//!
//! AUD-1 wiring point: identity creation (`identity.provisioned`) is an
//! audit emission point; until the hash-chained log lands it is visible in
//! the gateway's `identity.provision` span and
//! `synveda_jit_provisions_total`.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{Error, Identity, IdentityId, Result, ScopeId, TenantId};
use uuid::Uuid;

/// The reserved slug of the quarantine scope: the org root's child that
/// unmapped users' personal scopes are created under (ADR-0013 decision 4).
pub const QUARANTINE_SLUG: &str = "quarantine";

struct IdentityRow {
    id: Uuid,
    tenant_id: Uuid,
    subject: String,
    email: Option<String>,
    display_name: Option<String>,
    scope_id: Uuid,
    quarantined: bool,
    created_at: DateTime<Utc>,
}

impl From<IdentityRow> for Identity {
    fn from(row: IdentityRow) -> Self {
        Identity {
            id: IdentityId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            subject: row.subject,
            email: row.email,
            display_name: row.display_name,
            scope_id: ScopeId::from_uuid(row.scope_id),
            quarantined: row.quarantined,
            created_at: row.created_at,
        }
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
            return Error::Internal {
                message: format!("row-level security or privilege violation: {db}"),
            };
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
        select i.id, i.tenant_id, i.subject, i.email, i.display_name,
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
    Ok(row.map(Into::into))
}

/// Provisions an identity bound to `scope_id` (the already-created personal
/// user node), returning it with its placement-derived quarantine status.
/// Fails with [`Error::Conflict`] when the subject or the node is already
/// bound — the JIT first-login race; the caller retries and adopts the
/// winner's row (ADR-0013 decision 2).
///
/// Takes a connection (insert + derive-read); callers wrap it in the same
/// transaction that created the node.
#[tracing::instrument(
    name = "store.identities.create",
    skip_all,
    fields(tenant.id = %tenant_id, identity.id = %id, scope.id = %scope_id),
    err(Display)
)]
pub async fn create(
    conn: &mut PgConnection,
    id: IdentityId,
    tenant_id: TenantId,
    subject: &str,
    email: Option<&str>,
    display_name: Option<&str>,
    scope_id: ScopeId,
) -> Result<Identity> {
    sqlx::query!(
        r#"
        insert into identities (id, tenant_id, subject, email, display_name, scope_id)
        values ($1, $2, $3, $4, $5, $6)
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        subject,
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
