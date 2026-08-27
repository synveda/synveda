//! FLOW-1 acceptance criteria (ADR-0030): property tests proving that
//! identical content dedups and that history is immutable under concurrent
//! writers.
//!
//! Both properties are claims about Postgres, not about Rust, so both are
//! tested against Postgres. The concurrency half in particular runs genuinely
//! concurrent writers on their own pooled connections with overlapping
//! transactions — a simulated race would prove nothing about the compare-and
//! -swap it exists to check (ADR-0030 decision 12).
//!
//! These tests need a live, migrated Postgres. They read `DATABASE_URL` and
//! skip when it is unset (CI has no database) or when the VedaFlow tables are
//! missing; run them locally with `make db-test` or via
//! `demos/flow-1-object-store.sh`.
//!
//! Layering note: `synveda-vedaflow` sits beside `synveda-store`, so this
//! suite carries its own tenant-GUC helper instead of importing
//! `rls::begin_tenant_tx` — same GUC, same transaction-local shape. Same
//! reason `synveda-audit`'s tamper suite carries one.

use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

use chrono::{DateTime, TimeZone, Utc};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_types::{AssetKind, Error, IdentityId, Result, ScopeId, TenantId};
use synveda_vedaflow::{
    CommitHash, Ed25519Signer, NewCommit, ObjectClass, PolicySnapshot, RefUpdate, Signer,
    StoreVerification, TreeEntry, commit, create_ref, is_ancestor, put_object, put_tree,
    read_commit, read_object, read_ref, read_tree, update_ref, verify, verify_ed25519,
};

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

/// Connects (once) to `DATABASE_URL` and checks the VedaFlow tables exist.
/// `None` = no database (or an unmigrated one); every test skips quietly.
fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping object store tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        // Multi-threaded on purpose: the concurrency property is about
        // writers that genuinely overlap, not about interleaved awaits.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                .max_connections(16)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            let migrated = sqlx::query_scalar::<_, Option<String>>(
                "select to_regclass('public.vedaflow_objects')::text",
            )
            .fetch_one(&pool)
            .await
            .expect("probe for vedaflow_objects");
            if migrated.is_none() {
                eprintln!(
                    "skipping object store tests: VedaFlow tables missing — apply \
                     migrations first (`synveda db migrate`)"
                );
                return None;
            }
            Some(pool)
        })?;
        Some(Db { rt, pool })
    })
    .as_ref()
}

/// A transaction with the tenant GUC set — the same transaction-local shape
/// as `synveda_store::rls::begin_tenant_tx`.
async fn tenant_tx(pool: &PgPool, tenant: TenantId) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("begin tenant transaction");
    sqlx::query("select set_config('synveda.tenant_id', $1, true)")
        .bind(tenant.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .expect("set tenant GUC");
    tx
}

/// Admits a fresh tenant. The FK from every VedaFlow table means history
/// cannot exist without one.
async fn create_tenant(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    sqlx::query("insert into tenants (id, slug, name, status) values ($1, $2, $3, 'active')")
        .bind(tenant.as_uuid())
        .bind(format!("flow1-{}", tenant.as_uuid().simple()))
        .bind("FLOW-1 property test")
        .execute(pool)
        .await
        .expect("admit tenant");
    tenant
}

/// A fixed instant: the properties under test are about content, not clocks,
/// and a `now()` in the hash would make "identical content" impossible to
/// state.
fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap()
}

fn pack() -> PolicySnapshot {
    PolicySnapshot::new("regulated-strict", 5)
        .with_config(serde_json::json!({"budget_tokens": 1500}))
}

/// A commit over a one-entry tree holding `body`.
async fn make_commit(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    author: IdentityId,
    body: &str,
    parents: Vec<CommitHash>,
) -> Result<CommitHash> {
    let object = put_object(conn, tenant, AssetKind::Knowledge, body.as_bytes()).await?;
    let tree = put_tree(conn, tenant, &[TreeEntry::object("note.md", object.hash)]).await?;
    let head = commit(
        conn,
        tenant,
        &NewCommit {
            tree: tree.hash,
            parents,
            author,
            message: format!("seed: {body}"),
            committed_at: at(),
            policy_snapshot: pack(),
        },
        &Signer::Unsigned,
    )
    .await?;
    Ok(head.hash)
}

