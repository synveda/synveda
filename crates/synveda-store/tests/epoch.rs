//! CPR-2 store tests: the schema epoch guard and the reset that is the only
//! way past it (ADR-0068 decision 3, ADR-0069).
//!
//! These tests need a live Postgres and, unlike the rest of the suite, they
//! need **databases of their own**: every one of them either drops a table,
//! rewrites the epoch marker, or destroys the whole database, so a shared one
//! would be a suite that breaks the suite. They read private URL files for the
//! server, credentials and port, mint their own database name from them, and
//! drop it on the way out — the same rule `scripts/db-test.sh` follows one
//! layer up. Without `DATABASE_URL` every test skips quietly (CI has no
//! database); run them with `make db-test`.

use std::sync::atomic::Ordering;

use sqlx::{ConnectOptions, Connection, PgPool};
use synveda_store::epoch::{
    self, CURRENT_BASELINE_REVISION, CURRENT_EPOCH, RESET_COMMAND, SchemaEpochError,
};
use synveda_store::runtime_role::DatabaseRoles;

#[path = "support/epoch_fixture.rs"]
mod epoch_fixture;
use epoch_fixture::*;

#[test]
fn missing_extensions_are_refused_before_baseline_ddl() {
    let Some(server) = server() else { return };
    for statement in ["drop extension vector", "drop extension btree_gin"] {
        let scratch = Scratch::empty(&server);
        scratch.block_on(async {
            let admin = connect_pool(&scratch.admin.clone().database(&scratch.name)).await;
            sqlx::query(statement)
                .execute(&admin)
                .await
                .expect("remove one extension from the isolated empty database");
            admin.close().await;
            let pool = connect_pool(&scratch.options).await;

            let error = synveda_store::migrate(&pool, &scratch.roles)
                .await
                .expect_err("missing extensions must refuse before SQLx DDL");
            let message = error.to_string();
            for requirement in [
                "vector 0.8.2, 0.8.3, 0.8.4, 0.8.5 or 0.8.6 and btree_gin 1.3 in public",
                "plpgsql 1.0 in pg_catalog",
                "declared trusted extension owners and no event triggers",
                "docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility",
            ] {
                assert!(message.contains(requirement), "{message}");
            }
            assert!(!has_marker(&pool).await);
            let no_ledger: bool = sqlx::query_scalar(
                "select pg_catalog.to_regclass('public._sqlx_migrations') is null",
            )
            .fetch_one(&pool)
            .await
            .expect("inspect the absent migration ledger");
            assert!(no_ledger, "extension refusal must precede baseline DDL");
            pool.close().await;
        });
    }
}

#[test]
fn extension_executable_drift_is_refused_on_fresh_and_current_databases() {
    let Some(server) = server() else { return };
    for migrated in [false, true] {
        let scratch = Scratch::empty(&server);
        scratch.block_on(async {
            let pool = connect_pool(&scratch.options).await;
            let before = if migrated {
                synveda_store::migrate(&pool, &scratch.roles)
                    .await
                    .expect("migrate the isolated database before drift");
                migration_ledger(&pool).await
            } else {
                Vec::new()
            };
            let admin = connect_pool(&scratch.admin.clone().database(&scratch.name)).await;
            sqlx::query("alter function public.vector_dims(public.vector) volatile")
                .execute(&admin)
                .await
                .expect("change an extension executable property in the isolated database");
            admin.close().await;

            let error = synveda_store::migrate(&pool, &scratch.roles)
                .await
                .expect_err("matching versions must not admit executable drift");
            let message = error.to_string();
            for requirement in [
                "extension catalogue member identities and executable definitions",
                "vector 0.8.2, 0.8.3, 0.8.4, 0.8.5 or 0.8.6, btree_gin 1.3 and plpgsql 1.0",
                "docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility",
            ] {
                assert!(message.contains(requirement), "{message}");
            }
            assert_eq!(has_marker(&pool).await, migrated);
            if migrated {
                assert_eq!(migration_ledger(&pool).await, before);
            } else {
                let no_ledger: bool = sqlx::query_scalar(
                    "select pg_catalog.to_regclass('public._sqlx_migrations') is null",
                )
                .fetch_one(&pool)
                .await
                .expect("inspect the absent migration ledger");
                assert!(no_ledger, "catalogue refusal must precede baseline DDL");
            }
            pool.close().await;
        });
    }
}

#[test]
fn native_vector_distance_and_hnsw_maintenance_remain_consistent() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("admit the genuine extension profile and migrate");
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("repeat migration without changing the extension");
        pool.close().await;

        // This bounded, migrator-owned temporary probe exercises the native
        // extension itself. Product Knowledge/RLS/PDP acceptance separately
        // uses ordinary tenant transactions and the authenticated gateway.
        let mut connection = scratch.options.connect().await.expect("native probe");
        for dimension in [16, 1024] {
            sqlx::query(&format!(
                "create temporary table synveda_vector_probe \
                 (id integer primary key, embedding public.vector({dimension}) not null)"
            ))
            .execute(&mut connection)
            .await
            .expect("create owned native probe");
            sqlx::query(&format!(
                "insert into synveda_vector_probe \
                 select value, array_prepend(1::real, array_prepend(value::real / 100, \
                 array_fill(0::real, array[{}])))::public.vector({dimension}) \
                 from generate_series(1, 32) value",
                dimension - 2
            ))
            .execute(&mut connection)
            .await
            .expect("insert bounded distinct cosine vectors");
            sqlx::query(
                "create index synveda_vector_probe_hnsw on synveda_vector_probe \
                 using hnsw (embedding public.vector_cosine_ops)",
            )
            .execute(&mut connection)
            .await
            .expect("build genuine HNSW index");
            sqlx::query("set enable_seqscan = off")
                .execute(&mut connection)
                .await
                .expect("require the indexed probe plan");
            let query = format!(
                "select id from synveda_vector_probe order by embedding <=> \
                 array_prepend(1::real, array_fill(0::real, array[{}]))::public.vector limit 4",
                dimension - 1
            );
            let plan: serde_json::Value =
                sqlx::query_scalar(&format!("explain (format json) {query}"))
                    .fetch_one(&mut connection)
                    .await
                    .expect("inspect native HNSW plan");
            assert!(
                plan.to_string().contains("synveda_vector_probe_hnsw"),
                "{plan}"
            );
            let initial: Vec<i32> = sqlx::query_scalar(&query)
                .fetch_all(&mut connection)
                .await
                .expect("ordered native cosine candidates");
            assert_eq!(initial, [1, 2, 3, 4]);
            sqlx::query("delete from synveda_vector_probe where id = 1")
                .execute(&mut connection)
                .await
                .expect("delete nearest vector");
            sqlx::query(&format!(
                "update synveda_vector_probe set embedding = \
                 array_prepend(1::real, array_prepend(0.025::real, \
                 array_fill(0::real, array[{}])))::public.vector({dimension}) where id = 2",
                dimension - 2
            ))
            .execute(&mut connection)
            .await
            .expect("exercise an indexed vector update");
            for maintenance in [
                "vacuum synveda_vector_probe",
                "reindex index synveda_vector_probe_hnsw",
            ] {
                sqlx::query(maintenance)
                    .execute(&mut connection)
                    .await
                    .expect("maintain the owned native index");
                let maintained: Vec<i32> = sqlx::query_scalar(&query)
                    .fetch_all(&mut connection)
                    .await
                    .expect("cosine candidates after maintenance");
                assert_eq!(maintained, [2, 3, 4, 5]);
            }
            sqlx::query("drop table synveda_vector_probe")
                .execute(&mut connection)
                .await
                .expect("remove owned native probe");
        }
    });
}

