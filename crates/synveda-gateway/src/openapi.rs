//! The OpenAPI contract (CPR-4, ADR-0071 decision 7).
//!
//! # Derived, not written
//!
//! Every path, parameter and schema in this document comes from the Rust types
//! and the `#[utoipa::path]` annotations on the handlers that serve them. That
//! is the entire content of "the contract is authoritative": a hand-authored
//! document is a second description of the surface, and a second description
//! drifts — which is exactly the condition ADR-0068 recorded at the base
//! commit, where every DTO was hand-written per handler and `console/src/api.mts`
//! held a second hand-written copy of the subset the console consumed.
//!
//! The generated document is committed at `docs/api/openapi.json` and
//! `crates/synveda-gateway/tests/openapi.rs` fails when the tree and the file
//! disagree; `console/src/generated/api.ts` is generated *from that file* by
//! `scripts/generate-api-types.mjs`, and `make check-api-types` fails when
//! those two disagree. So there are three artefacts and two checks, and the
//! only one a human edits is the Rust.
//!
//! # Exact executable coverage
//!
//! CPR-29 (ADR-0088) makes the document cover every bearer-authenticated
//! production `/v1` operation. [`crate::routes`] builds the Axum router and its
//! method/path inventory from one declaration; the contract test compares that
//! inventory with this generated document in both directions. Auth callbacks,
//! operational probes and the separately authenticated `/scim/v2` protocol
//! surface are intentionally outside this application contract.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::access;
use crate::capture;
use crate::context_api;
use crate::knowledge_api;
use crate::me;
use crate::okf;
use crate::sessions;
use crate::skills;
use crate::tool_registry;
use crate::workspaces;

/// The document root.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Synveda",
        description = "\
Governed Knowledge and context for AI agents.

**Coverage.** This is the exact bearer-authenticated production application \
surface mounted under `/v1`: identity and onboarding; scopes and access; \
workspaces, projects and repositories; sessions and immutable events; capture; \
Knowledge and explainable context; VedaFlow proposals and authored-artifact \
channels; policy administration and time-bounded relaxations; audit and \
quarantine; service identities and directory controls; Agent Skills; the \
trusted MCP catalogue; and OKF v0.2 import/export. The same route declaration \
builds the executable router and the inventory compared with this document. \
Auth callbacks, health/metrics/OpenAPI endpoints, static console assets and the \
separately authenticated `/scim/v2` protocol surface are intentionally outside \
this application contract. Since CPR-12 the session plane is the only adapter \
runtime plane: observations enter through session events and context is \
composed through session context runs. The deleted global observe, inject and \
recall routes are neither mounted nor documented.

**Tenancy.** Every path below sits behind bearer authentication and tenant \
resolution. A response is always scoped to the caller's tenant, which is why \
no response body echoes a tenant id.

**Idempotency.** Operations that accept `Idempotency-Key` declare it on that \
operation. Reusing a key with the same request replays the original result; \
reusing it with a different request is `409`. Session-event delivery is \
idempotent at the finer event unit through `client_event_id`. One-time secret \
issuance is deliberately not replayed: the plaintext token is returned once \
and never stored.

**Identity.** No request body on this surface accepts a tenant or an acting \
principal. Both are resolved from the bearer credential; a body naming either \
is refused rather than ignored.

