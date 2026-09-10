//! Live database-authority and tenant-convergence acceptance for CPR-36/CPR-45.
//!
//! This is test-only because deployment preflight and long-running processes
//! use the production sentinels in synveda-store and the canonical Compose
//! lifecycle. The fixture proves those roles and convergence paths against an
//! isolated PostgreSQL cluster.

use std::path::Path;
use std::time::Duration;

use synveda_types::{TenantId, TenantStatus};

#[cfg(test)]
async fn verify_runtime_role(
    pool: &sqlx::PgPool,
    role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
) -> Result<(), String> {
    synveda_store::runtime_role::verify_capability_role(pool, database_roles)
        .await
        .map_err(|error| format!("verify synveda_app capability role: {error}"))?;
    let facts = sqlx::query!(
        r#"select rolcanlogin as "can_login!", rolinherit as "inherits!",
                  rolsuper as "superuser!", rolcreatedb as "create_db!",
                  rolcreaterole as "create_role!", rolreplication as "replication!",
                  rolbypassrls as "bypass_rls!",
                  pg_has_role($1, 'synveda_app', 'member') as "app_member!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and not membership.admin_option
                       and membership.inherit_option
                       and membership.set_option
                  ) as "app_membership_safe!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and (
                         membership.admin_option
                         or not membership.inherit_option
                         or not membership.set_option
                       )
                  ) as "app_membership_unsafe!",
                  exists (
                    select 1 from pg_catalog.pg_database
                     where datdba = roles.oid
                  ) as "database_owner!",
                  exists (
                    select 1 from pg_catalog.pg_namespace
                     where nspowner = roles.oid
                  ) as "schema_owner!",
                  exists (
                    select 1 from pg_catalog.pg_class as objects
                     where objects.relowner = roles.oid
                    union all
                    select 1 from pg_catalog.pg_proc as routines
                     where routines.proowner = roles.oid
                    union all
                    select 1 from pg_catalog.pg_type as data_type
                     where data_type.typowner = roles.oid
                  ) as "application_object_owner!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as memberships
                      join pg_catalog.pg_roles as granted_roles
                        on granted_roles.oid = memberships.roleid
                     where memberships.member = roles.oid
                       and granted_roles.rolname <> 'synveda_app'
                  ) as "unexpected_membership!",
                  exists (
                    select 1 from pg_catalog.pg_auth_members as memberships
                     where memberships.roleid = roles.oid
                       and (
                         not memberships.admin_option
                         or memberships.inherit_option
                         or memberships.set_option
                       )
                  ) as "unsafe_inbound_membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = roles.oid
                       and acl.privilege_type = 'CONNECT'
                       and not acl.is_grantable
                  ) as "direct_connect!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = roles.oid
                       and (
                         acl.privilege_type <> 'CONNECT'
                         or acl.is_grantable
                       )
                  ) as "unsafe_direct_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = 0
                  ) as "public_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname <> current_database()
                       and acl.grantee = roles.oid
                  ) as "other_database_acl!"
             from pg_catalog.pg_roles as roles
            where rolname = $1"#,
        role,
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("verify runtime database login {role}: {error}"))?
    .ok_or_else(|| format!("runtime database login {role} is not provisioned"))?;
    if !facts.can_login
        || !facts.inherits
        || facts.superuser
        || facts.create_db
        || facts.create_role
        || facts.replication
        || facts.bypass_rls
        || !facts.app_member
        || !facts.app_membership_safe
        || facts.app_membership_unsafe
        || facts.database_owner
        || facts.schema_owner
        || facts.application_object_owner
        || facts.unexpected_membership
        || facts.unsafe_inbound_membership
        || !facts.direct_connect
        || facts.unsafe_direct_database_acl
        || facts.public_database_acl
        || facts.other_database_acl
    {
        return Err(format!(
            "runtime database login {role} is not an exact inheriting, non-elevated synveda_app login with only direct CONNECT on the selected database, no PUBLIC or other-database ACL, no privilege-bearing inbound membership, and no database, schema, relation, routine or type ownership"
        ));
    }
    Ok(())
}

