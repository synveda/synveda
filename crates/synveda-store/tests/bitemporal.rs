//! FND-4 acceptance criteria: as-of queries return historical row states;
//! property tests over random operation histories.
//!
//! These tests need a live Postgres. They read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test` (dev environment up) or via `demos/fnd-4-bitemporal.sh`.
//! Isolation is by freshly minted UUIDv7 record ids, so a shared dev database
//! is fine.

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::records::{self, RecordState};
use synveda_types::{
    Error, IdentityId, RecordClass, RecordId, RecordKind, ScopeId, Sensitivity, TenantId,
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
                    "skipping bitemporal tests: DATABASE_URL is not set \
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

/// Guarantees the next statement runs at a strictly later `now()` — Postgres
/// transaction timestamps have microsecond resolution, so 5ms is ample.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

/// Fixed valid-time epoch, safely before any test's transaction time.
fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn state(content: &str, scope: ScopeId, owner: IdentityId) -> RecordState {
    RecordState {
        scope_id: scope,
        owner_id: owner,
        kind: RecordKind::Derived,
        class: RecordClass::Fact,
        content: content.to_owned(),
        sensitivity: Sensitivity::Internal,
        provenance: serde_json::json!({"source": "fnd-4 acceptance test"}),
        valid_from: base(),
        valid_to: None,
    }
}

fn midpoint(a: DateTime<Utc>, b: DateTime<Utc>) -> DateTime<Utc> {
    a + (b - a) / 2
}

// ── The headline acceptance test ─────────────────────────────────────────────

/// Insert → update → update → delete → re-insert, then prove the as-of query
/// reproduces every historical row state, the deletion gap, and the current
/// state, purely from transaction time.
#[test]
fn as_of_returns_historical_row_states() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());

        records::insert(pool, id, tenant, &state("postgres 16", scope, owner))
            .await
            .expect("insert v1");
        tick().await;
        records::update(pool, id, &state("postgres 17", scope, owner))
            .await
            .expect("update to v2")
            .expect("v1 was current");
        tick().await;
        records::update(pool, id, &state("postgres 18", scope, owner))
            .await
            .expect("update to v3")
            .expect("v2 was current");
        tick().await;
        assert!(records::delete(pool, id).await.expect("delete"));
        tick().await;
        records::insert(
            pool,
            id,
            tenant,
            &state("postgres 18 (restored)", scope, owner),
        )
        .await
        .expect("re-insert after temporal delete");

        let versions = records::versions(pool, id).await.expect("versions");
        let contents: Vec<&str> = versions.iter().map(|v| v.state.content.as_str()).collect();
        assert_eq!(
            contents,
            [
                "postgres 16",
                "postgres 17",
                "postgres 18",
                "postgres 18 (restored)"
            ]
        );

        // Transaction periods: updates tile exactly; the delete leaves a gap.
        let [v1, v2, v3, v4] = versions.as_slice() else {
            panic!("expected 4 versions, got {}", versions.len());
        };
        assert_eq!(
            v1.tx_to,
            Some(v2.tx_from),
            "update closes v1 exactly at v2's start"
        );
        assert_eq!(
            v2.tx_to,
            Some(v3.tx_from),
            "update closes v2 exactly at v3's start"
        );
        let deleted_at = v3.tx_to.expect("v3 was closed by the delete");
        assert!(
            deleted_at < v4.tx_from,
            "delete then re-insert leaves a gap"
        );
        assert_eq!(v4.tx_to, None, "v4 is current");

        // As-of at each version's start and mid-period returns that version.
        for v in [v1, v2, v3] {
            let at_start = records::as_of(pool, id, v.tx_from).await.unwrap();
            assert_eq!(at_start.as_ref(), Some(v), "as-of at tx_from");
            let mid = midpoint(v.tx_from, v.tx_to.unwrap());
            let at_mid = records::as_of(pool, id, mid).await.unwrap();
            assert_eq!(at_mid.as_ref(), Some(v), "as-of mid-period");
        }

        // Before the record existed, and inside the deletion gap: nothing.
        assert_eq!(records::as_of(pool, id, base()).await.unwrap(), None);
        let in_gap = midpoint(deleted_at, v4.tx_from);
        assert_eq!(records::as_of(pool, id, in_gap).await.unwrap(), None);

        // The present agrees with `current`.
        let now = records::current(pool, id).await.unwrap();
        assert_eq!(now.as_ref(), Some(v4));
    });
}

// ── Focused behaviours ───────────────────────────────────────────────────────

