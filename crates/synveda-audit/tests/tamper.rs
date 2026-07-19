//! AUD-1 acceptance criteria: mutating any historic row breaks chain
//! verification (ADR-0019). The attacker modelled here holds the database
//! credentials — the strongest position tamper-*evidence* defends against —
//! and mutates history with triggers suppressed
//! (`session_replication_role = replica`), exactly what the append-only
//! guard cannot stop. Verification must name the broken sequence.
//!
//! These tests need a live, migrated Postgres. They read `DATABASE_URL`
//! and skip when it is unset (CI has no database) or when the audit tables
//! are missing (run `synveda db migrate` / any store test first); run them
//! locally with `make db-test`.
//!
//! Layering note: `synveda-audit` sits beside `synveda-store`, so this
//! suite carries its own three-line tenant-GUC helper instead of importing
//! `rls::begin_tenant_tx` — same GUC, same transaction-local shape.

use std::sync::OnceLock;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_audit::{
    Actor, AuditAction, AuditEvent, BreakReason, ChainVerification, Outcome, append, tail, verify,
};
use synveda_types::TenantId;

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

/// Connects (once) to `DATABASE_URL` and checks the audit tables exist.
/// `None` = no database (or an unmigrated one); every test skips quietly.
fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping tamper tests: DATABASE_URL is not set \
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
            let migrated = sqlx::query_scalar::<_, Option<String>>(
                "select to_regclass('public.audit_log')::text",
            )
            .fetch_one(&pool)
            .await
            .expect("probe for audit_log");
            if migrated.is_none() {
                eprintln!(
                    "skipping tamper tests: audit tables missing — apply \
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

fn event(i: i64) -> AuditEvent {
    AuditEvent {
        occurred_at: chrono::Utc::now(),
        actor: Actor::subject(format!("actor-{i}")),
        action: AuditAction::AuthzDecision,
        resource: format!("scope fixture-{i}"),
        outcome: Outcome::Allow,
        payload: serde_json::json!({"n": i, "note": "tamper fixture"}),
        // Exercise both canonical forms: present and absent trace ids.
        trace_id: (i % 2 == 0).then(|| format!("trace-{i}")),
    }
}

/// A fresh tenant with `n` chained events, appended through the real seam —
/// one transaction each, like distinct requests.
async fn chain_of(pool: &PgPool, n: i64) -> TenantId {
    let tenant = TenantId::new();
    for i in 1..=n {
        let mut tx = tenant_tx(pool, tenant).await;
        append(&mut tx, tenant, &event(i)).await.expect("append");
        tx.commit().await.expect("commit append");
    }
    tenant
}

/// Runs one tampering statement (binding the tenant uuid as `$1`) with
/// ordinary triggers suppressed for this transaction only
/// (`session_replication_role = replica` — the database-credentialed
/// attacker's move; no table lock, so parallel test binaries are unharmed).
async fn tamper(pool: &PgPool, tenant: TenantId, sql: &str) {
    let mut tx = pool.begin().await.expect("begin tamper transaction");
    sqlx::raw_sql("set local session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("suppress triggers");
    sqlx::query(sql)
        .bind(tenant.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("tampering statement");
    tx.commit().await.expect("commit tampering");
}

async fn verified(pool: &PgPool, tenant: TenantId) -> ChainVerification {
    let mut tx = tenant_tx(pool, tenant).await;
    verify(&mut tx, tenant).await.expect("verify chain")
}

// ── Baseline ─────────────────────────────────────────────────────────────────

#[test]
fn untampered_chains_verify() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = chain_of(&db.pool, 5).await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Valid { events: 5 }
        );
        // A tenant with no chain at all is trivially valid.
        assert_eq!(
            verified(&db.pool, TenantId::new()).await,
            ChainVerification::Valid { events: 0 }
        );
    });
}

#[test]
fn tail_returns_newest_first() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = chain_of(&db.pool, 3).await;
        let mut tx = tenant_tx(&db.pool, tenant).await;
        let events = tail(&mut tx, tenant, 2).await.expect("tail");
        let seqs: Vec<i64> = events.iter().map(|event| event.seq).collect();
        assert_eq!(seqs, [3, 2]);
    });
}

// ── The AC: mutating any historic row breaks verification ────────────────────

