//! Stable, sealed per-tenant secret aggregates (CPR-35, ADR-0094).
//!
//! A row's UUID, tenant, scope, kind, label and provider are content-free and
//! stable. Logical value rotation advances `value_revision`; DEK
//! re-encryption changes only the envelope and key generation. Revocation
//! destroys ciphertext while retaining the identifier an immutable artifact
//! may cite.
//!
//! This module never opens an envelope. Callers that possess the operator or
//! runtime custody authority use [`crate::keys::KeyRing`] and keep plaintext
//! outside store DTOs, logs and audit payloads.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_crypto::KeyVersion;
use synveda_types::secret::{TenantSecretKind, TenantSecretState};
use synveda_types::{
    Error, Result, ScopeId, TenantId, TenantSecretId, TenantSecretReencryptionJobId,
};
use uuid::Uuid;

/// Counter: logical secret transitions, labelled `operation` and `kind`.
pub const TENANT_SECRET_MUTATIONS_TOTAL: &str = "synveda_tenant_secret_mutations_total";
/// Counter: envelopes advanced between DEK generations.
pub const TENANT_SECRET_REENCRYPTIONS_TOTAL: &str = "synveda_tenant_secret_reencryptions_total";

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23503") => {
                return Error::NotFound {
                    entity: db.to_string(),
                };
            }
            Some("23505") => {
                return Error::Conflict {
                    message: db.to_string(),
                };
            }
            Some("23514" | "P0001") => {
                return Error::Invalid {
                    message: db.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(db),
            _ => {}
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// One stable tenant-secret aggregate. `sealed` is present only while active.
#[derive(Clone)]
pub struct StoredTenantSecret {
    /// Stable reference identity.
    pub id: TenantSecretId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Governing scope.
    pub scope_id: ScopeId,
    /// Closed consumer family.
    pub kind: TenantSecretKind,
    /// Credential-free operator label.
    pub label: String,
    /// Credential-free provider name.
    pub provider: Option<String>,
    /// Whether the reference currently resolves.
    pub state: TenantSecretState,
    /// Logical value revision; DEK re-encryption does not advance it.
    pub value_revision: u64,
    /// Envelope key generation while active.
    pub key_version: Option<KeyVersion>,
    /// Current envelope while active.
    pub sealed: Option<Vec<u8>>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last logical value rotation/reactivation instant.
    pub rotated_at: DateTime<Utc>,
    /// Last logical or cryptographic update.
    pub updated_at: DateTime<Utc>,
    /// Revocation instant.
    pub revoked_at: Option<DateTime<Utc>>,
}

// Ciphertext is deliberately omitted. It is not plaintext, but dumping the
// shape and bytes of credentials into a log is still a custody failure.
impl std::fmt::Debug for StoredTenantSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredTenantSecret")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("scope_id", &self.scope_id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("provider", &self.provider)
            .field("state", &self.state)
            .field("value_revision", &self.value_revision)
            .field("key_version", &self.key_version)
            .field("sealed", &self.sealed.as_ref().map(Vec::len))
            .field("created_at", &self.created_at)
            .field("rotated_at", &self.rotated_at)
            .field("updated_at", &self.updated_at)
            .field("revoked_at", &self.revoked_at)
            .finish()
    }
}

struct SecretRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    kind: String,
    label: String,
    provider: Option<String>,
    state: String,
    value_revision: i64,
    key_version: Option<i32>,
    sealed: Option<Vec<u8>>,
    created_at: DateTime<Utc>,
    rotated_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TryFrom<SecretRow> for StoredTenantSecret {
    type Error = Error;

    fn try_from(row: SecretRow) -> Result<Self> {
        Ok(Self {
            id: TenantSecretId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            kind: row.kind.parse().map_err(stored_vocabulary)?,
            label: row.label,
            provider: row.provider,
            state: row.state.parse().map_err(stored_vocabulary)?,
            value_revision: u64::try_from(row.value_revision).map_err(|_| Error::Internal {
                message: "stored tenant-secret revision is negative".to_owned(),
            })?,
            key_version: row.key_version.map(KeyVersion::from_i32).transpose()?,
            sealed: row.sealed,
            created_at: row.created_at,
            rotated_at: row.rotated_at,
            updated_at: row.updated_at,
            revoked_at: row.revoked_at,
        })
    }
}

fn stored_vocabulary(error: Error) -> Error {
    Error::Internal {
        message: format!("stored tenant-secret vocabulary drift: {error}"),
    }
}