/// Connects the resolved runtime DSN and proves the session, role, schema and
/// target it will actually give the process. Looking up a same-named role
/// through the bootstrap pool is insufficient when an explicitly configured
/// runtime URL may name another host or database.
async fn verify_runtime_database_url(
    setting: &str,
    runtime_url: &str,
    role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
) -> Result<synveda_store::runtime_role::DatabaseIdentity, String> {
    let connect_options = synveda_store::database_url::parse(setting, runtime_url)
        .map_err(|_| format!("{setting} is not a valid PostgreSQL URL"))?;
    tokio::time::timeout(Duration::from_secs(15), async move {
        let runtime = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(connect_options)
            .await
            .map_err(|_| {
                format!(
                    "connect through {setting} ({}) failed",
                    crate::settings::redacted_database_url(runtime_url)
                )
            })?;
        let result = async {
            synveda_store::epoch::verify(&runtime)
                .await
                .map_err(|error| format!("{setting} schema check failed: {error}"))?;
            synveda_store::runtime_role::verify(&runtime, role, database_roles)
                .await
                .map_err(|error| format!("{setting} runtime-role check failed: {error}"))?;
            synveda_store::runtime_role::database_identity(&runtime)
                .await
                .map_err(|error| format!("{setting} database-target check failed: {error}"))
        }
        .await;
        runtime.close().await;
        result
    })
    .await
    .map_err(|_| format!("{setting} verification exceeded the 15-second startup bound"))?
}

fn require_one_database_target(
    bootstrap: &synveda_store::runtime_role::DatabaseIdentity,
    gateway: &synveda_store::runtime_role::DatabaseIdentity,
    worker: &synveda_store::runtime_role::DatabaseIdentity,
) -> Result<(), String> {
    if bootstrap != gateway || bootstrap != worker {
        return Err("DATABASE_URL, SYNVEDA_GATEWAY_DATABASE_URL and \
             SYNVEDA_WORKER_DATABASE_URL must target one live PostgreSQL primary instance \
             and database"
            .to_owned());
    }
    Ok(())
}

fn private_test_database_url(setting: &str) -> Option<String> {
    let path = std::env::var_os(setting)?;
    Some(
        crate::settings::read_setting_file(setting, Path::new(&path))
            .expect("read isolated test database URL"),
    )
}

async fn tenant_admission_footprint(pool: &sqlx::PgPool, tenant_id: TenantId) -> [i64; 4] {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("open tenant-scoped footprint transaction");
    let row = sqlx::query!(
        r#"
            select (select count(*) from public.tenants
                     where id = $1) as "tenants!",
                   (select count(*) from public.scopes
                     where tenant_id = $1) as "scopes!",
                   (select count(*) from public.scope_grants
                     where tenant_id = $1) as "grants!",
                   (select count(*) from public.audit_log
                     where tenant_id = $1) as "audits!"
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("measure tenant-admission footprint");
    tx.rollback()
        .await
        .expect("roll back footprint transaction");
    [row.tenants, row.scopes, row.grants, row.audits]
}

async fn tenant_key_provision_events(
    pool: &sqlx::PgPool,
    tenant_id: TenantId,
) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("open tenant-scoped audit transaction");
    let events = synveda_audit::search(
        &mut tx,
        tenant_id,
        &synveda_audit::EventFilter {
            actions: vec![synveda_audit::AuditAction::TenantKeyProvisioned],
            resource: Some(format!("tenant {tenant_id} key")),
            ..synveda_audit::EventFilter::default()
        },
        0,
        10,
    )
    .await
    .expect("read tenant key-provision evidence")
    .items;
    tx.rollback()
        .await
        .expect("roll back audit inspection transaction");
    events
}