#[test]
fn mutating_any_historic_column_breaks_verification() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // Every hashed column, mutated one at a time on the same historic
        // row of a fresh chain. Values satisfy the schema's constraints:
        // the *database* accepts each rewrite — only verification objects.
        let mutations: &[(&str, &str)] = &[
            (
                "payload",
                r#"update audit_log set payload = '{"n": 999}' where tenant_id = $1 and seq = 2"#,
            ),
            (
                "actor_subject",
                "update audit_log set actor_subject = 'mallory' where tenant_id = $1 and seq = 2",
            ),
            (
                "actor_kind",
                "update audit_log set actor_kind = 'break_glass' where tenant_id = $1 and seq = 2",
            ),
            (
                "action",
                "update audit_log set action = 'role.bound' where tenant_id = $1 and seq = 2",
            ),
            (
                "resource",
                "update audit_log set resource = 'scope forged' where tenant_id = $1 and seq = 2",
            ),
            (
                "outcome",
                "update audit_log set outcome = 'deny' where tenant_id = $1 and seq = 2",
            ),
            (
                "occurred_at",
                "update audit_log set occurred_at = occurred_at + interval '1 second' \
                 where tenant_id = $1 and seq = 2",
            ),
            (
                "trace_id",
                "update audit_log set trace_id = 'planted' where tenant_id = $1 and seq = 2",
            ),
        ];
        for (column, sql) in mutations {
            let tenant = chain_of(&db.pool, 4).await;
            tamper(&db.pool, tenant, sql).await;
            assert_eq!(
                verified(&db.pool, tenant).await,
                ChainVerification::Broken {
                    seq: 2,
                    reason: BreakReason::Content,
                },
                "mutating {column} must break verification at the mutated row"
            );
        }
    });
}

#[test]
fn removing_a_row_leaves_a_gap() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = chain_of(&db.pool, 5).await;
        tamper(
            &db.pool,
            tenant,
            "delete from audit_log where tenant_id = $1 and seq = 2",
        )
        .await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Broken {
                seq: 3,
                reason: BreakReason::Gap { expected: 2 },
            }
        );
    });
}

#[test]
fn relinking_a_row_breaks_linkage() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = chain_of(&db.pool, 4).await;
        tamper(
            &db.pool,
            tenant,
            "update audit_log set prev_hash = decode(repeat('00', 32), 'hex') \
             where tenant_id = $1 and seq = 3",
        )
        .await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Broken {
                seq: 3,
                reason: BreakReason::Linkage,
            }
        );
    });
}

#[test]
fn moving_or_faking_the_head_is_detected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // A rewritten head hash.
        let tenant = chain_of(&db.pool, 3).await;
        tamper(
            &db.pool,
            tenant,
            "update audit_chain_heads set head_hash = decode(repeat('ff', 32), 'hex') \
             where tenant_id = $1",
        )
        .await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Broken {
                seq: 3,
                reason: BreakReason::Head,
            }
        );

        // A truncated tail: the last event removed, head left behind.
        let tenant = chain_of(&db.pool, 3).await;
        tamper(
            &db.pool,
            tenant,
            "delete from audit_log where tenant_id = $1 and seq = 3",
        )
        .await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Broken {
                seq: 3,
                reason: BreakReason::Head,
            }
        );

        // The head row removed entirely.
        let tenant = chain_of(&db.pool, 2).await;
        tamper(
            &db.pool,
            tenant,
            "delete from audit_chain_heads where tenant_id = $1",
        )
        .await;
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Broken {
                seq: 2,
                reason: BreakReason::MissingHead,
            }
        );
    });
}

// ── The append-only guard (what tampering had to suppress) ───────────────────

#[test]
fn triggers_reject_mutation_even_for_the_highest_privilege() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = chain_of(&db.pool, 2).await;
        // The test connection is the compose superuser: grants and RLS do
        // not bind it — only the triggers do. Every mutation path raises.
        for sql in [
            "update audit_log set resource = 'rewritten' where tenant_id = $1",
            "delete from audit_log where tenant_id = $1",
        ] {
            let error = sqlx::query(sql)
                .bind(tenant.as_uuid())
                .execute(&db.pool)
                .await
                .expect_err("the append-only trigger must raise");
            assert!(
                error.to_string().contains("append-only"),
                "unexpected error for {sql:?}: {error}"
            );
        }
        let error = sqlx::raw_sql("truncate audit_log")
            .execute(&db.pool)
            .await
            .expect_err("the truncate trigger must raise");
        assert!(
            error.to_string().contains("append-only"),
            "unexpected truncate error: {error}"
        );
        // The chain is untouched by the failed attempts.
        assert_eq!(
            verified(&db.pool, tenant).await,
            ChainVerification::Valid { events: 2 }
        );
    });
}