/// The first install: an empty database, migrated, accepted, and carrying a
/// marker that says who made it and when.
#[test]
fn a_fresh_empty_database_bootstraps_to_the_current_epoch() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;

        // Nothing there yet: no marker, and the guard says so rather than
        // erroring on a missing table.
        assert_eq!(epoch::verify(&pool).await, Err(SchemaEpochError::Missing));
        // An empty database is not a pre-cut database, so migrating is
        // allowed — this is the case `preflight` must let through.
        epoch::preflight(&pool).await.expect("an empty database");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");

        let metadata = epoch::verify(&pool).await.expect("a current database");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        assert_eq!(
            metadata.migration_head,
            format!(
                "{:04}",
                synveda_store::MIGRATOR
                    .migrations
                    .last()
                    .expect("an embedded migration")
                    .version
            ),
            "the marker has to name the head actually reached"
        );
        assert_eq!(metadata.created_by_version, env!("CARGO_PKG_VERSION"));
        assert!(
            (chrono::Utc::now() - metadata.created_at).num_minutes() < 5,
            "the epoch was created just now: {}",
            metadata.created_at
        );

        // And the schema is really there — the marker is not a claim about a
        // migration that did not run.
        assert_eq!(tenant_count(&pool).await, 0);
        admit_a_tenant(&pool).await;
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// The advisory lock must cover the epoch stamp, not just SQLx's DDL. Holding
/// the marker row exposes the old gap: both migrators could enter serializable
/// stamp transactions, and the second then failed after the first committed.
#[test]
fn concurrent_install_and_stamp_are_one_migration_boundary() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let (first, second) = tokio::join!(
            synveda_store::migrate(&pool, &scratch.roles),
            synveda_store::migrate(&pool, &scratch.roles),
        );
        first.expect("first fresh migrator");
        second.expect("concurrent fresh migrator");

        let mut blocker = scratch.options.connect().await.expect("stamp blocker");
        let mut transaction = blocker.begin().await.expect("block stamp transaction");
        sqlx::query("select * from schema_metadata for update")
            .fetch_all(&mut *transaction)
            .await
            .expect("hold only this disposable database's marker row");
        let runners = (0..2)
            .map(|_| {
                let pool = pool.clone();
                let roles = scratch.roles.clone();
                tokio::spawn(async move { synveda_store::migrate(&pool, &roles).await })
            })
            .collect::<Vec<_>>();
        // Observe actual database waits rather than guessing when both tasks
        // have reached the marker. One waits on the row, the other on SQLx.
        let mut waits = Vec::<String>::new();
        for _ in 0..80 {
            sqlx::query("select pg_stat_clear_snapshot()")
                .execute(&mut *transaction)
                .await
                .expect("refresh lock observation");
            waits = sqlx::query_scalar(
                "select wait_event from pg_stat_activity where datname=current_database() and pid<>pg_backend_pid() and wait_event_type='Lock' order by wait_event",
            )
            .fetch_all(&mut *transaction)
            .await
            .expect("observe migration waits");
            if waits.len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        transaction.rollback().await.expect("release marker row");
        assert!(waits.contains(&"advisory".to_owned()), "the second migrator must wait outside the stamp transaction: {waits:?}");
        for runner in runners {
            tokio::time::timeout(std::time::Duration::from_secs(10), runner)
                .await
                .expect("bounded migrator completion")
                .expect("migration task joined")
                .expect("stamp completes without serialization failure");
        }
        let rows: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations where success")
            .fetch_one(&pool)
            .await
            .expect("one migration ledger");
        assert_eq!(rows, synveda_store::MIGRATOR.migrations.len() as i64);
        epoch::verify(&pool).await.expect("complete current marker");

        // Cancellation while holding the outer lock must close the dedicated
        // connection, including any nested SQLx session lock on an error path.
        let mut transaction = blocker.begin().await.expect("block cancelled stamp");
        sqlx::query("select * from schema_metadata for update")
            .fetch_all(&mut *transaction)
            .await
            .expect("hold marker for cancellation");
        let cancel_pool = pool.clone();
        let cancel_roles = scratch.roles.clone();
        let cancelled = tokio::spawn(async move {
            synveda_store::migrate(&cancel_pool, &cancel_roles).await
        });
        let mut waiting = false;
        for _ in 0..80 {
            sqlx::query("select pg_stat_clear_snapshot()")
                .execute(&mut *transaction)
                .await
                .expect("refresh cancellation observation");
            waiting = sqlx::query_scalar("select exists(select 1 from pg_stat_activity where datname=current_database() and pid<>pg_backend_pid() and wait_event_type='Lock')")
                .fetch_one(&mut *transaction)
                .await
                .expect("observe cancelled stamp wait");
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        cancelled.abort();
        assert!(cancelled.await.expect_err("task cancelled").is_cancelled());
        transaction.rollback().await.expect("release cancelled stamp");
        assert!(waiting, "cancellation must interrupt an actual locked stamp");
        tokio::time::timeout(std::time::Duration::from_secs(10), synveda_store::migrate(&pool, &scratch.roles))
            .await
            .expect("no advisory lock survives cancellation")
            .expect("migration resumes after cancellation");
        blocker.close().await.expect("close stamp blocker");
        pool.close().await;
    });
}

/// SQLx commits a transactional migration and its success ledger before it
/// can update timing metadata or Synveda can begin the epoch-stamp
/// transaction. A process death at that exact seam must be restart-idempotent.
#[test]
fn an_exact_applied_baseline_without_a_stamp_is_recovered() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("acquire migration connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional baseline");
        drop(connection);

        assert!(has_marker(&pool).await);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from schema_metadata")
                .fetch_one(&pool)
                .await
                .expect("count pending marker rows"),
            0,
            "the fixture is exactly after SQLx and before the stamp"
        );
        epoch::preflight(&pool)
            .await
            .expect("the exact pending stamp is recoverable");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("resume and stamp the committed baseline");
        let metadata = epoch::verify(&pool).await.expect("recovered current epoch");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        assert_eq!(tenant_count(&pool).await, 0);
        pool.close().await;
    });
}

/// An empty marker is not by itself recovery authority. Even one checksum
/// byte of ledger drift keeps the hard cut closed and leaves both catalogues
/// untouched.
#[test]
fn an_empty_marker_with_a_drifted_ledger_is_refused_without_a_stamp() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("acquire migration connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional baseline");
        drop(connection);
        sqlx::query(
            "update _sqlx_migrations \
             set checksum = pg_catalog.decode(pg_catalog.repeat('00', 48), 'hex')",
        )
        .execute(&pool)
        .await
        .expect("drift the exact ledger checksums");
        let ledger_before: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read drifted ledger");

        let error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("a drifted ledger is not pending-stamp authority");
        assert!(error.to_string().contains("immutable migration prefix"));
        assert!(!error.to_string().contains(RESET_COMMAND));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from schema_metadata")
                .fetch_one(&pool)
                .await
                .expect("count marker rows after refusal"),
            0
        );
        let ledger_after: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read unchanged drifted ledger");
        assert_eq!(ledger_after, ledger_before);
        pool.close().await;
    });
}

/// Use SQLx itself to install the exact embedded v0.4.3 prefix, then prove the
/// new binary refuses to serve it, recognizes it read-only as upgradeable and
/// moves it forward without erasing a tenant or rewriting the baseline row.
#[test]
fn released_v043_prefix_upgrades_without_resetting_tenant_data() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let prior = released_v043_migrator();
        let mut connection = pool
            .acquire()
            .await
            .expect("acquire released-baseline connection");
        prior
            .run(&mut *connection)
            .await
            .expect("install the exact published v0.4.3 baseline");
        epoch::stamp_connection(&mut connection, "0.4.3")
            .await
            .expect("stamp the released migration head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let metadata_before = epoch::read(&pool).await.expect("released marker");
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(metadata_before.migration_head, "0001");
        assert_eq!(ledger_before.len(), 1);
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { .. })
        ));

        let checked = synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("released v0.4.3 is upgradeable without writes");
        assert_eq!(checked, metadata_before);
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert_eq!(epoch::read(&pool).await.unwrap(), metadata_before);

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("apply the forward migration and stamp the new head");
        let upgraded = epoch::verify(&pool).await.expect("new image may serve");
        assert_eq!(upgraded.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert_eq!(upgraded.created_by_version, "0.4.3");
        assert_eq!(upgraded.created_at, metadata_before.created_at);
        assert_eq!(
            migration_ledger(&pool).await.len(),
            synveda_store::MIGRATOR.migrations.len()
        );
        assert_eq!(migration_ledger(&pool).await[0], ledger_before[0]);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("lookup preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("new head matches embedded catalogue");
        pool.close().await;
    });
}

enum ForwardDdlInterruption {
    ClientCancellation,
    BackendTermination,
}

