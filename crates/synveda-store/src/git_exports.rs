//! FLOW-8 bounded immutable evidence reads. SQL stays in the store, and every
//! caller must use an ordinary forced-RLS tenant transaction. These functions
//! carry no disclosure authority and never resolve arbitrary hash collections.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId};

/// A ref and pin read from one PostgreSQL statement snapshot.
pub struct RefEvidence {
    /// Exact head address.
    pub commit_hash: Vec<u8>,
    /// Last head update time.
    pub updated_at: DateTime<Utc>,
    /// Last head updater.
    pub updated_by: IdentityId,
    /// Standing pin captured in the same statement.
    pub pin: Option<PinEvidence>,
}

/// Raw pin evidence accompanying its channel head.
pub struct PinEvidence {
    /// Exact pinned commit address.
    pub commit_hash: Vec<u8>,
    /// Original pin update time.
    pub pinned_at: DateTime<Utc>,
    /// Original pin updater.
    pub pinned_by: IdentityId,
}

/// Read only one exact channel and its pin. Separate read-committed statements
/// could otherwise combine a head and pin that never coexisted.
pub async fn read_ref(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
) -> Result<Option<RefEvidence>> {
    let row = sqlx::query!(
        r#"select r.commit_hash, r.updated_at, r.updated_by,
                  p.commit_hash as "pinned_commit?", p.updated_at as "pinned_at?",
                  p.updated_by as "pinned_by?"
           from vedaflow_refs r left join vedaflow_refs p
             on p.tenant_id = r.tenant_id and p.scope_id = r.scope_id and p.name = 'pin/' || r.name
           where r.tenant_id = $1 and r.scope_id = $2 and r.name = $3"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        name,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(crate::workspaces::storage_error)?;
    row.map(|row| {
        let pin = match (row.pinned_commit, row.pinned_at, row.pinned_by) {
            (None, None, None) => None,
            (Some(commit_hash), Some(pinned_at), Some(pinned_by)) => Some(PinEvidence {
                commit_hash,
                pinned_at,
                pinned_by: IdentityId::from_uuid(pinned_by),
            }),
            _ => {
                return Err(Error::Internal {
                    message: "Git export pin evidence is inconsistent".into(),
                });
            }
        };
        Ok(RefEvidence {
            commit_hash: row.commit_hash,
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
            pin,
        })
    })
    .transpose()
}

/// Raw immutable commit evidence; addresses remain typed by the application.
pub struct CommitEvidence {
    /// Source tree address.
    pub tree_hash: Vec<u8>,
    /// Source author.
    pub author_id: IdentityId,
    /// Exact source message.
    pub message: String,
    /// Original source timestamp.
    pub committed_at: DateTime<Utc>,
    /// Original governing policy fingerprint.
    pub policy_snapshot_hash: Vec<u8>,
    /// Original Ed25519 signature, when present.
    pub signature: Option<Vec<u8>>,
    /// Key lookup identifier paired with the signature.
    pub signer_key_id: Option<String>,
    /// Ordered parents, within the caller's hard work bound.
    pub parents: Vec<Vec<u8>>,
}

/// One bounded flat-tree entry. Subtrees are returned so callers can refuse
/// unsupported projections without silently truncating source evidence.
pub struct TreeEntryEvidence {
    /// Exact source name, never a filesystem path here.
    pub name: String,
    /// Source object address, if this is a blob.
    pub object_hash: Option<Vec<u8>>,
    /// Source subtree address, if this is a subtree.
    pub subtree_hash: Option<Vec<u8>>,
}

/// Read one exact commit and a bounded parent set; overflow refuses.
pub async fn read_commit(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: &[u8],
    maximum_parents: i64,
) -> Result<Option<CommitEvidence>> {
    let row = sqlx::query!(
        "select tree_hash, author_id, message, committed_at, policy_snapshot_hash,
                signature, signer_key_id from vedaflow_commits
         where tenant_id = $1 and hash = $2",
        tenant.as_uuid(),
        hash,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(crate::workspaces::storage_error)?;
    let Some(row) = row else { return Ok(None) };
    let parents = sqlx::query_scalar!(
        "select parent_hash from vedaflow_commit_parents
         where tenant_id = $1 and commit_hash = $2 order by ordinal limit $3",
        tenant.as_uuid(),
        hash,
        maximum_parents.saturating_add(1),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(crate::workspaces::storage_error)?;
    if parents.len() as i64 > maximum_parents {
        return Err(bound());
    }
    Ok(Some(CommitEvidence {
        tree_hash: row.tree_hash,
        author_id: IdentityId::from_uuid(row.author_id),
        message: row.message,
        committed_at: row.committed_at,
        policy_snapshot_hash: row.policy_snapshot_hash,
        signature: row.signature,
        signer_key_id: row.signer_key_id,
        parents,
    }))
}

/// Read at most the declared flat-tree limit plus one overflow witness.
pub async fn read_tree(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: &[u8],
    maximum_entries: i64,
) -> Result<Vec<TreeEntryEvidence>> {
    let rows = sqlx::query_as!(
        TreeEntryEvidence,
        "select name, object_hash, subtree_hash from vedaflow_tree_entries
         where tenant_id = $1 and tree_hash = $2 order by name collate \"C\" limit $3",
        tenant.as_uuid(),
        hash,
        maximum_entries.saturating_add(1),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(crate::workspaces::storage_error)?;
    if rows.len() as i64 > maximum_entries {
        return Err(bound());
    }
    Ok(rows)
}

/// Read exact blob bytes only when they fit the remaining disclosure budget.
/// An oversized object never enters application memory through this reader.
pub async fn read_object(
    conn: &mut PgConnection,
    tenant: TenantId,
    hash: &[u8],
    remaining_bytes: i64,
) -> Result<Option<(String, Vec<u8>)>> {
    let row =
        sqlx::query!(
        "select kind, case when octet_length(content)::bigint <= $3 then content else null end as content
         from vedaflow_objects where tenant_id = $1 and hash = $2",
        tenant.as_uuid(), hash, remaining_bytes,
    )
        .fetch_optional(&mut *conn)
        .await
        .map_err(crate::workspaces::storage_error)?;
    row.map(|row| Ok((row.kind, row.content.ok_or_else(bound)?)))
        .transpose()
}

fn bound() -> Error {
    Error::Invalid {
        message: "Git export exceeds its source history bounds".into(),
    }
}
