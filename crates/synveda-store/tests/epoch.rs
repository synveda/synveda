//! Store tests: the schema epoch guard and the reset that is the only
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
use synveda_store::MigrationBounds;
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
                "vector 0.8.7 (or a legacy 0.8.2-0.8.6 profile) and btree_gin 1.3 in public",
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
                "vector 0.8.7 on 14-18; legacy 0.8.6 on 14-18 or 0.8.2-0.8.5 on 17; btree_gin 1.3; plpgsql 1.0",
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

/// An extension version outside the measured profiles is refused before
/// baseline DDL and on a current schema, whatever its catalogue contains.
#[test]
fn an_unmeasured_vector_version_is_refused_on_fresh_and_current_databases() {
    let Some(server) = server() else { return };
    for migrated in [false, true] {
        let scratch = Scratch::empty(&server);
        scratch.block_on(async {
            let pool = connect_pool(&scratch.options).await;
            if migrated {
                synveda_store::migrate(&pool, &scratch.roles)
                    .await
                    .expect("migrate the isolated database before relabelling");
            }
            let admin = connect_pool(&scratch.admin.clone().database(&scratch.name)).await;
            sqlx::query(
                "update pg_catalog.pg_extension set extversion = '0.8.8' where extname = 'vector'",
            )
            .execute(&admin)
            .await
            .expect("relabel the extension in the isolated database");
            admin.close().await;

            let error = synveda_store::migrate(&pool, &scratch.roles)
                .await
                .expect_err("an unmeasured version must refuse");
            let message = error.to_string();
            assert!(message.contains("vector 0.8.7"), "{message}");
            assert!(!message.contains("postgresql://"), "{message}");
            assert_eq!(has_marker(&pool).await, migrated);
            pool.close().await;
        });
    }
}

