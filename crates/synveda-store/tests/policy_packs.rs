//! Store contract for policy packs and their application (AUTHZ-1
//! ADR-0012; AUTHZ-2 ADR-0014): `apply` upserts per (tenant, name) and
//! owns that name's version bump, product names are reserved by the check
//! constraint, `clear` refuses while assignments or the tenant default
//! still reference the pack, and assignments/defaults follow their own
//! upsert lifecycle. The adversarial RLS coverage lives in `tests/rls.rs`
//! (ADR-0009 structural rule).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::{hierarchy, policy_assignments, policy_packs, rls, tenants};
use synveda_types::{
    CompositionConfig, Error, InjectChannels, PackConfig, ScopeId, ScopeKind, TenantId,
    TenantStatus,
};

/// Connects and migrates. `None` = no database configured; the test skips
/// quietly.
async fn db() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping policy pack tests: DATABASE_URL is not set \
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
    let slug = format!("pack-{}", id.as_uuid().simple());
    tenants::create(pool, id, &slug, "AUTHZ-2 pack test", TenantStatus::Active)
        .await
        .expect("admit tenant");
    id
}

#[tokio::test]
async fn apply_versions_per_name_and_clear_removes() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    // One tenant transaction, dropped at the end: the fixture leaves no
    // pack rows behind on the shared dev database.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");

    assert!(
        policy_packs::stored(&mut *tx, tenant)
            .await
            .expect("read empty")
            .is_empty(),
        "a fresh tenant stores no packs"
    );

    let first = policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-test",
        "permit (p) v1;",
        &PackConfig::default(),
    )
    .await
    .expect("first apply");
    assert_eq!(first.version, 1);
    assert_eq!(first.name, "authz2-test");

    // A different name is a different pack with its own version counter.
    let sibling = policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-strict",
        "permit (p) v1;",
        &PackConfig::default(),
    )
    .await
    .expect("sibling apply");
    assert_eq!(sibling.version, 1);

    // Re-applying a name is a new version, even with identical content —
    // the reloader's unchanged-skip and the decision log both see the
    // change.
    let bumped = policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-test",
        "permit (p) v1;",
        &PackConfig::default(),
    )
    .await
    .expect("re-apply");
    assert_eq!(bumped.version, 2);

    let names: Vec<String> = policy_packs::stored(&mut *tx, tenant)
        .await
        .expect("list stored")
        .into_iter()
        .map(|pack| pack.name)
        .collect();
    assert_eq!(names, vec!["authz2-strict", "authz2-test"]);
    assert_eq!(
        policy_packs::get(&mut *tx, tenant, "authz2-test")
            .await
            .expect("get by name"),
        Some(bumped)
    );

    assert!(
        policy_packs::clear(&mut tx, tenant, "authz2-test")
            .await
            .expect("clear"),
        "clear removes the named pack"
    );
    assert!(
        !policy_packs::clear(&mut tx, tenant, "authz2-test")
            .await
            .expect("second clear"),
        "second clear is a no-op"
    );
    let remaining = policy_packs::stored(&mut *tx, tenant)
        .await
        .expect("read after clear");
    assert_eq!(remaining.len(), 1, "the sibling pack must survive");
}

/// The composition config rides the stored pack like the redaction
/// config does (CTX-2, ADR-0025 decision 3): an apply with a config
/// stores it, a re-apply without one clears it (an apply is a full
/// statement, never a partial patch), and unparseable stored json reads
/// back as unconfigured.
#[tokio::test]
async fn composition_config_rides_the_pack_and_clears_on_reapply() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");

    let config = CompositionConfig {
        budget_tokens: 900,
        channels: InjectChannels::PublishedOnly,
        ..CompositionConfig::DEFAULT
    };
    let stored = policy_packs::apply(
        &mut *tx,
        tenant,
        "ctx2-bank",
        "permit (p);",
        &PackConfig {
            composition: Some(config),
            ..Default::default()
        },
    )
    .await
    .expect("apply with composition config");
    assert_eq!(stored.config.composition, Some(config));
    assert_eq!(
        policy_packs::get(&mut *tx, tenant, "ctx2-bank")
            .await
            .expect("get")
            .expect("stored")
            .config
            .composition,
        Some(config),
        "the config reads back"
    );

    let cleared = policy_packs::apply(
        &mut *tx,
        tenant,
        "ctx2-bank",
        "permit (p);",
        &PackConfig::default(),
    )
    .await
    .expect("re-apply without config");
    assert_eq!(cleared.version, 2);
    assert_eq!(
        cleared.config.composition, None,
        "an apply is a full statement — the config cleared"
    );

    // Out-of-band garbage reads back as unconfigured, loudly (the
    // fail-safe downstream is the product default, which narrows
    // nothing).
    sqlx::query("update policy_packs set composition = '\"garbage\"'::jsonb where tenant_id = $1 and name = 'ctx2-bank'")
        .bind(tenant.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("corrupt stored config out of band");
    assert_eq!(
        policy_packs::get(&mut *tx, tenant, "ctx2-bank")
            .await
            .expect("get corrupted")
            .expect("stored")
            .config
            .composition,
        None,
        "unparseable stored json is unconfigured, never a panic"
    );
}