/// A root commit — the starting point for the ref tests.
async fn seed_commit(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    author: IdentityId,
    body: &str,
) -> Result<CommitHash> {
    make_commit(conn, tenant, author, body, vec![]).await
}

// ── Property 1: identical content dedups ─────────────────────────────────────

/// Blobs, trees, and commits alike: writing the same content twice yields one
/// row and the same address, and the second write says so.
async fn check_dedup_case(pool: &PgPool, blobs: Vec<(AssetKind, Vec<u8>)>) {
    let tenant = create_tenant(pool).await;
    let author = IdentityId::new();
    let mut tx = tenant_tx(pool, tenant).await;
    let conn = &mut *tx;

    // Objects: every write, then every write again.
    let mut first = Vec::new();
    for (kind, content) in &blobs {
        let written = put_object(conn, tenant, *kind, content)
            .await
            .expect("put object");
        first.push(written);
    }
    for (index, (kind, content)) in blobs.iter().enumerate() {
        let again = put_object(conn, tenant, *kind, content)
            .await
            .expect("re-put object");
        assert_eq!(
            again.hash, first[index].hash,
            "the same content must address the same object"
        );
        assert!(
            again.deduplicated,
            "the second write of identical content must dedup"
        );
        assert_eq!(
            read_object(conn, tenant, again.hash)
                .await
                .expect("read object")
                .expect("object exists"),
            synveda_vedaflow::StoredObject {
                kind: *kind,
                content: content.clone(),
            },
            "a deduplicated write must leave the original content intact"
        );
    }

    // Row count equals the number of *distinct* (kind, content) pairs, not
    // the number of writes — dedup is the primary key, not a code path.
    let distinct: BTreeSet<(&str, &[u8])> = blobs
        .iter()
        .map(|(kind, content)| (kind.as_str(), content.as_slice()))
        .collect();
    let stored = sqlx::query_scalar!(
        r#"select count(*) as "count!" from vedaflow_objects where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("count objects");
    assert_eq!(
        stored,
        distinct.len() as i64,
        "one row per distinct (kind, content), however many times it was written"
    );

    // Trees: the same entries in any order are the same tree.
    let entries: Vec<TreeEntry> = first
        .iter()
        .enumerate()
        .map(|(index, written)| TreeEntry::object(format!("entry-{index:03}"), written.hash))
        .collect();
    let tree = put_tree(conn, tenant, &entries).await.expect("put tree");
    let mut reversed = entries.clone();
    reversed.reverse();
    let tree_again = put_tree(conn, tenant, &reversed)
        .await
        .expect("re-put tree, reversed");
    assert_eq!(
        tree_again.hash, tree.hash,
        "entry order is the caller's, not the tree's"
    );
    assert!(tree_again.deduplicated, "an identical tree must dedup");
    let read_back = read_tree(conn, tenant, tree.hash)
        .await
        .expect("read tree")
        .expect("tree exists");
    let mut canonical = entries.clone();
    canonical.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    assert_eq!(read_back, canonical, "entries read back in canonical order");

    // Commits: same tree, parents, author, message, instant, and pack.
    let new = NewCommit {
        tree: tree.hash,
        parents: vec![],
        author,
        message: "identical".to_string(),
        committed_at: at(),
        policy_snapshot: pack(),
    };
    let one = commit(conn, tenant, &new, &Signer::Unsigned)
        .await
        .expect("commit");
    let two = commit(conn, tenant, &new, &Signer::Unsigned)
        .await
        .expect("re-commit");
    assert_eq!(one.hash, two.hash, "identical commits address alike");
    assert!(two.deduplicated, "an identical commit must dedup");

    // A different message is a different commit, on the same tree.
    let renamed = commit(
        conn,
        tenant,
        &NewCommit {
            message: "different".to_string(),
            ..new
        },
        &Signer::Unsigned,
    )
    .await
    .expect("commit with a different message");
    assert_ne!(renamed.hash, one.hash);
    assert!(!renamed.deduplicated);

    assert!(
        matches!(
            verify(conn, tenant).await.expect("verify"),
            StoreVerification::Valid { .. }
        ),
        "every address must recompute from its own row"
    );
    tx.rollback().await.expect("rollback");
}

fn blobs_strategy() -> impl Strategy<Value = Vec<(AssetKind, Vec<u8>)>> {
    let kind = prop::sample::select(AssetKind::ALL.to_vec());
    // Deliberately narrow bytes and lengths: collisions between generated
    // blobs are the interesting case, and wide random data never collides.
    let content = prop::collection::vec(0u8..4, 0..6);
    prop::collection::vec((kind, content), 1..8)
}

#[test]
fn identical_content_dedups() {
    let Some(db) = db() else { return };
    let mut runner = TestRunner::new(Config::with_cases(24));
    runner
        .run(&blobs_strategy(), |blobs| {
            db.rt.block_on(check_dedup_case(&db.pool, blobs));
            Ok(())
        })
        .unwrap_or_else(|err| panic!("dedup property failed: {err}"));
}

#[test]
fn dedup_is_per_tenant_never_across_one() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (left, right) = (create_tenant(pool).await, create_tenant(pool).await);
        let content = b"the same sentence in two organisations";

        let mut tx = tenant_tx(pool, left).await;
        let in_left = put_object(&mut tx, left, AssetKind::Knowledge, content)
            .await
            .expect("put in left");
        tx.commit().await.expect("commit left");

        let mut tx = tenant_tx(pool, right).await;
        let in_right = put_object(&mut tx, right, AssetKind::Knowledge, content)
            .await
            .expect("put in right");
        tx.commit().await.expect("commit right");

        // The address is global — that is what makes it a content address,
        // and what lets an auditor or the FLOW-8 mirror recompute it.
        assert_eq!(
            in_left.hash, in_right.hash,
            "identical bytes address identically everywhere"
        );
        // The storage is not. Neither write saw the other, so neither
        // deduplicated: a shared row would answer 'already present' for
        // content this tenant never wrote (ADR-0030 decision 3).
        assert!(!in_left.deduplicated && !in_right.deduplicated);

        // Counted per tenant, not globally: this suite shares a long-lived
        // dev database with every previous run of itself.
        let mut tx = tenant_tx(pool, left).await;
        let rows = sqlx::query!(
            r#"select
                 (select count(*) from vedaflow_objects
                  where tenant_id = $1 and hash = $3) as "left!",
                 (select count(*) from vedaflow_objects
                  where tenant_id = $2 and hash = $3) as "right!""#,
            left.as_uuid(),
            right.as_uuid(),
            in_left.hash.as_slice(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count rows at that address");
        assert_eq!(
            (rows.left, rows.right),
            (1, 1),
            "one row per tenant, at the same address"
        );
        tx.rollback().await.expect("rollback");
    });
}

