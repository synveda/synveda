//! TEN-4 store tests: the key plane, and the properties ADR-0064 claims.
//!
//! What is asserted here is that the *stored* key plane behaves the way the
//! ADR says, which is a different thing from `synveda-crypto`'s unit tests:
//! those hold key material in hand, and these go through a wrapped row, an
//! unwrap, a cache and RLS. Specifically:
//!
//!   * provisioning is idempotent, because tenant admission and an operator's
//!     backfill both call it and neither should care which ran first;
//!   * a tenant's ciphertext does not open under another tenant's key — the
//!     AAD binding of decision 4, measured against keys that came out of the
//!     database rather than out of a constructor;
//!   * rotation mints the next generation, retires the old one, and **leaves
//!     old payloads readable**, which is the whole of what makes it lazy
//!     (decision 6);
//!   * a `Kms::Disabled` deployment refuses rather than storing a plaintext.
//!
//! These tests need a live Postgres. They read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them with
//! `make db-test`.

use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_crypto::{KeyScope, KeyVersion, Kms, LocalKms, Purpose, RowKey};
use synveda_store::keys::KeyRing;
use synveda_store::{tenant_secrets, tenants};
use synveda_types::{TenantId, TenantStatus};

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping key-plane tests: DATABASE_URL is not set \
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

/// The KEK every suite in this workspace uses against a shared database.
///
/// It has to be the same everywhere, and finding out why is worth writing
/// down: the **deployment** key is a singleton per database (migration
/// 0038's `deployment_keys_current` index enforces exactly one un-retired
/// row). A suite that provisions it under a different KEK leaves a row no
/// other suite can unwrap, and the failure surfaces as "sealed payload for
/// kms.data_key did not open under this key" in a test that never mentioned
/// a KEK. In production that is the correct behaviour — one deployment, one
/// KEK — so the fix is for the tests to agree rather than for the plane to
/// be more forgiving.
const TEST_KEK: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// A ring over the shared test KEK.
fn ring() -> KeyRing {
    KeyRing::new(Kms::Local(
        LocalKms::from_hex(TEST_KEK, "local:test").expect("test kek"),
    ))
}

/// A ring whose cache expires immediately — what a test about rotation wants
/// instead of a sleep.
fn cold_ring() -> KeyRing {
    KeyRing::with_ttl(
        Kms::Local(LocalKms::from_hex(TEST_KEK, "local:test").expect("test kek")),
        std::time::Duration::ZERO,
    )
}

async fn seed_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    let slug = format!("ten4-{}", uuid::Uuid::now_v7().simple());
    tenants::create(pool, id, &slug, "TEN-4", TenantStatus::Active)
        .await
        .expect("admit tenant");
    id
}

#[test]
fn provisioning_is_idempotent_and_mints_the_first_generation() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);

        let first = ring.provision(&db.pool, scope).await.expect("provision");
        assert_eq!(first, KeyVersion::FIRST);
        let again = ring
            .provision(&db.pool, scope)
            .await
            .expect("provision again");
        assert_eq!(
            again, first,
            "a second provision must not mint a second key"
        );
    });
}

#[test]
fn sealing_before_provisioning_refuses_rather_than_minting() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // A seal that mints its own key is a seal that silently succeeds
        // against a key nobody recorded — and, for a tenant, one nobody can
        // hand to the customer whose contract asked for it.
        let tenant = seed_tenant(&db.pool).await;
        let error = ring()
            .sealing_key(&db.pool, KeyScope::Tenant(tenant))
            .await
            .expect_err("must refuse");
        assert!(
            error.to_string().contains("provision"),
            "the error should say what to do: {error}"
        );
    });
}

#[test]
fn a_round_trip_goes_through_a_wrapped_row() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        let sealed = ring
            .sealing_key(&db.pool, scope)
            .await
            .expect("sealing key")
            .seal(
                Purpose::DirectoryCredential,
                RowKey::Name("graph"),
                b"s3cret",
            )
            .expect("seal");
        let opened = ring
            .opening_key(&db.pool, scope, &sealed)
            .await
            .expect("opening key")
            .open(Purpose::DirectoryCredential, RowKey::Name("graph"), &sealed)
            .expect("open");
        assert_eq!(&opened[..], b"s3cret");

        // And the database holds no plaintext.
        let stored: Vec<u8> = sqlx::query_scalar("select wrapped_dek from tenant_keys limit 1")
            .fetch_one(&db.pool)
            .await
            .expect("read a wrapped key");
        assert!(
            !stored.windows(6).any(|window| window == b"s3cret"),
            "the key row must not contain the payload"
        );
    });
}

