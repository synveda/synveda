//! CPR-31: immutable policy relaxations are interpreted by Cedar rather
//! than as a post-decision override.

use chrono::{TimeDelta, Utc};
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, ScopeNode, relaxable};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    ConfigurationVersionId, CurrentRelaxation, IdentityId, ProposalId, Relaxation,
    RelaxationAction, RelaxationId, RelaxationTerms, RelaxationVersion, RelaxationVersionId,
    ScopeId, Sensitivity, TenantId,
};

struct Fixture {
    tenant: TenantId,
    root: ScopeNode,
    target: ScopeNode,
    private: ScopeNode,
    alice: Principal,
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    let root = ScopeNode {
        id: ScopeId::new(),
        tenant_id: tenant,
        parent_id: None,
        kind: ScopeKind::Tenant,
        slug: "root".to_owned(),
        sealed: false,
    };
    let target = ScopeNode {
        id: ScopeId::new(),
        tenant_id: tenant,
        parent_id: Some(root.id),
        kind: ScopeKind::Workspace,
        slug: "shared".to_owned(),
        sealed: false,
    };
    let private = ScopeNode {
        id: ScopeId::new(),
        tenant_id: tenant,
        parent_id: Some(root.id),
        kind: ScopeKind::Principal,
        slug: "private".to_owned(),
        sealed: false,
    };
    let alice = Principal {
        tenant_id: tenant,
        subject: "alice".to_owned(),
        quarantined: false,
        scope_id: Some(private.id),
        token_scope: None,
    };
    Fixture {
        tenant,
        root,
        target,
        private,
        alice,
    }
}

fn relaxation(fx: &Fixture, target: ScopeId, sensitivity: Sensitivity) -> CurrentRelaxation {
    let now = Utc::now();
    let id = RelaxationId::new();
    let version_id = RelaxationVersionId::new();
    let actor = IdentityId::new();
    let terms = RelaxationTerms {
        subject_identity_id: actor,
        target_scope_id: target,
        action: RelaxationAction::KnowledgeRead,
        max_sensitivity: sensitivity,
        requested_start_at: now - TimeDelta::minutes(1),
        requested_end_at: now + TimeDelta::minutes(30),
        reason: "joint incident response".to_owned(),
    };
    let version = RelaxationVersion {
        id: version_id,
        tenant_id: fx.tenant,
        relaxation_id: id,
        ordinal: 1,
        proposal_id: ProposalId::new(),
        subject_principal_id: "alice".to_owned(),
        effective_start_at: terms.requested_start_at,
        hard_expires_at: terms.requested_end_at,
        configuration_version_id: Some(ConfigurationVersionId::new()),
        configuration_hash: "1".repeat(64),
        content_hash: terms.content_hash().expect("hash terms"),
        creator_id: actor,
        approver_ids: Vec::new(),
        auto_applied: true,
        created_at: now,
        terms,
    };
    CurrentRelaxation {
        relaxation: Relaxation {
            id,
            tenant_id: fx.tenant,
            governing_scope_id: target,
            current_version_id: version_id,
            revision: 1,
            created_at: now,
            created_by: actor,
            updated_at: now,
            updated_by: actor,
            revoked_at: None,
            revoked_by: None,
            revocation_proposal_id: None,
            revocation_reason: None,
            expiry_recorded_at: None,
        },
        version,
    }
}

fn read(
    pdp: &Pdp,
    fx: &Fixture,
    target: &ScopeNode,
    sensitivity: Sensitivity,
    relaxations: &[CurrentRelaxation],
) -> bool {
    let scopes = [target.clone(), fx.root.clone()];
    let principal_scopes = [fx.private.clone(), fx.root.clone()];
    pdp.authorize(
        &fx.alice,
        Action::KnowledgeRead,
        Resource::Scope(target.id),
        &AuthzContext {
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            sensitivity: Some(sensitivity),
            relaxations,
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

#[test]
fn matched_relaxation_is_a_cedar_permit_and_expiry_restores_denial() {
    let pdp = Pdp::new().expect("build PDP");
    let fx = fixture();
    assert!(!read(&pdp, &fx, &fx.target, Sensitivity::Internal, &[]));
    let active = [relaxation(&fx, fx.target.id, Sensitivity::Internal)];
    assert!(read(&pdp, &fx, &fx.target, Sensitivity::Internal, &active));
    assert!(!read(&pdp, &fx, &fx.target, Sensitivity::Internal, &[]));
}

#[test]
fn subject_tier_and_principal_privacy_are_all_fail_closed() {
    let pdp = Pdp::new().expect("build PDP");
    let fx = fixture();
    let mut active = relaxation(&fx, fx.target.id, Sensitivity::Internal);
    active.version.subject_principal_id = "bob".to_owned();
    assert!(!read(
        &pdp,
        &fx,
        &fx.target,
        Sensitivity::Internal,
        &[active]
    ));

    let working = [relaxation(&fx, fx.target.id, Sensitivity::Internal)];
    assert!(!read(
        &pdp,
        &fx,
        &fx.target,
        Sensitivity::Confidential,
        &working
    ));

    let private = [relaxation(&fx, fx.private.id, Sensitivity::Restricted)];
    assert!(!read(
        &pdp,
        &fx,
        &fx.private,
        Sensitivity::Restricted,
        &private
    ));
}

#[test]
fn relaxation_action_vocabulary_is_closed() {
    assert_eq!(
        relaxable(Action::KnowledgeRead),
        Some(RelaxationAction::KnowledgeRead)
    );
    for action in Action::ALL {
        if action != Action::KnowledgeRead {
            assert_eq!(relaxable(action), None, "{action}");
        }
    }
}