// ── Property 2: history immutable under concurrent writers ───────────────────

/// What one writer did: the commits it landed, and how many times it lost a
/// compare-and-swap on the way. The loss count is reported so the test can
/// prove the writers genuinely contended rather than politely queueing.
struct WriterOutcome {
    landed: Vec<CommitHash>,
    races_lost: usize,
}

/// One writer's whole job: `per_writer` commits onto `published`, each
/// compare-and-swapped against the head it was parented on, retrying on a
/// lost race.
async fn writer(
    pool: PgPool,
    tenant: TenantId,
    scope: ScopeId,
    author: IdentityId,
    id: usize,
    per_writer: usize,
) -> WriterOutcome {
    let pool = &pool;
    let mut landed = Vec::new();
    let mut races_lost = 0;
    for round in 0..per_writer {
        let mut attempts = 0;
        loop {
            attempts += 1;
            assert!(attempts < 500, "writer {id} never won a compare-and-swap");
            let mut tx = tenant_tx(pool, tenant).await;
            let conn = &mut *tx;

            let head = read_ref(conn, tenant, scope, "published")
                .await
                .expect("read ref")
                .expect("ref exists");
            let body = format!("writer {id} round {round} attempt {attempts}");
            let object = put_object(conn, tenant, AssetKind::Knowledge, body.as_bytes())
                .await
                .expect("put object");
            let tree = put_tree(conn, tenant, &[TreeEntry::object("note.md", object.hash)])
                .await
                .expect("put tree");
            let next = commit(
                conn,
                tenant,
                &NewCommit {
                    tree: tree.hash,
                    parents: vec![head.commit_hash],
                    author,
                    message: body,
                    committed_at: at(),
                    policy_snapshot: pack(),
                },
                &Signer::Unsigned,
            )
            .await
            .expect("commit");

            let outcome = update_ref(
                conn,
                tenant,
                scope,
                "published",
                head.commit_hash,
                next.hash,
                author,
            )
            .await
            .expect("update ref");

            if outcome == RefUpdate::Updated {
                tx.commit().await.expect("commit transaction");
                landed.push(next.hash);
                break;
            }
            // Someone advanced the ref first. The whole attempt rolls back —
            // objects, tree, commit and all — so a lost race leaves no
            // orphaned history behind.
            assert_eq!(outcome, RefUpdate::Raced);
            races_lost += 1;
            tx.rollback().await.expect("roll back the lost race");
        }
    }
    WriterOutcome { landed, races_lost }
}

