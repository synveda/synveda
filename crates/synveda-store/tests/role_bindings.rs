//! Store contract for role bindings (AUTHZ-3, ADR-0015): bind upserts per
//! (tenant, subject, scope, role) with `None` as the tenant-wide scope,
//! unbind is exact, the chain query returns node rows plus tenant-wide
//! rows, and the vocabulary is closed by the check constraint. The
//! adversarial RLS coverage lives in `tests/rls.rs` (ADR-0009 structural
//! rule).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::{hierarchy, rls, role_bindings, tenants};
use synveda_types::{Error, Role, ScopeId, ScopeKind, TenantId, TenantStatus};

/// Connects and migrates. `None` = no database configured; the test skips
/// quietly.
async fn db() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping role binding tests: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    Some(pool)
}

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    let slug = format!("role-{}", id.as_uuid().simple());
    tenants::create(
        pool,
        id,
        &slug,
        "AUTHZ-3 binding test",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    id
}

/// org → team; returns (org, team).
async fn small_hierarchy(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
) -> (synveda_types::HierarchyNode, synveda_types::HierarchyNode) {
    let org = hierarchy::create(
        &mut *tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "org",
        "Org",
    )
    .await
    .expect("create org");
    let team = hierarchy::create(
        &mut *tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        "team",
        "Team",
    )
    .await
    .expect("create team");
    (org, team)
}

#[tokio::test]
async fn bind_upserts_and_unbind_is_exact() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    // One tenant transaction, dropped at the end: the fixture leaves no
    // rows behind on the shared dev database.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let (org, team) = small_hierarchy(&mut tx, tenant).await;

    let bound = role_bindings::bind(&mut *tx, tenant, "alice", Some(team.id), Role::Steward)
        .await
        .expect("bind");
    assert_eq!(bound.subject, "alice");
    assert_eq!(bound.scope_id, Some(team.id));
    assert_eq!(bound.role, Role::Steward);

    // Re-binding the same row is idempotent, not an error or a duplicate.
    role_bindings::bind(&mut *tx, tenant, "alice", Some(team.id), Role::Steward)
        .await
        .expect("re-bind");
    // A different role at the same node is a second binding.
    role_bindings::bind(&mut *tx, tenant, "alice", Some(team.id), Role::Viewer)
        .await
        .expect("bind second role");
    let at_team = role_bindings::for_scope(&mut *tx, tenant, team.id)
        .await
        .expect("list team");
    assert_eq!(at_team.len(), 2, "one steward + one viewer row");

    // Unbind is exact: the wrong role or the wrong scope removes nothing.
    assert!(
        !role_bindings::unbind(&mut *tx, tenant, "alice", Some(org.id), Role::Steward)
            .await
            .expect("unbind wrong scope"),
    );
    assert!(
        !role_bindings::unbind(&mut *tx, tenant, "alice", Some(team.id), Role::OrgAdmin)
            .await
            .expect("unbind wrong role"),
    );
    assert!(
        role_bindings::unbind(&mut *tx, tenant, "alice", Some(team.id), Role::Steward)
            .await
            .expect("unbind"),
    );
    let at_team = role_bindings::for_scope(&mut *tx, tenant, team.id)
        .await
        .expect("list team");
    assert_eq!(at_team.len(), 1);
    assert_eq!(at_team[0].role, Role::Viewer);
}

