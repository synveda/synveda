//! The authenticated application route catalogue (ADR-0088).
//!
//! One declaration below builds both the Axum router and the exact
//! method/path inventory compared with OpenAPI. Adding a handler without a
//! contract annotation, or documenting an operation the gateway does not
//! mount, therefore fails the same no-database test.

use axum::Router;
use axum::routing::MethodRouter;

use crate::app::AppState;
use crate::limits::OperationClass;

/// One production application operation as exposed by the executable router.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Operation {
    /// Uppercase HTTP method.
    pub method: &'static str,
    /// Canonical OpenAPI path; wildcards omit Axum's star marker.
    pub path: &'static str,
}

/// Unauthenticated browser landings on `/v1` paths, mounted outside tenant
/// resolution by `landing_router`.
///
/// None is an API operation: each only redirects into the console, consumes
/// nothing and is therefore absent from OpenAPI. The one entry is the `GET`
/// (and implicit `HEAD`) beside `POST /v1/invites/{invite_token}/accept`, for
/// invitation links shared in the redemption operation's shape
/// (ADR-0072 amendment of 2026-10-17). A new entry is a reviewed exception to
/// "every `/v1` route is in the catalogue", not a convenience.
pub const BROWSER_LANDINGS: &[Operation] = &[Operation {
    method: "GET",
    path: "/v1/invites/{invite_token}/accept",
}];

/// The router for [`BROWSER_LANDINGS`].
pub(crate) fn landing_router() -> Router<AppState> {
    Router::new().route(
        "/v1/invites/{invite_token}/accept",
        axum::routing::get(crate::access::invite_landing),
    )
}

macro_rules! route_path {
    ($contract:literal) => {
        $contract
    };
    ($contract:literal @ $router:literal) => {
        $router
    };
}

// Use Axum's method-specific builders rather than `MethodFilter` directly.
// In particular, `.get()` retains Axum's ordinary implicit HEAD handling;
// moving routes into the catalogue must not narrow their HTTP behaviour.
macro_rules! add_method {
    ($router:expr, GET, $handler:path) => {
        $router.get($handler)
    };
    ($router:expr, POST, $handler:path) => {
        $router.post($handler)
    };
    ($router:expr, PATCH, $handler:path) => {
        $router.patch($handler)
    };
    ($router:expr, PUT, $handler:path) => {
        $router.put($handler)
    };
    ($router:expr, DELETE, $handler:path) => {
        $router.delete($handler)
    };
}

macro_rules! define_routes {
    (
        $(
            $contract:literal $( @ $router:literal )? => [
                $( $method:ident $handler:path as $class:ident ),+ $(,)?
            ]
        ),+ $(,)?
    ) => {
        /// Builds every bearer-authenticated production application route.
        pub(crate) fn router() -> Router<AppState> {
            let router = Router::new();
            $(
                let methods = MethodRouter::<AppState>::new();
                $(let methods = add_method!(methods, $method, $handler);)+
                let router = router.route(
                    route_path!($contract $( @ $router )?),
                    methods,
                );
            )+
            router
        }

        /// Exact method/path inventory generated from the router's source.
        pub const OPERATIONS: &[Operation] = &[
            $(
                $(Operation {
                    method: stringify!($method),
                    path: $contract,
                },)+
            )+
        ];

        /// Each operation's admission class under its contract path.
        pub(crate) const CLASSIFIED: &[(&str, &str, OperationClass)] = &[
            $(
                $((stringify!($method), $contract, OperationClass::$class),)+
            )+
        ];
    };
}

