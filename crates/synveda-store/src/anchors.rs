//! The scope-anchor resolver (CPR-6, ADR-0073): where a request stands.
//!
//! One question — *which scopes is this request decided against, and what does
//! this caller hold at each* — answered once per request and handed to the PDP
//! as [`synveda_types::anchor::AnchorSet`].
//!
//! ## Six inputs, one ordered answer
//!
//! 1. The authenticated principal's **own scope** ([`crate::scopes::principal_scope`]).
//! 2. The **selected project**, when the request named one.
//! 3. The **selected workspace**, when the request named one.
//! 4. The **organisation-unit relationships** above either of them — every
//!    ancestor of the selection, because policy is assigned along that ancestry
//!    and a grant may be written at any of it.
//! 5. The **tenant root**, always, when the tenant has one.
//! 6. Every scope a **direct or group grant** reaches this caller at, whether
//!    or not the selection named it.
//!
//! The answer is ordered by specificity and merged per scope
//! ([`AnchorSet::new`]), so one scope is one anchor however many ways it
//! became applicable.
//!
//! ## Principal privacy is applied while the set is built
//!
//! A grant at an ancestor does not reach a `principal`-shaped scope
//! ([`synveda_types::access::inherits_into`]). That rule is in the SQL below,
//! in the same shape [`crate::access::members_of`] carries it, so a resolver
//! and a member listing cannot disagree about whose notes are whose.
//!
//! ## Tenancy
//!
//! Every statement filters on `tenant_id` in SQL as well as relying on the
//! forced-RLS backstop, for [`crate::scopes`]' reason: these run on owner
//! connections too.

use std::collections::BTreeMap;

use sqlx::PgConnection;
use synveda_types::access::RoleKey;
use synveda_types::anchor::{AnchorSet, AnchorSource, ScopeAnchor};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    Error, GroupId, IdentityId, ProjectId, Result, ScopeId, TenantId, WorkspaceId,
};
use uuid::Uuid;

/// Counter: anchor resolutions, labelled `outcome` = `resolved` | `empty`.
/// Emitted here, described by the gateway where the recorder lives (ADR-0007).
pub const ANCHOR_RESOLUTIONS_TOTAL: &str = "synveda_anchor_resolutions_total";

/// What the request selected, if anything.
///
/// Both fields absent is the ordinary case — most calls name no workspace and
/// no project, and the resolver then answers from the caller's own scope, their
/// grants and the tenant root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnchorSelection {
    /// The workspace the request is about.
    pub workspace_id: Option<WorkspaceId>,
    /// The project the request is about. Its workspace is resolved from it, so
    /// naming a project is enough.
    pub project_id: Option<ProjectId>,
}

impl AnchorSelection {
    /// A selection naming nothing.
    #[must_use]
    pub const fn none() -> Self {
        AnchorSelection {
            workspace_id: None,
            project_id: None,
        }
    }

    /// A selection naming one workspace.
    #[must_use]
    pub const fn workspace(workspace_id: WorkspaceId) -> Self {
        AnchorSelection {
            workspace_id: Some(workspace_id),
            project_id: None,
        }
    }

    /// A selection naming one project.
    #[must_use]
    pub const fn project(project_id: ProjectId) -> Self {
        AnchorSelection {
            workspace_id: None,
            project_id: Some(project_id),
        }
    }
}

/// One candidate scope on the way to an anchor: the scope and why it is here.
struct Candidate {
    scope: Scope,
    source: AnchorSource,
}