/// Stop the forward transaction after its first table alterations, while the
/// later context-candidate DDL waits on a read lock. Both client cancellation
/// and database-backend loss must roll those alterations back with SQLx's
/// success row; a retry must preserve the released prefix.
#[test]
fn interrupted_forward_ddl_preserves_released_prefix_and_retries_cleanly() {
    let Some(server) = server() else { return };
    for interruption in [
        ForwardDdlInterruption::ClientCancellation,
        ForwardDdlInterruption::BackendTermination,
    ] {
        let scratch = Scratch::empty(&server);
        scratch.block_on(async {
            let pool = connect_pool(&scratch.options).await;
            let mut connection = pool.acquire().await.expect("released baseline connection");
            released_v043_migrator()
                .run(&mut *connection)
                .await
                .expect("install exact released baseline");
            epoch::stamp_connection(&mut connection, "0.4.3")
                .await
                .expect("stamp released head");
            drop(connection);

            let tenant = admit_a_tenant(&pool).await;
            let marker_before = epoch::read(&pool).await.expect("released marker");
            let ledger_before = migration_ledger(&pool).await;
            assert_eq!(ledger_before.len(), 1);

            let mut blocker = scratch.options.connect().await.expect("DDL blocker");
            let mut transaction = blocker.begin().await.expect("begin DDL blocker");
            sqlx::query("lock table public.context_candidates in access share mode")
                .execute(&mut *transaction)
                .await
                .expect("block the later forward DDL only");

            let migrate_pool = pool.clone();
            let roles = scratch.roles.clone();
            let migration =
                tokio::spawn(async move { synveda_store::migrate(&migrate_pool, &roles).await });
            let mut waiting_pid: Option<i32> = None;
            for _ in 0..500 {
                sqlx::query("select pg_stat_clear_snapshot()")
                    .execute(&mut *transaction)
                    .await
                    .expect("refresh DDL wait observation");
                waiting_pid = sqlx::query_scalar(
                    "select activity.pid from pg_stat_activity activity \
                 join pg_locks held on held.pid = activity.pid \
                 where activity.datname = current_database() \
                   and activity.pid <> pg_backend_pid() \
                   and held.relation = 'public.context_candidates'::regclass \
                   and held.mode = 'AccessExclusiveLock' \
                   and not held.granted and activity.wait_event_type = 'Lock' \
                 limit 1",
                )
                .fetch_optional(&mut *transaction)
                .await
                .expect("observe actual forward DDL wait");
                if waiting_pid.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            let waiting_pid = waiting_pid.expect("interrupt an actual forward DDL wait");
            match interruption {
                ForwardDdlInterruption::BackendTermination => {
                    let terminated: bool = sqlx::query_scalar("select pg_terminate_backend($1)")
                        .bind(waiting_pid)
                        .fetch_one(&mut *transaction)
                        .await
                        .expect("terminate the waiting migrator backend");
                    assert!(terminated, "the waiting migrator backend must be signalled");
                    tokio::time::timeout(std::time::Duration::from_secs(10), migration)
                        .await
                        .expect("backend loss must finish the migration task")
                        .expect("migration task joined after backend loss")
                        .expect_err("backend loss must fail the migration");
                }
                ForwardDdlInterruption::ClientCancellation => {
                    migration.abort();
                    assert!(
                        migration
                            .await
                            .expect_err("migration task cancelled")
                            .is_cancelled()
                    );
                }
            }
            transaction.rollback().await.expect("release DDL blocker");

            assert_eq!(migration_ledger(&pool).await, ledger_before);
            assert_eq!(
                epoch::read(&pool).await.expect("unchanged marker"),
                marker_before
            );
            let checkpoint_column: bool = sqlx::query_scalar(
                "select exists (select 1 from pg_attribute \
             where attrelid = 'public.session_context_runs'::regclass \
               and attname = 'checkpoint_event_id' and not attisdropped)",
            )
            .fetch_one(&pool)
            .await
            .expect("check earlier DDL was rolled back");
            assert!(
                !checkpoint_column,
                "the earlier DDL must be atomic with the wait"
            );
            assert!(
                synveda_store::tenants::by_id(&pool, tenant)
                    .await
                    .expect("lookup preserved tenant")
                    .is_some()
            );

            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                synveda_store::migrate(&pool, &scratch.roles),
            )
            .await
            .expect("interruption released the migration lock")
            .expect("forward migrations retry cleanly");
            assert_eq!(
                epoch::verify(&pool).await.unwrap().migration_head,
                epoch::CURRENT_MIGRATION_HEAD
            );
            assert_eq!(&migration_ledger(&pool).await[..1], &ledger_before[..]);
            blocker.close().await.expect("close DDL blocker");
            pool.close().await;
        });
    }
}

#[test]
fn exact_forward_prefix_adds_login_state_without_rewriting_existing_rows() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("forward prefix connection");
        forward_migrator()
            .run(&mut *connection)
            .await
            .expect("install exact first forward prefix");
        epoch::stamp_connection(&mut connection, "0.4.3")
            .await
            .expect("stamp first forward head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let before = migration_ledger(&pool).await;
        assert_eq!(before.len(), 2);
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0002");
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { found }) if found == "0002"
        ));
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("exact first forward prefix is upgradeable");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("append the login ledger migration");
        assert_eq!(
            epoch::verify(&pool).await.unwrap().migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        let after = migration_ledger(&pool).await;
        assert_eq!(after.len(), synveda_store::MIGRATOR.migrations.len());
        assert_eq!(&after[..2], &before[..]);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .unwrap()
                .is_some()
        );
        pool.close().await;
    });
}

#[test]
fn exact_login_prefix_adds_delivery_proof_without_rewriting_existing_rows() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("forward prefix connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..3].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install exact login prefix");
        epoch::stamp_connection(&mut connection, "0.4.3")
            .await
            .expect("stamp login head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let before = migration_ledger(&pool).await;
        assert_eq!(before.len(), 3);
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0003");
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { found }) if found == "0003"
        ));
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("exact login prefix is upgradeable");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("append the delivery dependency migration");
        assert_eq!(
            epoch::verify(&pool).await.unwrap().migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        let after = migration_ledger(&pool).await;
        assert_eq!(after.len(), synveda_store::MIGRATOR.migrations.len());
        assert_eq!(&after[..3], &before[..]);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .unwrap()
                .is_some()
        );
        pool.close().await;
    });
}

#[test]
fn exact_delivery_prefix_backfills_source_addresses_without_rewriting_events() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool=connect_pool(&scratch.options).await;
        let mut connection=pool.acquire().await.unwrap();
        sqlx::migrate::Migrator{migrations:std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..4].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT}.run(&mut *connection).await.unwrap();
        epoch::stamp_connection(&mut connection,"0.4.3").await.unwrap();drop(connection);
        let tenant=admit_a_tenant(&pool).await;
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let workspace=synveda_store::workspaces::create(&mut tx,&synveda_store::workspaces::NewWorkspace{
            id:synveda_types::WorkspaceId::new(),tenant_id:tenant,slug:"source-prefix".to_owned(),display_name:"Source prefix".to_owned(),description:None,created_by:None}).await.unwrap();
        let session=synveda_store::sessions::create(&mut tx,&synveda_store::sessions::NewSession{
            id:synveda_types::SessionId::new(),tenant_id:tenant,workspace_id:workspace.id,project_id:None,
            principal_id:"source-prefix-reader".to_owned(),client_name:"claude-code".to_owned(),client_version:None,
            client_installation_id:None,external_session_id:None,agent_name:None,model_name:None,repository_id:None,
            branch:None,task_summary:None,metadata:serde_json::json!({})}).await.unwrap();
        let event=synveda_types::SessionEventId::new();
        let payload=serde_json::json!({"text":"retained prefix evidence"});
        let hash=blake3::hash(payload.to_string().as_bytes()).to_hex().to_string();
        sqlx::query("insert into session_events(id,tenant_id,session_id,event_type,client_event_id,sequence,occurred_at,payload,payload_hash,source_payload_hash) values($1,$2,$3,'message.user','source-prefix',1,clock_timestamp(),$4,$5,$5)")
            .bind(event.as_uuid()).bind(tenant.as_uuid()).bind(session.id.as_uuid()).bind(&payload).bind(&hash).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let before=migration_ledger(&pool).await;assert_eq!(before.len(),4);
        synveda_store::check_migration_compatibility(&pool,&scratch.roles).await.unwrap();
        synveda_store::migrate(&pool,&scratch.roles).await.unwrap();
        let after=migration_ledger(&pool).await;assert_eq!(after.len(),synveda_store::MIGRATOR.migrations.len());assert_eq!(&after[..4],&before[..]);
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let address=synveda_store::source_control::address(&mut tx,tenant,session.id,event).await.unwrap().unwrap();
        assert_eq!(address.payload_hash,hash);assert_eq!(address.scope_id,session.scope_id.as_uuid());assert_eq!(address.sequence,1);
        let retained=synveda_store::sessions::event(&mut *tx,tenant,session.id,event).await.unwrap().unwrap();
        assert_eq!(retained.payload,payload);assert_eq!(retained.payload_hash,hash);
        tx.commit().await.unwrap();pool.close().await;
    });
}

