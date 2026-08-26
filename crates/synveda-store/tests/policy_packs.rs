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
use synveda_store::{configuration, policy_assignments, policy_packs, rls, tenants};
use synveda_types::configuration::{
    ConfigurationCommand, ConfigurationDocument, ConfigurationTemplate,
};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    ArtifactFamily, ArtifactReference, CompositionConfig, ConfigurationArtifactId,
    ConfigurationBindingId, ConfigurationVersionId, Error, IdentityId, PackConfig, ProposalId,
    ScopeId, TenantId, TenantStatus,
};

/// Seeding shape the old hierarchy-create calls had, on the governed
/// substrate (CPR-7, ADR-0074): caller-chosen id, the old kinds mapped —
/// `org` is the tenant root, `division`/`department`/`team` are org
/// units, `user` a principal.
async fn mk_scope(
    conn: &mut sqlx::PgConnection,
    id: synveda_types::ScopeId,
    tenant: synveda_types::TenantId,
    parent: Option<synveda_types::ScopeId>,
    kind: synveda_types::scope::ScopeKind,
    slug: &str,
    name: &str,
) -> synveda_types::scope::Scope {
    synveda_store::scopes::create(
        conn,
        &synveda_store::scopes::NewScope {
            id,
            tenant_id: tenant,
            kind,
            parent_scope_id: parent,
            slug: slug.to_owned(),
            display_name: name.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("seed scope")
}

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

async fn apply_configuration_command(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    command: &ConfigurationCommand,
) -> configuration::AppliedConfiguration {
    let proposal = ProposalId::new();
    let actor = IdentityId::new();
    let proposal_uuid = proposal.as_uuid();
    let nonce = proposal_uuid.as_bytes();
    let object_hash = blake3::hash(&[b"object".as_slice(), nonce].concat());
    let tree_hash = blake3::hash(&[b"tree".as_slice(), nonce].concat());
    let commit_hash = blake3::hash(&[b"commit".as_slice(), nonce].concat());
    let content = serde_json::to_vec(command).expect("encode Configuration command");
    let payload_hash = blake3::hash(
        synveda_types::json::canonicalise(
            &serde_json::to_value(command).expect("encode Configuration command value"),
        )
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let reference = match command {
        ConfigurationCommand::Create {
            artifact_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_current_version_id.to_string()),
        ),
        ConfigurationCommand::Bind {
            binding_id,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.clone(), |id| id.to_string()),
            None,
        ),
        ConfigurationCommand::SetBinding {
            binding_id,
            expected_revision,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.clone(), |id| id.to_string()),
            Some(expected_revision.to_string()),
        ),
    }
    .expect("valid Configuration fixture reference");
    let artifact_references = serde_json::to_value([reference]).expect("encode typed reference");
    sqlx::query!(
        "insert into vedaflow_objects (tenant_id, hash, kind, content, size_bytes)
         values ($1, $2, 'configuration', $3, $4)",
        tenant.as_uuid(),
        object_hash.as_bytes().as_slice(),
        &content,
        i32::try_from(content.len()).expect("command size"),
    )
    .execute(&mut *tx)
    .await
    .expect("store Configuration command object");
    sqlx::query!(
        "insert into vedaflow_trees (tenant_id, hash) values ($1, $2)",
        tenant.as_uuid(),
        tree_hash.as_bytes().as_slice(),
    )
    .execute(&mut *tx)
    .await
    .expect("store Configuration tree");
    sqlx::query!(
        "insert into vedaflow_tree_entries (tenant_id, tree_hash, name, object_hash)
         values ($1, $2, 'command', $3)",
        tenant.as_uuid(),
        tree_hash.as_bytes().as_slice(),
        object_hash.as_bytes().as_slice(),
    )
    .execute(&mut *tx)
    .await
    .expect("store Configuration tree entry");
    sqlx::query!(
        "insert into vedaflow_commits
             (tenant_id, hash, tree_hash, author_id, message, committed_at,
              policy_snapshot_hash)
         values ($1, $2, $3, $4, 'test Configuration command', now(), $5)",
        tenant.as_uuid(),
        commit_hash.as_bytes().as_slice(),
        tree_hash.as_bytes().as_slice(),
        actor.as_uuid(),
        &[0_u8; 32][..],
    )
    .execute(&mut *tx)
    .await
    .expect("store Configuration commit");
    sqlx::query!(
        "insert into vedaflow_proposals
             (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
              target_channel, commit_hash, sensitivity, title, proposer_id,
              proposer_subject, artifact_references)
         values ($1, $2, $3, $3, 'configuration', 'apply', $4, 'internal',
                 'test Configuration change', $5, 'configuration-fixture', $6)",
        tenant.as_uuid(),
        proposal.as_uuid(),
        scope.as_uuid(),
        commit_hash.as_bytes().as_slice(),
        actor.as_uuid(),
        artifact_references,
    )
    .execute(&mut *tx)
    .await
    .expect("open Configuration proposal");
    configuration::insert_change(&mut *tx, tenant, proposal, command, &payload_hash)
        .await
        .expect("bind Configuration command to proposal");
    let applied =
        configuration::apply(&mut *tx, tenant, proposal, "configuration-fixture", command)
            .await
            .expect("apply Configuration command");
    configuration::complete_change(&mut *tx, tenant, proposal, applied)
        .await
        .expect("record Configuration result");
    sqlx::query!(
        "update vedaflow_proposals
            set state = 'applied', closed_at = now(), closed_by = $3,
                updated_at = now()
          where tenant_id = $1 and id = $2",
        tenant.as_uuid(),
        proposal.as_uuid(),
        actor.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("close Configuration proposal");
    applied
}

async fn create_bound_configuration(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    name: &str,
    pack: &str,
) -> (
    ConfigurationArtifactId,
    ConfigurationVersionId,
    ConfigurationBindingId,
) {
    let artifact_id = ConfigurationArtifactId::new();
    let version_id = ConfigurationVersionId::new();
    let binding_id = ConfigurationBindingId::new();
    let mut document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
    document.policy_pack = pack.to_owned();
    let content_hash = document.content_hash().expect("hash Configuration");
    apply_configuration_command(
        tx,
        tenant,
        scope,
        &ConfigurationCommand::Create {
            artifact_id,
            version_id,
            governing_scope_id: scope,
            name: name.to_owned(),
            document,
            content_hash,
            source_template: None,
        },
    )
    .await;
    apply_configuration_command(
        tx,
        tenant,
        scope,
        &ConfigurationCommand::Bind {
            binding_id,
            scope_id: scope,
            artifact_id,
            pinned_version_id: None,
            enabled: true,
        },
    )
    .await;
    (artifact_id, version_id, binding_id)
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
        summary_chars: 240,
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

/// A pack named by immutable Configuration history cannot be cleared. A
/// rollback may select that exact version again, so retaining only the
/// currently selected name would turn history into a dangling reference.
#[tokio::test]
async fn clear_refuses_while_configuration_history_references_the_pack() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = ScopeId::new();
    mk_scope(
        &mut tx,
        root,
        tenant,
        None,
        ScopeKind::Tenant,
        "acme",
        "ACME",
    )
    .await;
    policy_packs::apply(
        &mut *tx,
        tenant,
        "authz2-pinned",
        "permit (p);",
        &PackConfig::default(),
    )
    .await
    .expect("apply pack");

    create_bound_configuration(&mut tx, tenant, root, "pinned-runtime", "authz2-pinned").await;
    let refused = policy_packs::clear(&mut tx, tenant, "authz2-pinned").await;
    assert!(
        matches!(refused, Err(Error::Conflict { .. })),
        "clearing a pack in immutable Configuration history must be Conflict, got {refused:?}"
    );
}

/// Cedar's compact assignment input is a derived view of immutable
/// Configuration versions and revisioned bindings, not a mutable second
/// policy-selection model.
#[tokio::test]
async fn configuration_versions_and_bindings_drive_policy_projection() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = ScopeId::new();
    let team = ScopeId::new();
    mk_scope(
        &mut tx,
        root,
        tenant,
        None,
        ScopeKind::Tenant,
        "acme",
        "ACME",
    )
    .await;
    mk_scope(
        &mut tx,
        team,
        tenant,
        Some(root),
        ScopeKind::OrgUnit,
        "core",
        "Core",
    )
    .await;

    create_bound_configuration(&mut tx, tenant, root, "tenant-runtime", "standard").await;
    let (artifact_id, first_version_id, _) =
        create_bound_configuration(&mut tx, tenant, team, "team-runtime", "standard").await;
    let initial = policy_assignments::for_scopes(&mut tx, tenant, &[team, root])
        .await
        .expect("chain lookup");
    assert_eq!(initial.len(), 2);
    assert!(
        initial
            .iter()
            .all(|assignment| assignment.pack_name == "standard")
    );

    let next_version_id = ConfigurationVersionId::new();
    let mut document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
    document.policy_pack = "open-collaboration".to_owned();
    let content_hash = document.content_hash().expect("hash next Configuration");
    apply_configuration_command(
        &mut tx,
        tenant,
        team,
        &ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id: first_version_id,
            version_id: next_version_id,
            governing_scope_id: team,
            document,
            content_hash,
            source_template: None,
        },
    )
    .await;
    let projected = policy_assignments::for_scopes(&mut tx, tenant, &[team])
        .await
        .expect("read updated derived projection");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].pack_name, "open-collaboration");
    assert!(
        configuration::version(&mut tx, tenant, first_version_id)
            .await
            .expect("read immutable first version")
            .is_some()
    );
    assert_eq!(
        policy_assignments::default_pack(&mut tx, tenant)
            .await
            .expect("derive tenant root pack"),
        Some("standard".to_owned())
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
}