#[test]
fn one_tenants_ciphertext_does_not_open_under_anothers_key() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // The isolation property TEN-6 will fuzz, measured end to end: two
        // real tenants, two real keys, one ciphertext moved between them.
        let ring = ring();
        let one = KeyScope::Tenant(seed_tenant(&db.pool).await);
        let two = KeyScope::Tenant(seed_tenant(&db.pool).await);
        ring.provision(&db.pool, one).await.expect("provision one");
        ring.provision(&db.pool, two).await.expect("provision two");

        let sealed = ring
            .sealing_key(&db.pool, one)
            .await
            .expect("key one")
            .seal(Purpose::DirectoryCredential, RowKey::Name("graph"), b"one")
            .expect("seal");
        let opened = ring
            .opening_key(&db.pool, two, &sealed)
            .await
            .expect("key two")
            .open(Purpose::DirectoryCredential, RowKey::Name("graph"), &sealed);
        assert!(
            opened.is_err(),
            "a transplanted ciphertext must fail to open, not open"
        );
    });
}

#[test]
fn rotation_mints_the_next_generation_and_keeps_the_old_one_readable() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = cold_ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        let before = ring
            .sealing_key(&db.pool, scope)
            .await
            .expect("first key")
            .seal(Purpose::DirectoryCredential, RowKey::Name("graph"), b"old")
            .expect("seal");

        let next = ring.rotate(&db.pool, scope).await.expect("rotate");
        assert_eq!(next, KeyVersion::FIRST.next());

        // The old payload still opens — nothing was re-sealed, and this is
        // what "lazy" means (decision 6).
        let opened = ring
            .opening_key(&db.pool, scope, &before)
            .await
            .expect("retired key")
            .open(Purpose::DirectoryCredential, RowKey::Name("graph"), &before)
            .expect("open under the retired generation");
        assert_eq!(&opened[..], b"old");

        // And new payloads take the new generation.
        let after = ring
            .sealing_key(&db.pool, scope)
            .await
            .expect("second key")
            .seal(Purpose::DirectoryCredential, RowKey::Name("graph"), b"new")
            .expect("seal");
        assert_eq!(
            synveda_crypto::envelope_version(&after).expect("peek"),
            next,
            "a seal after a rotation must name the new generation"
        );
        assert_eq!(
            synveda_crypto::envelope_version(&before).expect("peek"),
            KeyVersion::FIRST,
            "and the old envelope must still name the old one"
        );
    });
}

#[test]
fn exactly_one_generation_is_current_at_a_time() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = cold_ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");
        ring.rotate(&db.pool, scope).await.expect("rotate");
        ring.rotate(&db.pool, scope).await.expect("rotate again");

        let current: i64 = sqlx::query_scalar(
            "select count(*) from tenant_keys where tenant_id = $1 and retired_at is null",
        )
        .bind(tenant.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count current keys");
        assert_eq!(current, 1, "the partial unique index is the enforcement");

        let total: i64 =
            sqlx::query_scalar("select count(*) from tenant_keys where tenant_id = $1")
                .bind(tenant.as_uuid())
                .fetch_one(&db.pool)
                .await
                .expect("count all keys");
        assert_eq!(total, 3, "retired generations stay, or their data is lost");
    });
}

#[test]
fn rotating_a_scope_with_no_key_is_not_a_provision() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_tenant(&db.pool).await;
        let error = ring()
            .rotate(&db.pool, KeyScope::Tenant(tenant))
            .await
            .expect_err("must refuse");
        assert!(
            matches!(error, synveda_types::Error::NotFound { .. }),
            "rotating nothing is not the same act as provisioning: {error}"
        );
    });
}

#[test]
fn a_disabled_kms_refuses_rather_than_storing_a_plaintext() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_tenant(&db.pool).await;
        let ring = KeyRing::new(Kms::Disabled);
        let error = ring
            .provision(&db.pool, KeyScope::Tenant(tenant))
            .await
            .expect_err("must refuse");
        assert!(
            error.to_string().contains("SYNVEDA_KMS_KEY"),
            "an operator reading this needs to know what to set: {error}"
        );

        let rows: i64 = sqlx::query_scalar("select count(*) from tenant_keys where tenant_id = $1")
            .bind(tenant.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count");
        assert_eq!(rows, 0, "a refused provision must write nothing");
    });
}

#[test]
fn a_wrapped_key_from_another_kek_does_not_unwrap() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // The claim the whole design rests on: the rows are useless without
        // the KEK, which is not in the database.
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring().provision(&db.pool, scope).await.expect("provision");

        let stranger = KeyRing::new(Kms::Local(
            LocalKms::from_hex(&"33".repeat(32), "local:other").expect("other kek"),
        ));
        let error = stranger
            .sealing_key(&db.pool, scope)
            .await
            .expect_err("must refuse");
        assert!(
            error.to_string().contains("did not open"),
            "the wrapped key must not unwrap under a different KEK: {error}"
        );
    });
}