#[test]
fn exact_artifact_prefix_adds_empty_source_observations_and_preserves_existing_rows() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.unwrap();
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..6].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .unwrap();
        epoch::stamp_connection(&mut connection, "0.4.5")
            .await
            .unwrap();
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let before = migration_ledger(&pool).await;
        assert_eq!(before.len(), 6);
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0006");
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .unwrap();
        synveda_store::migrate(&pool, &scratch.roles).await.unwrap();
        let after = migration_ledger(&pool).await;
        assert_eq!(after.len(), synveda_store::MIGRATOR.migrations.len());
        assert_eq!(&after[..6], &before);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .unwrap()
                .is_some()
        );
        let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
            .await
            .unwrap();
        let count: i64 =
            sqlx::query_scalar("select count(*) from source_reconcile_heads where tenant_id=$1")
                .bind(tenant.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(count, 0, "migration cannot invent configured sources");
        tx.commit().await.unwrap();
        pool.close().await;
    });
}

#[test]
fn exact_source_prefix_backfills_complete_artifacts_and_preserves_generic_metadata() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let gateway_url = private_database_url("SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE")
        .expect("ordinary lifecycle gateway credential");
    scratch.block_on(async {
        let pool=connect_pool(&scratch.options).await;
        let mut connection=pool.acquire().await.unwrap();
        sqlx::migrate::Migrator {migrations:std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..5].to_vec()),
          ..sqlx::migrate::Migrator::DEFAULT}.run(&mut *connection).await.unwrap();
        epoch::stamp_connection(&mut connection,"0.4.5").await.unwrap();drop(connection);
        let tenant=admit_a_tenant(&pool).await;
        let ordinary=PgPool::connect(&replace_database(&gateway_url,&scratch.name)).await.unwrap();
        let mut tx=synveda_store::rls::begin_tenant_tx(&ordinary,tenant).await.unwrap();
        let workspace=synveda_store::workspaces::create(&mut tx,&synveda_store::workspaces::NewWorkspace {
          id:synveda_types::WorkspaceId::new(),tenant_id:tenant,slug:"artifact-upgrade".to_owned(),display_name:"Artifact upgrade".to_owned(),description:None,created_by:None}).await.unwrap();
        let project=synveda_store::projects::create(&mut tx,&synveda_store::projects::NewProject {
          id:synveda_types::ProjectId::new(),tenant_id:tenant,workspace_id:workspace.id,slug:"artifact-upgrade".to_owned(),display_name:"Artifact upgrade".to_owned(),description:None,created_by:None}).await.unwrap();
        let job=uuid::Uuid::now_v7();let artifact=uuid::Uuid::now_v7();let batch=uuid::Uuid::now_v7();let candidate=uuid::Uuid::now_v7();
        let hash="a".repeat(64);let spec="ad30107c31c06aec8a7d5636e0d1058118604e6f";
        sqlx::query("insert into import_jobs(id,tenant_id,project_id,scope_id,workspace_id,principal_id,format,format_version,specification_commit,source_kind,source_locator,source_revision,bundle_digest,artifact_count,mapping_count) values($1,$2,$3,$4,$5,'artifact-upgrade','okf','0.2',$6,'git','fixture:artifact-upgrade','commit-a',$7,1,1)")
          .bind(job).bind(tenant.as_uuid()).bind(project.id.as_uuid()).bind(project.scope_id.as_uuid()).bind(workspace.id.as_uuid()).bind(spec).bind(&hash).execute(&mut *tx).await.unwrap();
        sqlx::query("insert into import_artifacts(id,tenant_id,job_id,ordinal,logical_path,artifact_kind,content_hash,frontmatter,body_markdown) values($1,$2,$3,1,'rule.md','concept',$4,'{}','The exact retained fictional artifact.')")
          .bind(artifact).bind(tenant.as_uuid()).bind(job).bind(&hash).execute(&mut *tx).await.unwrap();
        sqlx::query("insert into capture_batches(id,tenant_id,scope_id,workspace_id,project_id,principal_id,input_hash,event_count,source_kind,import_job_id) values($1,$2,$3,$4,$5,'artifact-upgrade',$6,0,'okf_import',$7)")
          .bind(batch).bind(tenant.as_uuid()).bind(project.scope_id.as_uuid()).bind(workspace.id.as_uuid()).bind(project.id.as_uuid()).bind(&hash).bind(job).execute(&mut *tx).await.unwrap();
        sqlx::query("insert into capture_candidates(id,tenant_id,batch_id,ordinal,proposed_scope_id,proposed_project_id,knowledge_type,origin,title,body_markdown,summary,sensitivity,confidence_permille,valid_from,content_hash,source_kind,import_job_id) values($1,$2,$3,1,$4,$5,'procedure','imported','Legacy artifact','The exact retained fictional artifact.','Retained artifact','internal',900,clock_timestamp(),$6,'okf_import',$7)")
          .bind(candidate).bind(tenant.as_uuid()).bind(batch).bind(project.scope_id.as_uuid()).bind(project.id.as_uuid()).bind(&hash).bind(job).execute(&mut *tx).await.unwrap();
        sqlx::query("insert into capture_candidate_import_artifacts(tenant_id,candidate_id,import_job_id,artifact_id,ordinal) values($1,$2,$3,$4,1)")
          .bind(tenant.as_uuid()).bind(candidate).bind(job).bind(artifact).execute(&mut *tx).await.unwrap();
        let proven=uuid::Uuid::now_v7();let ambiguous=uuid::Uuid::now_v7();let generic=uuid::Uuid::now_v7();
        let metadata=serde_json::json!({"import_job_id":job,"artifact_id":artifact,"bundle_digest":hash,"logical_path":"rule.md","artifact_kind":"concept","capture_batch_id":batch,"capture_candidate_id":candidate});
        for (id,kind,locator,value) in [(proven,"okf","fixture:artifact-upgrade#rule.md",metadata.clone()),
          (ambiguous,"okf","fixture:ambiguous",serde_json::json!({"artifact_id":"not-a-uuid"})),
          (generic,"document","external:independent",serde_json::json!({"artifact_id":"external-system-id"}))] {
            sqlx::query("insert into knowledge_sources(id,tenant_id,scope_id,source_type,locator,source_revision,content_hash,metadata) values($1,$2,$3,$4,$5,'commit-a',$6,$7)")
              .bind(id).bind(tenant.as_uuid()).bind(project.scope_id.as_uuid()).bind(kind).bind(locator).bind(&hash).bind(value).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();let before=migration_ledger(&pool).await;assert_eq!(before.len(),5);
        synveda_store::check_migration_compatibility(&pool,&scratch.roles).await.unwrap();
        synveda_store::migrate(&pool,&scratch.roles).await.unwrap();
        let after=migration_ledger(&pool).await;assert_eq!(after.len(),synveda_store::MIGRATOR.migrations.len());assert_eq!(&after[..5],&before[..]);
        let mut tx=synveda_store::rls::begin_tenant_tx(&ordinary,tenant).await.unwrap();
        let row:(Option<uuid::Uuid>,serde_json::Value)=sqlx::query_as("select import_artifact_id,metadata from knowledge_sources where tenant_id=$1 and id=$2")
          .bind(tenant.as_uuid()).bind(proven).fetch_one(&mut *tx).await.unwrap();assert_eq!(row.0,Some(artifact));assert_eq!(row.1,metadata);
        let address=synveda_store::artifact_control::address(&mut tx,tenant,synveda_types::ImportArtifactId::from_uuid(artifact)).await.unwrap().unwrap();assert_eq!(address.content_hash,hash);assert_eq!(address.scope_id,project.scope_id.as_uuid());
        let markers:Vec<uuid::Uuid>=sqlx::query_scalar("select dependent_id from unproven_artifact_dependencies where tenant_id=$1 order by dependent_id")
          .bind(tenant.as_uuid()).fetch_all(&mut *tx).await.unwrap();assert_eq!(markers,vec![ambiguous]);
        let retained:serde_json::Value=sqlx::query_scalar("select metadata from knowledge_sources where tenant_id=$1 and id=$2")
          .bind(tenant.as_uuid()).bind(generic).fetch_one(&mut *tx).await.unwrap();assert_eq!(retained,serde_json::json!({"artifact_id":"external-system-id"}));
        tx.commit().await.unwrap();ordinary.close().await;pool.close().await;
    });
}

#[test]
fn an_unstamped_released_baseline_resumes_from_its_exact_sqlx_prefix() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("released baseline connection");
        released_v043_migrator()
            .run(&mut *connection)
            .await
            .expect("commit the released baseline without a stamp");
        drop(connection);
        assert_eq!(migration_ledger(&pool).await.len(), 1);
        assert_eq!(epoch::read(&pool).await, Err(SchemaEpochError::Unstamped));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("resume with exact old catalogue and apply forward DDL");
        assert_eq!(
            epoch::verify(&pool).await.unwrap().migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        assert_eq!(
            migration_ledger(&pool).await.len(),
            synveda_store::MIGRATOR.migrations.len()
        );
        pool.close().await;
    });
}

