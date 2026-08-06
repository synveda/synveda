//! AUTH-5 (ADR-0060): the pull sync's store, exercised where it decides
//! things — the absence it accumulates, the completeness it records, and the
//! one-shot authorisation that releases a breaker trip.
//!
//! Migration 0037's invariants are the RLS suite's; these are the behaviours
//! built on top of them, and the sharpest of them is a negative:
//! `an_absence_pass_never_touches_the_mirrors_own_clock`. Absence-marking
//! runs on a loop against every unseen row, and `scim_users.updated_at` is
//! served to provisioning clients as `meta.lastModified`, so a statement that
//! bumped it would tell every SCIM client that every user changed, every
//! pass, because our connector had a bad afternoon.
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`. Isolation is by freshly minted UUIDv7
//! tenants, so a shared dev database is fine.

use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::directory::UserAttributes;
use synveda_store::{directory, directory_sync, rls, tenants};
use synveda_types::{DirectoryUserId, TenantId, TenantStatus};

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
                    "skipping directory sync tests: DATABASE_URL is not set \
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

/// A fresh tenant with `count` mirror users, named `person-0..n`.
async fn seed(pool: &PgPool, count: usize) -> (TenantId, Vec<DirectoryUserId>) {
    let tenant = TenantId::new();
    let slug = format!("sync-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "sync fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let mut users = Vec::with_capacity(count);
    for index in 0..count {
        let user = directory::create_user(
            &mut *tx,
            DirectoryUserId::new(),
            tenant,
            &UserAttributes {
                external_id: Some(format!("ext-{index}")),
                user_name: format!("person-{index}@example.test"),
                active: true,
                display_name: None,
                given_name: None,
                family_name: None,
                work_email: None,
            },
        )
        .await
        .expect("create mirror user");
        users.push(user.id);
    }
    tx.commit().await.expect("commit fixture");
    (tenant, users)
}

// ── Absence ──────────────────────────────────────────────────────────────────

/// The loop's core arithmetic: a complete pass advances everybody it did not
/// list and clears everybody it did.
///
/// The threshold read is the half that matters — `absent_at_least` is what a
/// leaver signal would be built from, and at `N = 2` somebody missing once is
/// not in it. That gap is ADR-0060 decision 3.2: one pass is a hypothesis,
/// two consecutive complete ones are a finding.
#[test]
fn absence_accumulates_only_for_the_unseen_and_resets_on_return() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, users) = seed(&db.pool, 4).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");

        // Pass 1 lists the first two. The other two go missing.
        let seen = &users[..2];
        let moved = directory_sync::mark_absent(&mut *tx, tenant, seen)
            .await
            .expect("mark absent");
        assert_eq!(moved, 2, "only the unseen move");
        directory_sync::mark_present(&mut *tx, tenant, seen)
            .await
            .expect("mark present");

        // One pass is a hypothesis, so nothing has reached the threshold.
        let at_two = directory_sync::absent_at_least(&mut *tx, tenant, 2)
            .await
            .expect("absent at least 2");
        assert!(at_two.is_empty(), "one missed pass is not a finding");
        let at_one = directory_sync::absent_at_least(&mut *tx, tenant, 1)
            .await
            .expect("absent at least 1");
        assert_eq!(at_one.len(), 2);

        // Pass 2 lists the same two. The other two reach the threshold.
        directory_sync::mark_absent(&mut *tx, tenant, seen)
            .await
            .expect("mark absent again");
        let at_two = directory_sync::absent_at_least(&mut *tx, tenant, 2)
            .await
            .expect("absent at least 2");
        assert_eq!(at_two.len(), 2, "two consecutive complete passes is one");
        let mut found: Vec<DirectoryUserId> = at_two.iter().map(|user| user.id).collect();
        found.sort_by_key(DirectoryUserId::as_uuid);
        let mut expected = users[2..].to_vec();
        expected.sort_by_key(DirectoryUserId::as_uuid);
        assert_eq!(found, expected, "and it is the two nobody listed");

        // Pass 3 lists everybody: the hypothesis is withdrawn, not decayed.
        let returned = directory_sync::mark_present(&mut *tx, tenant, &users)
            .await
            .expect("mark all present");
        assert_eq!(returned, 2, "two were missing and are not any more");
        let at_one = directory_sync::absent_at_least(&mut *tx, tenant, 1)
            .await
            .expect("absent at least 1");
        assert!(at_one.is_empty(), "a returning person is fully present");

        tx.rollback().await.expect("rollback");
    });
}

/// An absence pass leaves the directory's own clock alone.
///
/// `scim_users.updated_at` is served as `meta.lastModified` and `version` is
/// the ETag a provisioning agent uses to decide whether to re-send. Neither
/// is ours to move: the directory did not change these people, we merely
/// failed to see them. A statement that bumped either would announce that
/// every unseen user had been modified, on every pass, for as long as a
/// connector was misconfigured — and a client that trusts `meta` would
/// re-send the world in response.
#[test]
fn an_absence_pass_never_touches_the_mirrors_own_clock() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, users) = seed(&db.pool, 2).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");

        let before = directory::user(&mut *tx, tenant, users[1])
            .await
            .expect("read user")
            .expect("the fixture's user");

        // Two passes miss them, and a third finds them again — every write
        // this module makes to a mirror row, in both directions.
        directory_sync::mark_absent(&mut *tx, tenant, &users[..1])
            .await
            .expect("mark absent");
        directory_sync::mark_absent(&mut *tx, tenant, &users[..1])
            .await
            .expect("mark absent again");
        directory_sync::mark_present(&mut *tx, tenant, &users)
            .await
            .expect("mark present");

        let after = directory::user(&mut *tx, tenant, users[1])
            .await
            .expect("read user")
            .expect("the fixture's user");

        assert_eq!(
            after.version, before.version,
            "the ETag is the directory's; going missing is not an edit"
        );
        assert_eq!(
            after.updated_at, before.updated_at,
            "meta.lastModified is the directory's; our failure to see \
             somebody must not read as their record changing"
        );
        // And for completeness, that the row really was reached at all —
        // otherwise the two assertions above would pass on a no-op.
        assert_eq!(after.user_name, before.user_name);

        tx.rollback().await.expect("rollback");
    });
}

/// A connector change forgets every hypothesis beneath it.
///
/// A directory we have never enumerated has not failed to list anybody, so
/// carrying counts across would let one connector's blind spot seal people
/// under another connector's name.
#[test]
fn a_connector_change_forgets_what_the_previous_one_never_saw() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, users) = seed(&db.pool, 3).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");

        directory_sync::mark_absent(&mut *tx, tenant, &users[..1])
            .await
            .expect("mark absent");
        assert_eq!(
            directory_sync::absent_at_least(&mut *tx, tenant, 1)
                .await
                .expect("absent")
                .len(),
            2
        );

        let cleared = directory_sync::reset_absences(&mut *tx, tenant)
            .await
            .expect("reset absences");
        assert_eq!(cleared, 2, "both standing hypotheses are withdrawn");
        assert!(
            directory_sync::absent_at_least(&mut *tx, tenant, 1)
                .await
                .expect("absent")
                .is_empty(),
            "a new connector starts owing nobody an explanation"
        );

        tx.rollback().await.expect("rollback");
    });
}

// ── The pass and its breaker ────────────────────────────────────────────────

/// A completed pass records the breaker's verdict, and the next one replaces
/// it rather than accumulating.
///
/// `breaker_tripped_at` means "the most recent complete pass refused", which
/// is a fact about now. Recording the trip and completing the pass are one
/// statement because two would have an order, and the wrong order leaves the
/// row saying a pass finished cleanly when it had declined to seal 300
/// people.
#[test]
fn a_completed_pass_records_the_breakers_verdict_or_clears_it() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed(&db.pool, 1).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");

        directory_sync::begin_pass(&mut *tx, tenant, "entra")
            .await
            .expect("begin pass");
        let state = directory_sync::state(&mut *tx, tenant)
            .await
            .expect("state")
            .expect("a state row");
        assert_eq!(state.passes_completed, 0, "starting is not completing");
        assert!(state.last_pass_at.is_some());
        assert!(
            state.last_complete_pass_at.is_none(),
            "nothing has completed yet, and the proof is this column"
        );

        // A pass that tripped.
        assert!(
            directory_sync::complete_pass(&mut *tx, tenant, Some(300))
                .await
                .expect("complete pass")
        );
        let state = directory_sync::state(&mut *tx, tenant)
            .await
            .expect("state")
            .expect("a state row");
        assert_eq!(state.passes_completed, 1);
        assert!(state.last_complete_pass_at.is_some());
        assert_eq!(state.breaker_would_have_sealed, Some(300));
        assert!(state.breaker_tripped_at.is_some());

        // A later quiet pass replaces the verdict rather than leaving a
        // stale refusal standing.
        directory_sync::complete_pass(&mut *tx, tenant, None)
            .await
            .expect("complete pass");
        let state = directory_sync::state(&mut *tx, tenant)
            .await
            .expect("state")
            .expect("a state row");
        assert_eq!(state.passes_completed, 2);
        assert_eq!(state.breaker_would_have_sealed, None);
        assert!(
            state.breaker_tripped_at.is_none(),
            "the breaker does not latch; the chain is where trips are kept"
        );

        tx.rollback().await.expect("rollback");
    });
}

/// Authorising seals for a tenant that has never been enumerated is refused.
///
/// There is no trip to release and no connector on record, so the honest
/// answer is that there is nothing here to authorise.
#[test]
fn an_unsynced_tenant_has_nothing_to_authorise() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed(&db.pool, 1).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");

        assert!(
            directory_sync::state(&mut *tx, tenant)
                .await
                .expect("state")
                .is_none()
        );
        let granted = directory_sync::authorise_seals(
            &mut *tx,
            tenant,
            300,
            3600.0,
            "alice@example.test",
            "Q3 restructure",
        )
        .await
        .expect("authorise");
        assert!(!granted, "no sync state, nothing to authorise");

        tx.rollback().await.expect("rollback");
    });
}

// ── The release ─────────────────────────────────────────────────────────────

/// An authorisation covers what it sized, and is spent exactly once.
///
/// Three properties in one arc, because they are one rule (ADR-0060 decision
/// 10). The **ceiling is a bound**: a pass proposing 301 against a 300 does
/// not squeak through, which is what refuses "authorise 300, the directory
/// degrades further, seal 5,000". It is **one-shot**: the statement that
/// reads it clears it, so a second pass in the same window finds nothing and
/// trips again rather than inheriting somebody else's permission. And it
/// **survives being read** — `state` reports it before it is spent, which is
/// where the caller gets the reason and the grantor for the chain event.
#[test]
fn an_authorisation_covers_what_it_sized_and_is_spent_once() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed(&db.pool, 1).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");
        directory_sync::begin_pass(&mut *tx, tenant, "okta")
            .await
            .expect("begin pass");

        assert!(
            directory_sync::authorise_seals(
                &mut *tx,
                tenant,
                300,
                3600.0,
                "alice@example.test",
                "Q3 restructure, ticket OPS-1123",
            )
            .await
            .expect("authorise")
        );

        // Readable before it is spent: this is what the chain event is
        // built from, and it has to be available before the clear.
        let authorisation = directory_sync::state(&mut *tx, tenant)
            .await
            .expect("state")
            .expect("a state row")
            .authorisation
            .expect("an authorisation in force");
        assert_eq!(authorisation.ceiling, 300);
        assert_eq!(authorisation.granted_by, "alice@example.test");
        assert_eq!(authorisation.reason, "Q3 restructure, ticket OPS-1123");
        assert!(authorisation.expires_at > authorisation.granted_at);

        // One more than it sized is not covered.
        assert!(
            !directory_sync::spend_seal_authorisation(&mut *tx, tenant, 301)
                .await
                .expect("spend"),
            "the ceiling is a bound, not a hint"
        );
        assert!(
            directory_sync::state(&mut *tx, tenant)
                .await
                .expect("state")
                .expect("a state row")
                .authorisation
                .is_some(),
            "a refused spend leaves the authorisation standing"
        );

        // What it sized is covered, once.
        assert!(
            directory_sync::spend_seal_authorisation(&mut *tx, tenant, 300)
                .await
                .expect("spend")
        );
        assert!(
            directory_sync::state(&mut *tx, tenant)
                .await
                .expect("state")
                .expect("a state row")
                .authorisation
                .is_none(),
            "spending clears it whole"
        );
        assert!(
            !directory_sync::spend_seal_authorisation(&mut *tx, tenant, 1)
                .await
                .expect("spend"),
            "one-shot: the next pass inherits no permission"
        );

        tx.rollback().await.expect("rollback");
    });
}

/// An expired authorisation covers nothing, and the clock is the database's.
///
/// The window is checked in SQL against `now()` rather than in Rust against
/// a value the caller passed, so an authorisation ends whether or not
/// anything is running — `lapses::active_for_scopes`' rule, for the same
/// reason.
///
/// The window is backdated with direct SQL rather than by granting a short
/// one and waiting, and the reason is worth keeping: Postgres `now()` is
/// `transaction_timestamp()` and **does not advance inside a transaction**,
/// so a sleep between the grant and the spend moves nothing at all. The
/// first version of this test slept 1.1 seconds over a 1-second window and
/// watched the spend succeed. That is the documented semantics rather than a
/// defect — a pass judges one clock throughout, which is what the lapse
/// store wants too — but it makes elapsed time useless as a test instrument
/// here, and it would have made this assertion pass for the wrong reason if
/// the sleep had been long enough to cross into a second transaction.
#[test]
fn an_expired_authorisation_covers_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed(&db.pool, 1).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("begin tenant tx");
        directory_sync::begin_pass(&mut *tx, tenant, "entra")
            .await
            .expect("begin pass");

        assert!(
            directory_sync::authorise_seals(
                &mut *tx,
                tenant,
                300,
                3600.0,
                "alice@example.test",
                "a window that closes",
            )
            .await
            .expect("authorise")
        );

        // Backdated whole, so the row stays legal: the schema refuses a
        // window that ends before it opens, which is a neighbouring
        // invariant and not the one under test.
        sqlx::query!(
            r#"
            update directory_sync_state
               set seal_authorised_at = now() - interval '2 hours',
                   seal_authorised_until = now() - interval '1 hour'
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("backdate the window");

        assert!(
            !directory_sync::spend_seal_authorisation(&mut *tx, tenant, 10)
                .await
                .expect("spend"),
            "a closed window covers nothing, however small the ask"
        );
        // It is still on the row — expiry is not erasure, and the operator
        // can still see what was granted and when it lapsed.
        assert!(
            directory_sync::state(&mut *tx, tenant)
                .await
                .expect("state")
                .expect("a state row")
                .authorisation
                .is_some(),
            "an expired authorisation is visible, merely ineffective"
        );

        tx.rollback().await.expect("rollback");
    });
}
