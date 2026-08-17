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
//! The document describes the workspace, project, repository and `/v1/me`
//! routes — the surface CPR-4 adds — and **not** the fifty-four `/v1` paths
//! that predate it. That is a bounded start rather than a bounded ambition:
//! Prompt 19 of the context-platform programme owns bringing the whole surface
//! onto the contract, and the alternative here was annotating fifty-four
//! handlers this programme is about to delete or re-cut. The document's own
//! description says which routes it covers, so nobody reads its silence about
//! `/v1/observe` as a claim that the route does not exist.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::me;
use crate::workspaces;

/// The document root.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Synveda",
        description = "\
Governed organisational memory and context for AI agents.

**Coverage.** This document describes the context-platform surface added by \
CPR-4: `/v1/me`, and the workspace, project and repository planes. The rest of \
the `/v1` surface — observe, inject, recall, proposals, channels, the \
registries and the admin planes — predates the OpenAPI contract and is brought \
onto it by a later prompt of the context-platform programme. Its absence here \
is a statement about this document, not about the gateway.

**Tenancy.** Every path below sits behind bearer authentication and tenant \
resolution. A response is always scoped to the caller's tenant, which is why \
no response body echoes a tenant id.

**Idempotency.** Every creation takes a required `Idempotency-Key` header. \
Reusing a key with the same request replays the original resource with `200`; \
a fresh key creates and answers `201`; reusing a key with a *different* request \
is `409`.

**Preconditions.** Every update takes a required `expected_revision`. A \
mismatch is `409` and writes nothing.",
        version = env!("CARGO_PKG_VERSION"),
        license(name = "Proprietary"),
    ),
    servers((url = "/", description = "This gateway")),
    paths(
        me::get,
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
    ),
    components(schemas(
        me::MeView,
        me::PrincipalView,
        me::TenantView,
        me::OnboardingView,
        me::OnboardingState,
        crate::capabilities::TenantCapabilities,
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
    )),
    tags(
        (name = "me", description = "The caller, their tenant and their onboarding state"),
        (name = "workspaces", description = "Collaboration spaces, each owning a governed scope"),
        (name = "projects", description = "Units of work inside a workspace"),
        (name = "repositories", description = "What a project is about, by canonical identity"),
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
