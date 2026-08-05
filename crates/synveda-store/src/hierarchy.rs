//! The tenancy hierarchy store (HIER-1, ADR-0011): closure table +
//! materialised path over `hierarchy_nodes`/`hierarchy_closure`.
//!
//! Reads take any executor. [`create`], [`move_node`], and [`delete`] run
//! multiple statements and take a connection: callers MUST wrap them in a
//! transaction — on the data path that means [`crate::rls::begin_tenant_tx`]
//! — or a failure between statements leaves the closure inconsistent with
//! the adjacency. Closure maintenance is deliberately explicit SQL here,
//! not triggers (ADR-0011 decision 2).
//!
//! Every read here derives `sealed` (AUTH-4, ADR-0059 decision 7) by the
//! same left join, in one form: a user-kind node is sealed exactly when the
//! identity that owns it is departed. It rides the node rather than
//! travelling beside it because it reaches Cedar as a `Scope` entity
//! attribute, and a fact about a node that arrives by a second road is a
//! fact that can disagree with the node. The join is an index lookup on
//! `identities (tenant_id, scope_id)`, which is unique — so it can neither
//! multiply a row nor cost more than a probe.
//!
//! AUD-1 wiring point: create/rename/move/delete are audit emission points;
//! events are wired when the hash-chained log lands. Until then they are
//! visible in the `store.hierarchy.*` spans and the gateway's
//! `synveda_hierarchy_operations_total` counter.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{Error, HierarchyNode, Result, ScopeId, ScopeKind, TenantId};
use uuid::Uuid;

/// Raw row; converted with `TryFrom` so `kind` decodes through the
/// `synveda-types` enum (same pattern as [`crate::tenants`]).
struct NodeRow {
    id: Uuid,
    tenant_id: Uuid,
    parent_id: Option<Uuid>,
    kind: String,
    slug: String,
    name: String,
    depth: i32,
    path: String,
    sealed: bool,
    created_at: DateTime<Utc>,
}

impl TryFrom<NodeRow> for HierarchyNode {
    type Error = Error;

