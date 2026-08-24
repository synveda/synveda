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
//! # It covers this plane and says so
//!
//! The document describes the workspace, project, repository, access,
//! `/v1/me` and scope-admin routes — the surface CPR-4, CPR-5 and CPR-7
//! add — and **not** the `/v1` paths that predate it. That is a bounded
//! start rather than a bounded ambition:
//! Prompt 19 of the context-platform programme owns bringing the whole surface
//! onto the contract, and the alternative here was annotating fifty-four
//! handlers this programme is about to delete or re-cut. The document's own
//! description says which routes it covers, so nobody reads its silence about
//! `/v1/observe` as a claim that the route does not exist.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::access;
use crate::capture;
use crate::context_api;
use crate::knowledge_api;
use crate::me;
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

**Coverage.** This document describes the context-platform surface: `/v1/me`, \
the workspace, project and repository planes (CPR-4), the access plane — \
members, groups, grants and invitations (CPR-5), and the scope admin plane \
(CPR-7: `/v1/admin/scopes` — list, create, get, patch, ancestors, descendants), \
and the session ledger and runtime API (CPR-10: `/v1/sessions` — open, list, get, \
append events, end, timeline, and the context-run endpoint), with CPR-11's \
paginated and filtered listing and its diagnostic expansion of one event. \
CPR-17 adds stable Knowledge, immutable revision history, independently \
governed provenance and lifecycle mutations at `/v1/knowledge`; every write \
is an idempotent VedaFlow change and the collection is current-active by \
default with lexical and honestly degraded semantic search. \
CPR-18 adds `/v1/capture-batches` and `/v1/capture-candidates`: explicit or \
session-end extraction freezes exact event evidence and produces reviewable \
candidates only. Accept, edit, merge and replace enter the same Knowledge \
VedaFlow command layer; dismissal publishes nothing. \
CPR-20 makes context runs explainable over current immutable Knowledge, adds \
re-authorised trace and feedback operations, and provides ordinary and \
diagnostic session-scoped Knowledge query lenses without a global recall. \
CPR-23 adds the Agent Skills-compatible catalogue: stable Skills, immutable \
versions and exact file content, revisioned project/principal bindings, \
version-specific host/model usage evidence, and controlled non-executing test \
runs. Install, update, bind, disable and rollback are typed VedaFlow changes; \
declared tools are metadata and never authorisation. \
CPR-25 adds the trusted MCP catalogue: stable servers, immutable source and \
capability snapshots pinned to MCP 2026-07-28, quarantined changed versions, \
exact project bindings, secret-reference-only configuration and immutable \
read-only connection-test evidence. Registration, version approval and every \
binding transition are typed VedaFlow changes; capability descriptions grant \
no invocation authority and the gateway is not an execution proxy. \
Since CPR-12 the session plane is also the **only** runtime plane: \
`POST /v1/sessions/{session_id}/events` is where observations are admitted and \
`POST /v1/sessions/{session_id}/context-runs` is where context is composed. \
The global `/v1/observe`, `/v1/inject` and `/v1/recall` routes that used to do \
both are deleted, not merely undocumented. \
The rest of the `/v1` surface — proposals, channels, the registries and the \
older admin planes — predates the OpenAPI contract and is brought onto it by a \
later prompt of the context-platform programme. Its absence here is a \
statement about this document, not about the gateway.

**Tenancy.** Every path below sits behind bearer authentication and tenant \
resolution. A response is always scoped to the caller's tenant, which is why \
no response body echoes a tenant id.

**Idempotency.** Every creation takes a required `Idempotency-Key` header. \
Reusing a key with the same request replays the original resource with `200`; \
a fresh key creates and answers `201`; reusing a key with a *different* request \
is `409`. Appending session events is the one exception, and it is idempotent \
by a finer unit: each event carries the client's own `client_event_id`, unique \
within its session, so a redelivered batch appends what is new and reports \
`duplicate` for the rest.

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
