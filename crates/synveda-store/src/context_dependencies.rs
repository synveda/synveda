//! MEM-8 private immutable delivery proof, independent of diagnostic retention.

use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{ContextRunId, Error, Result, TenantId};

/// Content-free internal manifest; never a public trace projection.
pub struct StoredDeliveryDependencies {
    /// Closed producer schema version.
    pub dependency_version: i32,
    /// Domain-separated digest of the canonical manifest.
    pub manifest_hash: String,
    /// Producer-validated addresses and hashes, without plaintext.
    pub manifest: Value,
}

/// Retain producer-validated proof in the run's own transaction.
pub async fn record(
    connection: &mut PgConnection,
    tenant: TenantId,
    run: ContextRunId,
    manifest_hash: &str,
    manifest: &Value,
) -> Result<()> {
    sqlx::query!(
        r#"insert into context_run_delivery_dependencies
              (tenant_id, context_run_id, dependency_version, manifest_hash, manifest)
           values ($1, $2, 2, $3, $4)"#,
        tenant.as_uuid(),
        run.as_uuid(),
        manifest_hash,
        manifest,
    )
    .execute(connection)
    .await
    .map_err(storage)?;
    Ok(())
}

/// Read only for independent source admission. Absence is unproven lineage.
pub async fn for_run(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    run: ContextRunId,
) -> Result<Option<StoredDeliveryDependencies>> {
    sqlx::query_as!(
        StoredDeliveryDependencies,
        r#"select dependency_version, manifest_hash, manifest
           from context_run_delivery_dependencies
           where tenant_id = $1 and context_run_id = $2"#,
        tenant.as_uuid(),
        run.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage)
}

fn storage(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error
        && database.code().as_deref() == Some("42501")
    {
        return crate::rls::backstop_error(database);
    }
    Error::Storage {
        message: format!("context delivery dependency storage: {error}"),
    }
}