/// Two writes in one transaction share one `now()`: the intermediate version's
/// transaction period is empty, so it never existed in transaction time.
#[test]
fn same_transaction_update_folds_into_one_version() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());

        let mut tx = db.pool.begin().await.expect("begin");
        records::insert(&mut *tx, id, tenant, &state("draft", scope, owner))
            .await
            .expect("insert");
        records::update(&mut *tx, id, &state("final", scope, owner))
            .await
            .expect("update")
            .expect("current");
        tx.commit().await.expect("commit");

        let versions = records::versions(&db.pool, id).await.unwrap();
        assert_eq!(
            versions.len(),
            1,
            "the draft never existed in transaction time"
        );
        assert_eq!(versions[0].state.content, "final");
        assert_eq!(versions[0].tx_to, None);
    });
}

/// Transaction time is server truth: values supplied by SQL are overwritten
/// by the triggers on both insert and update.
#[test]
fn transaction_time_cannot_be_forged() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());
        let forged = base(); // 2026-01-01, well before any real now()

        // Forged insert: tx_from/tx_to supplied explicitly.
        sqlx::query!(
            r#"
            insert into records
                (id, tenant_id, scope_id, owner_id, kind, class, content,
                 sensitivity, provenance, valid_from, valid_to, tx_from, tx_to)
            values ($1, $2, $3, $4, 'derived', 'fact', 'forged insert',
                    'internal', '{}'::jsonb, $5, null, $5, $5)
            "#,
            id.as_uuid(),
            tenant.as_uuid(),
            scope.as_uuid(),
            owner.as_uuid(),
            forged,
        )
        .execute(pool)
        .await
        .expect("insert succeeds — but the trigger stamps real time");

        let v1 = records::current(pool, id).await.unwrap().expect("current");
        assert!(v1.tx_from > forged, "trigger overwrote the forged tx_from");
        assert_eq!(v1.tx_to, None);

        // Forged update: try to rewrite the version's transaction period.
        tick().await;
        sqlx::query!(
            "update records set tx_from = $2, tx_to = null where id = $1",
            id.as_uuid(),
            forged,
        )
        .execute(pool)
        .await
        .expect("update succeeds — but the trigger stamps real time");

        let v2 = records::current(pool, id).await.unwrap().expect("current");
        assert!(
            v2.tx_from > v1.tx_from,
            "trigger stamped a fresh, later tx_from"
        );
        let versions = records::versions(pool, id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(
            versions[0].tx_to,
            Some(v2.tx_from),
            "the real v1 was archived with a true closing time"
        );
    });
}

/// The history table refuses UPDATE, DELETE, and TRUNCATE outright.
#[test]
fn history_is_append_only() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());

        records::insert(pool, id, tenant, &state("v1", scope, owner))
            .await
            .unwrap();
        tick().await;
        records::update(pool, id, &state("v2", scope, owner))
            .await
            .unwrap()
            .unwrap();

        let err = sqlx::query!(
            "update records_history set content = 'tampered' where id = $1",
            id.as_uuid(),
        )
        .execute(pool)
        .await
        .expect_err("history update must be rejected");
        assert!(err.to_string().contains("append-only"), "got: {err}");

        let err = sqlx::query!("delete from records_history where id = $1", id.as_uuid())
            .execute(pool)
            .await
            .expect_err("history delete must be rejected");
        assert!(err.to_string().contains("append-only"), "got: {err}");

        let err = sqlx::query!("truncate table records_history")
            .execute(pool)
            .await
            .expect_err("history truncate must be rejected");
        assert!(err.to_string().contains("append-only"), "got: {err}");
    });
}

/// A record id with a current version cannot be inserted again.
#[test]
fn insert_of_existing_id_is_a_conflict() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());
        records::insert(&db.pool, id, tenant, &state("v1", scope, owner))
            .await
            .unwrap();
        let err = records::insert(&db.pool, id, tenant, &state("again", scope, owner))
            .await
            .expect_err("duplicate id");
        assert!(matches!(err, Error::Conflict { .. }), "got: {err:?}");
    });
}