    fn try_from(row: NodeRow) -> Result<Self> {
        Ok(HierarchyNode {
            id: ScopeId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            parent_id: row.parent_id.map(ScopeId::from_uuid),
            // The CHECK constraint keeps this inside the vocabulary; a parse
            // failure means schema and code have drifted — a bug.
            kind: row.kind.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            slug: row.slug,
            name: row.name,
            depth: row.depth,
            path: row.path,
            sealed: row.sealed,
            created_at: row.created_at,
        })
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation (sibling slug, path, duplicate id),
        // 23503 foreign_key_violation (parent vanished, or children appeared
        // under a concurrent delete), 40P01 deadlock_detected (two moves
        // locking in opposite order): all conflicts with concurrent state,
        // retryable by the caller.
        if matches!(
            db.code().as_deref(),
            Some("23505") | Some("23503") | Some("40P01")
        ) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 23514 check_violation: slug, kind, or root shape outside the
        // vocabulary — the caller sent something invalid.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (TEN-2, ADR-0009)
        // rejected a write for a tenant other than the transaction's GUC.
        // An application defect, never the caller's fault.
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn not_found(id: ScopeId) -> Error {
    Error::NotFound {
        entity: format!("scope {id}"),
    }
}

/// Fetches a node with a row lock, serialising concurrent structural edits
/// against it (create-under, move, delete).
async fn lock_node(conn: &mut PgConnection, id: ScopeId) -> Result<Option<HierarchyNode>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name, n.depth,
               n.path, n.created_at, coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_nodes n
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where n.id = $1
        -- `of n`: the seal derivation puts a nullable side in this query
        -- and Postgres will not lock one. The node row is the only row
        -- this lock was ever about.
        for update of n
        "#,
        id.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Creates a node. The root (parent `None`) must be the org and is unique
/// per tenant; any other node must outrank its parent (ADR-0011 rank rule
/// — skipped levels are legal, inversions are not). Fails with
/// [`Error::NotFound`] when the parent does not exist *in this tenant*,
/// [`Error::Conflict`] on sibling-slug or second-root collisions.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.hierarchy.create",
    skip_all,
    fields(scope.id = %id, scope.kind = %kind),
    err(Display)
)]
pub async fn create(
    conn: &mut PgConnection,
    id: ScopeId,
    tenant_id: TenantId,
    parent_id: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
    name: &str,
) -> Result<HierarchyNode> {
    let (depth, path) = match parent_id {
        None => {
            if kind != ScopeKind::Org {
                return Err(Error::Invalid {
                    message: format!("the hierarchy root must be an org, not a {kind}"),
                });
            }
            (0, slug.to_owned())
        }
        Some(parent_id) => {
            let parent = lock_node(&mut *conn, parent_id)
                .await?
                // Another tenant's node is indistinguishable from a missing
                // one: no existence oracle across tenants (ADR-0008).
                .filter(|parent| parent.tenant_id == tenant_id)
                .ok_or_else(|| not_found(parent_id))?;
            if kind.rank() <= parent.kind.rank() {
                return Err(Error::Invalid {
                    message: format!(
                        "a {kind} cannot sit under a {} (child must outrank parent)",
                        parent.kind
                    ),
                });
            }
            (parent.depth + 1, format!("{}/{slug}", parent.path))
        }
    };

    let row = sqlx::query_as!(
        NodeRow,
        r#"
        insert into hierarchy_nodes
            (id, tenant_id, parent_id, kind, slug, name, depth, path)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        returning id, tenant_id, parent_id, kind, slug, name, depth, path, created_at,
                  coalesce((select s.status = 'departed' from identities s
                            where s.tenant_id = hierarchy_nodes.tenant_id
                              and s.scope_id = hierarchy_nodes.id), false) as "sealed!"
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        parent_id.map(|parent| parent.as_uuid()) as Option<Uuid>,
        kind.as_str(),
        slug,
        name,
        depth,
        path,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Self-row plus one row per ancestor, derived from the parent's own
    // ancestry (empty when $2 is null — the root case).
    sqlx::query!(
        r#"
        insert into hierarchy_closure (tenant_id, ancestor_id, descendant_id, distance)
        select c.tenant_id, c.ancestor_id, $1::uuid, c.distance + 1
          from hierarchy_closure c
         where c.descendant_id = $2
        union all
        select $3::uuid, $1::uuid, $1::uuid, 0
        "#,
        id.as_uuid(),
        parent_id.map(|parent| parent.as_uuid()) as Option<Uuid>,
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    row.try_into()
}

/// Fetches a node by id.
#[tracing::instrument(name = "store.hierarchy.node", skip_all, fields(scope.id = %id), err(Display))]
pub async fn node(executor: impl PgExecutor<'_>, id: ScopeId) -> Result<Option<HierarchyNode>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name, n.depth,
               n.path, n.created_at, coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_nodes n
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where n.id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetches a tenant's org root, if the hierarchy has been seeded.
#[tracing::instrument(name = "store.hierarchy.root", skip_all, fields(tenant.id = %tenant_id), err(Display))]
pub async fn root(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<HierarchyNode>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name, n.depth,
               n.path, n.created_at, coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_nodes n
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where n.tenant_id = $1 and parent_id is null
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetches a node's direct child by slug — the quarantine-scope lookup
/// (AUTH-2, ADR-0013 decision 4) and generally cheaper than listing.
#[tracing::instrument(
    name = "store.hierarchy.child_by_slug",
    skip_all,
    fields(scope.id = %parent_id, scope.slug = slug),
    err(Display)
)]
pub async fn child_by_slug(
    executor: impl PgExecutor<'_>,
    parent_id: ScopeId,
    slug: &str,
) -> Result<Option<HierarchyNode>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name, n.depth,
               n.path, n.created_at, coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_nodes n
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where n.parent_id = $1 and slug = $2
        "#,
        parent_id.as_uuid(),
        slug,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Resolves convention candidates (AUTH-2, ADR-0013 decision 3): the
/// distinct team-kind nodes whose slug is a candidate's team half and
/// which have a department-kind ancestor with the paired department half.
/// `departments` and `teams` are parallel arrays (one candidate per index);
/// the caller treats "exactly one node" as a mapping and anything else as
/// unresolved.
#[tracing::instrument(
    name = "store.hierarchy.teams_matching",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn teams_matching(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    departments: &[String],
    teams: &[String],
) -> Result<Vec<HierarchyNode>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"
        select distinct n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name,
               n.depth, n.path, n.created_at,
               -- A team node never owns an identity, so this is always
               -- false here; derived rather than asserted so that one
               -- expression answers the question everywhere.
               coalesce(s.status = 'departed', false) as "sealed!"
        from unnest($2::text[], $3::text[]) as candidate(dept_slug, team_slug)
        join hierarchy_nodes n
          on n.tenant_id = $1 and n.kind = 'team' and n.slug = candidate.team_slug
        join hierarchy_closure c
          on c.descendant_id = n.id and c.distance > 0
        join hierarchy_nodes a
          on a.id = c.ancestor_id
         and a.kind = 'department' and a.slug = candidate.dept_slug
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        "#,
        tenant_id.as_uuid(),
        departments,
        teams,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists a node's direct children, ordered by slug.
#[tracing::instrument(name = "store.hierarchy.children", skip_all, fields(scope.id = %id), err(Display))]
pub async fn children(executor: impl PgExecutor<'_>, id: ScopeId) -> Result<Vec<HierarchyNode>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name, n.depth,
               n.path, n.created_at, coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_nodes n
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where n.parent_id = $1
        order by slug
        "#,
        id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists a node's ancestors, nearest first (parent, …, org root), excluding
/// the node itself. One closure index scan — the shape HIER-2's scope chain
/// rides on.
#[tracing::instrument(name = "store.hierarchy.ancestors", skip_all, fields(scope.id = %id), err(Display))]
pub async fn ancestors(executor: impl PgExecutor<'_>, id: ScopeId) -> Result<Vec<HierarchyNode>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name,
               n.depth, n.path, n.created_at,
               coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_closure c
        join hierarchy_nodes n on n.id = c.ancestor_id
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where c.descendant_id = $1 and c.distance > 0
        order by c.distance
        "#,
        id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The scope chain (HIER-2, ADR-0016): the node itself plus its
/// ancestors, nearest-first (self, parent, …, org root) — one closure
/// index scan over the `distance >= 0` rows. The tenant filter is
/// explicit in SQL so the scope-chain cache stays tenant-correct even on
/// connections where the RLS backstop does not bite (ADR-0009); an
/// unknown or foreign node yields an empty chain.
#[tracing::instrument(name = "store.hierarchy.chain", skip_all, fields(tenant.id = %tenant_id, scope.id = %id), err(Display))]
pub async fn chain(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Vec<HierarchyNode>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name,
               n.depth, n.path, n.created_at,
               coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_closure c
        join hierarchy_nodes n on n.id = c.ancestor_id
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where c.descendant_id = $1 and c.tenant_id = $2
        order by c.distance
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists a node's whole subtree (excluding the node itself), in stable
/// path order. One closure index scan.
#[tracing::instrument(name = "store.hierarchy.descendants", skip_all, fields(scope.id = %id), err(Display))]
pub async fn descendants(executor: impl PgExecutor<'_>, id: ScopeId) -> Result<Vec<HierarchyNode>> {
    let rows = sqlx::query_as!(
        NodeRow,
        r#"
        select n.id, n.tenant_id, n.parent_id, n.kind, n.slug, n.name,
               n.depth, n.path, n.created_at,
               coalesce(s.status = 'departed', false) as "sealed!"
        from hierarchy_closure c
        join hierarchy_nodes n on n.id = c.descendant_id
        left join identities s on s.tenant_id = n.tenant_id and s.scope_id = n.id
        where c.ancestor_id = $1 and c.distance > 0
        order by n.path
        "#,
        id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Renames a node's display name. Slugs (and therefore paths) are immutable
/// in HIER-1 (ADR-0011). Returns `None` for an unknown node.
#[tracing::instrument(name = "store.hierarchy.rename", skip_all, fields(scope.id = %id), err(Display))]
pub async fn rename(
    executor: impl PgExecutor<'_>,
    id: ScopeId,
    name: &str,
) -> Result<Option<HierarchyNode>> {
    let row = sqlx::query_as!(
        NodeRow,
        r#"
        update hierarchy_nodes set name = $2 where id = $1
        returning id, tenant_id, parent_id, kind, slug, name, depth, path, created_at,
                  coalesce((select s.status = 'departed' from identities s
                            where s.tenant_id = hierarchy_nodes.tenant_id
                              and s.scope_id = hierarchy_nodes.id), false) as "sealed!"
        "#,
        id.as_uuid(),
        name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Moves a node (and its whole subtree) under a new parent: closure
/// surgery plus subtree depth/path rewrites, all in the caller's
/// transaction (ADR-0011 decision 5). The root cannot move; the node must
/// outrank the new parent; moving under one's own subtree is rejected.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.hierarchy.move",
    skip_all,
    fields(scope.id = %id, scope.new_parent = %new_parent_id),
    err(Display)
)]
pub async fn move_node(
    conn: &mut PgConnection,
    id: ScopeId,
    new_parent_id: ScopeId,
) -> Result<HierarchyNode> {
    if new_parent_id == id {
        return Err(Error::Invalid {
            message: "cannot move a node under itself".to_owned(),
        });
    }
    let node_row = lock_node(&mut *conn, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    if node_row.parent_id.is_none() {
        return Err(Error::Invalid {
            message: "the org root cannot move".to_owned(),
        });
    }
    let parent = lock_node(&mut *conn, new_parent_id)
        .await?
        // Same no-existence-oracle doctrine as `create`.
        .filter(|parent| parent.tenant_id == node_row.tenant_id)
        .ok_or_else(|| not_found(new_parent_id))?;
    if node_row.kind.rank() <= parent.kind.rank() {
        return Err(Error::Invalid {
            message: format!(
                "a {} cannot sit under a {} (child must outrank parent)",
                node_row.kind, parent.kind
            ),
        });
    }
    let descends = sqlx::query_scalar!(
        r#"
        select exists(
            select 1 from hierarchy_closure
            where ancestor_id = $1 and descendant_id = $2
        ) as "descends!"
        "#,
        id.as_uuid(),
        new_parent_id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    if descends {
        return Err(Error::Invalid {
            message: "cannot move a node under its own descendant".to_owned(),
        });
    }

    // Adjacency first: a sibling-slug collision at the destination fails
    // here, before any closure surgery.
    sqlx::query!(
        "update hierarchy_nodes set parent_id = $2 where id = $1",
        id.as_uuid(),
        new_parent_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Unlink: drop every closure row that ties an outside ancestor to the
    // subtree. Rows internal to the subtree (ancestor inside it) survive.
    sqlx::query!(
        r#"
        delete from hierarchy_closure
        where descendant_id in
                (select descendant_id from hierarchy_closure where ancestor_id = $1)
          and ancestor_id not in
                (select descendant_id from hierarchy_closure where ancestor_id = $1)
        "#,
        id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Relink: cross-join the new parent's ancestry (self-row included) with
    // the subtree (self-row included).
    sqlx::query!(
        r#"
        insert into hierarchy_closure (tenant_id, ancestor_id, descendant_id, distance)
        select super.tenant_id, super.ancestor_id, sub.descendant_id,
               super.distance + sub.distance + 1
        from hierarchy_closure super
        cross join hierarchy_closure sub
        where super.descendant_id = $2
          and sub.ancestor_id = $1
        "#,
        id.as_uuid(),
        new_parent_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Subtree depth and path rewrites, one statement each.
    let depth_delta = parent.depth + 1 - node_row.depth;
    sqlx::query!(
        r#"
        update hierarchy_nodes n
        set depth = n.depth + $2
        from hierarchy_closure c
        where c.ancestor_id = $1 and c.descendant_id = n.id
        "#,
        id.as_uuid(),
        depth_delta,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    let new_path = format!("{}/{}", parent.path, node_row.slug);
    sqlx::query!(
        r#"
        update hierarchy_nodes n
        set path = $2 || substr(n.path, char_length($3::text) + 1)
        from hierarchy_closure c
        where c.ancestor_id = $1 and c.descendant_id = n.id
        "#,
        id.as_uuid(),
        new_path,
        node_row.path,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    node(&mut *conn, id).await?.ok_or_else(|| Error::Internal {
        message: format!("scope {id} vanished mid-move"),
    })
}

/// Deletes a leaf node; its closure rows cascade. A node with children is a
/// [`Error::Conflict`] — subtree deletion is a deliberate later feature,
/// never a cascade surprise (ADR-0011). Returns `false` for an unknown node.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(name = "store.hierarchy.delete", skip_all, fields(scope.id = %id), err(Display))]
pub async fn delete(conn: &mut PgConnection, id: ScopeId) -> Result<bool> {
    let has_children = sqlx::query_scalar!(
        r#"select exists(select 1 from hierarchy_nodes where parent_id = $1) as "has_children!""#,
        id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    if has_children {
        return Err(Error::Conflict {
            message: format!("scope {id} has children; only leaves can be deleted"),
        });
    }
    let result = sqlx::query!("delete from hierarchy_nodes where id = $1", id.as_uuid())
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}