#[test]
fn exact_github_prefix_backfills_snapshot_headers_without_rewriting_v1_payloads() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool=connect_pool(&scratch.options).await;
        let mut connection=pool.acquire().await.unwrap();
        sqlx::migrate::Migrator {migrations:std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..8].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT}.run(&mut *connection).await.unwrap();
        epoch::stamp_connection(&mut connection,"0.4.5").await.unwrap();drop(connection);
        let tenant=admit_a_tenant(&pool).await;
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let workspace=synveda_store::workspaces::create(&mut tx,&synveda_store::workspaces::NewWorkspace {
            id:synveda_types::WorkspaceId::new(),tenant_id:tenant,slug:"snapshot-prefix".to_owned(),display_name:"Snapshot prefix".to_owned(),description:None,created_by:None}).await.unwrap();
        let session=synveda_store::sessions::create(&mut tx,&synveda_store::sessions::NewSession {
            id:synveda_types::SessionId::new(),tenant_id:tenant,workspace_id:workspace.id,project_id:None,principal_id:"snapshot-reader".to_owned(),
            client_name:"claude-code".to_owned(),client_version:None,client_installation_id:None,external_session_id:None,agent_name:None,
            model_name:None,repository_id:None,branch:None,task_summary:None,metadata:serde_json::json!({})}).await.unwrap();
        let boundary=serde_json::json!({"schema_version":1,"local_high_sequence":1,"expected_client_event_ids":[],"expected_ids_truncated":false});
        let marker=synveda_store::sessions::append_events(&mut tx,tenant,session.id,&[synveda_store::sessions::NewSessionEvent {
            event_type:synveda_types::session::SessionEventType::CompactionBoundary,event_schema_version:1,client_event_id:"old-boundary".to_owned(),
            occurred_at:chrono::Utc::now(),source_payload_hash:synveda_store::sessions::payload_hash(&boundary),payload:boundary,redactions:None,quarantine:false}]).await.unwrap().remove(0).event;
        let payload=serde_json::json!({"schema_version":1,"method":"deterministic_event_excerpts_v1","boundary_event_id":marker.id,
            "previous_checkpoint_event_id":null,"source_first_sequence":null,"source_last_sequence":null,
            "source_digest":blake3::hash(b"synveda.session.checkpoint.sources.v1\0").to_hex().to_string(),"sources":[],"coverage":"observed_window",
            "declared_missing_count":0,"goals":[],"constraints":[],"decisions":[],"unresolved_questions":[],"artefact_references":[],
            "verification_evidence":[],"labelled_excerpts":[]});
        let old=synveda_store::sessions::append_events(&mut tx,tenant,session.id,&[synveda_store::sessions::NewSessionEvent {
            event_type:synveda_types::session::SessionEventType::Checkpoint,event_schema_version:1,client_event_id:"old-checkpoint".to_owned(),
            occurred_at:chrono::Utc::now(),source_payload_hash:synveda_store::sessions::payload_hash(&payload),payload:payload.clone(),redactions:None,quarantine:false}]).await.unwrap().remove(0).event;
        tx.commit().await.unwrap();
        let before=migration_ledger(&pool).await;assert_eq!(before.len(),8);
        assert_eq!(synveda_store::check_migration_compatibility(&pool,&scratch.roles).await.unwrap().migration_head,"0008");
        synveda_store::migrate(&pool,&scratch.roles).await.unwrap();
        let after=migration_ledger(&pool).await;assert_eq!(&after[..8],&before);assert_eq!(after.len(),synveda_store::MIGRATOR.migrations.len());
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let retained=synveda_store::sessions::event(&mut *tx,tenant,session.id,old.id).await.unwrap().unwrap();
        assert_eq!(retained.payload,payload);assert_eq!(retained.payload_hash,old.payload_hash);
        let header=synveda_store::task_state::latest_checkpoint(&mut tx,tenant,session.id,None).await.unwrap().unwrap();
        assert_eq!(header.id,old.id.as_uuid());assert_eq!(header.payload_hash,old.payload_hash);assert_eq!(header.sequence,old.sequence);
        tx.commit().await.unwrap();pool.close().await;
    });
}

#[test]
fn exact_task_state_prefix_retains_snapshot_bytes_and_adds_private_learning_custody() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool=connect_pool(&scratch.options).await;
        let mut connection=pool.acquire().await.unwrap();
        sqlx::migrate::Migrator {migrations:std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..9].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT}.run(&mut *connection).await.unwrap();
        epoch::stamp_connection(&mut connection,"0.4.5").await.unwrap();drop(connection);
        let tenant=admit_a_tenant(&pool).await;
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let workspace=synveda_store::workspaces::create(&mut tx,&synveda_store::workspaces::NewWorkspace {
            id:synveda_types::WorkspaceId::new(),tenant_id:tenant,slug:"snapshot-prefix".to_owned(),display_name:"Snapshot prefix".to_owned(),description:None,created_by:None}).await.unwrap();
        let session=synveda_store::sessions::create(&mut tx,&synveda_store::sessions::NewSession {
            id:synveda_types::SessionId::new(),tenant_id:tenant,workspace_id:workspace.id,project_id:None,principal_id:"snapshot-reader".to_owned(),
            client_name:"claude-code".to_owned(),client_version:None,client_installation_id:None,external_session_id:None,agent_name:None,
            model_name:None,repository_id:None,branch:None,task_summary:None,metadata:serde_json::json!({})}).await.unwrap();
        let boundary=serde_json::json!({"schema_version":1,"local_high_sequence":1,"expected_client_event_ids":[],"expected_ids_truncated":false});
        let marker=synveda_store::sessions::append_events(&mut tx,tenant,session.id,&[synveda_store::sessions::NewSessionEvent {
            event_type:synveda_types::session::SessionEventType::CompactionBoundary,event_schema_version:1,client_event_id:"old-boundary".to_owned(),
            occurred_at:chrono::Utc::now(),source_payload_hash:synveda_store::sessions::payload_hash(&boundary),payload:boundary,redactions:None,quarantine:false}]).await.unwrap().remove(0).event;
        let payload=serde_json::json!({"schema_version":1,"method":"deterministic_event_excerpts_v1","boundary_event_id":marker.id,
            "previous_checkpoint_event_id":null,"source_first_sequence":null,"source_last_sequence":null,
            "source_digest":blake3::hash(b"synveda.session.checkpoint.sources.v1\0").to_hex().to_string(),"sources":[],"coverage":"observed_window",
            "declared_missing_count":0,"goals":[],"constraints":[],"decisions":[],"unresolved_questions":[],"artefact_references":[],
            "verification_evidence":[],"labelled_excerpts":[]});
        let old=synveda_store::sessions::append_events(&mut tx,tenant,session.id,&[synveda_store::sessions::NewSessionEvent {
            event_type:synveda_types::session::SessionEventType::Checkpoint,event_schema_version:1,client_event_id:"old-checkpoint".to_owned(),
            occurred_at:chrono::Utc::now(),source_payload_hash:synveda_store::sessions::payload_hash(&payload),payload:payload.clone(),redactions:None,quarantine:false}]).await.unwrap().remove(0).event;
        tx.commit().await.unwrap();
        let before=migration_ledger(&pool).await;assert_eq!(before.len(),9);
        assert_eq!(synveda_store::check_migration_compatibility(&pool,&scratch.roles).await.unwrap().migration_head,"0009");
        synveda_store::migrate(&pool,&scratch.roles).await.unwrap();
        let after=migration_ledger(&pool).await;assert_eq!(&after[..9],&before);assert_eq!(after.len(),synveda_store::MIGRATOR.migrations.len());
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let retained=synveda_store::sessions::event(&mut *tx,tenant,session.id,old.id).await.unwrap().unwrap();
        assert_eq!(retained.payload,payload);assert_eq!(retained.payload_hash,old.payload_hash);
        let header=synveda_store::task_state::latest_checkpoint(&mut tx,tenant,session.id,None).await.unwrap().unwrap();
        assert_eq!(header.id,old.id.as_uuid());assert_eq!(header.payload_hash,old.payload_hash);assert_eq!(header.sequence,old.sequence);
        tx.commit().await.unwrap();pool.close().await;
    });
}