define_routes! {
    "/v1/whoami" => [GET crate::app::whoami as Read],
    "/v1/me" => [GET crate::me::get as Read],
    "/v1/me/console-sessions" => [GET crate::login_sessions::list as Read],
    "/v1/me/console-sessions/revoke-all" => [POST crate::login_sessions::revoke_all as Mutation],
    "/v1/me/console-sessions/{id}" => [DELETE crate::login_sessions::revoke as Mutation],
    "/v1/me/credential/revoke" => [POST crate::login_sessions::revoke_credential as Mutation],

    "/v1/knowledge" => [
        GET crate::knowledge_api::list as Search,
        POST crate::knowledge_api::create as Mutation,
    ],
    "/v1/knowledge/merge" => [POST crate::knowledge_api::merge as Mutation],
    "/v1/knowledge/{id}" => [
        GET crate::knowledge_api::get as Read,
        PATCH crate::knowledge_api::edit as Mutation,
        DELETE crate::knowledge_api::delete as Mutation,
    ],
    "/v1/knowledge/{id}/history" => [GET crate::knowledge_api::history as Read],
    "/v1/knowledge/{id}/sources" => [GET crate::knowledge_api::sources_for_item as Read],
    "/v1/knowledge/{id}/usage" => [GET crate::knowledge_api::usage as Read],
    "/v1/knowledge/{id}/verify" => [POST crate::knowledge_api::verify as Mutation],
    "/v1/knowledge/{id}/supersede" => [POST crate::knowledge_api::supersede as Mutation],
    "/v1/knowledge/{id}/archive" => [POST crate::knowledge_api::archive as Mutation],
    "/v1/knowledge/{id}/restore" => [POST crate::knowledge_api::restore as Mutation],
    "/v1/knowledge-conflicts" => [GET crate::knowledge_conflicts::list as Search],
    "/v1/knowledge-conflicts/{id}" => [GET crate::knowledge_conflicts::get as Read],
    "/v1/knowledge-conflicts/{id}/resolve" => [POST crate::knowledge_conflicts::resolve as Mutation],
    "/v1/knowledge-freshness-policies" => [GET crate::knowledge_conflicts::freshness_policies as Read],

    "/v1/workspaces" => [
        GET crate::workspaces::list as Read,
        POST crate::workspaces::create as Mutation,
    ],
    "/v1/workspaces/{workspace_id}" => [
        GET crate::workspaces::get as Read,
        PATCH crate::workspaces::update as Mutation,
    ],
    "/v1/workspaces/{workspace_id}/projects" => [
        GET crate::workspaces::list_projects as Read,
        POST crate::workspaces::create_project as Mutation,
    ],
    "/v1/projects/{project_id}" => [
        GET crate::workspaces::get_project as Read,
        PATCH crate::workspaces::update_project as Mutation,
    ],
    "/v1/projects/{project_id}/repositories" => [
        GET crate::workspaces::list_repositories as Read,
        POST crate::workspaces::attach_repository as Mutation,
    ],
    "/v1/projects/{project_id}/repositories/{repository_id}" => [DELETE crate::workspaces::detach_repository as Mutation],

    "/v1/projects/{project_id}/sources" => [GET crate::source_reconciliation::list as Read],
    "/v1/projects/{project_id}/sources/{source_id}" => [GET crate::source_reconciliation::get as Read],
    "/v1/projects/{project_id}/sources/{source_id}/refresh" => [POST crate::source_reconciliation::refresh as Bulk],
    "/v1/projects/{project_id}/okf/imports" => [POST crate::okf::plan_import as Bulk],
    "/v1/okf/imports" => [GET crate::okf::list_imports as Read],
    "/v1/okf/imports/{id}" => [GET crate::okf::get_import as Read],
    "/v1/okf/imports/{id}/artifacts/{artifact_id}/withdraw" => [POST crate::artifact_control::withdraw as Mutation],
    "/v1/okf/imports/{id}/artifacts/{artifact_id}/forget" => [POST crate::artifact_control::forget as Mutation],
    "/v1/okf/imports/{id}/artifacts/{artifact_id}/source-control" => [GET crate::artifact_control::get as Read],
    "/v1/okf/imports/{id}/materialize" => [POST crate::okf::materialize_import as Bulk],
    "/v1/projects/{project_id}/okf/exports" => [POST crate::okf::export as Bulk],

    "/v1/sessions" => [
        GET crate::sessions::list as Read,
        POST crate::sessions::open as SessionIngest,
    ],
    "/v1/sessions/{session_id}" => [GET crate::sessions::get as Read],
    "/v1/sessions/{session_id}/events" => [POST crate::sessions::append_events as SessionIngest],
    "/v1/sessions/{session_id}/outcomes" => [POST crate::procedure_learning::outcome as SessionIngest],
    "/v1/sessions/{session_id}/procedure-improvements" => [POST crate::procedure_learning::improve as ContextCompose],
    "/v1/capture-candidates/{candidate_id}/procedure-evaluations" => [POST crate::procedure_learning::freeze as Mutation],
    "/v1/capture-candidates/{candidate_id}/procedure-submit" => [POST crate::procedure_learning::submit as Mutation],
    "/v1/procedure-evaluations/{attempt_id}" => [GET crate::procedure_learning::get as Read],
    "/v1/procedure-evaluations/{attempt_id}/results" => [POST crate::procedure_learning::results as Mutation],
    "/v1/sessions/{session_id}/task-state" => [POST crate::sessions::task_state::derive as ContextCompose],
    "/v1/sessions/{session_id}/events/{event_id}" => [GET crate::sessions::get_event as Read],
    "/v1/sessions/{session_id}/events/{event_id}/withdraw" => [POST crate::source_control::withdraw as Mutation],
    "/v1/sessions/{session_id}/events/{event_id}/forget" => [POST crate::source_control::forget as Mutation],
    "/v1/sessions/{session_id}/events/{event_id}/source-control" => [GET crate::source_control::get as Read],
    "/v1/sessions/{session_id}/end" => [POST crate::sessions::end as SessionIngest],
    "/v1/sessions/{session_id}/timeline" => [GET crate::sessions::timeline as Read],
    "/v1/sessions/{session_id}/context-runs" => [POST crate::context_api::create_context_run as ContextCompose],
    "/v1/sessions/{session_id}/context-preview" => [POST crate::context_api::preview_context as ContextCompose],
    "/v1/sessions/{session_id}/knowledge-query" => [POST crate::context_api::knowledge_query as ContextCompose],
    "/v1/sessions/{session_id}/knowledge-evaluation" => [POST crate::context_api::knowledge_evaluation as ContextCompose],
    "/v1/context-runs" => [GET crate::context_api::list as Read],
    "/v1/context-runs/{id}" => [GET crate::context_api::get as Read],
    "/v1/context-runs/{id}/feedback" => [POST crate::context_api::feedback as SessionIngest],

    "/v1/sessions/{session_id}/capture-batches" => [POST crate::capture::create_batch as Mutation],
    "/v1/capture-batches" => [GET crate::capture::list_batches as Read],
    "/v1/capture-batches/{id}" => [GET crate::capture::get_batch as Read],
    "/v1/capture-batches/{id}/accept" => [POST crate::capture::accept_batch as Mutation],
    "/v1/capture-batches/{id}/retry" => [POST crate::capture::retry_batch as Bulk],
    "/v1/capture-candidates" => [GET crate::capture::list_candidates as Read],
    "/v1/capture-candidates/{id}/accept" => [POST crate::capture::accept_candidate as Mutation],
    "/v1/capture-candidates/{id}/merge" => [POST crate::capture::merge_candidate as Mutation],
    "/v1/capture-candidates/{id}/replace" => [POST crate::capture::replace_candidate as Mutation],
    "/v1/capture-candidates/{id}/dismiss" => [POST crate::capture::dismiss_candidate as Mutation],

    "/v1/workspaces/{workspace_id}/members" => [GET crate::access::list_workspace_members as Read],
    "/v1/workspaces/{workspace_id}/invites" => [
        GET crate::access::list_invites as Read,
        POST crate::access::create_invite as Mutation,
    ],
    "/v1/workspaces/{workspace_id}/invites/{invite_id}" => [DELETE crate::access::revoke_invite as Mutation],
    "/v1/invites/{invite_token}/accept" => [POST crate::access::accept_invite as Mutation],
    "/v1/projects/{project_id}/members" => [
        GET crate::access::list_project_members as Read,
        POST crate::access::add_project_member as Mutation,
    ],
    "/v1/projects/{project_id}/members/{principal_id}" => [DELETE crate::access::remove_project_member as Mutation],
    "/v1/admin/groups" => [
        GET crate::access::list_groups as Read,
        POST crate::access::create_group as Mutation,
    ],
    "/v1/admin/groups/{group_id}" => [PATCH crate::access::update_group as Mutation],
    "/v1/admin/grants" => [
        GET crate::access::list_grants as Read,
        POST crate::access::create_grant as Mutation,
    ],
    "/v1/admin/grants/{grant_id}" => [DELETE crate::access::revoke_grant as Mutation],
    "/v1/directory/access-assignments" => [POST crate::access::create_directory_access_assignment as Mutation],
    "/v1/directory/access-assignments/{grant_id}" => [DELETE crate::access::revoke_directory_access_assignment as Mutation],

    "/v1/admin/scopes" => [
        GET crate::admin_scopes::list as Read,
        POST crate::admin_scopes::create as Mutation,
    ],
    "/v1/admin/scopes/{scope_id}" => [
        GET crate::admin_scopes::get as Read,
        PATCH crate::admin_scopes::update as Mutation,
    ],
    "/v1/admin/scopes/{scope_id}/ancestors" => [GET crate::admin_scopes::ancestors as Read],
    "/v1/admin/scopes/{scope_id}/descendants" => [GET crate::admin_scopes::descendants as Read],
    "/v1/capabilities" => [GET crate::capabilities::batch as Read],
    "/v1/policy/packs" => [GET crate::policy::packs as Read],
    "/v1/configuration-templates" => [GET crate::configuration::templates as Read],
    "/v1/configurations" => [
        GET crate::configuration::list as Read,
        POST crate::configuration::create as Mutation,
    ],
    "/v1/configurations/effective" => [GET crate::configuration::effective as Read],
    "/v1/configurations/{id}" => [GET crate::configuration::get as Read],
    "/v1/configurations/{id}/versions" => [
        GET crate::configuration::versions as Read,
        POST crate::configuration::publish as Mutation,
    ],
    "/v1/configurations/{id}/compare" => [GET crate::configuration::compare as Read],
    "/v1/configuration-bindings" => [
        GET crate::configuration::bindings as Read,
        POST crate::configuration::create_binding as Mutation,
    ],
    "/v1/configuration-bindings/{id}" => [PATCH crate::configuration::update_binding as Mutation],
    "/v1/configuration-bindings/{id}/rollback" => [POST crate::configuration::rollback_binding as Mutation],

    "/v1/quarantine" => [GET crate::quarantine::list as Read],
    "/v1/quarantine/{event_id}/release" => [POST crate::quarantine::release as Mutation],
    "/v1/quarantine/{event_id}/reject" => [POST crate::quarantine::reject as Mutation],
    "/v1/audit/events" => [GET crate::audit_query::events as Search],
    "/v1/audit/export" => [GET crate::audit_query::export as Bulk],
    "/v1/audit/disclosures" => [GET crate::audit_query::disclosures as Search],
    "/v1/audit/knowledge" => [GET crate::audit_query::knowledge as Search],
    "/v1/audit/verify" => [GET crate::audit_query::verify as Bulk],

    "/v1/channels/{scope_id}" => [GET crate::channels::list as Read],
    "/v1/channels/{scope_id}/publish" => [POST crate::channels::publish as Mutation],
    "/v1/channels/{scope_id}/git-export" => [POST crate::git_bridge::export as Bulk],
    "/v1/channels/{scope_id}/history" => [GET crate::channels::history as Read],
    "/v1/channels/{scope_id}/rollback" => [POST crate::channels::rollback as Mutation],
    "/v1/channels/{scope_id}/pin" => [POST crate::channels::pin as Mutation],
    "/v1/channels/{scope_id}/unpin" => [POST crate::channels::unpin as Mutation],
    "/v1/proposals" => [
        GET crate::proposals::list as Read,
        POST crate::proposals::open as Mutation,
    ],
    "/v1/proposals/{id}" => [GET crate::proposals::get as Read],
    "/v1/proposals/{id}/approve" => [POST crate::proposals::approve as Mutation],
    "/v1/proposals/{id}/reject" => [POST crate::proposals::reject as Mutation],
    "/v1/proposals/{id}/withdraw" => [POST crate::proposals::withdraw as Mutation],
    "/v1/proposals/{id}/publish" => [POST crate::proposals::publish as Mutation],
    "/v1/proposals/{id}/apply" => [POST crate::proposals::apply as Mutation],
    "/v1/prompts" => [
        GET crate::prompts::list as Read,
        POST crate::prompts::author as Mutation,
    ],
    "/v1/prompts/{name}" @ "/v1/prompts/{*name}" => [GET crate::prompts::resolve as Read],
    "/v1/context-packs" => [
        GET crate::packs::list as Read,
        POST crate::packs::author as Mutation,
    ],

    "/v1/skills" => [
        GET crate::skills::list as Read,
        POST crate::skills::install as Mutation,
    ],
    "/v1/skills/available" => [GET crate::skills::available as Read],
    "/v1/skills/{id}" => [
        GET crate::skills::get as Read,
        PATCH crate::skills::update as Mutation,
    ],
    "/v1/skills/{id}/versions" => [GET crate::skills::list_versions as Read],
    "/v1/skills/{id}/versions/{version_id}" => [GET crate::skills::get_version as Read],
    "/v1/skills/{id}/versions/{version_id}/files" => [GET crate::skills::list_files as Read],
    "/v1/skills/{id}/versions/{version_id}/files/{path}" @ "/v1/skills/{id}/versions/{version_id}/files/{*path}" => [GET crate::skills::get_file as Read],
    "/v1/skills/{id}/versions/{version_id}/usage" => [GET crate::skills::list_usage as Read],
    "/v1/skills/{id}/versions/{version_id}/tests" => [
        GET crate::skills::list_tests as Read,
        POST crate::skills::run_test as Bulk,
    ],
    "/v1/skills/{id}/versions/{version_id}/validation-operations" => [POST crate::operations::create_skill_validation as Bulk],
    "/v1/operations" => [GET crate::operations::list as Read],
    "/v1/operations/{id}" => [GET crate::operations::get as Read],
    "/v1/operations/{id}/cancel" => [POST crate::operations::cancel as Mutation],
    "/v1/skill-bindings" => [
        GET crate::skills::list_bindings as Read,
        POST crate::skills::create_binding as Mutation,
    ],
    "/v1/skill-bindings/{id}" => [
        GET crate::skills::get_binding as Read,
        PATCH crate::skills::update_binding as Mutation,
    ],
    "/v1/skill-bindings/{id}/rollback" => [POST crate::skills::rollback_binding as Mutation],
    "/v1/skill-usage" => [POST crate::skills::record_usage as SessionIngest],

    "/v1/tool-servers/import-client-config" => [POST crate::tool_registry::import_client_config as Mutation],
    "/v1/tool-servers" => [
        GET crate::tool_registry::list as Read,
        POST crate::tool_registry::register as Mutation,
    ],
    "/v1/tool-servers/{id}" => [
        GET crate::tool_registry::get as Read,
        PATCH crate::tool_registry::stage_version as Mutation,
    ],
    "/v1/tool-servers/{id}/discoveries" => [POST crate::tool_registry::discover as Bulk],
    "/v1/tool-servers/{id}/versions" => [GET crate::tool_registry::list_versions as Read],
    "/v1/tool-servers/{id}/versions/{version_id}" => [GET crate::tool_registry::get_version as Read],
    "/v1/tool-servers/{id}/versions/{version_id}/diff" => [GET crate::tool_registry::diff as Read],
    "/v1/tool-servers/{id}/versions/{version_id}/tests" => [
        GET crate::tool_registry::list_tests as Read,
        POST crate::tool_registry::run_test as Bulk,
    ],
    "/v1/tool-bindings" => [
        GET crate::tool_registry::list_bindings as Read,
        POST crate::tool_registry::create_binding as Mutation,
    ],
    "/v1/tool-bindings/{id}" => [
        GET crate::tool_registry::get_binding as Read,
        PATCH crate::tool_registry::update_binding as Mutation,
    ],
    "/v1/projects/{project_id}/tool-config" => [GET crate::tool_registry::generate_config as Read],

    "/v1/relaxations" => [
        GET crate::relaxations::list as Read,
        POST crate::relaxations::create as Mutation,
    ],
    "/v1/relaxations/{id}" => [
        GET crate::relaxations::get as Read,
        PATCH crate::relaxations::revise as Mutation,
    ],
    "/v1/relaxations/{id}/versions" => [GET crate::relaxations::versions as Read],
    "/v1/relaxations/{id}/revoke" => [POST crate::relaxations::revoke as Mutation],
    "/v1/admin/scopes/{scope_id}/curators" => [
        GET crate::curators::get as Read,
        PUT crate::curators::put as Mutation,
    ],
    "/v1/service-identities" => [
        GET crate::service_identities::list as Read,
        POST crate::service_identities::register as Mutation,
    ],
    "/v1/service-identities/{id}" => [
        GET crate::service_identities::get as Read,
        DELETE crate::service_identities::remove as Mutation,
    ],
    "/v1/scim/credentials" => [
        GET crate::scim::credentials::list as Read,
        POST crate::scim::credentials::issue as Mutation,
    ],
    "/v1/scim/credentials/{id}/revoke" => [POST crate::scim::credentials::revoke as Mutation],
    "/v1/directory/sync" => [GET crate::directory_admin::status as Read],
    "/v1/usage" => [GET crate::usage::get as Read],
    "/v1/usage/budgets/{meter}" => [
        PUT crate::usage::set_budget as Mutation,
        DELETE crate::usage::clear_budget as Mutation,
    ],
    "/v1/directory/seal-authorisations" => [POST crate::directory_admin::authorise as Mutation],
}
