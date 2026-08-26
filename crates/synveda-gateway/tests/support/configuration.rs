//! Governed runtime-configuration fixture helpers.
//!
//! Existing integration suites need to select purpose-built policy packs
//! before the HTTP action under test. These helpers still create a typed
//! VedaFlow proposal, immutable Configuration version, revisioned binding and
//! terminal proposal; they do not restore the deleted assignment setters.

#![allow(dead_code)]

use chrono::Utc;
use sqlx::PgConnection;
use synveda_store::configuration;
use synveda_types::configuration::{
    ConfigurationCommand, ConfigurationDocument, ConfigurationTemplate,
};
use synveda_types::{
    AssetKind, ConfigurationArtifactId, ConfigurationBindingId, ConfigurationVersionId, IdentityId,
    ProposalEffect, ProposalState, ScopeId, Sensitivity, TenantId,
};
use synveda_vedaflow::{PolicySnapshot, Signer};

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub artifact_id: ConfigurationArtifactId,
    pub version_id: ConfigurationVersionId,
    pub binding_id: ConfigurationBindingId,
    pub binding_revision: u64,
}

async fn select_document(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    binding: synveda_types::configuration::ConfigurationBinding,
    document: ConfigurationDocument,
) -> Selection {
    let artifact = configuration::artifact(tx, tenant, binding.artifact_id)
        .await
        .expect("read Configuration fixture artifact")
        .expect("Configuration fixture artifact exists");
    let content_hash = document.content_hash().expect("hash Configuration fixture");
    let existing = configuration::versions(tx, tenant, artifact.id, None, i64::MAX)
        .await
        .expect("read Configuration fixture history")
        .into_iter()
        .find(|version| version.content_hash == content_hash);
    let selected_id = binding
        .pinned_version_id
        .unwrap_or(artifact.current_version_id);
    if let Some(existing) = existing {
        let binding = if binding.enabled && selected_id == existing.id {
            binding
        } else {
            let result = apply_command(
                tx,
                tenant,
                scope_id,
                &ConfigurationCommand::SetBinding {
                    binding_id: binding.id,
                    scope_id,
                    expected_revision: binding.revision,
                    artifact_id: artifact.id,
                    pinned_version_id: Some(existing.id),
                    enabled: true,
                    reason: "select existing integration-test Configuration".to_owned(),
                },
            )
            .await;
            let updated = configuration::binding(tx, tenant, binding.id)
                .await
                .expect("read updated Configuration fixture binding")
                .expect("updated Configuration fixture binding exists");
            assert_eq!(result.binding_revision, Some(updated.revision));
            updated
        };
        return Selection {
            artifact_id: artifact.id,
            version_id: existing.id,
            binding_id: binding.id,
            binding_revision: binding.revision,
        };
    }

    let version_id = ConfigurationVersionId::new();
    apply_command(
        tx,
        tenant,
        scope_id,
        &ConfigurationCommand::Publish {
            artifact_id: artifact.id,
            expected_current_version_id: artifact.current_version_id,
            version_id,
            governing_scope_id: artifact.governing_scope_id,
            document,
            content_hash,
            source_template: None,
        },
    )
    .await;
    let binding = if binding.enabled && binding.pinned_version_id.is_none() {
        binding
    } else {
        let result = apply_command(
            tx,
            tenant,
            scope_id,
            &ConfigurationCommand::SetBinding {
                binding_id: binding.id,
                scope_id,
                expected_revision: binding.revision,
                artifact_id: artifact.id,
                pinned_version_id: None,
                enabled: true,
                reason: "follow published integration-test Configuration".to_owned(),
            },
        )
        .await;
        let updated = configuration::binding(tx, tenant, binding.id)
            .await
            .expect("read updated Configuration fixture binding")
            .expect("updated Configuration fixture binding exists");
        assert_eq!(result.binding_revision, Some(updated.revision));
        updated
    };
    Selection {
        artifact_id: artifact.id,
        version_id,
        binding_id: binding.id,
        binding_revision: binding.revision,
    }
}