#[test]
fn a_committed_forward_migration_with_old_stamp_repairs_only_the_stamp() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("released baseline connection");
        released_v043_migrator()
            .run(&mut *connection)
            .await
            .expect("commit released baseline");
        epoch::stamp_connection(&mut connection, "0.4.3")
            .await
            .expect("stamp released head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;

        let mut connection = pool.acquire().await.expect("forward migration connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit forward DDL and SQLx row without marker stamp");
        drop(connection);
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(
            ledger_before.len(),
            synveda_store::MIGRATOR.migrations.len()
        );
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0001");
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { .. })
        ));
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("an interrupted marker is not a completed image transition");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("repair only the marker stamp");
        assert_eq!(
            epoch::verify(&pool).await.unwrap().migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

#[test]
fn released_baseline_catalogue_drift_refuses_before_forward_ddl() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("released baseline connection");
        released_v043_migrator()
            .run(&mut *connection)
            .await
            .expect("commit released baseline");
        epoch::stamp_connection(&mut connection, "0.4.3")
            .await
            .expect("stamp released head");
        drop(connection);
        sqlx::query("alter table public.audit_log disable trigger audit_log_no_update")
            .execute(&pool)
            .await
            .expect("drift one old trigger in the scratch database");
        let ledger_before = migration_ledger(&pool).await;

        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("read-only prior catalogue proof must refuse drift");
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("the prior catalogue proof must precede DDL");
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0001");
        pool.close().await;
    });
}

/// A database already at this epoch starts normally, and migrating it again
/// changes only the head — never who created it.
#[test]
fn a_current_epoch_database_starts_normally_and_keeps_its_provenance() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        let first = epoch::verify(&pool).await.expect("a current database");
        let tenant = admit_a_tenant(&pool).await;

        // The idempotent second run every deployment bootstrap and test
        // harness performs.
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate again");
        let second = epoch::verify(&pool).await.expect("still current");

        assert_eq!(second.epoch, first.epoch);
        assert_eq!(second.baseline_revision, first.baseline_revision);
        assert_eq!(
            second.created_at, first.created_at,
            "the epoch was not recreated"
        );
        assert_eq!(
            second.created_by_version, first.created_by_version,
            "`created_by_version` answers which release minted this database, \
             not which one last ran"
        );
        assert_eq!(second.migration_head, first.migration_head);
        // And it did not disturb what was in it.
        assert_eq!(tenant_count(&pool).await, 1);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("resolve")
                .is_some()
        );
        pool.close().await;
    });
}

/// The image-transition gate reads the complete current contract under a
/// database-enforced read-only snapshot and preserves both product and
/// migration evidence byte-for-byte.
#[test]
fn candidate_compatibility_accepts_the_exact_baseline_without_writes() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        let tenant = admit_a_tenant(&pool).await;
        let metadata_before = epoch::verify(&pool).await.expect("current metadata");
        let ledger_before = migration_ledger(&pool).await;

        let checked = synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the embedded candidate matches the current baseline");

        assert_eq!(checked, metadata_before);
        assert_eq!(epoch::verify(&pool).await.unwrap(), metadata_before);
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("resolve preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// A candidate never turns ledger drift into migration or reset advice. It
/// refuses, leaves the drift for the operator to investigate, and preserves
/// the tenant whose running image was already serving.
#[test]
fn candidate_compatibility_refuses_ledger_drift_without_writes_or_reset_advice() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        sqlx::query(
            "update _sqlx_migrations \
             set checksum = pg_catalog.decode(pg_catalog.repeat('00', 48), 'hex')",
        )
        .execute(&pool)
        .await
        .expect("drift the candidate ledger");
        let metadata_before = epoch::read(&pool).await.expect("read current marker");
        let ledger_before = migration_ledger(&pool).await;

        let error = synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("candidate must refuse a drifted ledger");
        let message = error.to_string();
        assert!(
            message.contains("keep the currently running image"),
            "{message}"
        );
        assert!(message.contains("database was not changed"), "{message}");
        assert!(!message.contains(RESET_COMMAND), "{message}");
        assert_eq!(epoch::read(&pool).await.unwrap(), metadata_before);
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// If a marker survives but SQLx's ledger does not, neither the candidate nor
/// the migrator may mistake the missing relation for a pre-cut database.
#[test]
fn a_missing_sqlx_ledger_with_a_marker_preserves_customer_data() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        let tenant = admit_a_tenant(&pool).await;
        sqlx::query("drop table _sqlx_migrations")
            .execute(&pool)
            .await
            .expect("remove scratch ledger");

        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("candidate must refuse the missing ledger");
        let error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("migrator must not recreate unknown history");
        assert!(!error.to_string().contains(RESET_COMMAND), "{error}");
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("lookup preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// The marker head is diagnostic during ordinary startup but exact during an
/// image transition: a candidate must bind it to its sole embedded baseline.
#[test]
fn candidate_compatibility_refuses_a_wrong_marker_head_without_writes() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        sqlx::query("update schema_metadata set migration_head = '9999'")
            .execute(&pool)
            .await
            .expect("drift the marker head");
        let metadata_before = epoch::read(&pool).await.expect("read drifted marker");
        let ledger_before = migration_ledger(&pool).await;

        let error = synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("candidate must refuse a marker for another migration head");
        let message = error.to_string();
        assert!(
            message.contains("keep the currently running image"),
            "{message}"
        );
        assert!(!message.contains(RESET_COMMAND), "{message}");
        assert_eq!(epoch::read(&pool).await.unwrap(), metadata_before);
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// CPR-45 amended the contents of the pre-release epoch-3 baseline without
/// changing the domain epoch. An epoch-3 marker from before the immutable
/// revision discriminator must be refused before SQLx compares checksums.
#[test]
fn an_unversioned_epoch_three_baseline_is_refused_before_migration() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        let ledger_before: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read migration ledger");

        sqlx::query("alter table schema_metadata drop column baseline_revision")
            .execute(&pool)
            .await
            .expect("model the unversioned interim epoch-3 marker");

        let refusal = epoch::verify(&pool).await.expect_err("unversioned epoch 3");
        assert_eq!(refusal, SchemaEpochError::OlderRevision { found: 0 });
        assert!(refusal.to_string().contains(RESET_COMMAND));
        let migrate_error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("revision refusal precedes SQLx");
        assert!(migrate_error.to_string().contains(RESET_COMMAND));

        let ledger_after: Vec<(i64, Vec<u8>, bool)> = sqlx::query_as(
            "select version, checksum, success from _sqlx_migrations order by version",
        )
        .fetch_all(&pool)
        .await
        .expect("read unchanged migration ledger");
        assert_eq!(ledger_after, ledger_before, "SQLx never reached the ledger");
        assert_eq!(tenant_count(&pool).await, 1, "product rows were untouched");
        pool.close().await;
    });
}

// ── a database from before the cut ───────────────────────────────────────

/// The whole point of the feature: a database written before the context
/// platform is refused, told what to run, and **left exactly as it was**.
#[test]
fn a_database_from_before_the_cut_is_refused_and_not_touched() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        remove_epoch_marker(&pool).await;

        // The startup guard.
        let refusal = epoch::verify(&pool).await.expect_err("a pre-cut database");
        assert_eq!(refusal, SchemaEpochError::Missing);
        assert!(refusal.is_refusal(), "an outage is not a verdict");
        let message = refusal.to_string();
        assert!(message.contains(RESET_COMMAND), "{message}");
        assert!(message.contains("hard cut"), "{message}");

        // The migrator. Refused *before* it runs, so this is also the
        // assertion that no migration attempted a translation and stopped
        // half way.
        let err = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("migrating a pre-cut database");
        assert!(err.to_string().contains(RESET_COMMAND), "{err}");

        // Untouched: still no marker, and the row that was there is still
        // there, unread and unmoved.
        assert!(
            !has_marker(&pool).await,
            "a refused migration wrote the marker anyway"
        );
        assert_eq!(tenant_count(&pool).await, 1);
        pool.close().await;
    });
}

/// An interrupted stamp leaves the table present but its row absent.
///
/// Reached by anything that truncates it, and by a database created by
/// running the migrator directly rather than through `synveda_store::migrate`.
/// The old runtime cannot infer the model from that row, but reset guidance
/// would destroy data that the migrator can recover from an exact SQLx prefix.
#[test]
fn a_marker_with_no_row_is_refused() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        sqlx::query("delete from schema_metadata")
            .execute(&pool)
            .await
            .expect("empty the marker");

        let refusal = epoch::verify(&pool).await.expect_err("no row");
        assert_eq!(refusal, SchemaEpochError::Unstamped);
        assert!(!refusal.to_string().contains(RESET_COMMAND));
        pool.close().await;
    });
}