fn validate_envelope(sealed: &[u8], key_version: KeyVersion) -> Result<()> {
    if synveda_crypto::envelope_is_deployment_scoped(sealed) != Some(false) {
        return Err(Error::Invalid {
            message: "tenant-secret envelope must be tenant-scoped".to_owned(),
        });
    }
    if synveda_crypto::envelope_version(sealed)? != key_version {
        return Err(Error::Invalid {
            message: "tenant-secret envelope and recorded key generation disagree".to_owned(),
        });
    }
    Ok(())
}

/// Insert a stable secret or logically rotate/reactivate the existing
/// `(kind, label)` aggregate. Scope/provider disagreement fails instead of
/// silently retargeting an immutable reference.
#[tracing::instrument(
    name = "store.tenant_secrets.put",
    skip_all,
    fields(tenant.id = %tenant_id, secret.id = %id, secret.kind = %kind, secret.label = label),
    err(Display)
)]
#[allow(clippy::too_many_arguments)]
pub async fn put(
    conn: &mut PgConnection,
    id: TenantSecretId,
    tenant_id: TenantId,
    scope_id: ScopeId,
    kind: TenantSecretKind,
    label: &str,
    provider: Option<&str>,
    key_version: KeyVersion,
    sealed: &[u8],
) -> Result<StoredTenantSecret> {
    validate_envelope(sealed, key_version)?;
    let row = sqlx::query_as!(
        SecretRow,
        r#"
        insert into tenant_secrets
            (id, tenant_id, scope_id, kind, label, provider, key_version, sealed)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        on conflict (tenant_id, kind, label) do update
            set state = 'active',
                value_revision = tenant_secrets.value_revision + 1,
                key_version = excluded.key_version,
                sealed = excluded.sealed,
                rotated_at = clock_timestamp(),
                updated_at = clock_timestamp(),
                revoked_at = null
            where tenant_secrets.id = excluded.id
              and tenant_secrets.scope_id = excluded.scope_id
              and tenant_secrets.provider is not distinct from excluded.provider
        returning id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, sealed, created_at, rotated_at,
                  updated_at, revoked_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        kind.as_str(),
        label,
        provider,
        key_version.as_i32(),
        sealed,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::Conflict {
        message: format!(
            "tenant-secret {kind}/{label} already exists under different ownership metadata"
        ),
    })?;
    metrics::counter!(
        TENANT_SECRET_MUTATIONS_TOTAL,
        "operation" => "put",
        "kind" => kind.as_str()
    )
    .increment(1);
    row.try_into()
}

