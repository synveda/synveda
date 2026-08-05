//! AUTH-2 store contract: identities (create/read, the first-login race
//! conflict, placement-derived quarantine), group-mapping overrides, and
//! the convention-candidate hierarchy queries (`teams_matching`,
//! `child_by_slug`) JIT resolution rides on (ADR-0013).
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip
//! with a message when it is unset (CI has no database); run them locally
//! with `make dev-up` then `make db-test`. Isolation is by freshly minted
//! UUIDv7 tenants, so a shared dev database is fine.

use std::sync::OnceLock;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{group_mappings, hierarchy, identities, tenants};
use synveda_types::{
    Error, HierarchyNode, IdentityId, IdentityKind, ScopeId, ScopeKind, TenantId, TenantStatus,
};

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

/// Connects (once) to `DATABASE_URL` and applies migrations. `None` = no
/// database configured; every test skips quietly.
fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping identity tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            synveda_store::migrate(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("auth2-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "AUTH-2 fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    tenant
}

async fn tx(pool: &PgPool) -> Transaction<'static, Postgres> {
    pool.begin().await.expect("begin transaction")
}

/// Creates a node in its own committed transaction.
async fn add(
    pool: &PgPool,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    let mut tx = tx(pool).await;
    let node = hierarchy::create(&mut tx, ScopeId::new(), tenant, parent, kind, slug, slug)
        .await
        .unwrap_or_else(|err| panic!("create {kind} {slug:?}: {err}"));
    tx.commit().await.expect("commit create");
    node
}

/// org → eng (dept) → platform (team), plus a quarantine team under the
/// root. Returns (org, eng, platform, quarantine).
async fn seed(
    pool: &PgPool,
    tenant: TenantId,
) -> (HierarchyNode, HierarchyNode, HierarchyNode, HierarchyNode) {
    let org = add(pool, tenant, None, ScopeKind::Org, "acme").await;
    let eng = add(pool, tenant, Some(org.id), ScopeKind::Department, "eng").await;
    let platform = add(pool, tenant, Some(eng.id), ScopeKind::Team, "platform").await;
    let quarantine = add(
        pool,
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
    )
    .await;
    (org, eng, platform, quarantine)
}

async fn provision(
    pool: &PgPool,
    tenant: TenantId,
    subject: &str,
    parent: ScopeId,
) -> synveda_types::Identity {
    let mut tx = tx(pool).await;
    let personal = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(parent),
        ScopeKind::User,
        &format!("{subject}-scope"),
        subject,
    )
    .await
    .expect("create personal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        Some(&format!("{subject}@example.test")),
        Some(subject),
        personal.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit provision");
    identity
}

// ── Identities ───────────────────────────────────────────────────────────────

#[test]
fn create_then_by_subject_roundtrips_with_derived_quarantine() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (_, _, platform, quarantine) = seed(&db.pool, tenant).await;

        let alice = provision(&db.pool, tenant, "alice", platform.id).await;
        assert!(!alice.quarantined, "a team placement is not quarantined");
        let read = identities::by_subject(&db.pool, tenant, "alice")
            .await
            .expect("read alice")
            .expect("alice exists");
        assert_eq!(read, alice);
        assert_eq!(read.email.as_deref(), Some("alice@example.test"));

        let bob = provision(&db.pool, tenant, "bob", quarantine.id).await;
        assert!(
            bob.quarantined,
            "a placement under the root's quarantine child derives true"
        );

        // Unknown subjects and foreign tenants read as absent.
        assert_eq!(
            identities::by_subject(&db.pool, tenant, "nobody")
                .await
                .expect("read nobody"),
            None
        );
        assert_eq!(
            identities::by_subject(&db.pool, TenantId::new(), "alice")
                .await
                .expect("read cross-tenant"),
            None
        );
    });
}

/// Only the *root's* quarantine child means quarantine: a team that merely
/// reuses the slug deeper down does not (ADR-0013 decision 4).
#[test]
fn a_nested_quarantine_slug_is_not_the_quarantine_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (_, eng, _, _) = seed(&db.pool, tenant).await;
        let nested = add(
            &db.pool,
            tenant,
            Some(eng.id),
            ScopeKind::Team,
            identities::QUARANTINE_SLUG,
        )
        .await;
        let carol = provision(&db.pool, tenant, "carol", nested.id).await;
        assert!(
            !carol.quarantined,
            "acme/eng/quarantine is an ordinary team, not the reserved scope"
        );
    });
}

/// The first-login race resolves at the unique constraint: the second
/// insert for a subject is a Conflict the caller retries into a read.
#[test]
fn duplicate_subject_is_a_conflict() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (_, _, platform, _) = seed(&db.pool, tenant).await;
        provision(&db.pool, tenant, "alice", platform.id).await;

        let mut tx = tx(&db.pool).await;
        let second_scope = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(platform.id),
            ScopeKind::User,
            "alice-second",
            "alice",
        )
        .await
        .expect("create second scope");
        let result = identities::create(
            &mut tx,
            IdentityId::new(),
            tenant,
            Some("alice"),
            IdentityKind::User,
            None,
            None,
            second_scope.id,
        )
        .await;
        assert!(
            matches!(result, Err(Error::Conflict { .. })),
            "a duplicate subject must be a Conflict, got {result:?}"
        );
    });
}