/// A marker this build cannot read is refused rather than guessed at.
///
/// Two shapes, and they are different failures: a table wearing the name with
/// other columns in it, and our own table holding a value the guard will not
/// stand behind. Both mean "nothing here can tell which model these rows are
/// in", which is the same answer as no marker at all — but said differently,
/// because an operator looking at a corrupted marker and one looking at an
/// old database are looking for different things.
#[test]
fn a_marker_this_build_cannot_read_is_refused() {
    let Some(server) = server() else { return };

    // (a) somebody else's `schema_metadata`.
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        sqlx::query("alter table schema_metadata rename column created_by_version to author")
            .execute(&pool)
            .await
            .expect("reshape the marker");

        let refusal = epoch::verify(&pool).await.expect_err("a foreign shape");
        assert!(
            matches!(refusal, SchemaEpochError::Malformed(_)),
            "expected a malformed verdict, got {refusal:?}"
        );
        assert!(refusal.is_refusal());
        assert!(!refusal.to_string().contains(RESET_COMMAND));
        pool.close().await;
    });
    drop(scratch);

    // (b) our shape, holding a value the guard will not accept. The CHECK
    // constraints stop *us* writing one; they do not stop a restore, a
    // hand-edit or another product's tooling, and this is the code that
    // meets that.
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        sqlx::query(
            "alter table schema_metadata
                 drop constraint schema_metadata_created_by_version_check",
        )
        .execute(&pool)
        .await
        .expect("drop the constraint this test is bypassing");
        sqlx::query("update schema_metadata set created_by_version = ''")
            .execute(&pool)
            .await
            .expect("blank the creating version");

        let refusal = epoch::verify(&pool).await.expect_err("a blank provenance");
        assert!(
            matches!(refusal, SchemaEpochError::Malformed(_)),
            "expected a malformed verdict, got {refusal:?}"
        );
        assert!(!refusal.to_string().contains(RESET_COMMAND));
        pool.close().await;
    });
}

/// An epoch this build has moved past is refused, and one it has not caught
/// up to is refused differently.
///
/// Epoch 2 is the immediately preceding development epoch and therefore a
/// concrete refusal, while a newer epoch still means the binary must move
/// forward rather than telling an operator to destroy readable data.
#[test]
fn an_epoch_that_is_not_this_one_is_refused_in_both_directions() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");

        sqlx::query("update schema_metadata set epoch = $1")
            .bind(CURRENT_EPOCH - 1)
            .execute(&pool)
            .await
            .expect("age the marker");
        let older = epoch::verify(&pool).await.expect_err("an older epoch");
        assert_eq!(
            older,
            SchemaEpochError::Older {
                found: CURRENT_EPOCH - 1
            }
        );
        let message = older.to_string();
        assert!(message.contains(RESET_COMMAND), "{message}");
        let migrate_error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("epoch 2 must be refused before checksum comparison");
        assert!(
            migrate_error.to_string().contains(RESET_COMMAND),
            "{migrate_error}"
        );

        // The other direction is not a reset. A database from a newer build
        // holds data this one cannot read, so telling its operator to destroy
        // it would be the worst advice this guard could give.
        sqlx::query("update schema_metadata set epoch = $1")
            .bind(CURRENT_EPOCH + 1)
            .execute(&pool)
            .await
            .expect("advance the marker");
        let newer = epoch::verify(&pool).await.expect_err("a newer epoch");
        assert_eq!(
            newer,
            SchemaEpochError::Newer {
                found: CURRENT_EPOCH + 1
            }
        );
        let message = newer.to_string();
        assert!(message.contains("Upgrade this installation"), "{message}");
        assert!(message.contains("would destroy it"), "{message}");

        sqlx::query("update schema_metadata set epoch = $1, baseline_revision = $2")
            .bind(CURRENT_EPOCH)
            .bind(CURRENT_BASELINE_REVISION + 1)
            .execute(&pool)
            .await
            .expect("advance only the baseline revision");
        let newer_revision = epoch::verify(&pool)
            .await
            .expect_err("a newer baseline revision");
        assert_eq!(
            newer_revision,
            SchemaEpochError::NewerRevision {
                found: CURRENT_BASELINE_REVISION + 1,
            }
        );
        let message = newer_revision.to_string();
        assert!(message.contains("Upgrade this installation"), "{message}");
        assert!(message.contains("would destroy it"), "{message}");
        pool.close().await;
    });
}

// ── reset ────────────────────────────────────────────────────────────────

/// `synveda reset --database --force`'s engine: a database with an old epoch
/// and rows in it becomes a working current-epoch database with nothing in
/// it, and running it again leaves the same thing.
#[test]
fn reset_creates_a_working_current_epoch_database_and_is_idempotent() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        // Something to destroy: a full pre-cut database with a tenant in it.
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("migrate");
        admit_a_tenant(&pool).await;
        remove_epoch_marker(&pool).await;
        assert!(
            epoch::verify(&pool).await.is_err(),
            "the fixture is refused"
        );
        pool.close().await;

        let first = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("reset a refused database");
        assert!(first.existed_before, "there was a database to destroy");
        assert_eq!(first.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(first.metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        assert_eq!(first.metadata.created_by_version, env!("CARGO_PKG_VERSION"));
        let extension_names: Vec<_> = first
            .extensions
            .iter()
            .map(|extension| extension.name)
            .collect();
        assert_eq!(extension_names, ["vector", "btree_gin"]);

        // Working: the guard accepts it, the schema is there, and it takes a
        // write.
        let pool = connect_pool(&scratch.options).await;
        let metadata = epoch::verify(&pool).await.expect("a current database");
        assert_eq!(metadata.epoch, CURRENT_EPOCH);
        assert_eq!(metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        assert_eq!(
            tenant_count(&pool).await,
            0,
            "a row survived the reset — there is no path that carries one \
             across, and this is what says so"
        );
        admit_a_tenant(&pool).await;
        assert_eq!(tenant_count(&pool).await, 1);

        // Forced RLS survives, because a fresh database that lost the tenant
        // backstop would be a worse outcome than the refusal it replaced.
        let forced: bool = sqlx::query_scalar(
            "select relrowsecurity and relforcerowsecurity
             from pg_class where relname = 'knowledge_items'",
        )
        .fetch_one(&pool)
        .await
        .expect("read the RLS flags");
        assert!(
            forced,
            "the fresh database has no forced RLS on `knowledge_items`"
        );
        pool.close().await;

        // Idempotent: the same command again leaves the same database, with
        // the row this test just wrote gone.
        let second = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("reset again");
        assert!(second.existed_before);
        assert_eq!(second.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(second.metadata.baseline_revision, CURRENT_BASELINE_REVISION);
        let pool = connect_pool(&scratch.options).await;
        assert_eq!(tenant_count(&pool).await, 0);
        epoch::verify(&pool).await.expect("still current");
        pool.close().await;
    });
}

/// Reset on a server that has no such database at all — the state a first
/// install and a re-run after a manual `drop database` are both in.
#[test]
fn reset_builds_a_database_that_was_not_there() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        // Take it away behind reset's back, so `existed_before` has something
        // to be false about.
        let mut admin = server
            .admin
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!(
            "drop database if exists \"{}\" with (force)",
            scratch.name
        ))
        .execute(&mut admin)
        .await
        .expect("drop it");
        drop(admin);

        let outcome = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect("build a database from nothing");
        assert!(!outcome.existed_before);
        assert_eq!(outcome.metadata.epoch, CURRENT_EPOCH);
        assert_eq!(
            outcome.metadata.baseline_revision,
            CURRENT_BASELINE_REVISION
        );

        let pool = connect_pool(&scratch.options).await;
        epoch::verify(&pool).await.expect("a current database");
        pool.close().await;
    });
}

/// A safe-looking name is not sufficient authority to destroy a database.
/// Wrong ownership is refused while its catalog identity, data and protected
/// membership graph remain byte-for-byte unchanged.
#[test]
fn reset_refuses_a_wrong_owner_without_mutation() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        let mut admin = server
            .admin
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!("drop database \"{}\" with (force)", scratch.name))
            .execute(&mut admin)
        .await
        .expect("drop the exact-owner scratch target");
        let wrong_owner = &server.roles.runtime().administrators()[0];
        sqlx::query(&format!(
            "create database \"{}\" with owner \"{wrong_owner}\" template template0 encoding 'UTF8'",
            scratch.name,
        ))
        .execute(&mut admin)
        .await
        .expect("create the wrong-owner target");
        let before_identity = sqlx::query_as::<_, (i64, String)>(
            "select database.oid::bigint, owner.rolname::text \
               from pg_catalog.pg_database as database \
               join pg_catalog.pg_roles as owner on owner.oid = database.datdba \
              where database.datname = $1",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("read wrong-owner target identity");
        let before_memberships = protected_membership_fingerprint(&mut admin, &scratch.roles).await;

        let mut target_admin = server
            .admin
            .clone()
            .database(&scratch.name)
            .connect()
            .await
            .expect("connect to the wrong-owner target");
        sqlx::query("create table reset_wrong_owner_sentinel (id integer primary key)")
            .execute(&mut target_admin)
            .await
            .expect("create wrong-owner sentinel");
        sqlx::query("insert into reset_wrong_owner_sentinel (id) values (1)")
            .execute(&mut target_admin)
            .await
            .expect("write wrong-owner sentinel");
        target_admin
            .close()
            .await
            .expect("close sentinel connection");

        let error = synveda_store::reset::recreate(&scratch.admin_url, &url, &scratch.roles)
            .await
            .expect_err("wrong ownership must refuse before DROP DATABASE");
        assert!(
            error.to_string().contains("database reset requires"),
            "{error}"
        );
        let after_identity = sqlx::query_as::<_, (i64, String)>(
            "select database.oid::bigint, owner.rolname::text \
               from pg_catalog.pg_database as database \
               join pg_catalog.pg_roles as owner on owner.oid = database.datdba \
              where database.datname = $1",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("re-read wrong-owner target identity");
        assert_eq!(after_identity, before_identity);
        let after_memberships = protected_membership_fingerprint(&mut admin, &scratch.roles).await;
        assert_eq!(after_memberships, before_memberships);

        let target_admin = connect_pool(&server.admin.clone().database(&scratch.name)).await;
        let sentinel: i64 = sqlx::query_scalar("select count(*) from reset_wrong_owner_sentinel")
            .fetch_one(&target_admin)
            .await
            .expect("read preserved wrong-owner sentinel");
        assert_eq!(sentinel, 1);
        target_admin.close().await;
    });
}