/// Races `writers` concurrent writers and checks that the resulting history
/// accounts for every one of them, exactly once. Returns how many
/// compare-and-swaps were lost — the evidence that the writers overlapped.
async fn check_concurrency_case(pool: &PgPool, writers: usize, per_writer: usize) -> usize {
    let tenant = create_tenant(pool).await;
    let scope = ScopeId::new();
    let author = IdentityId::new();

    let mut tx = tenant_tx(pool, tenant).await;
    let root = seed_commit(&mut tx, tenant, author, "root")
        .await
        .expect("seed");
    assert_eq!(
        create_ref(&mut tx, tenant, scope, "published", root, author)
            .await
            .expect("create ref"),
        RefUpdate::Updated
    );
    tx.commit().await.expect("commit seed");

    let outcomes = futures_join(
        (0..writers)
            .map(|id| writer(pool.clone(), tenant, scope, author, id, per_writer))
            .collect(),
    )
    .await;
    let races_lost: usize = outcomes.iter().map(|outcome| outcome.races_lost).sum();
    let landed: Vec<CommitHash> = outcomes
        .into_iter()
        .flat_map(|outcome| outcome.landed)
        .collect();

    let expected = writers * per_writer;
    assert_eq!(landed.len(), expected, "every writer landed every commit");
    assert_eq!(
        landed.iter().collect::<HashSet<_>>().len(),
        expected,
        "no two writers landed the same commit"
    );

    let mut tx = tenant_tx(pool, tenant).await;
    let conn = &mut *tx;
    let head = read_ref(conn, tenant, scope, "published")
        .await
        .expect("read ref")
        .expect("ref exists")
        .commit_hash;

    // 1. Every commit that reported success is reachable from the head. This
    //    is the lost-update property: a last-writer-wins ref would strand
    //    whichever writer lost, with no error anywhere.
    for hash in &landed {
        assert!(
            is_ancestor(conn, tenant, *hash, head)
                .await
                .expect("walk ancestry"),
            "a commit that reported success is not in the history: {hash}"
        );
    }
    assert!(
        is_ancestor(conn, tenant, root, head)
            .await
            .expect("walk ancestry from root"),
        "the ref only ever fast-forwarded"
    );

    // 2. The chain from the head is exactly the root plus every landed
    //    commit — no gaps, and nothing extra.
    let mut chain = vec![head];
    let mut cursor = head;
    while let Some(parent) = read_commit(conn, tenant, cursor)
        .await
        .expect("read commit")
        .expect("commit exists")
        .parents
        .first()
        .copied()
    {
        chain.push(parent);
        cursor = parent;
    }
    assert_eq!(
        chain.len(),
        expected + 1,
        "chain length: root + every commit"
    );
    assert_eq!(*chain.last().expect("non-empty chain"), root);

    // 3. Every commit row in the tenant is on that chain. A lost race rolls
    //    back its own commit, so there is no unreachable garbage either.
    let rows = sqlx::query_scalar!(
        r#"select count(*) as "count!" from vedaflow_commits where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("count commits");
    assert_eq!(
        rows,
        chain.len() as i64,
        "no commit exists that the head cannot reach"
    );

    // 4. Nothing was rewritten along the way.
    assert!(matches!(
        verify(conn, tenant).await.expect("verify"),
        StoreVerification::Valid { .. }
    ));
    tx.rollback().await.expect("rollback");
    races_lost
}

/// Awaits every future concurrently without pulling in `futures`: the tasks
/// are spawned, so they run on the runtime's worker threads rather than
/// interleaving on one.
async fn futures_join<F>(futures: Vec<F>) -> Vec<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        out.push(handle.await.expect("writer task"));
    }
    out
}