/// Both time dimensions at once: "as known at T, did the fact hold at V".
#[test]
fn bitemporal_query_combines_transaction_and_valid_time() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let pool = &db.pool;
        let (id, tenant) = (RecordId::new(), TenantId::new());
        let (scope, owner) = (ScopeId::new(), IdentityId::new());
        let june = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let march = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        let july = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();

        // v1: the office was in building A from January to June.
        let v1_state = RecordState {
            valid_from: base(),
            valid_to: Some(june),
            ..state("office is in building A", scope, owner)
        };
        records::insert(pool, id, tenant, &v1_state).await.unwrap();
        tick().await;
        // v2 (a correction): from June it is building B, open-ended.
        let v2_state = RecordState {
            valid_from: june,
            valid_to: None,
            ..state("office is in building B", scope, owner)
        };
        records::update(pool, id, &v2_state).await.unwrap().unwrap();

        // Re-fetch: v1 as archived (its transaction period is now closed).
        let versions = records::versions(pool, id).await.unwrap();
        let [v1, v2] = versions.as_slice() else {
            panic!("expected 2 versions, got {}", versions.len());
        };

        let probe = |tx_at, valid_at| records::as_of_bitemporal(pool, id, tx_at, valid_at);

        // As known when v1 was truth: March holds, July is unknown.
        assert_eq!(probe(v1.tx_from, march).await.unwrap().as_ref(), Some(v1));
        assert_eq!(probe(v1.tx_from, july).await.unwrap(), None);
        // As known now (v2 is truth): July holds, March does not (v2's window
        // starts in June — the v1 fact is no longer current knowledge).
        assert_eq!(probe(v2.tx_from, july).await.unwrap().as_ref(), Some(v2));
        assert_eq!(probe(v2.tx_from, march).await.unwrap(), None);
        // Half-open boundaries: at v1, June itself is already outside; at v2
        // it is the first covered instant.
        assert_eq!(probe(v1.tx_from, june).await.unwrap(), None);
        assert_eq!(probe(v2.tx_from, june).await.unwrap().as_ref(), Some(v2));
    });
}

// ── Property test: random operation histories ────────────────────────────────

/// The mutable fields a generated operation writes.
#[derive(Debug, Clone)]
struct StateSpec {
    content: String,
    valid_from_hours: u32,
    valid_len_hours: Option<u32>,
}

#[derive(Debug, Clone)]
enum OpSpec {
    Update(StateSpec),
    Delete,
    Reinsert(StateSpec),
}

/// What the model expects one stored version to look like.
struct Expected {
    spec: StateSpec,
    /// True when this version was created by a re-insert after a temporal
    /// delete — its transaction period must NOT touch its predecessor's.
    gap_before: bool,
}

fn spec_strategy() -> impl Strategy<Value = StateSpec> {
    ("[a-z]{1,10}", 0u32..48, proptest::option::of(1u32..48)).prop_map(
        |(content, valid_from_hours, valid_len_hours)| StateSpec {
            content,
            valid_from_hours,
            valid_len_hours,
        },
    )
}

fn ops_strategy() -> impl Strategy<Value = (StateSpec, Vec<OpSpec>)> {
    let op = prop_oneof![
        3 => spec_strategy().prop_map(OpSpec::Update),
        1 => Just(OpSpec::Delete),
        1 => spec_strategy().prop_map(OpSpec::Reinsert),
    ];
    (spec_strategy(), proptest::collection::vec(op, 0..8))
}

/// Rotates through the whole vocabulary so every kind/class/sensitivity value
/// crosses the wire and the CHECK constraints.
fn spec_to_state(spec: &StateSpec, seq: usize, scope: ScopeId, owner: IdentityId) -> RecordState {
    let valid_from = base() + chrono::Duration::hours(i64::from(spec.valid_from_hours));
    RecordState {
        scope_id: scope,
        owner_id: owner,
        kind: RecordKind::ALL[seq % RecordKind::ALL.len()],
        class: RecordClass::ALL[seq % RecordClass::ALL.len()],
        content: spec.content.clone(),
        sensitivity: Sensitivity::ALL[seq % Sensitivity::ALL.len()],
        provenance: serde_json::json!({"source": "fnd-4 property test", "seq": seq}),
        valid_from,
        valid_to: spec
            .valid_len_hours
            .map(|len| valid_from + chrono::Duration::hours(i64::from(len))),
    }
}