/// Port of upstream pgvector's `047_hnsw_vacuum_insert` regression: concurrent
/// inserts, deletes, index scans and VACUUM on one HNSW index must not error,
/// and the maintained index must still return only live, unique rows close to
/// an exact scan. The 0.8.4 and 0.8.7 native fixes address this interaction.
#[test]
fn concurrent_hnsw_writes_and_vacuum_keep_the_index_consistent() {
    const CLIENTS: u64 = 5;
    const TRANSACTIONS_PER_CLIENT: u64 = 200;
    const VACUUMS: usize = 10;
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let mut setup = scratch.options.connect().await.expect("probe setup");
        for statement in [
            "create table synveda_hnsw_concurrency (i bigserial primary key, v public.vector(16) not null) \
             with (autovacuum_enabled = false)",
            "select pg_catalog.setseed(0.25)",
            "insert into synveda_hnsw_concurrency (v) \
             select array(select pg_catalog.random()::real from generate_series(1, 16) where value > 0)::public.vector(16) \
             from generate_series(1, 1000) value",
            "set max_parallel_maintenance_workers = 0",
            "create index synveda_hnsw_concurrency_v on synveda_hnsw_concurrency \
             using hnsw (v public.vector_cosine_ops)",
        ] {
            sqlx::query(statement)
                .execute(&mut setup)
                .await
                .expect("prepare the owned concurrency probe");
        }

        let mut workers = tokio::task::JoinSet::new();
        for client in 0..CLIENTS {
            let options = scratch.options.clone();
            workers.spawn(async move {
                let mut connection = options.connect().await?;
                sqlx::query("set enable_seqscan = off").execute(&mut connection).await?;
                let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ client;
                for _ in 0..TRANSACTIONS_PER_CLIENT {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let statement = match (state >> 33) % 1020 {
                        0..500 => "insert into synveda_hnsw_concurrency (v) \
                                   select array(select pg_catalog.random()::real \
                                   from generate_series(1, 16))::public.vector(16)",
                        500..1000 => "delete from synveda_hnsw_concurrency where i = \
                                      (select i from synveda_hnsw_concurrency limit 1)",
                        _ => "select i from synveda_hnsw_concurrency order by v <=> \
                              (select array(select pg_catalog.random()::real \
                              from generate_series(1, 16))::public.vector(16)) limit 10",
                    };
                    sqlx::query(statement).execute(&mut connection).await?;
                }
                Ok::<_, sqlx::Error>(())
            });
        }
        let options = scratch.options.clone();
        workers.spawn(async move {
            let mut connection = options.connect().await?;
            for _ in 0..VACUUMS {
                sqlx::query("vacuum synveda_hnsw_concurrency")
                    .execute(&mut connection)
                    .await?;
            }
            Ok(())
        });
        let finished = tokio::time::timeout(std::time::Duration::from_secs(300), async {
            while let Some(joined) = workers.join_next().await {
                joined
                    .expect("join one bounded probe client")
                    .expect("concurrent HNSW maintenance must not error");
            }
        })
        .await;
        assert!(finished.is_ok(), "the bounded concurrency probe timed out");

        sqlx::query("vacuum synveda_hnsw_concurrency")
            .execute(&mut setup)
            .await
            .expect("final probe vacuum");
        let live: i64 = sqlx::query_scalar("select count(*) from synveda_hnsw_concurrency")
            .fetch_one(&mut setup)
            .await
            .expect("count live probe rows");
        assert!(live > 0, "the probe keeps live rows");
        for setting in ["set enable_seqscan = off", "set hnsw.ef_search = 1000"] {
            sqlx::query(setting)
                .execute(&mut setup)
                .await
                .expect("select the indexed probe plan");
        }
        let indexed: Vec<i64> = sqlx::query_scalar(
            "select i from synveda_hnsw_concurrency order by v <=> \
             array_fill(1::real, array[16])::public.vector(16) limit 1000",
        )
        .fetch_all(&mut setup)
        .await
        .expect("scan the maintained index");
        let unique: std::collections::BTreeSet<i64> = indexed.iter().copied().collect();
        assert_eq!(unique.len(), indexed.len(), "an index scan returned a duplicate row");
        let missing: i64 = sqlx::query_scalar(
            "select count(*) from unnest($1::bigint[]) returned(i) \
             where not exists (select 1 from synveda_hnsw_concurrency probe where probe.i = returned.i)",
        )
        .bind(&indexed)
        .fetch_one(&mut setup)
        .await
        .expect("compare index results with live rows");
        assert_eq!(missing, 0, "an index scan returned a deleted row");

        let mut matched = 0_usize;
        for query in 0..5 {
            let probe = format!(
                "array_prepend({}::real, array_fill(0.5::real, array[15]))::public.vector(16)",
                f64::from(query) / 4.0
            );
            let order = format!("select i from synveda_hnsw_concurrency order by v <=> {probe} limit 10");
            sqlx::query("set enable_indexscan = on").execute(&mut setup).await.expect("index on");
            let approximate: Vec<i64> = sqlx::query_scalar(&order)
                .fetch_all(&mut setup)
                .await
                .expect("indexed nearest neighbours");
            sqlx::query("set enable_indexscan = off").execute(&mut setup).await.expect("index off");
            sqlx::query("set enable_seqscan = on").execute(&mut setup).await.expect("seqscan on");
            let exact: Vec<i64> = sqlx::query_scalar(&order)
                .fetch_all(&mut setup)
                .await
                .expect("exact nearest neighbours");
            sqlx::query("set enable_seqscan = off").execute(&mut setup).await.expect("seqscan off");
            matched += approximate.iter().filter(|id| exact.contains(id)).count();
        }
        assert!(matched >= 48, "maintained HNSW recall fell to {matched}/50");
        sqlx::query("drop table synveda_hnsw_concurrency")
            .execute(&mut setup)
            .await
            .expect("remove the owned concurrency probe");
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
        let ledger = migration_ledger(&pool).await;
        let portable = &synveda_store::PORTABLE_MIGRATOR.migrations;
        assert_eq!(
            ledger.len(),
            portable.len(),
            "a fresh install runs only the portable lineage"
        );
        for (row, migration) in ledger.iter().zip(portable.iter()) {
            assert_eq!(
                (row.0, row.1.as_str(), row.4.as_slice()),
                (
                    migration.version,
                    migration.description.as_ref(),
                    migration.checksum.as_ref()
                ),
            );
        }
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
        assert_eq!(rows, synveda_store::PORTABLE_MIGRATOR.migrations.len() as i64);
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
        synveda_store::PORTABLE_MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional portable baseline");
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
        synveda_store::PORTABLE_MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the transactional portable baseline");
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
        // Previous-release writers stop before an installed schema changes.
        ordinary.close().await;
        synveda_store::migrate(&pool,&scratch.roles).await.unwrap();
        let after=migration_ledger(&pool).await;assert_eq!(after.len(),synveda_store::MIGRATOR.migrations.len());assert_eq!(&after[..5],&before[..]);
        let ordinary=PgPool::connect(&replace_database(&gateway_url,&scratch.name)).await.unwrap();
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

/// The portable reference deployment amended the contents of the pre-release epoch-3 baseline without
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
            "0010_verified_procedure_improvements.sql",
            "0011_portable_postgres_convergence.sql",
            "0012_source_admissibility_plans.sql",
            "0013_session_inventory_and_token_revocation.sql",
            "0014_restorable_tenant_foreign_keys.sql",
            "0015_tenant_erasure.sql",
            "0016_capture_retry_schedule.sql",
            "0017_tenant_usage_budgets.sql"
        ],
        "epoch 3 has the released baseline and sixteen forward migrations"
    );
    let portable = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("portable-migrations")
        .join("0011_portable_context_platform.sql");

    let down: Vec<_> = names
        .iter()
        .filter(|name| name.ends_with(".down.sql"))
        .collect();
    assert!(
        down.is_empty(),
        "a down-migration would make the epoch look reversible and is where a \
         translation would hide: {down:?}"
    );

    files.push(portable.clone());
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

    for baseline in [&files[0], &portable] {
        let source = std::fs::read_to_string(baseline).expect("read an epoch-3 baseline");

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

/// A retained database at the published 0010 head converges on the portable
/// schema through the append-only 0011 migration. Its ledger prefix, marker
/// provenance and Knowledge rows survive, and the result serves the same
/// catalogue as a fresh portable installation (ADR-0152).
#[test]
fn published_procedure_head_converges_without_rewriting_retained_rows() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..10].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0010 head");
        epoch::stamp_connection(&mut connection, "0.4.6")
            .await
            .expect("stamp the published 0010 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let (item, revisions) = retained_knowledge(&pool, tenant).await;
        let known_at = chrono::Utc::now();
        let current_before = read_current(&pool, tenant, item).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 10);
        assert_eq!(public_view_count(&pool).await, 2);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0010 head is upgradeable without writes")
                .migration_head,
            "0010"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { .. })
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("converge the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..10], &ledger_before[..]);
        let convergence = &synveda_store::MIGRATOR.migrations[10];
        assert_eq!(
            (ledger.len(), ledger[10].0, ledger[10].4.as_slice()),
            (
                synveda_store::MIGRATOR.migrations.len(),
                convergence.version,
                convergence.checksum.as_ref()
            ),
        );
        let metadata = epoch::verify(&pool)
            .await
            .expect("the converged head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert_eq!(metadata.created_by_version, "0.4.6");
        assert_eq!(public_view_count(&pool).await, 0);
        assert_eq!(read_current(&pool, tenant, item).await, current_before);
        let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
            .await
            .expect("begin retained as-known read");
        let known = synveda_store::knowledge::as_known_at(&mut *tx, tenant, item, known_at)
            .await
            .expect("read retained Knowledge as known")
            .expect("retained Knowledge is visible");
        assert_eq!(known.revision.id, revisions[1]);
        tx.rollback().await.expect("finish retained reads");
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the converged head matches the embedded catalogue");
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("a converged published lineage reruns idempotently");
        assert_eq!(migration_ledger(&pool).await, ledger);
        pool.close().await;
    });
}