/// Read one reference, active or revoked. Tenant qualification deliberately
/// makes another tenant's UUID indistinguishable from an absent one.
#[tracing::instrument(
    name = "store.tenant_secrets.get",
    skip_all,
    fields(tenant.id = %tenant_id, secret.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: TenantSecretId,
) -> Result<Option<StoredTenantSecret>> {
    let row = sqlx::query_as!(
        SecretRow,
        r#"select id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, sealed, created_at, rotated_at,
                  updated_at, revoked_at
             from tenant_secrets
            where tenant_id = $1 and id = $2"#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Resolve the stable aggregate behind a well-known consumer label, including
/// a revoked row so callers can distinguish "never configured" from "must not
/// fall back".
pub async fn by_label(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    kind: TenantSecretKind,
    label: &str,
) -> Result<Option<StoredTenantSecret>> {
    let row = sqlx::query_as!(
        SecretRow,
        r#"select id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, sealed, created_at, rotated_at,
                  updated_at, revoked_at
             from tenant_secrets
            where tenant_id = $1 and kind = $2 and label = $3"#,
        tenant_id.as_uuid(),
        kind.as_str(),
        label,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Whether an internal reference is active under the exact consumer kind and
/// governing scope. One boolean intentionally collapses absence, revocation,
/// wrong kind/scope and cross-tenant identity into one non-oracular answer.
pub async fn reference_is_active(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: TenantSecretId,
    kind: TenantSecretKind,
    scope_id: ScopeId,
) -> Result<bool> {
    sqlx::query_scalar!(
        r#"select exists(
               select 1 from tenant_secrets
                where tenant_id = $1 and id = $2 and kind = $3
                  and scope_id = $4 and state = 'active'
           ) as "active!""#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        kind.as_str(),
        scope_id.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}

/// Content-free operator inventory. Envelopes never leave this module through
/// a listing operation.
pub async fn list(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Vec<StoredTenantSecret>> {
    let rows = sqlx::query_as!(
        SecretRow,
        r#"select id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, null::bytea as sealed,
                  created_at, rotated_at, updated_at, revoked_at
             from tenant_secrets
            where tenant_id = $1
            order by kind, label, id"#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Destroy the current envelope and retain its content-free stable identity.
/// Returns the advanced logical revision, or `None` for absent/already revoked.
#[tracing::instrument(
    name = "store.tenant_secrets.revoke",
    skip_all,
    fields(tenant.id = %tenant_id, secret.id = %id),
    err(Display)
)]
pub async fn revoke(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: TenantSecretId,
) -> Result<Option<StoredTenantSecret>> {
    let row = sqlx::query_as!(
        SecretRow,
        r#"update tenant_secrets
              set state = 'revoked', value_revision = value_revision + 1,
                  key_version = null, sealed = null,
                  updated_at = clock_timestamp(), revoked_at = clock_timestamp()
            where tenant_id = $1 and id = $2 and state = 'active'
        returning id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, sealed, created_at, rotated_at,
                  updated_at, revoked_at"#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let secret: Option<StoredTenantSecret> = row.map(TryInto::try_into).transpose()?;
    if let Some(secret) = &secret {
        metrics::counter!(
            TENANT_SECRET_MUTATIONS_TOTAL,
            "operation" => "revoke",
            "kind" => secret.kind.as_str()
        )
        .increment(1);
    }
    Ok(secret)
}

/// Active envelopes on one retired key generation, in stable order.
pub async fn active_for_key_generation(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    key_version: KeyVersion,
) -> Result<Vec<StoredTenantSecret>> {
    let rows = sqlx::query_as!(
        SecretRow,
        r#"select id, tenant_id, scope_id, kind, label, provider, state,
                  value_revision, key_version, sealed, created_at, rotated_at,
                  updated_at, revoked_at
             from tenant_secrets
            where tenant_id = $1 and state = 'active' and key_version = $2
            order by id"#,
        tenant_id.as_uuid(),
        key_version.as_i32(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Replace one envelope during DEK re-encryption without advancing the
/// logical value revision.
pub async fn replace_envelope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: TenantSecretId,
    expected_value_revision: u64,
    from: KeyVersion,
    to: KeyVersion,
    sealed: &[u8],
) -> Result<bool> {
    validate_envelope(sealed, to)?;
    let expected_value_revision =
        i64::try_from(expected_value_revision).map_err(|_| Error::Invalid {
            message: "tenant-secret revision exceeds storage range".to_owned(),
        })?;
    let result = sqlx::query!(
        r#"update tenant_secrets
              set key_version = $1, sealed = $2, updated_at = clock_timestamp()
            where tenant_id = $3 and id = $4 and state = 'active'
              and value_revision = $5 and key_version = $6"#,
        to.as_i32(),
        sealed,
        tenant_id.as_uuid(),
        id.as_uuid(),
        expected_value_revision,
        from.as_i32(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() > 0 {
        metrics::counter!(TENANT_SECRET_REENCRYPTIONS_TOTAL).increment(1);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Durable DEK re-encryption job metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretReencryptionJob {
    /// Retry-stable job id.
    pub id: TenantSecretReencryptionJobId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Retired source generation.
    pub from_key_version: KeyVersion,
    /// Current target generation.
    pub to_key_version: KeyVersion,
    /// Durable state.
    pub state: String,
    /// Eligible envelopes when this attempt began.
    pub secrets_total: u64,
    /// Envelopes completed by this attempt.
    pub secrets_reencrypted: u64,
    /// Number of starts.
    pub attempt: u64,
    /// Content-free failure class.
    pub failure_code: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Most recent start.
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal instant.
    pub completed_at: Option<DateTime<Utc>>,
    /// Last transition.
    pub updated_at: DateTime<Utc>,
}

struct JobRow {
    id: Uuid,
    tenant_id: Uuid,
    from_key_version: i32,
    to_key_version: i32,
    state: String,
    secrets_total: i64,
    secrets_reencrypted: i64,
    attempt: i64,
    failure_code: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<JobRow> for SecretReencryptionJob {
    type Error = Error;

    fn try_from(row: JobRow) -> Result<Self> {
        let count = |value: i64, field: &str| {
            u64::try_from(value).map_err(|_| Error::Internal {
                message: format!("stored re-encryption {field} is negative"),
            })
        };
        Ok(Self {
            id: TenantSecretReencryptionJobId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            from_key_version: KeyVersion::from_i32(row.from_key_version)?,
            to_key_version: KeyVersion::from_i32(row.to_key_version)?,
            state: row.state,
            secrets_total: count(row.secrets_total, "total")?,
            secrets_reencrypted: count(row.secrets_reencrypted, "completed count")?,
            attempt: count(row.attempt, "attempt")?,
            failure_code: row.failure_code,
            created_at: row.created_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
            updated_at: row.updated_at,
        })
    }
}

/// Create or recover the one job for a generation pair.
pub async fn create_reencryption_job(
    conn: &mut PgConnection,
    id: TenantSecretReencryptionJobId,
    tenant_id: TenantId,
    from: KeyVersion,
    to: KeyVersion,
) -> Result<SecretReencryptionJob> {
    let row = sqlx::query_as!(
        JobRow,
        r#"insert into tenant_secret_reencryption_jobs
               (id, tenant_id, from_key_version, to_key_version)
           values ($1, $2, $3, $4)
           on conflict (tenant_id, from_key_version, to_key_version) do update
               set updated_at = tenant_secret_reencryption_jobs.updated_at
        returning id, tenant_id, from_key_version, to_key_version, state,
                  secrets_total, secrets_reencrypted, attempt, failure_code,
                  created_at, started_at, completed_at, updated_at"#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        from.as_i32(),
        to.as_i32(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Start or retry a non-completed job with a freshly measured eligible count.
pub async fn start_reencryption_job(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: TenantSecretReencryptionJobId,
    total: u64,
) -> Result<Option<SecretReencryptionJob>> {
    let total = i64::try_from(total).map_err(|_| Error::Invalid {
        message: "re-encryption secret count exceeds storage range".to_owned(),
    })?;
    let row = sqlx::query_as!(
        JobRow,
        r#"update tenant_secret_reencryption_jobs
              set state = 'running', secrets_total = $1,
                  secrets_reencrypted = 0, attempt = attempt + 1,
                  failure_code = null, started_at = clock_timestamp(),
                  completed_at = null, updated_at = clock_timestamp()
            where tenant_id = $2 and id = $3 and state <> 'completed'
        returning id, tenant_id, from_key_version, to_key_version, state,
                  secrets_total, secrets_reencrypted, attempt, failure_code,
                  created_at, started_at, completed_at, updated_at"#,
        total,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Mark a fully re-encrypted job complete.
pub async fn complete_reencryption_job(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: TenantSecretReencryptionJobId,
    completed: u64,
) -> Result<Option<SecretReencryptionJob>> {
    let completed = i64::try_from(completed).map_err(|_| Error::Invalid {
        message: "re-encryption completion count exceeds storage range".to_owned(),
    })?;
    let row = sqlx::query_as!(
        JobRow,
        r#"update tenant_secret_reencryption_jobs
              set state = 'completed', secrets_reencrypted = $1,
                  completed_at = clock_timestamp(), updated_at = clock_timestamp()
            where tenant_id = $2 and id = $3 and state = 'running'
              and secrets_total = $1
        returning id, tenant_id, from_key_version, to_key_version, state,
                  secrets_total, secrets_reencrypted, attempt, failure_code,
                  created_at, started_at, completed_at, updated_at"#,
        completed,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Mark a failed attempt with one content-free failure class.
pub async fn fail_reencryption_job(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: TenantSecretReencryptionJobId,
    completed: u64,
    failure_code: &str,
) -> Result<Option<SecretReencryptionJob>> {
    let completed = i64::try_from(completed).map_err(|_| Error::Invalid {
        message: "re-encryption completion count exceeds storage range".to_owned(),
    })?;
    let row = sqlx::query_as!(
        JobRow,
        r#"update tenant_secret_reencryption_jobs
              set state = 'failed', secrets_reencrypted = $1,
                  failure_code = $2, completed_at = clock_timestamp(),
                  updated_at = clock_timestamp()
            where tenant_id = $3 and id = $4 and state = 'running'
        returning id, tenant_id, from_key_version, to_key_version, state,
                  secrets_total, secrets_reencrypted, attempt, failure_code,
                  created_at, started_at, completed_at, updated_at"#,
        completed,
        failure_code,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Content-free re-encryption job history, newest generation first.
pub async fn reencryption_jobs(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Vec<SecretReencryptionJob>> {
    let rows = sqlx::query_as!(
        JobRow,
        r#"select id, tenant_id, from_key_version, to_key_version, state,
                  secrets_total, secrets_reencrypted, attempt, failure_code,
                  created_at, started_at, completed_at, updated_at
             from tenant_secret_reencryption_jobs
            where tenant_id = $1
            order by to_key_version desc, id desc"#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}