#[test]
fn history_is_immutable_under_concurrent_writers() {
    let Some(db) = db() else { return };
    let mut runner = TestRunner::new(Config::with_cases(6));
    runner
        .run(&(2usize..5, 1usize..4), |(writers, per_writer)| {
            db.rt
                .block_on(check_concurrency_case(&db.pool, writers, per_writer));
            Ok(())
        })
        .unwrap_or_else(|err| panic!("concurrency property failed: {err}"));
}

/// The headline case at a size worth naming: eight writers, five commits
/// each, one ref.
///
/// This one also asserts the writers actually collided. A concurrency test in
/// which nobody ever loses a compare-and-swap has proved nothing about the
/// compare-and-swap — it has proved that the scheduler happened to queue.
#[test]
fn eight_writers_forty_commits_one_ref() {
    let Some(db) = db() else { return };
    let races_lost = db.rt.block_on(check_concurrency_case(&db.pool, 8, 5));
    eprintln!("8 writers × 5 commits: {races_lost} compare-and-swaps lost and retried");
    assert!(
        races_lost > 0,
        "no writer ever lost a race — this run did not test concurrency"
    );
}

// ── Immutability, at the schema ──────────────────────────────────────────────

/// A writer with the application's own privileges cannot rewrite history at
/// all: the grants withhold UPDATE and DELETE, and the triggers raise even
/// for the table owner (ADR-0030 decision 6).
#[test]
fn recorded_history_cannot_be_rewritten_or_removed() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = create_tenant(pool).await;
        let author = IdentityId::new();

        // A two-commit chain, so every history table — the parent rows
        // included — actually holds something for the attacker to reach for.
        let mut tx = tenant_tx(pool, tenant).await;
        let root = seed_commit(&mut tx, tenant, author, "immutable")
            .await
            .expect("seed");
        let head = make_commit(&mut tx, tenant, author, "immutable-two", vec![root])
            .await
            .expect("seed child");
        let tree = read_commit(&mut tx, tenant, head)
            .await
            .expect("read commit")
            .expect("commit exists")
            .tree;
        tx.commit().await.expect("commit seed");

        // As synveda_app: no UPDATE or DELETE grant exists to exercise.
        let mut tx = tenant_tx(pool, tenant).await;
        sqlx::raw_sql("set local role synveda_app")
            .execute(&mut *tx)
            .await
            .expect("set role");
        let denied = sqlx::query("update vedaflow_commits set message = 'rewritten' where tenant_id = $1")
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await;
        assert!(
            denied.is_err(),
            "synveda_app must hold no UPDATE on vedaflow_commits"
        );
        tx.rollback().await.expect("rollback");

        // Every history table must actually hold a row of this tenant's, or
        // the statements below would "pass" by matching nothing.
        let mut tx = tenant_tx(pool, tenant).await;
        let populated = sqlx::query!(
            r#"select
                 (select count(*) from vedaflow_objects where tenant_id = $1) as "objects!",
                 (select count(*) from vedaflow_trees where tenant_id = $1) as "trees!",
                 (select count(*) from vedaflow_tree_entries where tenant_id = $1) as "entries!",
                 (select count(*) from vedaflow_commits where tenant_id = $1) as "commits!",
                 (select count(*) from vedaflow_commit_parents where tenant_id = $1) as "parents!""#,
            tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count history rows");
        assert!(
            populated.objects > 0
                && populated.trees > 0
                && populated.entries > 0
                && populated.commits > 0
                && populated.parents > 0,
            "every history table must hold something to attack: {populated:?}"
        );
        tx.rollback().await.expect("rollback");

        // As the table owner, which grants cannot stop: the triggers do.
        for statement in [
            "update vedaflow_objects set content = 'tampered' where tenant_id = $1",
            "update vedaflow_trees set created_at = now() where tenant_id = $1",
            "update vedaflow_tree_entries set name = 'renamed' where tenant_id = $1",
            "update vedaflow_commits set message = 'rewritten' where tenant_id = $1",
            "update vedaflow_commit_parents set ordinal = 9 where tenant_id = $1",
            "delete from vedaflow_objects where tenant_id = $1",
            "delete from vedaflow_trees where tenant_id = $1",
            "delete from vedaflow_tree_entries where tenant_id = $1",
            "delete from vedaflow_commits where tenant_id = $1",
            "delete from vedaflow_commit_parents where tenant_id = $1",
        ] {
            let mut tx = pool.begin().await.expect("begin");
            let outcome = sqlx::query(statement)
                .bind(tenant.as_uuid())
                .execute(&mut *tx)
                .await;
            let err = outcome
                .err()
                .unwrap_or_else(|| panic!("expected a refusal from: {statement}"));
            assert!(
                err.to_string().contains("append-only"),
                "expected the append-only trigger, got: {err}"
            );
            tx.rollback().await.expect("rollback");
        }

        // The tree and its entries are still exactly what the commit claims.
        let mut tx = tenant_tx(pool, tenant).await;
        assert_eq!(
            read_tree(&mut tx, tenant, tree)
                .await
                .expect("read tree")
                .expect("tree exists")
                .len(),
            1
        );
        assert!(matches!(
            verify(&mut tx, tenant).await.expect("verify"),
            StoreVerification::Valid { .. }
        ));
        tx.rollback().await.expect("rollback");
    });
}