#[tokio::test]
async fn tenant_wide_bindings_upsert_under_nulls_not_distinct() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");

    // No hierarchy at all: a fresh tenant is bindable tenant-wide — the
    // bootstrap property (ADR-0015 decisions 2 and 6).
    role_bindings::bind(&mut *tx, tenant, "root-admin", None, Role::OrgAdmin)
        .await
        .expect("tenant-wide bind");
    // The same tenant-wide row upserts rather than duplicating (`nulls
    // not distinct`).
    role_bindings::bind(&mut *tx, tenant, "root-admin", None, Role::OrgAdmin)
        .await
        .expect("tenant-wide re-bind");
    let all = role_bindings::all(&mut *tx, tenant).await.expect("list");
    assert_eq!(all.len(), 1, "nulls-not-distinct upsert, not a duplicate");
    assert_eq!(all[0].scope_id, None);

    assert!(
        role_bindings::unbind(&mut *tx, tenant, "root-admin", None, Role::OrgAdmin)
            .await
            .expect("tenant-wide unbind"),
    );
    assert!(
        role_bindings::all(&mut *tx, tenant)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn chain_query_returns_chain_rows_plus_tenant_wide() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let (org, team) = small_hierarchy(&mut tx, tenant).await;

    role_bindings::bind(&mut *tx, tenant, "alice", Some(org.id), Role::Auditor)
        .await
        .expect("bind at org");
    role_bindings::bind(&mut *tx, tenant, "alice", Some(team.id), Role::Viewer)
        .await
        .expect("bind at team");
    role_bindings::bind(&mut *tx, tenant, "alice", None, Role::Compliance)
        .await
        .expect("bind tenant-wide");
    role_bindings::bind(&mut *tx, tenant, "carol", Some(team.id), Role::Steward)
        .await
        .expect("bind other subject");

    // The team's chain: node rows for org+team, plus the tenant-wide row;
    // never another subject's rows.
    let chain = [team.id, org.id];
    let rows = role_bindings::for_subject_on_scopes(&mut *tx, tenant, "alice", &chain)
        .await
        .expect("chain query");
    let mut roles: Vec<Role> = rows.iter().map(|binding| binding.role).collect();
    roles.sort_by_key(Role::as_str);
    assert_eq!(roles, [Role::Auditor, Role::Compliance, Role::Viewer]);

    // A tenant resource (empty chain): tenant-wide rows only.
    let rows = role_bindings::for_subject_on_scopes(&mut *tx, tenant, "alice", &[])
        .await
        .expect("tenant-resource query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, Role::Compliance);
    assert_eq!(rows[0].scope_id, None);

    // Only the org on the chain: the team row drops out.
    let rows = role_bindings::for_subject_on_scopes(&mut *tx, tenant, "alice", &[org.id])
        .await
        .expect("partial chain query");
    let mut roles: Vec<Role> = rows.iter().map(|binding| binding.role).collect();
    roles.sort_by_key(Role::as_str);
    assert_eq!(roles, [Role::Auditor, Role::Compliance]);
}

#[tokio::test]
async fn cross_tenant_scope_and_unknown_subject_shapes_are_rejected() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let other = admit_tenant(&pool).await;

    // A node of another tenant is unrepresentable as a binding target:
    // the composite FK (tenant_id, scope_id) cannot match.
    let foreign_team = {
        let mut tx = rls::begin_tenant_tx(&pool, other).await.expect("begin");
        let (_, team) = small_hierarchy(&mut tx, other).await;
        tx.commit().await.expect("commit foreign hierarchy");
        team
    };
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let err = role_bindings::bind(
        &mut *tx,
        tenant,
        "alice",
        Some(foreign_team.id),
        Role::Viewer,
    )
    .await
    .expect_err("cross-tenant binding must be rejected");
    assert!(matches!(err, Error::NotFound { .. }), "got {err:?}");
    drop(tx);

    // The empty subject fails the check constraint as caller input.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let err = role_bindings::bind(&mut *tx, tenant, "", None, Role::Viewer)
        .await
        .expect_err("empty subject must be rejected");
    assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");

    // Cleanup of the committed foreign hierarchy rows.
    let mut tx = rls::begin_tenant_tx(&pool, other).await.expect("begin");
    hierarchy::delete(&mut tx, foreign_team.id)
        .await
        .expect("delete foreign team");
    let root = hierarchy::root(&mut *tx, other)
        .await
        .expect("read root")
        .expect("root exists");
    hierarchy::delete(&mut tx, root.id)
        .await
        .expect("delete foreign org");
    tx.commit().await.expect("commit cleanup");
}