async fn append_fixture_key_provision_event(
    pool: &sqlx::PgPool,
    tenant_id: TenantId,
    payload: serde_json::Value,
) {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("open tenant-scoped fixture audit transaction");
    crate::record_break_glass(
        &mut tx,
        tenant_id,
        synveda_audit::AuditAction::TenantKeyProvisioned,
        format!("tenant {tenant_id} key"),
        payload,
    )
    .await
    .expect("append fixture key-provision event");
    tx.commit().await.expect("commit fixture audit event");
}

struct TestSchemaEpoch {
    epoch: i32,
    updated_at: chrono::DateTime<chrono::Utc>,
}

async fn test_schema_epoch(pool: &sqlx::PgPool) -> TestSchemaEpoch {
    let row = sqlx::query!(
        r#"
            select epoch as "epoch!",
                   updated_at as "updated_at!"
              from public.schema_metadata
             where id
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("snapshot test schema epoch");
    TestSchemaEpoch {
        epoch: row.epoch,
        updated_at: row.updated_at,
    }
}

async fn set_test_schema_epoch(pool: &sqlx::PgPool, state: &TestSchemaEpoch) {
    sqlx::query!(
        r#"
            update public.schema_metadata
               set epoch = $1,
                   updated_at = $2
             where id
        "#,
        state.epoch,
        state.updated_at,
    )
    .execute(pool)
    .await
    .expect("set test schema epoch");
}

#[test]
fn runtime_targets_cannot_split_one_deployment_across_databases() {
    let bootstrap = synveda_store::runtime_role::DatabaseIdentity {
        database: "synveda".to_owned(),
        cluster_system_identifier: "cluster-1".to_owned(),
        database_oid: 16_384,
        postmaster_started_at: chrono::DateTime::UNIX_EPOCH,
    };
    assert!(
        require_one_database_target(&bootstrap, &bootstrap, &bootstrap).is_ok(),
        "three credentials for one target are accepted"
    );

    let another_database = synveda_store::runtime_role::DatabaseIdentity {
        database: "another".to_owned(),
        ..bootstrap.clone()
    };
    assert!(
        require_one_database_target(&bootstrap, &bootstrap, &another_database).is_err(),
        "a worker database split is refused"
    );

    let another_server = synveda_store::runtime_role::DatabaseIdentity {
        cluster_system_identifier: "cluster-2".to_owned(),
        ..bootstrap.clone()
    };
    assert!(
        require_one_database_target(&bootstrap, &another_server, &bootstrap).is_err(),
        "a gateway server split is refused"
    );

    let another_primary_generation = synveda_store::runtime_role::DatabaseIdentity {
        postmaster_started_at: bootstrap.postmaster_started_at + chrono::Duration::seconds(1),
        ..bootstrap.clone()
    };
    assert!(
        require_one_database_target(&bootstrap, &bootstrap, &another_primary_generation).is_err(),
        "a separately started writable fork is refused"
    );
}

/// Both deployment logins are subject to forced RLS and can see a tenant
/// only after the application establishes its transaction-local tenant
/// context. This is the executable boundary between bootstrap authority
/// and the one normal gateway runtime (CPR-36, ADR-0095).
#[tokio::test]
async fn compose_runtime_logins_are_distinct_and_rls_enforced() {
    let Some(admin_url) = private_test_database_url("SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE") else {
        eprintln!(
            "SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE not set; skipping CPR-36 database acceptance"
        );
        return;
    };
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .expect("connect as deployment owner");
    let database_roles =
        crate::settings::database_roles().expect("read the exact test role contract");
    verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
        .await
        .expect("accept the least-privilege gateway login");
    verify_runtime_role(&admin, database_roles.worker(), &database_roles)
        .await
        .expect("accept the least-privilege worker login");
    assert!(
        verify_runtime_role(&admin, database_roles.migrator(), &database_roles)
            .await
            .is_err(),
        "the bootstrap owner must never pass runtime-role validation",
    );

    let migrator_url = private_test_database_url("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE")
        .expect("test harness migrator URL file");
    let migrator = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&migrator_url)
        .await
        .expect("connect as synveda_migrator");
    let runtime_url = private_test_database_url("SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE")
        .expect("test harness gateway URL file");
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&runtime_url)
        .await
        .expect("connect as synveda_gateway");

    let refused_id = TenantId::new();
    let refused_suffix = refused_id.to_string();
    let refused_slug = format!(
        "cpr36-refused-{}",
        &refused_suffix[refused_suffix.len() - 12..]
    );
    assert_eq!(
        tenant_admission_footprint(&runtime, refused_id).await,
        [0; 4]
    );
    assert!(
        crate::create_tenant_with_admission_id(
            &admin,
            &database_roles,
            refused_id,
            &refused_slug,
            "CPR-36 refused owner admission",
            TenantStatus::Active,
            crate::TenantAdmission::Standard,
        )
        .await
        .is_err(),
        "the deployment owner must not admit a tenant"
    );
    assert_eq!(
        tenant_admission_footprint(&runtime, refused_id).await,
        [0; 4],
        "refused owner admission wrote tenant, scope, grant or audit state"
    );

    let wrong_role_id = TenantId::new();
    let wrong_role_suffix = wrong_role_id.to_string();
    assert!(
        crate::create_tenant_with_admission_id(
            &runtime,
            &database_roles,
            wrong_role_id,
            &format!(
                "cpr45-wrong-role-{}",
                &wrong_role_suffix[wrong_role_suffix.len() - 12..]
            ),
            "CPR-45 refused gateway admission",
            TenantStatus::Active,
            crate::TenantAdmission::Standard,
        )
        .await
        .is_err(),
        "the gateway login must not admit a tenant"
    );
    assert_eq!(
        tenant_admission_footprint(&runtime, wrong_role_id).await,
        [0; 4],
        "wrong-role admission wrote tenant, scope, grant or audit state"
    );

    let wrong_epoch_id = TenantId::new();
    let wrong_epoch_suffix = wrong_epoch_id.to_string();
    let original_epoch = test_schema_epoch(&admin).await;
    let future_epoch = TestSchemaEpoch {
        epoch: synveda_store::epoch::CURRENT_EPOCH + 1,
        updated_at: chrono::Utc::now(),
    };
    set_test_schema_epoch(&admin, &future_epoch).await;
    let wrong_epoch_result = crate::create_tenant_with_admission_id(
        &migrator,
        &database_roles,
        wrong_epoch_id,
        &format!(
            "cpr45-wrong-epoch-{}",
            &wrong_epoch_suffix[wrong_epoch_suffix.len() - 12..]
        ),
        "CPR-45 refused future-epoch admission",
        TenantStatus::Active,
        crate::TenantAdmission::Standard,
    )
    .await;
    // This test runs only through db-test.sh's disposable fixture. Restore
    // the exact marker before inspecting the refusal; on abnormal process
    // termination the harness retains the whole failed fixture as evidence.
    set_test_schema_epoch(&admin, &original_epoch).await;
    assert!(
        wrong_epoch_result.is_err(),
        "a future schema epoch must refuse tenant admission"
    );
    assert_eq!(
        tenant_admission_footprint(&runtime, wrong_epoch_id).await,
        [0; 4],
        "wrong-epoch admission wrote tenant, scope, grant or audit state"
    );

    let converged_id = TenantId::new();
    let converged_suffix = converged_id.to_string();
    let converged_slug = format!(
        "cpr45-converged-{}",
        &converged_suffix[converged_suffix.len() - 12..]
    );
    let first = crate::converge_tenant(
        &migrator,
        &database_roles,
        converged_id,
        &converged_slug,
        "CPR-45 converged tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit the exact deployment-owned tenant");
    let second = crate::converge_tenant(
        &migrator,
        &database_roles,
        converged_id,
        &converged_slug,
        "CPR-45 converged tenant",
        TenantStatus::Active,
    )
    .await
    .expect("repeat exact tenant convergence");
    assert_eq!(first, second, "a rerun must resolve the same tenant row");
    assert_eq!(
        tenant_admission_footprint(&runtime, converged_id).await,
        [1, 0, 0, 1],
        "a rerun must not duplicate tenant or audit state"
    );
    assert!(
        crate::converge_tenant(
            &migrator,
            &database_roles,
            converged_id,
            &converged_slug,
            "CPR-45 conflicting tenant",
            TenantStatus::Active,
        )
        .await
        .is_err(),
        "the same UUID with changed immutable admission fields must fail"
    );
    assert_eq!(
        tenant_admission_footprint(&runtime, converged_id).await,
        [1, 0, 0, 1],
        "a conflicting rerun must not change admitted state"
    );

    let local_kms = synveda_crypto::LocalKms::from_hex(
        &"45".repeat(32),
        "local:cpr45-convergence-test".to_owned(),
    )
    .expect("construct fixture KMS");
    let ring = synveda_store::keys::KeyRing::new(synveda_crypto::Kms::Local(local_kms));
    let key_scope = synveda_crypto::KeyScope::Tenant(converged_id);

    // The key commit can win immediately before the process disappears.
    // A rerun must repair the missing evidence rather than suppressing it
    // merely because the key row now exists.
    ring.provision(&migrator, key_scope)
        .await
        .expect("simulate committed key before audit");
    assert!(
        tenant_key_provision_events(&runtime, converged_id)
            .await
            .is_empty(),
        "the crash fixture must begin with the key/evidence gap"
    );
    let wrong_local_kms = synveda_crypto::LocalKms::from_hex(
        &"46".repeat(32),
        "local:cpr45-convergence-test".to_owned(),
    )
    .expect("construct wrong fixture KMS");
    for refusing_ring in [
        synveda_store::keys::KeyRing::new(synveda_crypto::Kms::Disabled),
        synveda_store::keys::KeyRing::new(synveda_crypto::Kms::Local(wrong_local_kms)),
    ] {
        assert!(
            crate::keys::provision_quietly_with_ring(&migrator, converged_id, &refusing_ring,)
                .await
                .is_err(),
            "an existing wrapped row must not substitute for KMS custody"
        );
        assert!(
            tenant_key_provision_events(&runtime, converged_id)
                .await
                .is_empty(),
            "failed custody proof must not append success evidence"
        );
    }
    assert_eq!(
        crate::keys::provision_quietly_with_ring(&migrator, converged_id, &ring)
            .await
            .expect("repair key-provision evidence"),
        1
    );
    assert_eq!(
        crate::keys::provision_quietly_with_ring(&migrator, converged_id, &ring)
            .await
            .expect("repeat exact key convergence"),
        1
    );
    let provisioned = tenant_key_provision_events(&runtime, converged_id).await;
    assert_eq!(provisioned.len(), 1, "a rerun must retain one witness");
    assert_eq!(
        provisioned[0].payload,
        serde_json::json!({
            "version": 1,
            "kek_ref": "local:cpr45-convergence-test",
        })
    );

    ring.rotate(&migrator, key_scope)
        .await
        .expect("rotate fixture key");
    assert_eq!(
        crate::keys::provision_quietly_with_ring(&migrator, converged_id, &ring)
            .await
            .expect("converge after rotation"),
        2,
        "the current key may advance without inventing another provision witness"
    );
    assert_eq!(
        tenant_key_provision_events(&runtime, converged_id)
            .await
            .len(),
        1,
        "post-rotation convergence must keep the generation-1 witness"
    );

    let concurrent_id = TenantId::new();
    let concurrent_suffix = concurrent_id.to_string();
    crate::converge_tenant(
        &migrator,
        &database_roles,
        concurrent_id,
        &format!(
            "cpr45-concurrent-{}",
            &concurrent_suffix[concurrent_suffix.len() - 12..]
        ),
        "CPR-45 concurrent tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit concurrent key fixture tenant");
    let (left, right) = tokio::join!(
        crate::keys::provision_quietly_with_ring(&migrator, concurrent_id, &ring),
        crate::keys::provision_quietly_with_ring(&migrator, concurrent_id, &ring),
    );
    assert_eq!(left.expect("left key convergence"), 1);
    assert_eq!(right.expect("right key convergence"), 1);
    assert_eq!(
        tenant_key_provision_events(&runtime, concurrent_id)
            .await
            .len(),
        1,
        "concurrent convergence must serialize to one audit witness"
    );

    let duplicate_id = TenantId::new();
    let duplicate_suffix = duplicate_id.to_string();
    crate::converge_tenant(
        &migrator,
        &database_roles,
        duplicate_id,
        &format!(
            "cpr45-duplicate-{}",
            &duplicate_suffix[duplicate_suffix.len() - 12..]
        ),
        "CPR-45 historic duplicate tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit historic duplicate fixture tenant");
    ring.provision(&migrator, synveda_crypto::KeyScope::Tenant(duplicate_id))
        .await
        .expect("provision historic duplicate fixture key");
    let exact_payload = serde_json::json!({
        "version": 1,
        "kek_ref": "local:cpr45-convergence-test",
    });
    append_fixture_key_provision_event(&migrator, duplicate_id, exact_payload.clone()).await;
    append_fixture_key_provision_event(&migrator, duplicate_id, exact_payload).await;
    assert_eq!(
        crate::keys::provision_quietly_with_ring(&migrator, duplicate_id, &ring)
            .await
            .expect("accept historic exact duplicate evidence"),
        1
    );
    assert_eq!(
        tenant_key_provision_events(&runtime, duplicate_id)
            .await
            .len(),
        2,
        "historic exact duplicates are retained but never extended"
    );

    let inconsistent_id = TenantId::new();
    let inconsistent_suffix = inconsistent_id.to_string();
    crate::converge_tenant(
        &migrator,
        &database_roles,
        inconsistent_id,
        &format!(
            "cpr45-inconsistent-{}",
            &inconsistent_suffix[inconsistent_suffix.len() - 12..]
        ),
        "CPR-45 inconsistent witness tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit inconsistent witness fixture tenant");
    ring.provision(&migrator, synveda_crypto::KeyScope::Tenant(inconsistent_id))
        .await
        .expect("provision inconsistent witness fixture key");
    let expected_inconsistent_payload = serde_json::json!({
        "version": 1,
        "kek_ref": "local:cpr45-convergence-test",
    });
    append_fixture_key_provision_event(
        &migrator,
        inconsistent_id,
        expected_inconsistent_payload.clone(),
    )
    .await;
    append_fixture_key_provision_event(&migrator, inconsistent_id, expected_inconsistent_payload)
        .await;
    append_fixture_key_provision_event(
        &migrator,
        inconsistent_id,
        serde_json::json!({
            "version": 1,
            "kek_ref": "local:conflicting-authority",
        }),
    )
    .await;
    append_fixture_key_provision_event(
        &migrator,
        inconsistent_id,
        serde_json::json!({
            "version": 1,
            "kek_ref": "local:cpr45-convergence-test",
            "unexpected": true,
        }),
    )
    .await;
    assert!(
        crate::keys::provision_quietly_with_ring(&migrator, inconsistent_id, &ring)
            .await
            .is_err(),
        "a malformed candidate witness must fail closed"
    );
    assert_eq!(
        tenant_key_provision_events(&runtime, inconsistent_id)
            .await
            .len(),
        4,
        "later conflicting references or supersets must not hide behind an exact duplicate prefix"
    );

    let legacy_rotation_id = TenantId::new();
    let legacy_rotation_suffix = legacy_rotation_id.to_string();
    crate::converge_tenant(
        &migrator,
        &database_roles,
        legacy_rotation_id,
        &format!(
            "cpr45-legacy-rotation-{}",
            &legacy_rotation_suffix[legacy_rotation_suffix.len() - 12..]
        ),
        "CPR-45 legacy rotation tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit legacy rotation fixture tenant");
    let legacy_scope = synveda_crypto::KeyScope::Tenant(legacy_rotation_id);
    ring.provision(&migrator, legacy_scope)
        .await
        .expect("provision legacy rotation fixture key");
    ring.rotate(&migrator, legacy_scope)
        .await
        .expect("rotate legacy fixture key");
    append_fixture_key_provision_event(
        &migrator,
        legacy_rotation_id,
        serde_json::json!({
            "version": 2,
            "kek_ref": "local:cpr45-convergence-test",
        }),
    )
    .await;
    assert_eq!(
        crate::keys::provision_quietly_with_ring(&migrator, legacy_rotation_id, &ring)
            .await
            .expect("repair generation-1 evidence beside legacy generation-2 evidence"),
        2
    );
    let legacy_events = tenant_key_provision_events(&runtime, legacy_rotation_id).await;
    assert_eq!(legacy_events.len(), 2);
    assert!(
        legacy_events
            .iter()
            .any(|event| event.payload["version"] == 1),
        "generation-1 evidence must be repaired without deleting history"
    );

    let suffix_source = TenantId::new().to_string();
    let tenant = crate::create_tenant(
        &migrator,
        &database_roles,
        &format!("cpr36-{}", &suffix_source[suffix_source.len() - 12..]),
        "CPR-36 deployment acceptance",
        TenantStatus::Active,
    )
    .await
    .expect("admit an isolated tenant");
    let tenant_id = tenant.id;

    let worker_url = private_test_database_url("SYNVEDA_TEST_WORKER_DATABASE_URL_FILE")
        .expect("test harness worker URL file");
    let worker = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&worker_url)
        .await
        .expect("connect as synveda_worker");
    synveda_store::runtime_role::verify(&runtime, database_roles.gateway(), &database_roles)
        .await
        .expect("gateway session role sentinel");
    synveda_store::runtime_role::verify(&worker, database_roles.worker(), &database_roles)
        .await
        .expect("worker session role sentinel");
    let migrator_identity = synveda_store::runtime_role::database_identity(&migrator)
        .await
        .expect("migrator database identity");
    let gateway_identity = synveda_store::runtime_role::database_identity(&runtime)
        .await
        .expect("healthy gateway database identity");
    let worker_identity = synveda_store::runtime_role::database_identity(&worker)
        .await
        .expect("healthy worker database identity");
    require_one_database_target(&migrator_identity, &gateway_identity, &worker_identity)
        .expect("all healthy deployment credentials target one database");
    assert!(
        synveda_store::runtime_role::verify(&worker, database_roles.gateway(), &database_roles,)
            .await
            .is_err(),
        "worker credentials must not pass as gateway credentials"
    );

    sqlx::query!("drop schema if exists synveda_cli_role_drift cascade")
        .execute(&admin)
        .await
        .expect("remove stale role-drift schema");
    sqlx::query!("create schema synveda_cli_role_drift authorization synveda_gateway")
        .execute(&admin)
        .await
        .expect("give gateway a non-public schema");
    let catalogue_rejected = verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
        .await
        .is_err();
    let connection_rejected = verify_runtime_database_url(
        "SYNVEDA_GATEWAY_DATABASE_URL",
        &runtime_url,
        database_roles.gateway(),
        &database_roles,
    )
    .await
    .is_err();
    sqlx::query!("drop schema synveda_cli_role_drift")
        .execute(&admin)
        .await
        .expect("restore non-owning gateway role");
    assert!(
        catalogue_rejected && connection_rejected,
        "non-public schema ownership must fail both deployment sentinels"
    );

    sqlx::query!(
        r#"do $synveda$
           begin
             if not exists (
               select 1 from pg_catalog.pg_roles
                where rolname = 'synveda_cli_role_grantor'
             ) then
               create role synveda_cli_role_grantor nologin;
             end if;
           end
           $synveda$"#
    )
    .execute(&admin)
    .await
    .expect("create alternate CLI membership grantor");
    sqlx::query!(
        r#"alter role synveda_cli_role_grantor
              with nologin inherit nosuperuser nocreatedb nocreaterole
                   noreplication nobypassrls connection limit -1"#
    )
    .execute(&admin)
    .await
    .expect("converge alternate CLI membership grantor");
    sqlx::query!(
        "grant synveda_app to synveda_cli_role_grantor with admin true, inherit true, set true"
    )
    .execute(&admin)
    .await
    .expect("authorise alternate CLI membership grantor");
    sqlx::query!("revoke synveda_app from synveda_gateway granted by synveda_cli_role_grantor")
        .execute(&admin)
        .await
        .expect("remove stale alternate gateway grant");
    sqlx::query!(
        "grant synveda_app to synveda_gateway with admin true, inherit true, set true granted by synveda_cli_role_grantor"
    )
    .execute(&admin)
    .await
    .expect("add concurrent unsafe gateway grant");
    let duplicate_catalogue_rejected =
        verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
            .await
            .is_err();
    let duplicate_connection_rejected = verify_runtime_database_url(
        "SYNVEDA_GATEWAY_DATABASE_URL",
        &runtime_url,
        database_roles.gateway(),
        &database_roles,
    )
    .await
    .is_err();
    sqlx::query!("revoke synveda_app from synveda_gateway granted by synveda_cli_role_grantor")
        .execute(&admin)
        .await
        .expect("remove concurrent unsafe gateway grant");
    sqlx::query!("revoke synveda_app from synveda_cli_role_grantor")
        .execute(&admin)
        .await
        .expect("remove alternate CLI grantor capability");
    sqlx::query!("drop role synveda_cli_role_grantor")
        .execute(&admin)
        .await
        .expect("drop alternate CLI membership grantor");
    assert!(
        duplicate_catalogue_rejected && duplicate_connection_rejected,
        "a safe grant must not hide an unsafe grant from another grantor"
    );

    let without_context = sqlx::query_scalar!("select count(*) from scopes")
        .fetch_one(&runtime)
        .await
        .expect("query under forced RLS")
        .unwrap_or_default();
    assert_eq!(without_context, 0, "a missing tenant GUC must fail closed");

    let mut tx = synveda_store::rls::begin_tenant_tx(&runtime, tenant_id)
        .await
        .expect("establish the tenant context");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("create through the RLS-enforced application role");
    tx.commit().await.expect("commit the tenant root");

    let mut visible = synveda_store::rls::begin_tenant_tx(&runtime, tenant_id)
        .await
        .expect("restore the tenant context");
    assert_eq!(
        synveda_store::scopes::get(&mut *visible, tenant_id, root.id)
            .await
            .expect("read the tenant root")
            .map(|scope| scope.id),
        Some(root.id),
    );
    visible
        .rollback()
        .await
        .expect("close the read transaction");

    let mut wrong_tenant = synveda_store::rls::begin_tenant_tx(&runtime, TenantId::new())
        .await
        .expect("establish a different tenant context");
    assert!(
        synveda_store::scopes::get(&mut *wrong_tenant, tenant_id, root.id)
            .await
            .expect("cross-tenant reads fail closed")
            .is_none(),
        "the runtime role crossed a tenant boundary",
    );
    wrong_tenant
        .rollback()
        .await
        .expect("close the cross-tenant transaction");

    let mut worker_visible = synveda_store::rls::begin_tenant_tx(&worker, tenant_id)
        .await
        .expect("establish worker tenant context");
    assert_eq!(
        synveda_store::scopes::get(&mut *worker_visible, tenant_id, root.id)
            .await
            .expect("worker reads through forced RLS")
            .map(|scope| scope.id),
        Some(root.id),
    );
    worker_visible
        .rollback()
        .await
        .expect("close worker tenant transaction");
}