**Preconditions.** Every update on the workspace plane — workspaces, \
projects, repositories — takes a required `expected_revision`; a mismatch is \
`409` and writes nothing. The scope admin plane does not: a scope carries no \
revision, and its mutations are last-writer-wins under the PDP.",
        version = env!("CARGO_PKG_VERSION"),
        license(name = "Proprietary"),
    ),
    servers((url = "/", description = "This gateway")),
    paths(
        crate::app::whoami,
        crate::capabilities::batch,
        crate::policy::packs,
        crate::configuration::templates,
        crate::configuration::list,
        crate::configuration::create,
        crate::configuration::effective,
        crate::configuration::get,
        crate::configuration::versions,
        crate::configuration::publish,
        crate::configuration::compare,
        crate::configuration::bindings,
        crate::configuration::create_binding,
        crate::configuration::update_binding,
        crate::configuration::rollback_binding,
        crate::service_identities::register,
        crate::service_identities::list,
        crate::service_identities::get,
        crate::service_identities::remove,
        crate::directory_admin::status,
        crate::directory_admin::authorise,
        crate::scim::credentials::issue,
        crate::scim::credentials::list,
        crate::scim::credentials::revoke,
        crate::quarantine::list,
        crate::quarantine::release,
        crate::quarantine::reject,
        crate::audit_query::events,
        crate::audit_query::disclosures,
        crate::audit_query::knowledge,
        crate::audit_query::verify,
        crate::channels::list,
        crate::channels::publish,
        crate::channels::history,
        crate::channels::rollback,
        crate::channels::pin,
        crate::channels::unpin,
        crate::prompts::author,
        crate::prompts::resolve,
        crate::prompts::list,
        crate::packs::author,
        crate::packs::list,
        crate::relaxations::create,
        crate::relaxations::revise,
        crate::relaxations::revoke,
        crate::relaxations::list,
        crate::relaxations::get,
        crate::relaxations::versions,
        crate::curators::get,
        crate::curators::put,
        crate::proposals::list,
        crate::proposals::get,
        crate::proposals::open,
        crate::proposals::approve,
        crate::proposals::reject,
        crate::proposals::withdraw,
        crate::proposals::apply,
        crate::proposals::publish,
        crate::admin_scopes::list,
        crate::admin_scopes::create,
        crate::admin_scopes::get,
        crate::admin_scopes::update,
        crate::admin_scopes::ancestors,
        crate::admin_scopes::descendants,
        me::get,
        knowledge_api::list,
        knowledge_api::create,
        knowledge_api::get,
        knowledge_api::edit,
        knowledge_api::delete,
        knowledge_api::history,
        knowledge_api::sources_for_item,
        knowledge_api::usage,
        knowledge_api::verify,
        knowledge_api::supersede,
        knowledge_api::archive,
        knowledge_api::restore,
        knowledge_api::merge,
        workspaces::list,
        workspaces::create,
        workspaces::get,
        workspaces::update,
        workspaces::list_projects,
        workspaces::create_project,
        workspaces::get_project,
        workspaces::update_project,
        workspaces::list_repositories,
        workspaces::attach_repository,
        workspaces::detach_repository,
        sessions::open,
        sessions::list,
        sessions::get,
        sessions::append_events,
        sessions::get_event,
        sessions::end,
        sessions::timeline,
        context_api::create_context_run,
        context_api::list,
        context_api::get,
        context_api::feedback,
        context_api::knowledge_query,
        context_api::knowledge_evaluation,
        capture::create_batch,
        capture::list_batches,
        capture::get_batch,
        capture::accept_batch,
        capture::list_candidates,
        capture::accept_candidate,
        capture::merge_candidate,
        capture::replace_candidate,
        capture::dismiss_candidate,
        okf::plan_import,
        okf::list_imports,
        okf::get_import,
        okf::materialize_import,
        okf::export,
        skills::install,
        skills::update,
        skills::list,
        skills::get,
        skills::list_versions,
        skills::get_version,
        skills::list_files,
        skills::get_file,
        skills::create_binding,
        skills::update_binding,
        skills::rollback_binding,
        skills::list_bindings,
        skills::get_binding,
        skills::available,
        skills::record_usage,
        skills::list_usage,
        skills::run_test,
        skills::list_tests,
        tool_registry::register,
        tool_registry::import_client_config,
        tool_registry::stage_version,
        tool_registry::discover,
        tool_registry::list,
        tool_registry::get,
        tool_registry::list_versions,
        tool_registry::get_version,
        tool_registry::diff,
        tool_registry::create_binding,
        tool_registry::update_binding,
        tool_registry::list_bindings,
        tool_registry::get_binding,
        tool_registry::generate_config,
        tool_registry::run_test,
        tool_registry::list_tests,
        access::list_workspace_members,
        access::list_invites,
        access::create_invite,
        access::revoke_invite,
        access::accept_invite,
        access::list_project_members,
        access::add_project_member,
        access::remove_project_member,
        access::list_groups,
        access::create_group,
        access::update_group,
        access::list_grants,
        access::create_grant,
        access::revoke_grant,
    ),
    components(schemas(
        crate::app::WhoamiResponse,
        crate::capabilities::NodeCapabilities,
        crate::capabilities::PackView,
        crate::capabilities::BatchResponse,
        crate::policy::PackSummary,
        crate::policy::PacksResponse,
        crate::policy::OriginView,
        crate::configuration::CaptureConfigurationBody,
        crate::configuration::ContextConfigurationBody,
        crate::configuration::FreshnessConfigurationBody,
        crate::configuration::AdvertisementConfigurationBody,
        crate::configuration::ConfigurationDocumentBody,
        crate::configuration::CreateConfigurationBody,
        crate::configuration::PublishConfigurationBody,
        crate::configuration::CreateConfigurationBindingBody,
        crate::configuration::UpdateConfigurationBindingBody,
        crate::configuration::RollbackConfigurationBindingBody,
        crate::configuration::ConfigurationMutationView,
        crate::configuration::ConfigurationTemplateView,
        crate::configuration::ConfigurationTemplateListView,
        crate::configuration::ConfigurationArtifactView,
        crate::configuration::ConfigurationArtifactListView,
        crate::configuration::ConfigurationVersionView,
        crate::configuration::ConfigurationVersionListView,
        crate::configuration::ConfigurationBindingView,
        crate::configuration::ConfigurationBindingListView,
        crate::configuration::EffectiveConfigurationView,
        crate::configuration::ConfigurationComparisonView,
        crate::service_identities::RegisterBody,
        crate::service_identities::ServiceIdentityView,
        crate::service_identities::ServiceIdentitiesResponse,
        crate::directory_admin::SyncStatus,
        crate::directory_admin::AuthorisationView,
        crate::directory_admin::AuthoriseRequest,
        crate::directory_admin::AuthoriseResponse,
        crate::scim::credentials::IssueRequest,
        crate::scim::credentials::IssuedCredential,
        crate::scim::credentials::ScimCredentialView,
        crate::scim::credentials::ScimCredentialsResponse,
        crate::quarantine::QuarantineView,
        crate::quarantine::QueueResponse,
        crate::quarantine::ReviewBody,
        crate::audit_query::Frame,
        crate::audit_query::EventView,
        crate::audit_query::EventsResponse,
        crate::audit_query::DisclosureView,
        crate::audit_query::DisclosuresResponse,
        crate::audit_query::KnownView,
        crate::audit_query::KnowledgeResponse,
        crate::audit_query::VerifyResponse,
        crate::approvals::RequirementView,
        crate::approvals::RoleView,
        crate::channels::ChannelView,
        crate::channels::PinView,
        crate::channels::ChannelsResponse,
        crate::channels::PublishBody,
        crate::channels::PublishResponse,
        crate::channels::PublishedMember,
        crate::channels::HistoryEntryView,
        crate::channels::HistoryResponse,
        crate::channels::RollbackBody,
        crate::channels::RollbackResponse,
        crate::channels::PinBody,
        crate::channels::UnpinBody,
        crate::channels::PinResponse,
        crate::channels::UnpinResponse,
        crate::prompts::PromptVariableSchema,
        crate::prompts::AuthorBody,
        crate::prompts::PublishedView,
        crate::prompts::PromptView,
        crate::prompts::Origin,
        crate::prompts::ResolveResponse,
        crate::prompts::ListEntry,
        crate::prompts::ListResponse,
        crate::packs::DocumentBody,
        crate::packs::AuthorBody,
        crate::packs::PublishedView,
        crate::packs::DocumentView,
        crate::packs::PackView,
        crate::packs::ListEntry,
        crate::packs::ListResponse,
        crate::relaxations::RelaxationTermsBody,
        crate::relaxations::CreateRelaxationBody,
        crate::relaxations::ReviseRelaxationBody,
        crate::relaxations::RevokeRelaxationBody,
        crate::relaxations::RelaxationMutationView,
        crate::relaxations::RelaxationVersionView,
        crate::relaxations::RelaxationView,
        crate::relaxations::RelaxationListView,
        crate::relaxations::RelaxationVersionListView,
        crate::curators::RuleView,
        crate::curators::CuratorsResponse,
        crate::curators::PutBody,
        crate::curators::PutResponse,
        crate::proposals::ProposalSummary,
        crate::proposals::PromotionMemberEvidenceSchema,
        crate::proposals::PromotionEvidenceSchema,
        crate::proposals::ApprovalView,
        crate::proposals::MemberEffect,
        crate::proposals::BaselineView,
        crate::proposals::MemberView,
        crate::proposals::ProposalDetail,
        crate::proposals::ListResponse,
        crate::proposals::OpenBody,
        crate::proposals::OpenResponse,
        crate::proposals::ReviewBody,
        crate::proposals::RejectBody,
        crate::proposals::ReviewResponse,
        crate::proposals::PublishResponse,
        me::MeView,
        me::PrincipalView,
        me::TenantView,
        me::OnboardingView,
        me::OnboardingState,
        crate::capabilities::TenantCapabilities,
        knowledge_api::KnowledgeRevisionView,
        knowledge_api::KnowledgeRelationView,
        knowledge_api::KnowledgeItemView,
        knowledge_api::KnowledgeListView,
        knowledge_api::KnowledgeHistoryView,
        knowledge_api::KnowledgeSourceView,
        knowledge_api::KnowledgeSourcesView,
        knowledge_api::KnowledgeUsageView,
        knowledge_api::KnowledgeUsageListView,
        knowledge_api::KnowledgeMutationView,
        knowledge_api::KnowledgeContentBody,
        knowledge_api::KnowledgeSourceBody,
        knowledge_api::CreateKnowledgeBody,
        knowledge_api::EditKnowledgeBody,
        knowledge_api::VerifyKnowledgeBody,
        knowledge_api::SupersedeKnowledgeBody,
        knowledge_api::MergeInputBody,
        knowledge_api::MergeKnowledgeBody,
        knowledge_api::LifecycleKnowledgeBody,
        knowledge_api::DeleteKnowledgeBody,
        workspaces::WorkspaceView,
        workspaces::WorkspaceList,
        workspaces::ProjectView,
        workspaces::ProjectList,
        workspaces::RepositoryView,
        workspaces::RepositoryList,
        workspaces::CreateWorkspaceBody,
        workspaces::CreateProjectBody,
        workspaces::UpdateBody,
        workspaces::AttachRepositoryBody,
        workspaces::ApiErrorBody,
        sessions::SessionView,
        sessions::SessionList,
        sessions::SessionEventView,
        sessions::AppendedEventView,
        sessions::AppendResponse,
        sessions::ContextRunView,
        sessions::TimelineEntry,
        sessions::TimelineView,
        sessions::OpenSessionBody,
        sessions::NewEventBody,
        sessions::AppendEventsBody,
        sessions::EndSessionBody,
        context_api::ContextScoreView,
        context_api::ContextCandidateView,
        context_api::ContextSelectionView,
        context_api::ContextFeedbackView,
        context_api::ContextRunDetailView,
        context_api::ContextRunListView,
        context_api::ContextKnowledgeView,
        context_api::ContextKnowledgeQueryView,
        context_api::CreateContextRunBody,
        context_api::KnowledgeQueryBody,
        context_api::KnowledgeEvaluationBody,
        context_api::ContextFeedbackBody,
        capture::CaptureBatchView,
        capture::CaptureBatchListView,
        capture::CaptureMatchView,
        capture::CaptureCandidateView,
        capture::CaptureCandidateListView,
        capture::CaptureDecisionView,
        capture::AcceptCandidateBody,
        capture::MergeCandidateBody,
        capture::ReplaceCandidateBody,
        capture::DismissCandidateBody,
        capture::AcceptBatchBody,
        okf::OkfInputEntryBody,
        okf::PlanOkfImportBody,
        okf::OkfArtifactView,
        okf::OkfMappingView,
        okf::OkfImportJobView,
        okf::OkfImportPlanView,
        okf::OkfImportJobListView,
        okf::OkfMaterializationView,
        okf::ExportOkfBody,
        okf::OkfExportFileView,
        okf::OkfExportView,
        skills::SkillFileBody,
        skills::SkillProvenanceBody,
        skills::InstallSkillBody,
        skills::UpdateSkillBody,
        skills::CreateSkillBindingBody,
        skills::UpdateSkillBindingBody,
        skills::RollbackSkillBindingBody,
        skills::RecordSkillUsageBody,
        skills::RunSkillTestBody,
        skills::SkillMutationView,
        skills::SkillVersionView,
        skills::SkillView,
        skills::SkillListView,
        skills::SkillVersionFileView,
        skills::SkillVersionFileContentView,
        skills::SkillVersionListView,
        skills::SkillVersionFileListView,
        skills::SkillBindingView,
        skills::SkillBindingListView,
        skills::AvailableSkillView,
        skills::AvailableSkillListView,
        skills::SkillUsageEventView,
        skills::SkillUsageListView,
        skills::SkillTestRunView,
        skills::SkillTestRunListView,
        tool_registry::ToolServerDescriptorBody,
        tool_registry::RegisterToolServerBody,
        tool_registry::ImportToolClientConfigBody,
        tool_registry::StageToolVersionBody,
        tool_registry::DiscoverToolServerBody,
        tool_registry::CreateToolBindingBody,
        tool_registry::UpdateToolBindingBody,
        tool_registry::RunToolTestBody,
        tool_registry::ToolMutationView,
        tool_registry::ToolServerView,
        tool_registry::ToolServerListView,
        tool_registry::ToolServerVersionView,
        tool_registry::ToolServerVersionListView,
        tool_registry::ToolBindingView,
        tool_registry::ToolBindingListView,
        tool_registry::ToolVersionDiffView,
        tool_registry::ToolClientConfigurationView,
        tool_registry::ToolConfigurationBindingView,
        tool_registry::ToolTestRunView,
        tool_registry::ToolTestRunListView,
        access::MemberView,
        access::MemberList,
        access::GroupRefView,
        access::GroupView,
        access::GroupList,
        access::GrantView,
        access::GrantList,
        access::InviteView,
        access::InviteList,
        access::CreatedInviteView,
        access::AcceptedInviteView,
        access::CreateInviteBody,
        access::GrantSubjectBody,
        access::CreateGroupBody,
        access::UpdateGroupBody,
    )),
    tags(
        (name = "me", description = "The caller, their tenant and their onboarding state"),
        (name = "workspaces", description = "Collaboration spaces, each owning a governed scope"),
        (name = "projects", description = "Units of work inside a workspace"),
        (name = "repositories", description = "What a project is about, by canonical identity"),
        (name = "access", description = "Who may act where: members, groups, grants and invitations"),
        (name = "sessions", description = "Agent runs, their immutable event ledger, and the context composed for them"),
        (name = "knowledge", description = "Stable governed Knowledge, immutable revisions, provenance and lifecycle"),
        (name = "capture", description = "Session evidence extraction into reviewable Knowledge candidates"),
        (name = "context", description = "Explainable Knowledge planning, scoped query and explicit feedback"),
        (name = "proposals", description = "VedaFlow proposals, review evidence, and governed effects"),
        (name = "channels", description = "Immutable authored-artifact channel state, publication, rollback and pins"),
        (name = "prompts", description = "Governed prompt drafts and policy-visible resolution"),
        (name = "context-packs", description = "Governed authored context-pack documents"),
        (name = "policy", description = "Cedar policy packs and curator requirements"),
        (name = "configuration", description = "Immutable governed runtime documents and revisioned scope bindings"),
        (name = "policy-relaxations", description = "Time-bounded governed policy relaxations"),
        (name = "audit", description = "Hash-chained audit query and verification"),
        (name = "quarantine", description = "Redacted event quarantine and governed review"),
        (name = "directory", description = "Directory synchronisation controls and provisioning credential metadata"),
        (name = "service-identities", description = "Headless principal registration and revocation"),
        (name = "skills", description = "Agent Skills-compatible immutable versions, governed bindings, usage and controlled tests"),
        (name = "tools", description = "Trusted MCP server metadata, immutable quarantined versions, exact project bindings and read-only evidence"),
    ),
    modifiers(&BearerAuth),
)]
pub struct ApiDoc;