/// The two lineages never mix: a portable ledger stamped with a published
/// intermediate head is drift and is refused without a write.
#[test]
fn a_portable_ledger_under_a_published_intermediate_stamp_is_refused() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("fresh portable installation");
        sqlx::query("update schema_metadata set migration_head = '0010'")
            .execute(&pool)
            .await
            .expect("drift this disposable marker");
        let ledger_before = migration_ledger(&pool).await;
        let error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("a mixed-lineage marker is refused");
        assert!(
            error.to_string().contains("immutable migration prefix"),
            "{error}"
        );
        assert!(!error.to_string().contains(RESET_COMMAND));
        assert_eq!(migration_ledger(&pool).await, ledger_before);
        assert_eq!(
            epoch::read(&pool).await.expect("marker").migration_head,
            "0010"
        );
        pool.close().await;
    });
}

/// The two source-admissibility routines as `(name, language, settings)`.
async fn source_admissibility_routines(pool: &PgPool) -> Vec<(String, String, Vec<String>)> {
    sqlx::query_as(
        "select p.proname::text, l.lanname::text, coalesce(p.proconfig, '{}')::text[] \
         from pg_proc p join pg_language l on l.oid = p.prolang \
         join pg_namespace n on n.oid = p.pronamespace \
         where n.nspname = 'public' \
           and p.proname in ('synveda_source_admissible', 'synveda_source_available') \
         order by 1",
    )
    .fetch_all(pool)
    .await
    .expect("read the source-admissibility routines")
}