async fn apply_command(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    command: &ConfigurationCommand,
) -> configuration::AppliedConfiguration {
    let actor = IdentityId::new();
    let bytes = serde_json::to_vec(command).expect("encode Configuration fixture command");
    let object = synveda_vedaflow::put_object(tx, tenant, AssetKind::Configuration, &bytes)
        .await
        .expect("store Configuration fixture object");
    let artifact_reference = synveda_types::ArtifactReference::new(
        synveda_types::ArtifactFamily::Configuration,
        command.binding_id().map_or_else(
            || command.artifact_id().expect("artifact id").to_string(),
            |id| id.to_string(),
        ),
        command.kind(),
        command
            .version_id()
            .map_or_else(|| object.hash.to_hex(), |id| id.to_string()),
        None,
    )
    .expect("valid Configuration fixture reference");
    let proposal = synveda_vedaflow::proposals::open(
        tx,
        tenant,
        &synveda_vedaflow::NewProposal {
            target_scope: scope_id,
            source_scope: scope_id,
            asset: AssetKind::Configuration,
            effect: ProposalEffect::Apply,
            members: &[("command".to_owned(), object.hash)],
            artifact_references: &[artifact_reference],
            sensitivity: Sensitivity::Internal,
            title: "integration-test runtime Configuration",
            proposer: actor,
            proposer_subject: "configuration-test-fixture",
            committed_at: Utc::now(),
            policy_snapshot: &PolicySnapshot::new("configuration-test-fixture", 1),
        },
        &Signer::Unsigned,
    )
    .await
    .expect("open Configuration fixture proposal");
    let payload_hash = blake3::hash(
        synveda_types::json::canonicalise(
            &serde_json::to_value(command).expect("encode Configuration fixture value"),
        )
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    configuration::insert_change(tx, tenant, proposal.id, command, &payload_hash)
        .await
        .expect("bind Configuration fixture command");
    let result = configuration::apply(
        tx,
        tenant,
        proposal.id,
        "configuration-test-fixture",
        command,
    )
    .await
    .expect("apply Configuration fixture command");
    configuration::complete_change(tx, tenant, proposal.id, result)
        .await
        .expect("complete Configuration fixture command");
    assert!(
        synveda_vedaflow::proposals::close(
            tx,
            tenant,
            proposal.id,
            ProposalState::Applied,
            actor,
            None,
        )
        .await
        .expect("close Configuration fixture proposal")
    );
    result
}

pub async fn bind_pack(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    pack: &str,
) -> Selection {
    let mut document = ConfigurationDocument::template(ConfigurationTemplate::Personal);
    document.policy_pack = pack.to_owned();
    let content_hash = document.content_hash().expect("hash Configuration fixture");
    if let Some(binding) = configuration::bindings(tx, tenant, Some(scope_id), None, 2)
        .await
        .expect("read Configuration fixture binding")
        .into_iter()
        .next()
    {
        return select_document(tx, tenant, scope_id, binding, document).await;
    }

    let artifact_id = ConfigurationArtifactId::new();
    let version_id = ConfigurationVersionId::new();
    let binding_id = ConfigurationBindingId::new();
    apply_command(
        tx,
        tenant,
        scope_id,
        &ConfigurationCommand::Create {
            artifact_id,
            version_id,
            governing_scope_id: scope_id,
            name: format!("fixture-{}", artifact_id.as_uuid().simple()),
            document,
            content_hash,
            source_template: None,
        },
    )
    .await;
    let result = apply_command(
        tx,
        tenant,
        scope_id,
        &ConfigurationCommand::Bind {
            binding_id,
            scope_id,
            artifact_id,
            pinned_version_id: None,
            enabled: true,
        },
    )
    .await;
    Selection {
        artifact_id,
        version_id,
        binding_id,
        binding_revision: result.binding_revision.expect("binding revision"),
    }
}

pub async fn bind_tenant_pack(tx: &mut PgConnection, tenant: TenantId, pack: &str) -> Selection {
    let root = synveda_store::scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read tenant root for Configuration fixture")
        .expect("tenant root exists for Configuration fixture");
    bind_pack(tx, tenant, root.id, pack).await
}

pub async fn set_trace_retention(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    mode: synveda_types::TraceRetentionMode,
) -> Selection {
    let binding = configuration::bindings(tx, tenant, Some(scope_id), None, 2)
        .await
        .expect("read Configuration fixture binding")
        .into_iter()
        .next()
        .expect("Configuration fixture binding exists");
    let artifact = configuration::artifact(tx, tenant, binding.artifact_id)
        .await
        .expect("read Configuration fixture artifact")
        .expect("Configuration fixture artifact exists");
    let selected_id = binding
        .pinned_version_id
        .unwrap_or(artifact.current_version_id);
    let current = configuration::version(tx, tenant, selected_id)
        .await
        .expect("read Configuration fixture version")
        .expect("Configuration fixture version exists");
    let mut document = current.document;
    if document.context.trace_retention == mode {
        return Selection {
            artifact_id: artifact.id,
            version_id: current.id,
            binding_id: binding.id,
            binding_revision: binding.revision,
        };
    }
    document.context.trace_retention = mode;
    select_document(tx, tenant, scope_id, binding, document).await
}

pub async fn set_graph_enabled(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    enabled: bool,
) -> Selection {
    let binding = configuration::bindings(tx, tenant, Some(scope_id), None, 2)
        .await
        .expect("read Configuration fixture binding")
        .into_iter()
        .next()
        .expect("Configuration fixture binding exists");
    let artifact = configuration::artifact(tx, tenant, binding.artifact_id)
        .await
        .expect("read Configuration fixture artifact")
        .expect("Configuration fixture artifact exists");
    let selected_id = binding
        .pinned_version_id
        .unwrap_or(artifact.current_version_id);
    let current = configuration::version(tx, tenant, selected_id)
        .await
        .expect("read Configuration fixture version")
        .expect("Configuration fixture version exists");
    let mut document = current.document;
    document.context.graph = if enabled {
        ConfigurationDocument::template(ConfigurationTemplate::Personal)
            .context
            .graph
    } else {
        synveda_types::configuration::GraphRetrievalConfiguration {
            enabled: false,
            max_hops: 0,
            fan_out_per_node: 0,
            max_expanded_candidates: 0,
            time_budget_ms: 0,
            token_budget: 0,
        }
    };
    select_document(tx, tenant, scope_id, binding, document).await
}

pub async fn set_advertisement(
    tx: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    skills: bool,
    tools: bool,
) -> Selection {
    let binding = configuration::bindings(tx, tenant, Some(scope_id), None, 2)
        .await
        .expect("read Configuration fixture binding")
        .into_iter()
        .next()
        .expect("Configuration fixture binding exists");
    let artifact = configuration::artifact(tx, tenant, binding.artifact_id)
        .await
        .expect("read Configuration fixture artifact")
        .expect("Configuration fixture artifact exists");
    let selected_id = binding
        .pinned_version_id
        .unwrap_or(artifact.current_version_id);
    let current = configuration::version(tx, tenant, selected_id)
        .await
        .expect("read Configuration fixture version")
        .expect("Configuration fixture version exists");
    let mut document = current.document;
    if document.advertisement.skills == skills && document.advertisement.tools == tools {
        return Selection {
            artifact_id: artifact.id,
            version_id: current.id,
            binding_id: binding.id,
            binding_revision: binding.revision,
        };
    }
    document.advertisement.skills = skills;
    document.advertisement.tools = tools;
    select_document(tx, tenant, scope_id, binding, document).await
}

pub async fn set_tenant_advertisement(
    tx: &mut PgConnection,
    tenant: TenantId,
    skills: bool,
    tools: bool,
) -> Selection {
    let root = synveda_store::scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read tenant root for Configuration fixture")
        .expect("tenant root exists for Configuration fixture");
    set_advertisement(tx, tenant, root.id, skills, tools).await
}

pub async fn disable(tx: &mut PgConnection, tenant: TenantId, scope_id: ScopeId) -> bool {
    let Some(binding) = configuration::bindings(tx, tenant, Some(scope_id), None, 2)
        .await
        .expect("read Configuration fixture binding")
        .into_iter()
        .next()
    else {
        return false;
    };
    if !binding.enabled {
        return false;
    }
    apply_command(
        tx,
        tenant,
        scope_id,
        &ConfigurationCommand::SetBinding {
            binding_id: binding.id,
            scope_id,
            expected_revision: binding.revision,
            artifact_id: binding.artifact_id,
            pinned_version_id: binding.pinned_version_id,
            enabled: false,
            reason: "disable integration-test policy selection".to_owned(),
        },
    )
    .await;
    true
}

pub async fn disable_tenant(tx: &mut PgConnection, tenant: TenantId) -> bool {
    let root = synveda_store::scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read tenant root for Configuration fixture")
        .expect("tenant root exists for Configuration fixture");
    disable(tx, tenant, root.id).await
}