#[test]
fn a_secret_round_trips_sealed_and_the_column_holds_no_plaintext() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        const NAME: &str = "directory.credential";
        let sealed = ring
            .sealing_key(&db.pool, scope)
            .await
            .expect("sealing key")
            .seal(
                Purpose::DirectoryCredential,
                RowKey::Name(NAME),
                b"graph-client-secret",
            )
            .expect("seal");

        let mut tx = synveda_store::rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        tenant_secrets::put(&mut *tx, tenant, NAME, &sealed)
            .await
            .expect("put");
        let stored = tenant_secrets::get(&mut *tx, tenant, NAME)
            .await
            .expect("get")
            .expect("a secret");
        assert_eq!(
            tenant_secrets::names(&mut *tx, tenant)
                .await
                .expect("names"),
            vec![NAME.to_string()]
        );
        tx.commit().await.expect("commit");

        assert!(
            !stored.sealed.windows(5).any(|window| window == b"graph"),
            "the stored column must not contain the credential"
        );
        let opened = ring
            .opening_key(&db.pool, scope, &stored.sealed)
            .await
            .expect("opening key")
            .open(
                Purpose::DirectoryCredential,
                RowKey::Name(NAME),
                &stored.sealed,
            )
            .expect("open");
        assert_eq!(&opened[..], b"graph-client-secret");

        let mut tx = synveda_store::rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        assert!(
            tenant_secrets::delete(&mut *tx, tenant, NAME)
                .await
                .expect("delete"),
            "a credential must be destroyable, or it cannot be revoked"
        );
        assert!(
            !tenant_secrets::delete(&mut *tx, tenant, NAME)
                .await
                .expect("delete again"),
            "and a replayed revocation must be distinguishable from a real one"
        );
        tx.commit().await.expect("commit");
    });
}

#[test]
fn a_secret_sealed_for_one_name_does_not_open_under_another() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        let sealed = ring
            .sealing_key(&db.pool, scope)
            .await
            .expect("sealing key")
            .seal(
                Purpose::DirectoryCredential,
                RowKey::Name("directory.credential"),
                b"value",
            )
            .expect("seal");
        // Renaming a secret makes its ciphertext unopenable rather than
        // silently re-pointing it, which is the safe direction and the reason
        // the name is in the AAD.
        assert!(
            ring.opening_key(&db.pool, scope, &sealed)
                .await
                .expect("opening key")
                .open(
                    Purpose::DirectoryCredential,
                    RowKey::Name("directory.credential.old"),
                    &sealed
                )
                .is_err()
        );
    });
}

#[test]
fn the_deployment_scope_has_its_own_key_and_it_is_not_a_tenants() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        // ADR-0064 decision 5: `console_sessions` cannot select a per-tenant
        // key, so the plane carries a deployment scope. This asserts the two
        // are genuinely different keys rather than the same one twice.
        let ring = ring();
        ring.provision(&db.pool, KeyScope::Deployment)
            .await
            .expect("provision deployment");
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        let sealed = ring
            .sealing_key(&db.pool, KeyScope::Deployment)
            .await
            .expect("deployment key")
            .seal(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[9_u8; 32]),
                b"bearer",
            )
            .expect("seal");
        assert_eq!(
            synveda_crypto::envelope_is_deployment_scoped(&sealed),
            Some(true)
        );
        // Refused at key selection, before a KMS call — and `open` would
        // refuse it again if this check were deleted.
        let error = ring
            .opening_key(&db.pool, scope, &sealed)
            .await
            .expect_err("a deployment-sealed payload must not resolve a tenant key");
        assert!(
            error.to_string().contains("deployment-scoped"),
            "the error should name the disagreement: {error}"
        );
        assert!(
            ring.sealing_key(&db.pool, scope)
                .await
                .expect("tenant key")
                .open(
                    Purpose::ConsoleAccessToken,
                    RowKey::Hash(&[9_u8; 32]),
                    &sealed
                )
                .is_err(),
            "and the load-bearing refusal is still the one inside open()"
        );
    });
}

#[test]
fn the_stored_row_says_which_kek_it_needs_and_carries_only_a_wrapped_key() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let ring = ring();
        let tenant = seed_tenant(&db.pool).await;
        let scope = KeyScope::Tenant(tenant);
        ring.provision(&db.pool, scope).await.expect("provision");

        let mut tx = synveda_store::rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        let stored = synveda_store::keys::tenant_at(&mut *tx, tenant, KeyVersion::FIRST)
            .await
            .expect("read")
            .expect("a key");
        tx.commit().await.expect("commit");

        // `kek_ref` per row is what makes BYOK a column value rather than a
        // redesign (decision 1): this is where a customer's own KMS key would
        // be named.
        assert_eq!(stored.kek_ref, "local:test");
        assert!(!stored.retired);
        // 34 header + 32 key + 16 tag, which migration 0038's check pins
        // exactly rather than as a range.
        assert_eq!(stored.wrapped_dek.len(), 82);
        // The wrapped form is an envelope in its own right, and it names the
        // scope it belongs to — which is why it will not unwrap as another
        // tenant's (asserted in synveda-crypto's kms tests).
        assert_eq!(
            synveda_crypto::envelope_is_deployment_scoped(&stored.wrapped_dek),
            Some(false)
        );
    });
}
