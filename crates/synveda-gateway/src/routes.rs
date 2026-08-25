//! The authenticated application route catalogue (CPR-29, ADR-0088).
//!
//! One declaration below builds both the Axum router and the exact
//! method/path inventory compared with OpenAPI. Adding a handler without a
//! contract annotation, or documenting an operation the gateway does not
//! mount, therefore fails the same no-database test.

use axum::Router;
use axum::routing::MethodRouter;

use crate::app::AppState;

/// One production application operation as exposed by the executable router.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Operation {
    /// Uppercase HTTP method.
    pub method: &'static str,
    /// Canonical OpenAPI path; wildcards omit Axum's star marker.
    pub path: &'static str,
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
                $( $method:ident $handler:path ),+ $(,)?
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
    };
}

define_routes! {
    "/v1/whoami" => [GET crate::app::whoami],
    "/v1/me" => [GET crate::me::get],

    "/v1/knowledge" => [
        GET crate::knowledge_api::list,
        POST crate::knowledge_api::create,
    ],
    "/v1/knowledge/merge" => [POST crate::knowledge_api::merge],
    "/v1/knowledge/{id}" => [
        GET crate::knowledge_api::get,
        PATCH crate::knowledge_api::edit,
        DELETE crate::knowledge_api::delete,
    ],
    "/v1/knowledge/{id}/history" => [GET crate::knowledge_api::history],
    "/v1/knowledge/{id}/sources" => [GET crate::knowledge_api::sources_for_item],
    "/v1/knowledge/{id}/usage" => [GET crate::knowledge_api::usage],
    "/v1/knowledge/{id}/verify" => [POST crate::knowledge_api::verify],
    "/v1/knowledge/{id}/supersede" => [POST crate::knowledge_api::supersede],
    "/v1/knowledge/{id}/archive" => [POST crate::knowledge_api::archive],
    "/v1/knowledge/{id}/restore" => [POST crate::knowledge_api::restore],

    "/v1/workspaces" => [
        GET crate::workspaces::list,
        POST crate::workspaces::create,
    ],
    "/v1/workspaces/{workspace_id}" => [
        GET crate::workspaces::get,
        PATCH crate::workspaces::update,
    ],
    "/v1/workspaces/{workspace_id}/projects" => [
        GET crate::workspaces::list_projects,
        POST crate::workspaces::create_project,
    ],
    "/v1/projects/{project_id}" => [
        GET crate::workspaces::get_project,
        PATCH crate::workspaces::update_project,
    ],
    "/v1/projects/{project_id}/repositories" => [
        GET crate::workspaces::list_repositories,
        POST crate::workspaces::attach_repository,
    ],
    "/v1/projects/{project_id}/repositories/{repository_id}" => [DELETE crate::workspaces::detach_repository],

    "/v1/projects/{project_id}/okf/imports" => [POST crate::okf::plan_import],
    "/v1/okf/imports" => [GET crate::okf::list_imports],
    "/v1/okf/imports/{id}" => [GET crate::okf::get_import],
    "/v1/okf/imports/{id}/materialize" => [POST crate::okf::materialize_import],
    "/v1/projects/{project_id}/okf/exports" => [POST crate::okf::export],

    "/v1/sessions" => [
        GET crate::sessions::list,
        POST crate::sessions::open,
    ],
    "/v1/sessions/{session_id}" => [GET crate::sessions::get],
    "/v1/sessions/{session_id}/events" => [POST crate::sessions::append_events],
    "/v1/sessions/{session_id}/events/{event_id}" => [GET crate::sessions::get_event],
    "/v1/sessions/{session_id}/end" => [POST crate::sessions::end],
    "/v1/sessions/{session_id}/timeline" => [GET crate::sessions::timeline],
    "/v1/sessions/{session_id}/context-runs" => [POST crate::context_api::create_context_run],
    "/v1/sessions/{session_id}/knowledge-query" => [POST crate::context_api::knowledge_query],
    "/v1/sessions/{session_id}/knowledge-evaluation" => [POST crate::context_api::knowledge_evaluation],
    "/v1/context-runs" => [GET crate::context_api::list],
    "/v1/context-runs/{id}" => [GET crate::context_api::get],
    "/v1/context-runs/{id}/feedback" => [POST crate::context_api::feedback],

    "/v1/sessions/{session_id}/capture-batches" => [POST crate::capture::create_batch],
    "/v1/capture-batches" => [GET crate::capture::list_batches],
    "/v1/capture-batches/{id}" => [GET crate::capture::get_batch],
    "/v1/capture-batches/{id}/accept" => [POST crate::capture::accept_batch],
    "/v1/capture-candidates" => [GET crate::capture::list_candidates],
    "/v1/capture-candidates/{id}/accept" => [POST crate::capture::accept_candidate],
    "/v1/capture-candidates/{id}/merge" => [POST crate::capture::merge_candidate],
    "/v1/capture-candidates/{id}/replace" => [POST crate::capture::replace_candidate],
    "/v1/capture-candidates/{id}/dismiss" => [POST crate::capture::dismiss_candidate],

    "/v1/workspaces/{workspace_id}/members" => [GET crate::access::list_workspace_members],
    "/v1/workspaces/{workspace_id}/invites" => [
        GET crate::access::list_invites,
        POST crate::access::create_invite,
    ],
    "/v1/workspaces/{workspace_id}/invites/{invite_id}" => [DELETE crate::access::revoke_invite],
    "/v1/invites/{invite_token}/accept" => [POST crate::access::accept_invite],
    "/v1/projects/{project_id}/members" => [
        GET crate::access::list_project_members,
        POST crate::access::add_project_member,
    ],
    "/v1/projects/{project_id}/members/{principal_id}" => [DELETE crate::access::remove_project_member],
    "/v1/admin/groups" => [
        GET crate::access::list_groups,
        POST crate::access::create_group,
    ],
    "/v1/admin/groups/{group_id}" => [PATCH crate::access::update_group],
    "/v1/admin/grants" => [
        GET crate::access::list_grants,
        POST crate::access::create_grant,
    ],
    "/v1/admin/grants/{grant_id}" => [DELETE crate::access::revoke_grant],

    "/v1/admin/scopes" => [
        GET crate::admin_scopes::list,
        POST crate::admin_scopes::create,
    ],
    "/v1/admin/scopes/{scope_id}" => [
        GET crate::admin_scopes::get,
        PATCH crate::admin_scopes::update,
    ],
    "/v1/admin/scopes/{scope_id}/ancestors" => [GET crate::admin_scopes::ancestors],
    "/v1/admin/scopes/{scope_id}/descendants" => [GET crate::admin_scopes::descendants],
    "/v1/capabilities" => [GET crate::capabilities::batch],
    "/v1/policy/packs" => [GET crate::policy::packs],
    "/v1/configuration-templates" => [GET crate::configuration::templates],
    "/v1/configurations" => [
        GET crate::configuration::list,
        POST crate::configuration::create,
    ],
    "/v1/configurations/effective" => [GET crate::configuration::effective],
    "/v1/configurations/{id}" => [GET crate::configuration::get],
    "/v1/configurations/{id}/versions" => [
        GET crate::configuration::versions,
        POST crate::configuration::publish,
    ],
    "/v1/configurations/{id}/compare" => [GET crate::configuration::compare],
    "/v1/configuration-bindings" => [
        GET crate::configuration::bindings,
        POST crate::configuration::create_binding,
    ],
    "/v1/configuration-bindings/{id}" => [PATCH crate::configuration::update_binding],
    "/v1/configuration-bindings/{id}/rollback" => [POST crate::configuration::rollback_binding],

    "/v1/quarantine" => [GET crate::quarantine::list],
    "/v1/quarantine/{event_id}/release" => [POST crate::quarantine::release],
    "/v1/quarantine/{event_id}/reject" => [POST crate::quarantine::reject],
    "/v1/audit/events" => [GET crate::audit_query::events],
    "/v1/audit/disclosures" => [GET crate::audit_query::disclosures],
    "/v1/audit/knowledge" => [GET crate::audit_query::knowledge],
    "/v1/audit/verify" => [GET crate::audit_query::verify],

    "/v1/channels/{scope_id}" => [GET crate::channels::list],
    "/v1/channels/{scope_id}/publish" => [POST crate::channels::publish],
    "/v1/channels/{scope_id}/history" => [GET crate::channels::history],
    "/v1/channels/{scope_id}/rollback" => [POST crate::channels::rollback],
    "/v1/channels/{scope_id}/pin" => [POST crate::channels::pin],
    "/v1/channels/{scope_id}/unpin" => [POST crate::channels::unpin],
    "/v1/proposals" => [
        GET crate::proposals::list,
        POST crate::proposals::open,
    ],
    "/v1/proposals/{id}" => [GET crate::proposals::get],
    "/v1/proposals/{id}/approve" => [POST crate::proposals::approve],
    "/v1/proposals/{id}/reject" => [POST crate::proposals::reject],
    "/v1/proposals/{id}/withdraw" => [POST crate::proposals::withdraw],
    "/v1/proposals/{id}/publish" => [POST crate::proposals::publish],
    "/v1/proposals/{id}/apply" => [POST crate::proposals::apply],
    "/v1/prompts" => [
        GET crate::prompts::list,
        POST crate::prompts::author,
    ],
    "/v1/prompts/{name}" @ "/v1/prompts/{*name}" => [GET crate::prompts::resolve],
    "/v1/context-packs" => [
        GET crate::packs::list,
        POST crate::packs::author,
    ],

    "/v1/skills" => [
        GET crate::skills::list,
        POST crate::skills::install,
    ],
    "/v1/skills/available" => [GET crate::skills::available],
    "/v1/skills/{id}" => [
        GET crate::skills::get,
        PATCH crate::skills::update,
    ],
    "/v1/skills/{id}/versions" => [GET crate::skills::list_versions],
    "/v1/skills/{id}/versions/{version_id}" => [GET crate::skills::get_version],
    "/v1/skills/{id}/versions/{version_id}/files" => [GET crate::skills::list_files],
    "/v1/skills/{id}/versions/{version_id}/files/{path}" @ "/v1/skills/{id}/versions/{version_id}/files/{*path}" => [GET crate::skills::get_file],
    "/v1/skills/{id}/versions/{version_id}/usage" => [GET crate::skills::list_usage],
    "/v1/skills/{id}/versions/{version_id}/tests" => [
        GET crate::skills::list_tests,
        POST crate::skills::run_test,
    ],
    "/v1/skill-bindings" => [
        GET crate::skills::list_bindings,
        POST crate::skills::create_binding,
    ],
    "/v1/skill-bindings/{id}" => [
        GET crate::skills::get_binding,
        PATCH crate::skills::update_binding,
    ],
    "/v1/skill-bindings/{id}/rollback" => [POST crate::skills::rollback_binding],
    "/v1/skill-usage" => [POST crate::skills::record_usage],

    "/v1/tool-servers/import-client-config" => [POST crate::tool_registry::import_client_config],
    "/v1/tool-servers" => [
        GET crate::tool_registry::list,
        POST crate::tool_registry::register,
    ],
    "/v1/tool-servers/{id}" => [
        GET crate::tool_registry::get,
        PATCH crate::tool_registry::stage_version,
    ],
    "/v1/tool-servers/{id}/discoveries" => [POST crate::tool_registry::discover],
    "/v1/tool-servers/{id}/versions" => [GET crate::tool_registry::list_versions],
    "/v1/tool-servers/{id}/versions/{version_id}" => [GET crate::tool_registry::get_version],
    "/v1/tool-servers/{id}/versions/{version_id}/diff" => [GET crate::tool_registry::diff],
    "/v1/tool-servers/{id}/versions/{version_id}/tests" => [
        GET crate::tool_registry::list_tests,
        POST crate::tool_registry::run_test,
    ],
    "/v1/tool-bindings" => [
        GET crate::tool_registry::list_bindings,
        POST crate::tool_registry::create_binding,
    ],
    "/v1/tool-bindings/{id}" => [
        GET crate::tool_registry::get_binding,
        PATCH crate::tool_registry::update_binding,
    ],
    "/v1/projects/{project_id}/tool-config" => [GET crate::tool_registry::generate_config],

    "/v1/relaxations" => [
        GET crate::relaxations::list,
        POST crate::relaxations::create,
    ],
    "/v1/relaxations/{id}" => [
        GET crate::relaxations::get,
        PATCH crate::relaxations::revise,
    ],
    "/v1/relaxations/{id}/versions" => [GET crate::relaxations::versions],
    "/v1/relaxations/{id}/revoke" => [POST crate::relaxations::revoke],
    "/v1/admin/scopes/{scope_id}/curators" => [
        GET crate::curators::get,
        PUT crate::curators::put,
    ],
    "/v1/service-identities" => [
        GET crate::service_identities::list,
        POST crate::service_identities::register,
    ],
    "/v1/service-identities/{id}" => [
        GET crate::service_identities::get,
        DELETE crate::service_identities::remove,
    ],
    "/v1/scim/credentials" => [
        GET crate::scim::credentials::list,
        POST crate::scim::credentials::issue,
    ],
    "/v1/scim/credentials/{id}/revoke" => [POST crate::scim::credentials::revoke],
    "/v1/directory/sync" => [GET crate::directory_admin::status],
    "/v1/directory/seal-authorisations" => [POST crate::directory_admin::authorise],
}