/// 0011 left both routines as SQL functions, which PostgreSQL before 18
/// re-plans on every call; 0012 keeps their bodies under plpgsql with a
/// generic plan.
fn source_admissibility_at(head: &str) -> Vec<(String, String, Vec<String>)> {
    let (language, settings): (&str, &[&str]) = match head {
        "0011" => ("sql", &["search_path=public, pg_temp"]),
        _ => (
            "plpgsql",
            &[
                "search_path=public, pg_temp",
                "plan_cache_mode=force_generic_plan",
            ],
        ),
    };
    ["synveda_source_admissible", "synveda_source_available"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                language.to_owned(),
                settings
                    .iter()
                    .map(|setting| (*setting).to_owned())
                    .collect(),
            )
        })
        .collect()
}

/// A retained database at the published convergence head takes every later
/// forward migration: its ledger prefix, marker provenance and rows survive,
/// and the result serves the current catalogue.
#[test]
fn a_published_convergence_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..11].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0011 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0011 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 11);
        assert_eq!(
            source_admissibility_routines(&pool).await,
            source_admissibility_at("0011")
        );
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0011 head is upgradeable without writes")
                .migration_head,
            "0011"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { .. })
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..11], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        for (row, migration) in ledger[11..]
            .iter()
            .zip(&synveda_store::MIGRATOR.migrations[11..])
        {
            assert_eq!(
                (row.0, row.4.as_slice()),
                (migration.version, migration.checksum.as_ref())
            );
        }
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert_eq!(metadata.created_by_version, "0.4.7");
        assert_eq!(
            source_admissibility_routines(&pool).await,
            source_admissibility_at(epoch::CURRENT_MIGRATION_HEAD)
        );
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("the current published lineage reruns idempotently");
        assert_eq!(migration_ledger(&pool).await, ledger);
        pool.close().await;
    });
}

/// The source-admissibility head is pending, not current: both lineages
/// advance from it through the 0013 forward migration, keep their ledger
/// prefix and rows, and serve the current catalogue.
#[test]
fn a_published_admissibility_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..12].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0012 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0012 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 12);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0012 head is upgradeable without writes")
                .migration_head,
            "0012"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0012"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..12], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

#[test]
fn a_portable_admissibility_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..2].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable 0012 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable 0012 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 2);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable 0012 head is upgradeable without writes")
                .migration_head,
            "0012"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0012"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..2], &ledger_before[..]);
        assert_eq!(
            ledger.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// The session-revocation head is pending, not current: both lineages
/// advance from it through the 0014 forward migration, keep their ledger
/// prefix and rows, and serve the current catalogue.
#[test]
fn a_published_revocation_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..13].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0013 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0013 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 13);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0013 head is upgradeable without writes")
                .migration_head,
            "0013"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0013"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..13], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

#[test]
fn a_portable_revocation_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..3].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable 0013 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable 0013 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 3);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable 0013 head is upgradeable without writes")
                .migration_head,
            "0013"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0013"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..3], &ledger_before[..]);
        assert_eq!(
            ledger.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// The restorable-foreign-key head is pending, not current: both lineages
/// advance from it through the 0015 forward migration, keep their ledger
/// prefix and rows, and serve the current catalogue.
#[test]
fn a_published_restorable_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..14].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0014 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0014 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 14);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0014 head is upgradeable without writes")
                .migration_head,
            "0014"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0014"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..14], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

#[test]
fn a_portable_restorable_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..4].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable 0014 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable 0014 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 4);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable 0014 head is upgradeable without writes")
                .migration_head,
            "0014"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0014"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..4], &ledger_before[..]);
        assert_eq!(
            ledger.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// The tenant-erasure head is pending, not current: both lineages advance
/// from it through the later forward migrations, keep their ledger prefix and
/// rows, and serve the current catalogue.
#[test]
fn a_published_erasure_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..15].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0015 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0015 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 15);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0015 head is upgradeable without writes")
                .migration_head,
            "0015"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0015"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..15], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