/// What the triggers cannot stop, verification detects. The attacker here
/// holds database credentials and suppresses triggers with
/// `session_replication_role = replica` — the AUD-1 tamper test's move, and
/// the reason ADR-0030 decision 6 does not claim more than it can.
#[test]
fn a_trigger_suppressing_attacker_is_still_caught_by_verification() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = create_tenant(pool).await;
        let author = IdentityId::new();

        let mut tx = tenant_tx(pool, tenant).await;
        seed_commit(&mut tx, tenant, author, "original")
            .await
            .expect("seed");
        tx.commit().await.expect("commit seed");

        let mut tx = tenant_tx(pool, tenant).await;
        sqlx::raw_sql("set local session_replication_role = replica")
            .execute(&mut *tx)
            .await
            .expect("suppress triggers");
        sqlx::query("update vedaflow_objects set content = $2, size_bytes = length($2) where tenant_id = $1")
            .bind(tenant.as_uuid())
            .bind("forged".as_bytes())
            .execute(&mut *tx)
            .await
            .expect("rewrite content with triggers suppressed");
        sqlx::raw_sql("set local session_replication_role = origin")
            .execute(&mut *tx)
            .await
            .expect("restore triggers");

        match verify(&mut tx, tenant).await.expect("verify") {
            StoreVerification::Broken {
                class,
                stored,
                recomputed,
            } => {
                assert_eq!(class, ObjectClass::Object);
                assert_ne!(stored, recomputed, "verification names both addresses");
            }
            other => panic!("a rewritten object must break verification, got {other}"),
        }
        tx.rollback().await.expect("rollback");
    });
}

// ── Refs: fast-forward, force, and the closed DAG ───────────────────────────

#[test]
fn a_ref_moves_only_forward_unless_forced() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = create_tenant(pool).await;
        let (scope, author) = (ScopeId::new(), IdentityId::new());
        let mut tx = tenant_tx(pool, tenant).await;
        let conn = &mut *tx;

        let first = seed_commit(conn, tenant, author, "one")
            .await
            .expect("seed");
        let object = put_object(conn, tenant, AssetKind::Knowledge, b"two")
            .await
            .expect("put object");
        let tree = put_tree(conn, tenant, &[TreeEntry::object("note.md", object.hash)])
            .await
            .expect("put tree");
        let second = commit(
            conn,
            tenant,
            &NewCommit {
                tree: tree.hash,
                parents: vec![first],
                author,
                message: "two".to_string(),
                committed_at: at(),
                policy_snapshot: pack(),
            },
            &Signer::Unsigned,
        )
        .await
        .expect("commit two")
        .hash;
        // A sibling of `first`, not a descendant: an unrelated branch.
        let sibling = seed_commit(conn, tenant, author, "sideways")
            .await
            .expect("seed sibling");

        create_ref(conn, tenant, scope, "published", first, author)
            .await
            .expect("create ref");
        assert_eq!(
            create_ref(conn, tenant, scope, "published", second, author)
                .await
                .expect("re-create ref"),
            RefUpdate::Raced,
            "creating a ref that exists is the same 'someone got here first'"
        );

        assert_eq!(
            update_ref(conn, tenant, scope, "published", first, second, author)
                .await
                .expect("fast-forward"),
            RefUpdate::Updated
        );
        // Stale expectation: the caller is reasoning about a head that moved.
        assert_eq!(
            update_ref(conn, tenant, scope, "published", first, second, author)
                .await
                .expect("stale expectation"),
            RefUpdate::Raced
        );
        // Backwards, and sideways, are both refused.
        assert_eq!(
            update_ref(conn, tenant, scope, "published", second, first, author)
                .await
                .expect("rewind"),
            RefUpdate::NotFastForward
        );
        assert_eq!(
            update_ref(conn, tenant, scope, "published", second, sibling, author)
                .await
                .expect("sideways"),
            RefUpdate::NotFastForward
        );
        assert_eq!(
            read_ref(conn, tenant, scope, "published")
                .await
                .expect("read ref")
                .expect("ref exists")
                .commit_hash,
            second,
            "a refused move writes nothing"
        );

        // FLOW-7's rollback is a different call, by name.
        assert_eq!(
            synveda_vedaflow::force_update_ref(
                conn,
                tenant,
                scope,
                "published",
                second,
                first,
                author
            )
            .await
            .expect("force"),
            RefUpdate::Updated
        );
        assert_eq!(
            read_ref(conn, tenant, scope, "published")
                .await
                .expect("read ref")
                .expect("ref exists")
                .commit_hash,
            first
        );
        tx.rollback().await.expect("rollback");
    });
}