// ── Convention & override queries ────────────────────────────────────────────

#[test]
fn teams_matching_validates_candidates_against_the_hierarchy() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (org, eng, platform, _) = seed(&db.pool, tenant).await;

        // The real pair matches; the impostor halves do not.
        let matched = hierarchy::teams_matching(
            &db.pool,
            tenant,
            &["eng".to_owned(), "platform".to_owned(), "eng".to_owned()],
            &["platform".to_owned(), "eng".to_owned(), "nosuch".to_owned()],
        )
        .await
        .expect("match candidates");
        assert_eq!(
            matched.iter().map(|n| n.id).collect::<Vec<_>>(),
            [platform.id],
            "exactly the (eng, platform) candidate resolves"
        );

        // A division between org and department does not break ancestry
        // (closure, not adjacency), and a same-slug team under another
        // department makes the candidate ambiguous — two rows come back.
        let emea = add(&db.pool, tenant, Some(org.id), ScopeKind::Division, "emea").await;
        let data = add(
            &db.pool,
            tenant,
            Some(emea.id),
            ScopeKind::Department,
            "data",
        )
        .await;
        let data_platform = add(&db.pool, tenant, Some(data.id), ScopeKind::Team, "platform").await;
        let via_division = hierarchy::teams_matching(
            &db.pool,
            tenant,
            &["data".to_owned()],
            &["platform".to_owned()],
        )
        .await
        .expect("match through a division");
        assert_eq!(
            via_division.iter().map(|n| n.id).collect::<Vec<_>>(),
            [data_platform.id]
        );

        let dup = add(&db.pool, tenant, Some(eng.id), ScopeKind::Team, "core").await;
        let dup2 = add(&db.pool, tenant, Some(data.id), ScopeKind::Team, "core").await;
        let _ = (dup, dup2);
        let ambiguous = hierarchy::teams_matching(
            &db.pool,
            tenant,
            &["eng".to_owned(), "data".to_owned()],
            &["core".to_owned(), "core".to_owned()],
        )
        .await
        .expect("match ambiguous candidates");
        assert_eq!(
            ambiguous.len(),
            2,
            "both cores match — the caller must treat this as unresolved"
        );

        // Another tenant's identical shape never leaks in.
        let foreign = admit_tenant(&db.pool).await;
        seed(&db.pool, foreign).await;
        let cross = hierarchy::teams_matching(
            &db.pool,
            foreign,
            &["eng".to_owned()],
            &["platform".to_owned()],
        )
        .await
        .expect("match in foreign tenant");
        assert_eq!(cross.len(), 1);
        assert_ne!(cross[0].id, platform.id, "same shape, that tenant's node");
    });
}

#[test]
fn child_by_slug_finds_exactly_the_named_child() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (org, _, _, quarantine) = seed(&db.pool, tenant).await;
        let found = hierarchy::child_by_slug(&db.pool, org.id, identities::QUARANTINE_SLUG)
            .await
            .expect("lookup quarantine");
        assert_eq!(found.as_ref().map(|n| n.id), Some(quarantine.id));
        assert_eq!(
            hierarchy::child_by_slug(&db.pool, org.id, "nosuch")
                .await
                .expect("lookup missing"),
            None
        );
    });
}

#[test]
fn group_mapping_upsert_read_remove_roundtrips() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (_, eng, platform, _) = seed(&db.pool, tenant).await;

        let mapping = group_mappings::upsert(&db.pool, tenant, "contractors", platform.id)
            .await
            .expect("upsert mapping");
        assert_eq!(mapping.scope_id, platform.id);
        // Re-pointing is the same upsert.
        let repointed = group_mappings::upsert(&db.pool, tenant, "contractors", eng.id)
            .await
            .expect("re-point mapping");
        assert_eq!(repointed.scope_id, eng.id);

        group_mappings::upsert(&db.pool, tenant, "auditors", eng.id)
            .await
            .expect("second mapping");
        let matched = group_mappings::for_groups(
            &db.pool,
            tenant,
            &[
                "contractors".to_owned(),
                "auditors".to_owned(),
                "unmapped".to_owned(),
            ],
        )
        .await
        .expect("read mappings");
        assert_eq!(
            matched
                .iter()
                .map(|m| m.group_name.as_str())
                .collect::<Vec<_>>(),
            ["auditors", "contractors"],
            "group-name order is the resolver's determinism contract"
        );

        // A mapping to a vanished scope is unrepresentable (FK).
        let dangling = group_mappings::upsert(&db.pool, tenant, "ghost", ScopeId::new()).await;
        assert!(
            matches!(dangling, Err(Error::NotFound { .. })),
            "a mapping to a missing scope must be NotFound, got {dangling:?}"
        );

        assert!(
            group_mappings::remove(&db.pool, tenant, "contractors")
                .await
                .expect("remove mapping")
        );
        assert!(
            !group_mappings::remove(&db.pool, tenant, "contractors")
                .await
                .expect("remove again"),
            "second removal is a no-op"
        );
    });
}