#[test]
fn a_portable_erasure_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..5].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable 0015 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable 0015 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 5);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable 0015 head is upgradeable without writes")
                .migration_head,
            "0015"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0015"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..5], &ledger_before[..]);
        assert_eq!(
            ledger.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// The Capture retry head is pending, not current: both lineages advance from
/// it through the 0017 tenant-usage migration, keep their ledger prefix and
/// rows, and serve the current catalogue.
#[test]
fn a_published_capture_retry_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("published lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..16].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0016 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the published 0016 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 16);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the published 0016 head is upgradeable without writes")
                .migration_head,
            "0016"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0016"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the published lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..16], &ledger_before[..]);
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

#[test]
fn a_portable_capture_retry_head_advances_to_the_current_head() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..6].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable 0016 head");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable 0016 head");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(ledger_before.len(), 6);
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable 0016 head is upgradeable without writes")
                .migration_head,
            "0016"
        );
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0016"
        ));

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        assert_eq!(&ledger[..6], &ledger_before[..]);
        assert_eq!(
            ledger.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current head matches the embedded catalogue");
        pool.close().await;
    });
}

/// A portable installation stamped at its baseline continues on the portable
/// lineage through the byte-identical forward copy.
#[test]
fn a_portable_baseline_advances_through_its_forward_copy() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..1].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable baseline");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable baseline");
        drop(connection);
        let tenant = admit_a_tenant(&pool).await;
        assert_eq!(migration_ledger(&pool).await.len(), 1);
        assert_eq!(
            source_admissibility_routines(&pool).await,
            source_admissibility_at("0011")
        );
        assert_eq!(
            synveda_store::check_migration_compatibility(&pool, &scratch.roles)
                .await
                .expect("the portable baseline is upgradeable without writes")
                .migration_head,
            "0011"
        );

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("advance the portable lineage");
        let ledger = migration_ledger(&pool).await;
        let portable = &synveda_store::PORTABLE_MIGRATOR.migrations;
        assert_eq!(ledger.len(), portable.len());
        for (row, migration) in ledger.iter().zip(portable.iter()) {
            assert_eq!(
                (row.0, row.4.as_slice()),
                (migration.version, migration.checksum.as_ref())
            );
        }
        let metadata = epoch::verify(&pool).await.expect("the current head serves");
        assert_eq!(metadata.migration_head, epoch::CURRENT_MIGRATION_HEAD);
        assert_eq!(metadata.created_by_version, "0.4.7");
        assert_eq!(
            source_admissibility_routines(&pool).await,
            source_admissibility_at(epoch::CURRENT_MIGRATION_HEAD)
        );
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect("the current portable head matches the embedded catalogue");
        pool.close().await;
    });
}

/// A fresh install interrupted after the portable baseline committed, before
/// any marker row, resumes through the forward copy and stamps the head.
#[test]
fn an_unstamped_portable_baseline_is_recovered_through_its_forward_copy() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..1].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("commit the portable baseline alone");
        drop(connection);
        epoch::preflight(&pool)
            .await
            .expect("the exact pending baseline is recoverable");
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("an unstamped baseline is not a completed image transition");

        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("resume the portable lineage and stamp it");
        assert_eq!(
            migration_ledger(&pool).await.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        assert_eq!(
            epoch::verify(&pool)
                .await
                .expect("recovered head")
                .migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        pool.close().await;
    });
}

/// The forward copy committed on the portable lineage but its stamp did not:
/// the marker trails at the baseline head and only the stamp is repaired.
#[test]
fn a_portable_forward_copy_with_the_baseline_stamp_repairs_only_the_stamp() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let mut connection = pool.acquire().await.expect("portable lineage connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                synveda_store::PORTABLE_MIGRATOR.migrations[..1].to_vec(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the portable baseline");
        epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .expect("stamp the portable baseline");
        synveda_store::PORTABLE_MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit the forward copy without its stamp");
        drop(connection);
        let ledger_before = migration_ledger(&pool).await;
        assert_eq!(
            ledger_before.len(),
            synveda_store::PORTABLE_MIGRATOR.migrations.len()
        );
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0011");
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
        pool.close().await;
    });
}

async fn public_view_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace \
         where n.nspname = 'public' and c.relkind in ('v', 'm')",
    )
    .fetch_one(pool)
    .await
    .expect("count public views")
}