/// Declares the bearer scheme the whole surface uses.
///
/// A modifier rather than an annotation because `utoipa` has no way to say
/// "every path" declaratively, and repeating the scheme on twelve handlers is
/// twelve chances to leave one off — which would document an authenticated
/// route as an open one.
struct BearerAuth;

impl utoipa::Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .description(Some(
                        "An OIDC access token from `synveda login`, or the console session \
                         cookie the gateway sets on its own origin.",
                    ))
                    .build(),
            ),
        );
    }
}

/// The document, rendered as the committed file's exact bytes.
///
/// Pretty-printed with a trailing newline, because it is a file in a
/// repository that people read in diffs — a one-line JSON document would make
/// every contract change a single unreviewable line.
///
/// # Panics
///
/// If the derived document does not serialise, which is a `utoipa` bug rather
/// than a runtime condition.
#[must_use]
pub fn document() -> String {
    let mut json = ApiDoc::openapi()
        .to_pretty_json()
        .expect("the derived OpenAPI document serialises");
    json.push('\n');
    json
}

/// Every path the document declares, in the order it declares them. The
/// route-parity test reads this rather than re-parsing the JSON.
#[must_use]
pub fn declared_paths() -> Vec<String> {
    ApiDoc::openapi()
        .paths
        .paths
        .keys()
        .cloned()
        .collect::<Vec<_>>()
}

/// Every method/path pair declared by the generated application contract.
///
/// Kept as strings so the contract test compares this inventory directly
/// with [`crate::routes::OPERATIONS`] without depending on Utoipa's internal
/// path-item representation.
#[must_use]
pub fn declared_operations() -> Vec<(String, String)> {
    const METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];
    let value: serde_json::Value =
        serde_json::from_str(&document()).expect("the generated OpenAPI document parses as JSON");
    let mut operations = Vec::new();
    for (path, item) in value["paths"]
        .as_object()
        .expect("the generated document has a paths object")
    {
        for method in METHODS {
            if item.get(*method).is_some() {
                operations.push((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    operations.sort();
    operations
}