/// A pack still referenced by an assignment or the tenant default cannot
/// be cleared (ADR-0014 decision 7): the dangling-name fallback exists for
/// out-of-band writes, never the product path.
#[tokio::test]
async fn clear_refuses_while_assignments_reference_the_pack() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = ScopeId::new();
    hierarchy::create(&mut tx, root, tenant, None, ScopeKind::Org, "acme", "ACME")
        .await
        .expect("create org root");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-pinned",
        "permit (p);",
        &PackConfig::default(),
    )
    .await
    .expect("apply pack");

    // Referenced by a node assignment.
    policy_assignments::assign(&mut *tx, tenant, root, "authz2-pinned")
        .await
        .expect("assign pack");
    let refused = policy_packs::clear(&mut tx, tenant, "authz2-pinned").await;
    assert!(
        matches!(refused, Err(Error::Conflict { .. })),
        "clearing an assigned pack must be Conflict, got {refused:?}"
    );
    assert!(
        policy_assignments::unassign(&mut *tx, tenant, root)
            .await
            .expect("unassign")
    );

    // Referenced by the tenant default.
    policy_assignments::set_default(&mut *tx, tenant, "authz2-pinned")
        .await
        .expect("set default");
    let refused = policy_packs::clear(&mut tx, tenant, "authz2-pinned").await;
    assert!(
        matches!(refused, Err(Error::Conflict { .. })),
        "clearing the default pack must be Conflict, got {refused:?}"
    );
    assert!(
        policy_assignments::clear_default(&mut *tx, tenant)
            .await
            .expect("clear default")
    );

    // Unreferenced: clear succeeds.
    assert!(
        policy_packs::clear(&mut tx, tenant, "authz2-pinned")
            .await
            .expect("clear unreferenced")
    );
}

/// Assignments and the tenant default follow their own upsert lifecycle,
/// and cascade with their node (HIER-1 deletes are leaf-only).
#[tokio::test]
async fn assignments_upsert_resolve_by_chain_and_cascade() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = ScopeId::new();
    let team = ScopeId::new();
    hierarchy::create(&mut tx, root, tenant, None, ScopeKind::Org, "acme", "ACME")
        .await
        .expect("create org root");
    hierarchy::create(
        &mut tx,
        team,
        tenant,
        Some(root),
        ScopeKind::Team,
        "core",
        "Core",
    )
    .await
    .expect("create team");

    let assigned = policy_assignments::assign(&mut *tx, tenant, team, "standard")
        .await
        .expect("assign");
    assert_eq!(assigned.pack_name, "standard");
    let replaced = policy_assignments::assign(&mut *tx, tenant, team, "open-collaboration")
        .await
        .expect("replace assignment");
    assert_eq!(replaced.pack_name, "open-collaboration");

    // Chain lookup returns only assigned nodes, any of the asked ids.
    let for_chain = policy_assignments::for_scopes(&mut *tx, tenant, &[team, root])
        .await
        .expect("chain lookup");
    assert_eq!(for_chain, vec![replaced]);

    // The tenant default has its own lifecycle.
    assert_eq!(
        policy_assignments::default_pack(&mut *tx, tenant)
            .await
            .expect("empty default"),
        None
    );
    policy_assignments::set_default(&mut *tx, tenant, "standard")
        .await
        .expect("set default");
    assert_eq!(
        policy_assignments::default_pack(&mut *tx, tenant)
            .await
            .expect("read default"),
        Some("standard".to_owned())
    );

    // Deleting the node deletes its assignment with it.
    assert!(
        hierarchy::delete(&mut tx, team).await.expect("delete team"),
        "the leaf team must delete"
    );
    assert!(
        policy_assignments::for_scopes(&mut *tx, tenant, &[team])
            .await
            .expect("post-delete lookup")
            .is_empty(),
        "the deleted node's assignment must cascade away"
    );
}

#[tokio::test]
async fn constraints_map_onto_the_taxonomy() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    // One transaction per failing statement: a constraint violation aborts
    // the whole Postgres transaction, so cases cannot share one.

    // Malformed pack name (slug grammar).
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let bad_name = policy_packs::apply(
        &mut *tx,
        tenant,
        "Not A Slug!",
        "permit;",
        &PackConfig::default(),
    )
    .await;
    assert!(
        matches!(bad_name, Err(Error::Invalid { .. })),
        "malformed name must be Invalid, got {bad_name:?}"
    );
    drop(tx);

    // Reserved product names (ADR-0014 decision 6).
    for reserved in [
        "regulated-strict",
        "standard",
        "open-collaboration",
        "bootstrap",
    ] {
        let mut tx = rls::begin_tenant_tx(&pool, tenant)
            .await
            .expect("begin tenant tx");
        let refused = policy_packs::apply(
            &mut *tx,
            tenant,
            reserved,
            "permit;",
            &PackConfig::default(),
        )
        .await;
        assert!(
            matches!(refused, Err(Error::Invalid { .. })),
            "storing {reserved} must be Invalid, got {refused:?}"
        );
        drop(tx);
    }

    // Empty source.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let empty =
        policy_packs::apply(&mut *tx, tenant, "authz2-empty", "", &PackConfig::default()).await;
    assert!(
        matches!(empty, Err(Error::Invalid { .. })),
        "empty source must be Invalid, got {empty:?}"
    );
    drop(tx);

    // Unknown tenant: the FK reports it as NotFound.
    let ghost = TenantId::new();
    let mut tx = rls::begin_tenant_tx(&pool, ghost)
        .await
        .expect("begin ghost tx");
    let orphan = policy_packs::apply(
        &mut *tx,
        ghost,
        "authz2-ghost",
        "permit;",
        &PackConfig::default(),
    )
    .await;
    assert!(
        matches!(orphan, Err(Error::NotFound { .. })),
        "unknown tenant must be NotFound, got {orphan:?}"
    );
    drop(tx);

    // An assignment to a node of another tenant (or none) is NotFound.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let foreign = policy_assignments::assign(&mut *tx, tenant, ScopeId::new(), "standard").await;
    assert!(
        matches!(foreign, Err(Error::NotFound { .. })),
        "assigning a ghost scope must be NotFound, got {foreign:?}"
    );
}