/// Managed-service administrators may be allowlisted for extension ownership
/// without having superuser authority. The local destructive reset command
/// refuses that shape before creating an absent target.
#[test]
fn reset_refuses_an_allowlisted_non_superuser_without_mutation() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let url = scratch.url();
    scratch.block_on(async {
        let role = format!(
            "synveda_reset_admin_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let password = "ResetOnlyCredential_93";
        let mut admin = server
            .admin
            .clone()
            .database("postgres")
            .connect()
            .await
            .expect("connect to the maintenance database");
        sqlx::query(&format!("drop database \"{}\" with (force)", scratch.name))
            .execute(&mut admin)
            .await
            .expect("make the reset target absent");
        sqlx::query(&format!(
            "create role \"{role}\" login inherit nosuperuser nocreatedb nocreaterole \
             noreplication nobypassrls password '{password}'"
        ))
        .execute(&mut admin)
        .await
        .expect("create an ordinary reset administrator");
        sqlx::query(&format!("grant connect on database postgres to \"{role}\""))
            .execute(&mut admin)
            .await
            .expect("allow the ordinary administrator to reach maintenance");
        let roles = DatabaseRoles::new(
            server.roles.migrator().to_owned(),
            server.roles.gateway().to_owned(),
            server.roles.worker().to_owned(),
            vec![
                server.roles.runtime().administrators()[0].clone(),
                role.clone(),
            ],
            Vec::new(),
            server.roles.forbidden_databases().to_vec(),
            server.roles.isolated_peer_roles().to_vec(),
        )
        .expect("extended reset administrator contract");
        let before_memberships = protected_membership_fingerprint(&mut admin, &roles).await;
        let non_superuser_url = replace_credentials(&scratch.admin_url, &role, password);

        let error = synveda_store::reset::recreate(&non_superuser_url, &url, &roles)
            .await
            .expect_err("an ordinary allowlisted administrator cannot run local reset");
        assert!(
            error.to_string().contains("database reset requires"),
            "{error}"
        );
        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from pg_catalog.pg_database where datname = $1)",
        )
        .bind(&scratch.name)
        .fetch_one(&mut admin)
        .await
        .expect("prove the reset target stayed absent");
        assert!(!exists);
        let after_memberships = protected_membership_fingerprint(&mut admin, &roles).await;
        assert_eq!(after_memberships, before_memberships);

        sqlx::query(&format!(
            "revoke connect on database postgres from \"{role}\""
        ))
        .execute(&mut admin)
        .await
        .expect("remove the test administrator ACL");
        sqlx::query(&format!("drop role \"{role}\""))
            .execute(&mut admin)
            .await
            .expect("drop the ordinary reset administrator");
    });
}

/// A database name this command will not destroy is refused before anything
/// is dropped. The unit tests in `reset.rs` pin the grammar; this pins that
/// the grammar is actually on the path.
#[test]
fn reset_refuses_a_database_name_it_will_not_quote() {
    let Some(server) = server() else { return };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(async {
        let hostile = replace_database(&server.migrator_url, "syn%22veda");
        let err = synveda_store::reset::recreate(&server.admin_url, &hostile, &server.roles)
            .await
            .expect_err("a name with a quote in it");
        assert!(
            err.to_string()
                .contains("not a database name this command will destroy"),
            "{err}"
        );
    });
}

// ── no old-to-new migrator ───────────────────────────────────────────────

/// Epoch 3 has the released schema-only baseline and nine additive forward
/// migrations: no pre-epoch translator, reversible pair or top-level statement
/// that can carry or seed data. Function
/// bodies may of course mutate current tables at runtime; the scanner removes
/// every PostgreSQL dollar-quoted body before classifying top-level verbs.
#[test]
fn no_old_to_new_data_migrator_exists() {
    let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    let mut files: Vec<_> = std::fs::read_dir(&migrations)
        .expect("read the migrations directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    files.sort();
    let names: Vec<_> = files
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("UTF-8 migration name")
        })
        .collect();
    assert_eq!(
        names,
        [
            "0001_context_platform.sql",
            "0002_context_restart_and_excerpt.sql",
            "0003_one_time_login_ledger.sql",
            "0004_context_delivery_dependencies.sql",
            "0005_session_source_control.sql",
            "0006_import_artifact_source_control.sql",
            "0007_inbound_git_source_reconciliation.sql",
            "0008_private_github_source_identity.sql",
            "0009_grounded_task_state_and_handoff.sql",
            "0010_verified_procedure_improvements.sql"
        ],
        "epoch 3 has the released baseline and nine forward migrations"
    );

    let down: Vec<_> = names
        .iter()
        .filter(|name| name.ends_with(".down.sql"))
        .collect();
    assert!(
        down.is_empty(),
        "a down-migration would make the epoch look reversible and is where a \
         translation would hide: {down:?}"
    );

    for path in &files {
        let source = std::fs::read_to_string(path).expect("read the epoch migration");
        for statement in top_level_statements(&source) {
            let verb = statement.split_whitespace().next().unwrap_or_default();
            assert!(
                !matches!(
                    verb,
                    "select" | "insert" | "update" | "delete" | "copy" | "with"
                ),
                "{} runs top-level `{verb}`. Schema migration cannot carry or seed \
                 data outside the PDP.\n  {}",
                path.display(),
                statement.trim(),
            );
        }
    }

    let source = std::fs::read_to_string(&files[0]).expect("read the released baseline");

    for retired in [
        "create table public.records",
        "records_versions",
        "memory_usage",
        "policy_lapses",
        "role_bindings",
        "'prompt', 'context_pack', 'memory'",
        "new.evidence is distinct from old.evidence",
        "create extension if not exists age",
        "create extension if not exists pgmq",
        "set row_security = off",
    ] {
        assert!(
            !source.to_lowercase().contains(retired),
            "the clean baseline retained `{retired}`"
        );
    }
}

/// A migration's statements, comments stripped and every dollar-quoted body
/// skipped. PostgreSQL permits both `$$` and tagged forms such as `$_$`.
fn top_level_statements(source: &str) -> Vec<String> {
    let without_bodies = strip_dollar_quoted(source);
    let stripped: String = without_bodies
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    stripped
        .split(';')
        .map(|statement| statement.trim().to_owned())
        .filter(|statement| !statement.is_empty())
        .collect()
}

fn strip_dollar_quoted(source: &str) -> String {
    let mut remaining = source;
    let mut stripped = String::with_capacity(source.len());
    while let Some(open) = remaining.find('$') {
        stripped.push_str(&remaining[..open]);
        let candidate = &remaining[open..];
        let Some(tag_tail) = candidate[1..].find('$') else {
            stripped.push_str(candidate);
            return stripped;
        };
        let tag_end = tag_tail + 1;
        let name = &candidate[1..tag_end];
        if !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            stripped.push('$');
            remaining = &candidate[1..];
            continue;
        }
        let delimiter = &candidate[..=tag_end];
        let body = &candidate[delimiter.len()..];
        let Some(close) = body.find(delimiter) else {
            stripped.push_str(candidate);
            return stripped;
        };
        stripped.push(' ');
        remaining = &body[close + delimiter.len()..];
    }
    stripped.push_str(remaining);
    stripped
}

async fn has_marker(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>("select to_regclass('public.schema_metadata') is not null")
        .fetch_one(pool)
        .await
        .expect("look for the marker")
}
