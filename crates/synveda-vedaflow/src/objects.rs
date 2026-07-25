//! Immutable content-addressed blobs (FLOW-1, ADR-0030).
//!
//! Every function takes the caller's `&mut PgConnection`: the transaction was
//! opened by `synveda_store::rls::begin_tenant_tx`, so content lands in the
//! same transaction as the records it describes and the audit event that
//! attests to it. A caller who skipped it writes zero rows — forced RLS with
//! an unset GUC matches nothing (ADR-0009).

use std::collections::HashMap;

use sqlx::PgConnection;
use synveda_types::{AssetKind, Error, Result, TenantId};

use crate::hash::{ObjectHash, object_hash};
use crate::{Written, storage_error};

/// Counts object writes, labelled by asset kind and by whether the content
/// was already present.
pub const OBJECTS_WRITTEN_TOTAL: &str = "synveda_vedaflow_objects_written_total";

/// The largest blob the store accepts, matching migration 0018's CHECK.
/// A governed store that accepts arbitrary blobs is a file server with an
/// approval workflow; raising this is a reviewed diff (ADR-0030 reversal
/// trigger b).
pub const MAX_OBJECT_BYTES: usize = 8 * 1024 * 1024;

/// A blob as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    /// What kind of asset the content is — part of its address, not a label
    /// beside it (ADR-0030 decision 4).
    pub kind: AssetKind,
    /// The bytes.
    pub content: Vec<u8>,
}

/// Writes `content` as an object of `kind`, returning its address.
///
/// Idempotent by construction: the address *is* the primary key, so a second
/// write of identical content conflicts with the first and is reported as
/// deduplicated rather than stored again. Dedup is per tenant — two tenants
/// holding the same bytes hold two rows (ADR-0030 decision 3).
#[tracing::instrument(
    name = "vedaflow.put_object",
    skip_all,
    fields(
        tenant.id = %tenant,
        vedaflow.kind = kind.as_str(),
        vedaflow.size = content.len(),
        vedaflow.hash = tracing::field::Empty,
        vedaflow.deduplicated = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn put_object(
    conn: &mut PgConnection,
    tenant: TenantId,
    kind: AssetKind,
    content: &[u8],
) -> Result<Written<ObjectHash>> {
    if content.len() > MAX_OBJECT_BYTES {
        return Err(Error::Invalid {
            message: format!(
                "object is {} bytes, over the {MAX_OBJECT_BYTES}-byte limit",
                content.len()
            ),
        });
    }
    let hash = object_hash(kind, content);
    let inserted = sqlx::query!(
        "insert into vedaflow_objects (tenant_id, hash, kind, content, size_bytes)
         values ($1, $2, $3, $4, $5)
         on conflict (tenant_id, hash) do nothing",
        tenant.as_uuid(),
        hash.as_slice(),
        kind.as_str(),
        content,
        i32::try_from(content.len()).expect("length is bounded by MAX_OBJECT_BYTES"),
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("insert object", &err))?
    .rows_affected();

    let written = Written {
        hash,
        deduplicated: inserted == 0,
    };
    let span = tracing::Span::current();
    span.record("vedaflow.hash", hash.to_hex());
    span.record("vedaflow.deduplicated", written.deduplicated);
    metrics::counter!(
        OBJECTS_WRITTEN_TOTAL,
        "kind" => kind.as_str(),
        "result" => if written.deduplicated { "deduplicated" } else { "stored" },
    )
    .increment(1);
    Ok(written)
}

/// Reads an object back. `None` = no such address in this tenant — which is
/// also what another tenant's object looks like.
#[tracing::instrument(name = "vedaflow.read_object", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn read_object(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: ObjectHash,
) -> Result<Option<StoredObject>> {
    let row = sqlx::query!(
        "select kind, content from vedaflow_objects where tenant_id = $1 and hash = $2",
        tenant.as_uuid(),
        hash.as_slice(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read object", &err))?;

    row.map(|row| {
        // The CHECK constraint keeps `kind` inside the vocabulary; a parse
        // failure here means schema and code have drifted — a bug.
        Ok(StoredObject {
            kind: row.kind.parse().map_err(|err| Error::Internal {
                message: format!("stored object kind outside vocabulary: {err}"),
            })?,
            content: row.content,
        })
    })
    .transpose()
}

/// [`read_object`] for a set of addresses, in one statement.
///
/// Addresses with no row are simply absent from the map — the same answer
/// `read_object` gives one at a time, and also what another tenant's object
/// looks like. Duplicates in `hashes` cost nothing: the address is the key.
///
/// FLOW-6 renders a proposal's diff from two objects per member (ADR-0035
/// decision 6), so a per-member read would make the review route's statement
/// count grow with the member set. This keeps it constant.
#[tracing::instrument(
    name = "vedaflow.read_objects",
    skip_all,
    fields(tenant.id = %tenant, vedaflow.requested = hashes.len()),
    err(Display)
)]
pub async fn read_objects(
    conn: &mut PgConnection,
    tenant: TenantId,
    hashes: &[ObjectHash],
) -> Result<HashMap<ObjectHash, StoredObject>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let addresses: Vec<Vec<u8>> = hashes.iter().map(|hash| hash.as_slice().to_vec()).collect();
    let rows = sqlx::query!(
        "select hash, kind, content from vedaflow_objects
         where tenant_id = $1 and hash = any($2)",
        tenant.as_uuid(),
        &addresses,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read objects", &err))?;

    let mut objects = HashMap::with_capacity(rows.len());
    for row in rows {
        objects.insert(
            ObjectHash::from_slice(&row.hash)?,
            StoredObject {
                kind: row.kind.parse().map_err(|err| Error::Internal {
                    message: format!("stored object kind outside vocabulary: {err}"),
                })?,
                content: row.content,
            },
        );
    }
    Ok(objects)
}