#[test]
fn a_commit_cannot_claim_a_parent_or_tree_that_does_not_exist() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = create_tenant(pool).await;
        let author = IdentityId::new();

        // A tree that was never written.
        let mut tx = tenant_tx(pool, tenant).await;
        let orphan = commit(
            &mut tx,
            tenant,
            &NewCommit {
                tree: synveda_vedaflow::TreeHash::from_bytes([0xab; 32]),
                parents: vec![],
                author,
                message: "dangling tree".to_string(),
                committed_at: at(),
                policy_snapshot: pack(),
            },
            &Signer::Unsigned,
        )
        .await;
        assert!(
            matches!(orphan, Err(Error::Storage { .. })),
            "the tree foreign key must refuse it, got {orphan:?}"
        );
        tx.rollback().await.expect("rollback");

        // A parent that was never written.
        let mut tx = tenant_tx(pool, tenant).await;
        let object = put_object(&mut tx, tenant, AssetKind::Knowledge, b"x")
            .await
            .expect("put object");
        let tree = put_tree(&mut tx, tenant, &[TreeEntry::object("x", object.hash)])
            .await
            .expect("put tree");
        let orphan = commit(
            &mut tx,
            tenant,
            &NewCommit {
                tree: tree.hash,
                parents: vec![CommitHash::from_bytes([0xcd; 32])],
                author,
                message: "dangling parent".to_string(),
                committed_at: at(),
                policy_snapshot: pack(),
            },
            &Signer::Unsigned,
        )
        .await;
        assert!(
            matches!(orphan, Err(Error::Storage { .. })),
            "the parent foreign key must refuse it, got {orphan:?}"
        );
        tx.rollback().await.expect("rollback");

        // A tree entry pointing at an object that was never written.
        let mut tx = tenant_tx(pool, tenant).await;
        let dangling = put_tree(
            &mut tx,
            tenant,
            &[TreeEntry::object(
                "ghost",
                synveda_vedaflow::ObjectHash::from_bytes([0xef; 32]),
            )],
        )
        .await;
        assert!(
            matches!(dangling, Err(Error::Storage { .. })),
            "the tree-entry foreign key must refuse it, got {dangling:?}"
        );
        tx.rollback().await.expect("rollback");
    });
}

// ── Commits record what ADR-0003 promised ───────────────────────────────────