/// Applies a random operation history to a fresh record id, tracking the
/// expected surviving versions, then checks every bitemporal invariant the
/// schema promises.
async fn check_history_case(pool: &PgPool, initial: StateSpec, ops: Vec<OpSpec>) {
    let (id, tenant) = (RecordId::new(), TenantId::new());
    let (scope, owner) = (ScopeId::new(), IdentityId::new());

    let mut expected: Vec<Expected> = Vec::new();
    let mut alive = true;
    records::insert(pool, id, tenant, &spec_to_state(&initial, 0, scope, owner))
        .await
        .expect("initial insert");
    expected.push(Expected {
        spec: initial,
        gap_before: false,
    });

    for (i, op) in ops.into_iter().enumerate() {
        tick().await;
        let seq = i + 1;
        match op {
            OpSpec::Update(spec) => {
                let updated = records::update(pool, id, &spec_to_state(&spec, seq, scope, owner))
                    .await
                    .expect("update");
                if alive {
                    assert!(
                        updated.is_some(),
                        "update of a live record returns the new version"
                    );
                    expected.push(Expected {
                        spec,
                        gap_before: false,
                    });
                } else {
                    assert_eq!(updated, None, "update of a deleted record is a no-op");
                }
            }
            OpSpec::Delete => {
                let deleted = records::delete(pool, id).await.expect("delete");
                assert_eq!(
                    deleted, alive,
                    "delete reports whether a current version existed"
                );
                alive = false;
            }
            OpSpec::Reinsert(spec) => {
                let result =
                    records::insert(pool, id, tenant, &spec_to_state(&spec, seq, scope, owner))
                        .await;
                if alive {
                    assert!(
                        matches!(result, Err(Error::Conflict { .. })),
                        "inserting over a live record must conflict, got {result:?}"
                    );
                } else {
                    result.expect("re-insert after delete");
                    expected.push(Expected {
                        spec,
                        gap_before: true,
                    });
                    alive = true;
                }
            }
        }
    }

    // ── Invariants ───────────────────────────────────────────────────────────
    let versions = records::versions(pool, id).await.expect("versions");
    assert_eq!(
        versions.len(),
        expected.len(),
        "one stored version per surviving write"
    );

    for (v, e) in versions.iter().zip(&expected) {
        assert_eq!(v.id, id);
        assert_eq!(v.tenant_id, tenant);
        assert_eq!(v.state.content, e.spec.content);
        let valid_from = base() + chrono::Duration::hours(i64::from(e.spec.valid_from_hours));
        assert_eq!(
            v.state.valid_from, valid_from,
            "valid time is stored verbatim"
        );
        assert_eq!(
            v.state.valid_to,
            e.spec
                .valid_len_hours
                .map(|len| valid_from + chrono::Duration::hours(i64::from(len))),
        );
    }

    // Transaction periods are well-formed and tile the timeline: updates abut
    // exactly; delete/re-insert cycles leave a strict gap.
    for i in 1..versions.len() {
        let (v, next) = (&versions[i - 1], &versions[i]);
        let closed_at = v.tx_to.expect("every non-final version is closed");
        assert!(v.tx_from < closed_at, "closed periods have positive length");
        if expected[i].gap_before {
            assert!(closed_at < next.tx_from, "gap after a temporal delete");
            let mid = midpoint(closed_at, next.tx_from);
            assert_eq!(
                records::as_of(pool, id, mid).await.unwrap(),
                None,
                "inside a deletion gap the record does not exist"
            );
        } else {
            assert_eq!(
                closed_at, next.tx_from,
                "an update closes and opens at one instant"
            );
        }
    }
    let last = versions.last().expect("at least the initial version");
    assert_eq!(
        last.tx_to.is_none(),
        alive,
        "only a live record has an open version"
    );

    // As-of reproduces every historical row state at its period start, and
    // the boundary instant of a closure belongs to the successor (half-open).
    for (i, v) in versions.iter().enumerate() {
        let got = records::as_of(pool, id, v.tx_from).await.unwrap();
        assert_eq!(got.as_ref(), Some(v), "as-of(tx_from) returns version {i}");
        if let Some(closed_at) = v.tx_to {
            let successor = versions.get(i + 1).filter(|n| n.tx_from == closed_at);
            let got = records::as_of(pool, id, closed_at).await.unwrap();
            assert_eq!(
                got.as_ref(),
                successor,
                "as-of at closing instant of version {i}"
            );
        }
    }
    assert_eq!(
        records::as_of(pool, id, base()).await.unwrap(),
        None,
        "before first insert"
    );
    assert_eq!(
        records::current(pool, id).await.unwrap().as_ref(),
        if alive { versions.last() } else { None },
        "current agrees with the model"
    );

    // Bitemporal: pinning tx to one version, the valid window answers alone.
    for v in &versions {
        let hit = records::as_of_bitemporal(pool, id, v.tx_from, v.state.valid_from)
            .await
            .unwrap();
        assert_eq!(
            hit.as_ref(),
            Some(v),
            "valid_from is covered (half-open start)"
        );
        let miss = records::as_of_bitemporal(
            pool,
            id,
            v.tx_from,
            v.state.valid_from - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
        assert_eq!(miss, None, "instants before valid_from are not covered");
        if let Some(valid_to) = v.state.valid_to {
            let miss = records::as_of_bitemporal(pool, id, v.tx_from, valid_to)
                .await
                .unwrap();
            assert_eq!(miss, None, "valid_to is excluded (half-open end)");
        }
    }
}

#[test]
fn random_histories_uphold_bitemporal_invariants() {
    let Some(db) = db() else { return };
    let mut runner = TestRunner::new(Config::with_cases(32));
    runner
        .run(&ops_strategy(), |(initial, ops)| {
            db.rt.block_on(check_history_case(&db.pool, initial, ops));
            Ok(())
        })
        .unwrap_or_else(|err| panic!("bitemporal property failed: {err}"));
}