/// Resolves the anchors for one request.
///
/// Never fails for a selection that does not exist or belongs to another
/// tenant: an absent workspace contributes no anchor, exactly as a scope of
/// another tenant is absent rather than forbidden everywhere else in this
/// crate. The route's own uniform-404 ownership check is what turns "not
/// yours" into a status code; this resolver's job is only to answer *where*.
#[tracing::instrument(
    name = "store.anchors.resolve",
    skip_all,
    fields(tenant.id = %tenant_id, anchors = tracing::field::Empty),
    err(Display)
)]
pub async fn resolve(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    principal_id: &str,
    identity_id: Option<IdentityId>,
    selection: AnchorSelection,
) -> Result<AnchorSet> {
    let mut candidates: Vec<Candidate> = Vec::new();

    // 1. The caller's own scope. Looked up, never minted here: a read path
    //    must not write, and the one route that mints is `/v1/me`.
    if let Some(scope) = crate::scopes::principal_scope(&mut *conn, tenant_id, principal_id).await?
    {
        candidates.push(Candidate {
            scope,
            source: AnchorSource::PrincipalScope,
        });
    }

    // 2 and 3. The selection. A project names its workspace through the scope
    //    tree, so resolving the project's ancestry covers both.
    let mut selected_scopes: Vec<ScopeId> = Vec::new();
    if let Some(project_id) = selection.project_id
        && let Some(project) = crate::projects::get(&mut *conn, tenant_id, project_id).await?
        && let Some(scope) = crate::scopes::get(&mut *conn, tenant_id, project.scope_id).await?
    {
        selected_scopes.push(scope.id);
        candidates.push(Candidate {
            scope,
            source: AnchorSource::SelectedProject,
        });
    }
    if let Some(workspace_id) = selection.workspace_id
        && let Some(workspace) = crate::workspaces::get(&mut *conn, tenant_id, workspace_id).await?
        && let Some(scope) = crate::scopes::get(&mut *conn, tenant_id, workspace.scope_id).await?
    {
        selected_scopes.push(scope.id);
        candidates.push(Candidate {
            scope,
            source: AnchorSource::SelectedWorkspace,
        });
    }

    // 4. The organisation-unit relationships above the selection. Every
    //    ancestor, whatever its shape: a workspace's ancestry can be org units
    //    nested to any depth, or nothing at all but the tenant root.
    for scope_id in &selected_scopes {
        for ancestor in crate::scopes::ancestors(&mut *conn, tenant_id, *scope_id).await? {
            let source = source_for_ancestor(ancestor.kind);
            candidates.push(Candidate {
                scope: ancestor,
                source,
            });
        }
    }

    // 5. The tenant root. Always applicable when it exists — it is where a
    //    tenant-wide grant is written and where the default profile sits.
    if let Some(root) = crate::scopes::tenant_root(&mut *conn, tenant_id).await? {
        candidates.push(Candidate {
            scope: root,
            source: AnchorSource::TenantRoot,
        });
    }

    // 6. Everywhere a grant reaches this caller, selected or not. This is what
    //    makes project-only access work: somebody granted one project inside a
    //    workspace they cannot otherwise see still has that project as an
    //    anchor.
    for scope in granted_scopes(&mut *conn, tenant_id, principal_id, identity_id).await? {
        candidates.push(Candidate {
            scope,
            source: AnchorSource::Grant,
        });
    }

    if candidates.is_empty() {
        metrics::counter!(ANCHOR_RESOLUTIONS_TOTAL, "outcome" => "empty").increment(1);
        tracing::Span::current().record("anchors", 0);
        return Ok(AnchorSet::default());
    }

    let ids: Vec<ScopeId> = candidates
        .iter()
        .map(|candidate| candidate.scope.id)
        .collect();
    let depths = depths_of(&mut *conn, tenant_id, &ids).await?;
    let roles = roles_at(&mut *conn, tenant_id, principal_id, identity_id, &ids).await?;

    let anchors: Vec<ScopeAnchor> = candidates
        .into_iter()
        .map(|candidate| {
            let held = roles.get(&candidate.scope.id);
            ScopeAnchor {
                scope_id: candidate.scope.id,
                kind: candidate.scope.kind,
                parent_scope_id: candidate.scope.parent_scope_id,
                depth: depths.get(&candidate.scope.id).copied().unwrap_or(0),
                source: candidate.source,
                roles: held.map(|held| held.roles.clone()).unwrap_or_default(),
                granted_at: held.map(|held| held.granted_at.clone()).unwrap_or_default(),
                via_groups: held.map(|held| held.via_groups.clone()).unwrap_or_default(),
            }
        })
        .collect();
    let set = AnchorSet::new(anchors);
    metrics::counter!(ANCHOR_RESOLUTIONS_TOTAL, "outcome" => "resolved").increment(1);
    tracing::Span::current().record("anchors", set.len());
    Ok(set)
}

/// Which source an ancestor of the selection counts as.
///
/// The tenant root is its own source because it is the widest thing the model
/// can express; everything else on a selection's ancestry is an organisation
/// unit as far as this resolver is concerned — including a workspace above a
/// project, which is already a more specific source of its own when the
/// request selected it.
const fn source_for_ancestor(kind: ScopeKind) -> AnchorSource {
    match kind {
        ScopeKind::Tenant => AnchorSource::TenantRoot,
        _ => AnchorSource::OrgUnit,
    }
}