async fn retained_knowledge(
    pool: &PgPool,
    tenant: synveda_types::TenantId,
) -> (
    synveda_types::KnowledgeItemId,
    [synveda_types::KnowledgeRevisionId; 2],
) {
    use synveda_store::knowledge::{
        self, NewKnowledgeItem, NewKnowledgeRevision, NewKnowledgeSource,
    };
    use synveda_types::knowledge::{
        KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
    };
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin retained Knowledge fixture");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure tenant root");
    let source = NewKnowledgeSource {
        id: synveda_types::KnowledgeSourceId::new(),
        tenant_id: tenant,
        scope_id: root.id,
        source_type: KnowledgeSourceType::Manual,
        session_event_id: None,
        import_artifact_id: None,
        locator: None,
        source_revision: None,
        content_hash: None,
        metadata: serde_json::json!({"method": "retained_epoch_fixture"}),
        created_by: Some("test:epoch".to_owned()),
    };
    knowledge::create_source(&mut tx, &source)
        .await
        .expect("create retained source");
    let item = NewKnowledgeItem {
        id: synveda_types::KnowledgeItemId::new(),
        tenant_id: tenant,
        scope_id: root.id,
        project_id: None,
        owner_principal_id: None,
        knowledge_type: KnowledgeType::Convention,
        origin: KnowledgeOrigin::Authored,
        created_by: Some("test:epoch".to_owned()),
    };
    let revision = |title: &str| NewKnowledgeRevision {
        id: synveda_types::KnowledgeRevisionId::new(),
        content: KnowledgeRevisionContent {
            title: title.to_owned(),
            body_markdown: format!("{title} body."),
            summary: title.to_owned(),
            tags: vec!["retained".to_owned()],
            sensitivity: synveda_types::Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: chrono::Utc::now() - chrono::Duration::days(1),
            valid_to: None,
            stale_after: None,
            verification_metadata: serde_json::json!({"method": "reviewed"}),
            metadata: serde_json::json!({}),
        },
        created_by: Some("test:epoch".to_owned()),
    };
    let first = revision("Retained first");
    let second = revision("Retained second");
    knowledge::create_item(&mut tx, &item, &first, &[source.id])
        .await
        .expect("create retained Knowledge");
    knowledge::append_revision(&mut tx, tenant, item.id, first.id, &second, &[source.id])
        .await
        .expect("append retained revision")
        .expect("retained item exists");
    tx.commit().await.expect("commit retained Knowledge");
    (item.id, [first.id, second.id])
}

async fn read_current(
    pool: &PgPool,
    tenant: synveda_types::TenantId,
    item: synveda_types::KnowledgeItemId,
) -> synveda_store::knowledge::KnowledgeSnapshot {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin retained current read");
    let current = synveda_store::knowledge::current(&mut *tx, tenant, item)
        .await
        .expect("read retained current Knowledge")
        .expect("retained Knowledge is visible");
    tx.rollback().await.expect("finish retained current read");
    current
}

/// Installs the published v0.4.6 head (`0010`) with its own stamp, then a
/// tenant whose rows must survive every refusal below.
async fn install_published_v046_head(pool: &PgPool) -> synveda_types::TenantId {
    let mut connection = pool.acquire().await.expect("published lineage connection");
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..10].to_vec()),
        ..sqlx::migrate::Migrator::DEFAULT
    }
    .run(&mut *connection)
    .await
    .expect("install the published 0010 head");
    epoch::stamp_connection(&mut connection, "0.4.6")
        .await
        .expect("stamp the published 0010 head");
    drop(connection);
    admit_a_tenant(pool).await
}

fn short_bounds(boundary_lock: u64, ddl_lock: u64, writer_drain: u64) -> MigrationBounds {
    MigrationBounds::new(
        std::time::Duration::from_secs(boundary_lock),
        std::time::Duration::from_secs(ddl_lock),
        std::time::Duration::from_secs(writer_drain),
    )
    .expect("test migration bounds")
}

async fn runtime_connection(scratch: &Scratch, setting: &str) -> sqlx::PgConnection {
    let url = private_database_url(setting).expect("ordinary lifecycle runtime credential");
    sqlx::PgConnection::connect(&replace_database(&url, &scratch.name))
        .await
        .expect("connect as an ordinary runtime login")
}

type LedgerRows = Vec<(
    i64,
    String,
    chrono::DateTime<chrono::Utc>,
    bool,
    Vec<u8>,
    i64,
)>;