#[test]
fn a_commit_records_its_author_its_pack_and_its_signature() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let tenant = create_tenant(pool).await;
        let author = IdentityId::new();
        let signer = Signer::Ed25519(Box::new(
            Ed25519Signer::new([9u8; 32], "flow-1-test-key").expect("build signer"),
        ));
        let Signer::Ed25519(ref key) = signer else {
            unreachable!()
        };

        let mut tx = tenant_tx(pool, tenant).await;
        let conn = &mut *tx;
        let object = put_object(conn, tenant, AssetKind::Prompt, b"be terse")
            .await
            .expect("put object");
        let tree = put_tree(
            conn,
            tenant,
            &[TreeEntry::object("house-style.md", object.hash)],
        )
        .await
        .expect("put tree");
        let snapshot = pack();
        let written = commit(
            conn,
            tenant,
            &NewCommit {
                tree: tree.hash,
                parents: vec![],
                author,
                message: "house style".to_string(),
                committed_at: at(),
                policy_snapshot: snapshot.clone(),
            },
            &signer,
        )
        .await
        .expect("commit");

        let stored = read_commit(conn, tenant, written.hash)
            .await
            .expect("read commit")
            .expect("commit exists");
        assert_eq!(stored.author, author);
        assert_eq!(stored.committed_at, at());
        assert_eq!(
            stored.policy_snapshot_hash,
            snapshot.hash().expect("snapshot hash"),
            "an auditor can prove which pack governed this commit"
        );
        let signature = stored.signature.expect("signed");
        assert_eq!(signature.key_id, "flow-1-test-key");
        assert!(
            verify_ed25519(written.hash, &signature.signature, &key.verifying_key()),
            "the signature verifies against the commit address alone — no schema needed"
        );
        // And covers everything, because the address does.
        assert!(!verify_ed25519(
            CommitHash::from_bytes([0u8; 32]),
            &signature.signature,
            &key.verifying_key()
        ));

        // The default signer records no signature rather than an empty one.
        let unsigned = commit(
            conn,
            tenant,
            &NewCommit {
                tree: tree.hash,
                parents: vec![],
                author,
                message: "unsigned".to_string(),
                committed_at: at(),
                policy_snapshot: snapshot,
            },
            &Signer::Unsigned,
        )
        .await
        .expect("commit unsigned");
        assert!(
            read_commit(conn, tenant, unsigned.hash)
                .await
                .expect("read commit")
                .expect("commit exists")
                .signature
                .is_none()
        );
        tx.rollback().await.expect("rollback");
    });
}

// ── FLOW-7: the first-parent line (ADR-0036 decision 1) ──────────────────────

/// The distinction a rewind rests on, at the substrate: **reachable is not
/// the same as "was a state".**
///
/// A merge commit's second parent is reachable from the head — FLOW-1's
/// `is_ancestor` says so, and that is correct for the fast-forward test it
/// was written for. Walking ordinal 0 answers the other question, which is
/// the one a rollback has to ask: which commits has this ref actually
/// pointed at?
#[test]
fn a_side_parent_is_reachable_and_is_not_on_the_first_parent_line() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = create_tenant(&db.pool).await;
        let author = IdentityId::new();
        let mut tx = tenant_tx(&db.pool, tenant).await;
        let conn = &mut *tx;

        // A mainline of two, and a side commit merged into a third — the
        // shape every reviewed publication has had since FLOW-3.
        let first = seed_commit(conn, tenant, author, "first")
            .await
            .expect("first");
        let second = make_commit(conn, tenant, author, "second", vec![first])
            .await
            .expect("second");
        let side = seed_commit(conn, tenant, author, "side")
            .await
            .expect("side");
        let merge = make_commit(conn, tenant, author, "merge", vec![second, side])
            .await
            .expect("merge");

        for ancestor in [first, second, merge] {
            assert!(
                synveda_vedaflow::is_first_parent_ancestor(conn, tenant, ancestor, merge)
                    .await
                    .expect("walk"),
                "{ancestor} is a state the mainline passed through"
            );
        }

        assert!(
            synveda_vedaflow::is_ancestor(conn, tenant, side, merge)
                .await
                .expect("walk"),
            "the side commit is reachable — this is what makes the other check load-bearing"
        );
        assert!(
            !synveda_vedaflow::is_first_parent_ancestor(conn, tenant, side, merge)
                .await
                .expect("walk"),
            "…and it was never a state the mainline held"
        );

        // Descendants are not on their ancestors' line either, which is
        // what makes "a rewind never advances" a property of the walk
        // rather than a comparison the caller has to remember.
        assert!(
            !synveda_vedaflow::is_first_parent_ancestor(conn, tenant, merge, first)
                .await
                .expect("walk"),
            "the line runs one way"
        );
        tx.rollback().await.expect("rollback");
    });
}