/// What this caller holds at one scope.
#[derive(Default)]
struct Held {
    roles: Vec<RoleKey>,
    granted_at: Vec<ScopeId>,
    via_groups: Vec<GroupId>,
}

/// Every scope a direct or group grant reaches this caller at.
///
/// Archived groups resolve to nobody, the same rule
/// [`crate::access::members_of`] applies — a group taken out of use must stop
/// conferring on the next request, not on the next sweep.
async fn granted_scopes(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    principal_id: &str,
    identity_id: Option<IdentityId>,
) -> Result<Vec<Scope>> {
    let rows = sqlx::query!(
        r#"
        select distinct
               s.id             as "id!",
               s.tenant_id      as "tenant_id!",
               s.kind           as "kind!",
               s.parent_scope_id,
               s.slug           as "slug!",
               s.display_name   as "display_name!",
               s.status         as "status!",
               s.attributes     as "attributes!",
               s.principal_id,
               s.created_by,
               s.created_at     as "created_at!",
               s.updated_at     as "updated_at!"
        from scope_grants g
        join scopes s
          on s.tenant_id = g.tenant_id and s.id = g.scope_id
        left join groups grp
          on grp.tenant_id = g.tenant_id and grp.id = g.group_id and grp.status = 'active'
        left join group_members gm
          on gm.tenant_id = g.tenant_id and gm.group_id = grp.id and gm.identity_id = $3
        where g.tenant_id = $1
          and ((g.subject_kind = 'principal' and g.principal_id = $2)
               or (g.subject_kind = 'group' and gm.identity_id is not null))
        "#,
        tenant_id.as_uuid(),
        principal_id,
        identity_id.map(|id| id.as_uuid()) as Option<Uuid>,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;

    rows.into_iter()
        .map(|row| {
            let vocabulary = |err: Error| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            };
            Ok(Scope {
                id: ScopeId::from_uuid(row.id),
                tenant_id: TenantId::from_uuid(row.tenant_id),
                kind: row.kind.parse().map_err(vocabulary)?,
                parent_scope_id: row.parent_scope_id.map(ScopeId::from_uuid),
                slug: row.slug,
                display_name: row.display_name,
                status: row.status.parse().map_err(vocabulary)?,
                attributes: row.attributes,
                principal_id: row.principal_id,
                created_by: row.created_by.map(synveda_types::IdentityId::from_uuid),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
        })
        .collect()
}

/// Each scope's depth: edges from the tenant root, read off the closure.
///
/// A structural measurement of the tree, not a rank: it is what orders an
/// anchor set most-specific-first and nothing else reads it.
async fn depths_of(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    ids: &[ScopeId],
) -> Result<BTreeMap<ScopeId, i32>> {
    let raw: Vec<Uuid> = ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query!(
        r#"
        select c.descendant_id as "scope_id!", max(c.distance) as "depth!"
        from scope_closure c
        where c.tenant_id = $1 and c.descendant_id = any($2::uuid[])
        group by c.descendant_id
        "#,
        tenant_id.as_uuid(),
        &raw,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| (ScopeId::from_uuid(row.scope_id), row.depth))
        .collect())
}

/// The roles reaching this caller at each of `ids`, through the closure.
///
/// The three rules are the ones [`crate::access::members_of`] states, narrowed
/// to one principal: inheritance walks the ancestry, a `principal`-shaped
/// scope inherits nothing, and groups resolve while archived ones resolve to
/// nobody.
async fn roles_at(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    principal_id: &str,
    identity_id: Option<IdentityId>,
    ids: &[ScopeId],
) -> Result<BTreeMap<ScopeId, Held>> {
    let raw: Vec<Uuid> = ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query!(
        r#"
        select tgt.id      as "anchor_id!",
               g.scope_id  as "granted_at!",
               g.role_key  as "role_key!",
               grp.id      as "group_id?"
        from scopes tgt
        join scope_closure c
          on c.tenant_id = tgt.tenant_id
         and c.descendant_id = tgt.id
         -- Principal-private scope isolation: a `principal` scope is somebody's
         -- own, and nothing above it reaches in. The same predicate
         -- `access::members_of` carries, in the same place — the query — so no
         -- caller can apply one of the three rules and forget this one.
         and (c.distance = 0 or tgt.kind <> 'principal')
        join scope_grants g
          on g.tenant_id = tgt.tenant_id and g.scope_id = c.ancestor_id
        left join groups grp
          on grp.tenant_id = g.tenant_id and grp.id = g.group_id and grp.status = 'active'
        left join group_members gm
          on gm.tenant_id = g.tenant_id and gm.group_id = grp.id and gm.identity_id = $3
        where tgt.tenant_id = $1
          and tgt.id = any($4::uuid[])
          and ((g.subject_kind = 'principal' and g.principal_id = $2)
               or (g.subject_kind = 'group' and gm.identity_id is not null))
        order by c.distance, g.role_key, g.id
        "#,
        tenant_id.as_uuid(),
        principal_id,
        identity_id.map(|id| id.as_uuid()) as Option<Uuid>,
        &raw,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;

    let mut held: BTreeMap<ScopeId, Held> = BTreeMap::new();
    for row in rows {
        let role: RoleKey = row.role_key.parse().map_err(|err| Error::Internal {
            message: format!("stored value outside vocabulary: {err}"),
        })?;
        let entry = held.entry(ScopeId::from_uuid(row.anchor_id)).or_default();
        entry.roles.push(role);
        entry.granted_at.push(ScopeId::from_uuid(row.granted_at));
        if let Some(group_id) = row.group_id {
            entry.via_groups.push(GroupId::from_uuid(group_id));
        }
    }
    Ok(held)
}

/// The groups this caller is in — what the PDP materialises `Group` entities
/// from, so a pack can name one directly.
///
/// Archived groups are absent, for [`granted_scopes`]' reason: a group taken
/// out of use confers nothing, and a pack rule naming it must stop matching at
/// the same instant a grant through it stops applying.
#[tracing::instrument(
    name = "store.anchors.groups_of",
    skip_all,
    fields(tenant.id = %tenant_id, groups = tracing::field::Empty),
    err(Display)
)]
pub async fn groups_of(
    executor: impl sqlx::PgExecutor<'_>,
    tenant_id: TenantId,
    identity_id: Option<IdentityId>,
) -> Result<Vec<GroupId>> {
    let rows = sqlx::query_scalar!(
        r#"
        select grp.id as "id!"
        from group_members gm
        join groups grp
          on grp.tenant_id = gm.tenant_id and grp.id = gm.group_id and grp.status = 'active'
        where gm.tenant_id = $1 and gm.identity_id = $2
        order by grp.slug
        "#,
        tenant_id.as_uuid(),
        identity_id.map(|id| id.as_uuid()) as Option<Uuid>,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("groups", rows.len());
    Ok(rows.into_iter().map(GroupId::from_uuid).collect())
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy — the
/// resolver only reads, so there is nothing here to classify as a conflict.
fn storage_error(err: sqlx::Error) -> Error {
    Error::Storage {
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ancestor_of_the_selection_is_an_org_unit_unless_it_is_the_root() {
        assert_eq!(
            source_for_ancestor(ScopeKind::Tenant),
            AnchorSource::TenantRoot
        );
        for kind in [
            ScopeKind::OrgUnit,
            ScopeKind::Workspace,
            ScopeKind::Project,
            ScopeKind::Principal,
        ] {
            assert_eq!(
                source_for_ancestor(kind),
                AnchorSource::OrgUnit,
                "{kind} above a selection is an organisation-unit relationship"
            );
        }
    }

    /// The privacy rule this module states in SQL, asserted against the type
    /// that owns it — so a change to one is a failure here rather than a
    /// silent divergence between the query and the predicate.
    #[test]
    fn nothing_inherits_into_a_principal_scope() {
        use synveda_types::access::inherits_into;
        assert!(!inherits_into(ScopeKind::Principal));
        for kind in [
            ScopeKind::Tenant,
            ScopeKind::OrgUnit,
            ScopeKind::Workspace,
            ScopeKind::Project,
        ] {
            assert!(inherits_into(kind), "{kind} inherits from its ancestry");
        }
    }

    #[test]
    fn a_selection_names_at_most_what_it_was_given() {
        assert_eq!(AnchorSelection::none(), AnchorSelection::default());
        let workspace = WorkspaceId::new();
        assert_eq!(
            AnchorSelection::workspace(workspace).workspace_id,
            Some(workspace)
        );
        let project = ProjectId::new();
        assert_eq!(AnchorSelection::project(project).project_id, Some(project));
    }
}