async fn assert_published_v046_head_unchanged(
    pool: &PgPool,
    marker: &epoch::SchemaMetadata,
    ledger: &LedgerRows,
) {
    assert_eq!(&epoch::read(pool).await.expect("unchanged marker"), marker);
    assert_eq!(&migration_ledger(pool).await, ledger);
    assert_eq!(public_view_count(pool).await, 2, "0011 must not have run");
}

/// A still-connected writer from the previous release refuses DDL; the
/// database stays at its published head until every such session leaves.
#[test]
fn a_connected_previous_release_writer_holds_the_upgrade_before_ddl() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let tenant = install_published_v046_head(&pool).await;
        let marker = epoch::read(&pool).await.expect("published marker");
        let ledger = migration_ledger(&pool).await;
        let gateway =
            runtime_connection(&scratch, "SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE").await;

        let started = std::time::Instant::now();
        let error = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 2))
            .await
            .expect_err("a connected gateway session refuses the schema change");
        let message = error.to_string();
        assert!(
            message.contains("migration phase writer-drain"),
            "{message}"
        );
        assert!(message.contains("1 gateway or worker"), "{message}");
        assert!(!message.contains(RESET_COMMAND), "{message}");
        assert!(started.elapsed() >= std::time::Duration::from_secs(2));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_published_v046_head_unchanged(&pool, &marker, &ledger).await;

        gateway.close().await.expect("stop the previous gateway");
        let mut late_writer =
            runtime_connection(&scratch, "SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE").await;
        sqlx::query("select 1")
            .execute(&mut late_writer)
            .await
            .expect("the late writer is live");
        let draining = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            late_writer.close().await.expect("the late writer stops");
        });
        let report = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 20))
            .await
            .expect("the upgrade proceeds once writers leave");
        draining.await.expect("late writer task");
        assert_eq!(report.starting_head.as_deref(), Some("0010"));
        assert_eq!(
            report.metadata.migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        assert_eq!(report.metadata.created_by_version, "0.4.6");
        assert!(report.writer_drain >= std::time::Duration::from_secs(1));
        assert!(report.total >= report.writer_drain + report.schema_change);
        assert_eq!(&migration_ledger(&pool).await[..10], &ledger[..]);
        assert_eq!(public_view_count(&pool).await, 0);
        assert!(
            synveda_store::tenants::by_id(&pool, tenant)
                .await
                .expect("preserved tenant")
                .is_some()
        );
        pool.close().await;
    });
}

/// Every start reruns the migrator beside the running gateway; an unchanged
/// current head must not wait for it.
#[test]
fn an_unchanged_current_head_reruns_beside_live_writers() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect("fresh installation");
        let ledger = migration_ledger(&pool).await;
        let gateway =
            runtime_connection(&scratch, "SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE").await;
        let report = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 1))
            .await
            .expect("an unchanged head reruns beside a live gateway");
        assert_eq!(report.writer_drain, std::time::Duration::ZERO);
        assert_eq!(
            report.starting_head.as_deref(),
            Some(epoch::CURRENT_MIGRATION_HEAD)
        );
        assert_eq!(migration_ledger(&pool).await, ledger);
        gateway.close().await.expect("close gateway session");
        pool.close().await;
    });
}

/// A competing migrator's boundary and a reader's table lock each end
/// server-side within their bounds, leave the published head intact and let
/// the same command retry to the current head.
#[test]
fn competing_boundary_and_table_locks_fail_within_bounds_and_retry() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        use sqlx::migrate::Migrate;
        let pool = connect_pool(&scratch.options).await;
        install_published_v046_head(&pool).await;
        let marker = epoch::read(&pool).await.expect("published marker");
        let ledger = migration_ledger(&pool).await;

        let mut competitor = scratch.options.connect().await.expect("competing migrator");
        competitor
            .lock()
            .await
            .expect("hold the migration boundary");
        let started = std::time::Instant::now();
        let error = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(1, 5, 5))
            .await
            .expect_err("a held boundary ends the wait");
        let message = error.to_string();
        assert!(
            message.contains("migration phase boundary-lock"),
            "{message}"
        );
        assert!(message.contains("SQLSTATE 55P03"), "{message}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_published_v046_head_unchanged(&pool, &marker, &ledger).await;
        competitor.unlock().await.expect("release the boundary");
        competitor.close().await.expect("close competitor");

        let mut reader = scratch.options.connect().await.expect("long reader");
        let mut transaction = reader.begin().await.expect("begin long read");
        sqlx::query("lock table public.scopes in access share mode")
            .execute(&mut *transaction)
            .await
            .expect("hold a table the convergence migration alters");
        let started = std::time::Instant::now();
        let error = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 1, 5))
            .await
            .expect_err("a held table lock ends the schema change");
        let message = error.to_string();
        assert!(
            message.contains("migration phase schema-change"),
            "{message}"
        );
        assert!(message.contains("migration 0011"), "{message}");
        assert!(message.contains("SQLSTATE 55P03"), "{message}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_published_v046_head_unchanged(&pool, &marker, &ledger).await;
        transaction.rollback().await.expect("release the reader");
        reader.close().await.expect("close reader");

        let report = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 5))
            .await
            .expect("the same command retries to the current head");
        assert_eq!(
            report.metadata.migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        assert_eq!(&migration_ledger(&pool).await[..10], &ledger[..]);
        pool.close().await;
    });
}

/// After 0011 and 0012 commit but before the stamp, the marker still names
/// the published head, so a previous-release writer would accept it. The
/// candidate refuses to serve or switch images, and the resumed migration
/// repairs only the stamp, after the writer leaves.
#[test]
fn committed_published_upgrade_ddl_without_its_stamp_resumes_only_after_writers_leave() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        let tenant = install_published_v046_head(&pool).await;
        let (item, _) = retained_knowledge(&pool, tenant).await;
        let current_before = read_current(&pool, tenant, item).await;
        let mut connection = pool
            .acquire()
            .await
            .expect("interrupted upgrade connection");
        synveda_store::MIGRATOR
            .run(&mut *connection)
            .await
            .expect("commit 0011 and 0012 without the stamp");
        drop(connection);
        let ledger = migration_ledger(&pool).await;
        assert_eq!(ledger.len(), synveda_store::MIGRATOR.migrations.len());
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0010");
        assert!(matches!(
            epoch::verify(&pool).await,
            Err(SchemaEpochError::PendingMigration { ref found }) if found == "0010"
        ));
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("a trailing stamp is not a completed image transition");

        let previous_writer =
            runtime_connection(&scratch, "SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE").await;
        let error = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 1))
            .await
            .expect_err("a connected writer also blocks the stamp repair");
        assert!(error.to_string().contains("writer-drain"), "{error}");
        assert_eq!(epoch::read(&pool).await.unwrap().migration_head, "0010");
        previous_writer
            .close()
            .await
            .expect("stop the previous writer");

        let report = synveda_store::migrate_bounded(&pool, &scratch.roles, short_bounds(5, 5, 5))
            .await
            .expect("repair only the stamp");
        assert_eq!(report.starting_head.as_deref(), Some("0010"));
        assert_eq!(
            report.metadata.migration_head,
            epoch::CURRENT_MIGRATION_HEAD
        );
        assert_eq!(migration_ledger(&pool).await, ledger);
        assert_eq!(read_current(&pool, tenant, item).await, current_before);
        pool.close().await;
    });
}

/// A published ledger cannot be stamped ahead of its committed DDL: a marker
/// naming the current head over the 0010 ledger is drift, refused unchanged.
#[test]
fn a_published_stamp_ahead_of_its_ledger_is_refused_without_writes() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    scratch.block_on(async {
        let pool = connect_pool(&scratch.options).await;
        install_published_v046_head(&pool).await;
        sqlx::query("update schema_metadata set migration_head = $1")
            .bind(epoch::CURRENT_MIGRATION_HEAD)
            .execute(&pool)
            .await
            .expect("drift this disposable marker ahead");
        let marker = epoch::read(&pool).await.expect("drifted marker");
        let ledger = migration_ledger(&pool).await;
        let error = synveda_store::migrate(&pool, &scratch.roles)
            .await
            .expect_err("a stamp ahead of committed DDL is refused");
        let message = error.to_string();
        assert!(message.contains("migration phase preflight"), "{message}");
        assert!(message.contains("immutable migration prefix"), "{message}");
        assert!(!message.contains(RESET_COMMAND), "{message}");
        synveda_store::check_migration_compatibility(&pool, &scratch.roles)
            .await
            .expect_err("the candidate refuses the drifted marker");
        assert_eq!(epoch::read(&pool).await.unwrap(), marker);
        assert_eq!(migration_ledger(&pool).await, ledger);
        assert_eq!(public_view_count(&pool).await, 2);
        pool.close().await;
    });
}
