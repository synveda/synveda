//! The gateway's HTTP surface: the unauthenticated ops plane (liveness,
//! readiness, Prometheus) and the authenticated `/v1` plane behind tenant
//! resolution (TEN-1). The three primitives land on `/v1` behind the full
//! AuthN → tenant → PDP → audit chain (CTX-3, MEM-1).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::service_identities;

use axum::Router;
use axum::extract::{MatchedPath, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post, put};
use metrics_exporter_prometheus::PrometheusHandle;
// The two extension traits W3C trace-context extraction needs: `.span()` on
// an extracted `Context`, and `.set_parent()` on the request span.
use opentelemetry::trace::TraceContextExt as _;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use synveda_identity::{LoginFlow, TokenVerifier};
use synveda_policy::Pdp;
use synveda_types::{Error, Tenant};
use tower_http::trace::TraceLayer;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::audit_query;
use crate::auth;
use crate::capabilities;
use crate::channels;
use crate::curators;
use crate::directory_admin;
use crate::error::ApiError;
use crate::packs;
use crate::policy;
use crate::prompts;
use crate::proposals;
use crate::quarantine;
use crate::skills;
use crate::telemetry::{HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL};
use crate::tenant;

/// Shared state for all routes.
#[derive(Clone)]
pub struct AppState {
    /// Lazily-connecting Postgres pool; `/readyz` surfaces connection
    /// failures rather than the process refusing to boot.
    pub pool: PgPool,
    /// Renders the Prometheus exposition for `GET /metrics`.
    pub metrics: PrometheusHandle,
    /// The AuthN seam (ADR-0008): the OIDC/JWKS verifier (ADR-0010), the
    /// HS256 dev verifier, or the fail-closed
    /// [`synveda_identity::DisabledVerifier`] when neither is configured.
    pub verifier: Arc<dyn TokenVerifier>,
    /// The code+PKCE login flow when OIDC is configured (AUTH-1); `None`
    /// otherwise, in which case `/auth/*` answers 404.
    pub login: Option<Arc<LoginFlow>>,
    /// This gateway's own origin (`scheme://host[:port]`), derived from
    /// `SYNVEDA_PUBLIC_URL`. The value a cookie-authenticated mutation's
    /// `Origin` header must equal (CNSL-1, ADR-0056 decision 4). Bearer
    /// requests never consult it.
    pub public_origin: String,
    /// The embedded PDP (AUTHZ-1, ADR-0012): handlers authorize through it
    /// before acting; the pack refresher hot-swaps stored packs into it.
    pub pdp: Arc<Pdp>,
    /// The service-token lifetime cap (AUTH-3, ADR-0018 decision 5):
    /// the enforcement seam refuses a service identity's token whose
    /// lifetime (`exp − iat`) is unknown or exceeds this.
    /// `SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS`, default 3600.
    pub service_token_max_ttl: Duration,
    /// The search-index sidecar (CTX-1, ADR-0024): the inject route's
    /// lexical leg (CTX-3, ADR-0026); the indexer task converges it.
    pub search_index: Arc<synveda_retrieval::SearchIndex>,
    /// The MEM-4 embedder seam (ADR-0023): the inject route's
    /// query-embedding call and the pipeline worker's record vectors
    /// share one config-declared model identity (ADR-0026 decision 3).
    pub embedder: Arc<synveda_ingest::embedding::AnyEmbedder>,
    /// The inject route's embed deadline (ADR-0026 decision 3):
    /// `SYNVEDA_INJECT_EMBED_TIMEOUT_MS`, default 100. Expiry drops the
    /// dense leg (sparse-only, marked degraded), never the request.
    pub inject_embed_timeout: Duration,
    /// The key plane (TEN-4, ADR-0064): materialises the sealing keys the
    /// console session columns and the per-tenant secrets need. Backed by
    /// `Kms::Disabled` when `SYNVEDA_KMS_KEY` is unset, in which case the
    /// surfaces that need a key fail closed and `/v1` is untouched.
    pub keys: Arc<synveda_store::keys::KeyRing>,
}

impl AppState {
    /// The one post-commit seam for every scope-tree mutation (ADR-0017
    /// decision 5, kept when the chain cache left with the hierarchy —
    /// CPR-7, ADR-0074): flushes the tenant's Cedar entity fragments, so
    /// the very next request re-reads committed truth. Every scope writer
    /// — the admin plane, provisioning, the directory sync — calls this.
    pub fn invalidate_scopes(&self, tenant_id: synveda_types::TenantId) {
        self.pdp.flush_entities(tenant_id);
    }

    /// Seals one of a console session's tokens under the **deployment** key
    /// (TEN-4, ADR-0064 decision 5).
    ///
    /// The deployment scope rather than a tenant one because the row this
    /// belongs to has no tenant, on purpose: a session is read before the
    /// tenant exists, so there is nothing to select a per-tenant key by. See
    /// migration 0034's header for why that column is absent and migration
    /// 0038's for why that makes a second key scope rather than an exemption.
    ///
    /// Bound to the session's own `token_hash`, so a sealed token moved to
    /// another session's row does not open.
    pub async fn seal_console_token(
        &self,
        token_hash: &[u8; 32],
        purpose: synveda_crypto::Purpose,
        token: &str,
    ) -> synveda_types::Result<Vec<u8>> {
        self.keys
            .sealing_key(&self.pool, synveda_crypto::KeyScope::Deployment)
            .await?
            .seal(
                purpose,
                synveda_crypto::RowKey::Hash(token_hash),
                token.as_bytes(),
            )
    }

    /// Opens one of a console session's sealed tokens.
    ///
    /// A failure here is corruption, a transplanted ciphertext, or a key that
    /// is gone — and it is counted rather than chained, because there is no
    /// tenant to chain it to. That is the same fact decision 5 turns on: this
    /// row is read before a tenant exists, and the audit chain is per-tenant
    /// (AUD-1). The metric carries the purpose, the log carries the error,
    /// and the caller turns it into the same uniform 401 an unknown session
    /// gets.
    pub async fn open_console_token(
        &self,
        token_hash: &[u8; 32],
        purpose: synveda_crypto::Purpose,
        sealed: &[u8],
    ) -> synveda_types::Result<String> {
        let opened = self
            .keys
            .opening_key(&self.pool, synveda_crypto::KeyScope::Deployment, sealed)
            .await?
            .open(purpose, synveda_crypto::RowKey::Hash(token_hash), sealed)
            .inspect_err(|error| {
                metrics::counter!(
                    synveda_store::keys::KEY_OPEN_FAILURES_TOTAL,
                    "scope" => "deployment",
                    "purpose" => purpose.as_str(),
                )
                .increment(1);
                tracing::warn!(
                    %error,
                    purpose = purpose.as_str(),
                    "a sealed console token did not open"
                );
            })?;
        String::from_utf8(opened.to_vec()).map_err(|_| synveda_types::Error::Internal {
            message: "a sealed console token opened to bytes that are not a token".to_owned(),
        })
    }
}

/// The console bundle's routes, nested under its prefix — or nothing at
/// all when no bundle is built (CNSL-1, ADR-0056 decision 1). Resolved
/// once per router build rather than per request: whether a directory
/// exists is not a question worth asking on the hot path, and a bundle
/// that appears while the process is running is a deployment nobody
/// performed.
fn console_routes() -> Router<AppState> {
    match crate::console::bundle_dir() {
        Some(dir) => {
            Router::new().nest_service(crate::console::CONSOLE_PREFIX, crate::console::router(&dir))
        }
        None => Router::new(),
    }
}

/// Builds the gateway router: ops-plane routes plus the authenticated `/v1`
/// plane, wrapped in the per-request trace span and HTTP metrics middleware.
pub fn router(state: AppState) -> Router {
    // Every /v1 route sits behind tenant resolution; ops routes do not.
    // The admin planes authorize every operation through the PDP inside
    // their handlers (AUTHZ-1, ADR-0012).
    let authenticated = Router::new()
        .route("/v1/whoami", get(whoami))
        // The context platform's own plane (CPR-4, ADR-0071). `/v1/me` is the
        // one call a client makes first: who, which tenant, what exists, what
        // is missing, and what this caller may do — so a client never has to
        // infer "is this person set up yet" from a 404.
        .route("/v1/me", get(crate::me::get))
        // Stable Knowledge and immutable revisions (CPR-17, ADR-0082). Every
        // mutation creates one idempotent VedaFlow change; every read decides
        // the exact item and independently filters sources/relations.
        .route(
            "/v1/knowledge",
            get(crate::knowledge_api::list).post(crate::knowledge_api::create),
        )
        .route("/v1/knowledge/merge", post(crate::knowledge_api::merge))
        .route(
            "/v1/knowledge/{id}",
            get(crate::knowledge_api::get)
                .patch(crate::knowledge_api::edit)
                .delete(crate::knowledge_api::delete),
        )
        .route(
            "/v1/knowledge/{id}/history",
            get(crate::knowledge_api::history),
        )
        .route(
            "/v1/knowledge/{id}/sources",
            get(crate::knowledge_api::sources_for_item),
        )
        .route("/v1/knowledge/{id}/usage", get(crate::knowledge_api::usage))
        .route(
            "/v1/knowledge/{id}/verify",
            post(crate::knowledge_api::verify),
        )
        .route(
            "/v1/knowledge/{id}/supersede",
            post(crate::knowledge_api::supersede),
        )
        .route(
            "/v1/knowledge/{id}/archive",
            post(crate::knowledge_api::archive),
        )
        .route(
            "/v1/knowledge/{id}/restore",
            post(crate::knowledge_api::restore),
        )
        // Workspaces and projects are product-level subtypes of a governed
        // scope: creating one creates its scope in the same transaction, and
        // there is no personal/team/enterprise variant of either row
        // (ADR-0068 decision 1). Creation takes a required `Idempotency-Key`;
        // update takes a required `expected_revision`.
        .route(
            "/v1/workspaces",
            get(crate::workspaces::list).post(crate::workspaces::create),
        )
        .route(
            "/v1/workspaces/{workspace_id}",
            get(crate::workspaces::get).patch(crate::workspaces::update),
        )
        .route(
            "/v1/workspaces/{workspace_id}/projects",
            get(crate::workspaces::list_projects).post(crate::workspaces::create_project),
        )
        .route(
            "/v1/projects/{project_id}",
            get(crate::workspaces::get_project).patch(crate::workspaces::update_project),
        )
        // What a project is *about*, addressed by canonical identity rather
        // than by where a checkout happens to sit — a filesystem path is
        // refused by name (ADR-0071 decision 4). There is no update verb:
        // re-pointing a project at a different repository is a detach and an
        // attach, so the chain records two acts.
        .route(
            "/v1/projects/{project_id}/repositories",
            get(crate::workspaces::list_repositories).post(crate::workspaces::attach_repository),
        )
        .route(
            "/v1/projects/{project_id}/repositories/{repository_id}",
            axum::routing::delete(crate::workspaces::detach_repository),
        )
        // The session ledger and runtime API (CPR-10, ADR-0076): what an agent
        // does, as governed records rather than a correlation string. Opening a
        // run and composing context for it take a required `Idempotency-Key`;
        // appending events is idempotent per **event**, by the client's own
        // `client_event_id`, because a redelivered batch that overlaps a
        // previous one must append what is new and say `duplicate` for the
        // rest. Nothing on this plane accepts a tenant or an acting principal
        // from a client — both come from the verified token.
        .route(
            "/v1/sessions",
            get(crate::sessions::list).post(crate::sessions::open),
        )
        .route("/v1/sessions/{session_id}", get(crate::sessions::get))
        .route(
            "/v1/sessions/{session_id}/events",
            post(crate::sessions::append_events),
        )
        // The diagnostic expansion (CPR-11): one event, payload included,
        // behind `session.diagnostics` rather than `session.read`.
        .route(
            "/v1/sessions/{session_id}/events/{event_id}",
            get(crate::sessions::get_event),
        )
        .route("/v1/sessions/{session_id}/end", post(crate::sessions::end))
        .route(
            "/v1/sessions/{session_id}/timeline",
            get(crate::sessions::timeline),
        )
        // Explainable Knowledge-backed context planning (CPR-20, ADR-0084).
        // The session POST remains the only delivery seam; inspection,
        // feedback and deep/evaluation query lenses stay session/project
        // scoped and re-authorise every exact Knowledge revision.
        .route(
            "/v1/sessions/{session_id}/context-runs",
            post(crate::context_api::create_context_run),
        )
        .route(
            "/v1/sessions/{session_id}/knowledge-query",
            post(crate::context_api::knowledge_query),
        )
        .route(
            "/v1/sessions/{session_id}/knowledge-evaluation",
            post(crate::context_api::knowledge_evaluation),
        )
        .route("/v1/context-runs", get(crate::context_api::list))
        .route("/v1/context-runs/{id}", get(crate::context_api::get))
        .route(
            "/v1/context-runs/{id}/feedback",
            post(crate::context_api::feedback),
        )
        // Extraction freezes immutable session evidence and stops at
        // reviewable candidates (CPR-18, ADR-0083). Accepted decisions enter
        // the same Knowledge/VedaFlow command seam as manual mutations.
        .route(
            "/v1/sessions/{session_id}/capture-batches",
            post(crate::capture::create_batch),
        )
        .route("/v1/capture-batches", get(crate::capture::list_batches))
        .route("/v1/capture-batches/{id}", get(crate::capture::get_batch))
        .route(
            "/v1/capture-batches/{id}/accept",
            post(crate::capture::accept_batch),
        )
        .route(
            "/v1/capture-candidates",
            get(crate::capture::list_candidates),
        )
        .route(
            "/v1/capture-candidates/{id}/accept",
            post(crate::capture::accept_candidate),
        )
        .route(
            "/v1/capture-candidates/{id}/merge",
            post(crate::capture::merge_candidate),
        )
        .route(
            "/v1/capture-candidates/{id}/replace",
            post(crate::capture::replace_candidate),
        )
        .route(
            "/v1/capture-candidates/{id}/dismiss",
            post(crate::capture::dismiss_candidate),
        )
        // Membership and access assignment (CPR-5, ADR-0072). One model for a
        // person working alone, four people sharing agent context, and a
        // company with a directory: a grant gives a principal or a group a
        // **role key** at a scope, and the scope's subtree inherits it — so a
        // workspace grant reaches its projects without a second row. What a
        // role key permits is the policy pack's; there is no permission table
        // here for it to disagree with.
        .route(
            "/v1/workspaces/{workspace_id}/members",
            get(crate::access::list_workspace_members),
        )
        .route(
            "/v1/workspaces/{workspace_id}/invites",
            get(crate::access::list_invites).post(crate::access::create_invite),
        )
        .route(
            "/v1/workspaces/{workspace_id}/invites/{invite_id}",
            axum::routing::delete(crate::access::revoke_invite),
        )
        // Redeeming an invitation. Behind the tenant middleware like every
        // `/v1` route — the token says which access to add, never who is
        // asking, so the recipient presents their own credential beside it.
        // The token is in the path, which is why `make_request_span` records
        // this route's *pattern* rather than its URI (see `SECRET_IN_PATH`).
        .route(
            "/v1/invites/{invite_token}/accept",
            post(crate::access::accept_invite),
        )
        .route(
            "/v1/projects/{project_id}/members",
            get(crate::access::list_project_members).post(crate::access::add_project_member),
        )
        .route(
            "/v1/projects/{project_id}/members/{principal_id}",
            axum::routing::delete(crate::access::remove_project_member),
        )
        .route(
            "/v1/admin/groups",
            get(crate::access::list_groups).post(crate::access::create_group),
        )
        .route(
            "/v1/admin/groups/{group_id}",
            axum::routing::patch(crate::access::update_group),
        )
        .route(
            "/v1/admin/grants",
            get(crate::access::list_grants).post(crate::access::create_grant),
        )
        .route(
            "/v1/admin/grants/{grant_id}",
            axum::routing::delete(crate::access::revoke_grant),
        )
        // The scope admin plane (CPR-7, ADR-0074 decision 5): one tree,
        // administered publicly. Creation is idempotent; a move is decided
        // at both ends; there is no delete — retiring a scope is a status
        // transition. The `/v1/hierarchy` routes this replaces are gone
        // with no alias: old clients get 404s and old scope kinds fail
        // validation by name, which is the pre-1.0 contract.
        .route(
            "/v1/admin/scopes",
            get(crate::admin_scopes::list).post(crate::admin_scopes::create),
        )
        .route(
            "/v1/admin/scopes/{scope_id}",
            get(crate::admin_scopes::get).patch(crate::admin_scopes::update),
        )
        .route(
            "/v1/admin/scopes/{scope_id}/ancestors",
            get(crate::admin_scopes::ancestors),
        )
        .route(
            "/v1/admin/scopes/{scope_id}/descendants",
            get(crate::admin_scopes::descendants),
        )
        // The capability probe (CNSL-2, ADR-0058): what the *caller* may
        // do, asked of the PDP. A forecast rather than a grant (decision
        // 2) — nothing downstream reads the answer to decide anything —
        // and it never takes a `subject`, so it cannot answer about a
        // third party (decision 3).
        .route("/v1/capabilities", get(capabilities::batch))
        // The policy admin plane (AUTHZ-2, ADR-0014 decision 8): the pack
        // registry and the tenant default, and — re-homed from the
        // hierarchy nodes it used to hang on — per-scope assignment.
        .route("/v1/policy/packs", get(policy::packs))
        .route(
            "/v1/policy/default",
            get(policy::get_default)
                .put(policy::set_default)
                .delete(policy::clear_default),
        )
        .route(
            "/v1/admin/scopes/{scope_id}/policy",
            put(policy::assign_scope_policy)
                .get(policy::get_scope_policy)
                .delete(policy::unassign_scope_policy),
        )
        // The quarantine review plane (MEM-2, ADR-0021 decisions 5–7),
        // re-anchored on session events by CPR-12 (ADR-0078 decision 4).
        .route("/v1/quarantine", get(quarantine::list))
        .route(
            "/v1/quarantine/{event_id}/release",
            post(quarantine::release),
        )
        .route("/v1/quarantine/{event_id}/reject", post(quarantine::reject))
        // The audit query plane (AUD-2, ADR-0045): one action, `AuditRead`,
        // decided at the tenant — there is no scope-resource variant, so
        // an audit answer covers the whole chain or is refused. The two
        // AC questions get one call each; `verify` is the chain check the
        // CLI has had since AUD-1, now reachable without DATABASE_URL.
        .route("/v1/audit/events", get(audit_query::events))
        .route("/v1/audit/disclosures", get(audit_query::disclosures))
        .route("/v1/audit/knowledge", get(audit_query::knowledge))
        .route("/v1/audit/verify", get(audit_query::verify))
        // The VedaFlow channel plane (FLOW-2, ADR-0031 decision 12):
        // reading a scope's standing authored-artifact channels, and
        // publishing immutable versions across the trust boundary. Since FLOW-3
        // the publish resolves the same approval matrix a proposal does,
        // satisfied by the acting principal alone (ADR-0032 decision 8).
        .route("/v1/channels/{scope_id}", get(channels::list))
        .route("/v1/channels/{scope_id}/publish", post(channels::publish))
        // Rollback and pinning (FLOW-7, ADR-0036). `history` is the
        // listing a rewind is chosen from and renders exactly the set the
        // rewind accepts; `pin` holds what the channel serves without
        // moving where it points, and is released by deleting it.
        .route("/v1/channels/{scope_id}/history", get(channels::history))
        .route("/v1/channels/{scope_id}/rollback", post(channels::rollback))
        .route("/v1/channels/{scope_id}/pin", post(channels::pin))
        .route("/v1/channels/{scope_id}/unpin", post(channels::unpin))
        // The VedaFlow proposal plane (FLOW-3, ADR-0032): the review in
        // front of a publication. Opening asks, approving counts, and
        // publishing runs the effect under `ChannelPublish` — approvals
        // go in front of that decision, they do not replace it.
        .route("/v1/proposals", get(proposals::list).post(proposals::open))
        .route("/v1/proposals/{id}", get(proposals::get))
        .route("/v1/proposals/{id}/approve", post(proposals::approve))
        .route("/v1/proposals/{id}/reject", post(proposals::reject))
        .route("/v1/proposals/{id}/withdraw", post(proposals::withdraw))
        .route("/v1/proposals/{id}/publish", post(proposals::publish))
        .route("/v1/proposals/{id}/apply", post(proposals::apply))
        // The prompt registry (PRMT-1, ADR-0049). Authoring writes a draft
        // and moves nothing a consumer reads; resolution walks the caller's
        // own placement chain nearest-first, or serves a named scope's
        // draft or a commit the caller pins. The wildcard is the path
        // shape of a prompt name (decision 3), and it sits *after* the
        // collection route so `GET /v1/prompts?scope_id=…` still lists.
        .route("/v1/prompts", get(prompts::list).post(prompts::author))
        .route("/v1/prompts/{*name}", get(prompts::resolve))
        // The context-pack registry (PRMT-2, ADR-0050). There is no
        // `GET /v1/context-packs/{name}` resolve route, and that is the
        // difference between the two authored asset types rather than an
        // omission: a prompt is fetched by name, and a pack's content
        // arrives through a context run as ranked pinned material.
        .route("/v1/context-packs", get(packs::list).post(packs::author))
        // Stable skills, immutable versions and explicit project/principal
        // bindings (CPR-23, ADR-0085). Mutations are typed VedaFlow apply
        // changes; file reads return exact version bytes and never execute
        // them in the gateway.
        .route("/v1/skills", get(skills::list).post(skills::install))
        .route("/v1/skills/available", get(skills::available))
        .route("/v1/skills/{id}", get(skills::get).patch(skills::update))
        .route("/v1/skills/{id}/versions", get(skills::list_versions))
        .route(
            "/v1/skills/{id}/versions/{version_id}",
            get(skills::get_version),
        )
        .route(
            "/v1/skills/{id}/versions/{version_id}/files",
            get(skills::list_files),
        )
        .route(
            "/v1/skills/{id}/versions/{version_id}/files/{*path}",
            get(skills::get_file),
        )
        .route(
            "/v1/skills/{id}/versions/{version_id}/usage",
            get(skills::list_usage),
        )
        .route(
            "/v1/skills/{id}/versions/{version_id}/tests",
            get(skills::list_tests).post(skills::run_test),
        )
        .route(
            "/v1/skill-bindings",
            get(skills::list_bindings).post(skills::create_binding),
        )
        .route(
            "/v1/skill-bindings/{id}",
            get(skills::get_binding).patch(skills::update_binding),
        )
        .route(
            "/v1/skill-bindings/{id}/rollback",
            post(skills::rollback_binding),
        )
        .route("/v1/skill-usage", post(skills::record_usage))
        // The lapse plane (AUTHZ-4, ADR-0037). `POST /v1/lapses` opens a
        // *proposal* and grants nothing; the grant is that proposal's
        // effect, beside `/publish` and taking the same shape.
        .route("/v1/proposals/{id}/lapse", post(crate::lapses::grant))
        .route(
            "/v1/lapses",
            get(crate::lapses::list).post(crate::lapses::propose),
        )
        .route("/v1/lapses/{id}/revoke", post(crate::lapses::revoke))
        // CODEOWNERS-style curator files (FLOW-3, ADR-0032 decisions
        // 13–15), under the policy plane's own actions: they add required
        // approvers and grant nothing.
        .route(
            "/v1/admin/scopes/{scope_id}/curators",
            get(curators::get).put(curators::put),
        )
        // The service-identity plane (AUTH-3, ADR-0018 decision 3).
        .route(
            "/v1/service-identities",
            get(service_identities::list).post(service_identities::register),
        )
        .route(
            "/v1/service-identities/{id}",
            get(service_identities::get).delete(service_identities::remove),
        )
        // The directory plane's credentials (AUTH-4, ADR-0059
        // decision 13). On `/v1` rather than on `/scim/v2`: issuing one is
        // an act of the product's own authority, PDP-gated at the tenant,
        // and a credential that could mint another would make the
        // directory the authority on its own access.
        .merge(crate::scim::credential_routes())
        // The pull sync's operator surface (AUTH-5, ADR-0060 decision 10).
        // On `/v1` and reachable from nowhere else: a breaker the directory
        // can wave through is not a breaker, so releasing one is an act of
        // the product's own authority and never the provisioning
        // credential's. It takes its own action rather than
        // `DirectoryManage`'s, because handing out a token and authorising
        // irreversible bulk sealing are not the same magnitude and a tenant
        // must be able to hold them apart.
        .route("/v1/directory/sync", get(directory_admin::status))
        .route(
            "/v1/directory/seal-authorisations",
            post(directory_admin::authorise),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            tenant::resolve_tenant,
        ));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(render_metrics))
        // The auth plane is unauthenticated by nature: it is how a caller
        // becomes authenticated (AUTH-1, ADR-0010). The two CLI routes
        // serve `synveda login` over the same flow (ADPT-1, ADR-0027
        // decisions 5 and 6) — they exist so the client needs no OAuth
        // configuration and no client credentials of its own.
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/cli/exchange", post(auth::cli_exchange))
        .route("/auth/refresh", post(auth::refresh))
        // Sign-out (CNSL-1, ADR-0056). Unauthenticated for the same reason
        // its siblings are: destroying a credential needs only that
        // credential, and requiring a valid session to end one would mean
        // an expired session could not be cleared.
        .route("/auth/console/logout", post(auth::console_logout))
        // The console bundle (CNSL-1, ADR-0056 decision 1), when one is
        // built. Unauthenticated by nature: it is the page a signed-out
        // operator lands on to sign in, and it holds no data — every fact
        // it shows comes from a `/v1` call the cookie authenticates.
        .merge(console_routes())
        // The SCIM plane (AUTH-4, ADR-0059 decision 1). Outside the `/v1`
        // tenant middleware by nature: it authenticates with a
        // provisioning credential rather than a bearer, and resolves its
        // tenant from that credential (`synveda_identity::scim`).
        .merge(crate::scim::router(state.clone()))
        .merge(authenticated)
        .layer(middleware::from_fn(track_http_metrics))
        // Added last so the request span is outermost and every inner span —
        // middleware included — nests under it.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_request_span)
                .on_response(record_response),
        )
        .with_state(state)
}

/// Liveness: the process is up. No dependencies touched.
async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: walks the real crate layering (gateway→retrieval→store) down to
/// a Postgres round-trip, then asks that database which schema epoch it is.
/// This is the FND-5 end-to-end traced request; it reads no application data
/// (ADR-0007).
///
/// The epoch check is here as well as at boot (CPR-2, ADR-0069) because the
/// gateway is allowed to start without a database — that is what lets an
/// outage be reported instead of crash-looping — and a check that only ran at
/// boot would therefore be a check a database could arrive after. Answering it
/// per probe costs one single-row select and closes that window: nothing that
/// routes on readiness sends traffic to a gateway sitting on the wrong epoch.
async fn readyz(State(state): State<AppState>) -> Response {
    if let Err(err) = synveda_retrieval::readiness(&state.pool).await {
        tracing::error!(error = %err, "readiness check failed");
        // The detail is in the trace and the log; the body stays generic.
        return (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response();
    }
    match synveda_store::epoch::verify(&state.pool).await {
        Ok(_) => (StatusCode::OK, "ready").into_response(),
        Err(err) => {
            tracing::error!(error = %err, "readiness check failed: schema epoch");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
    }
}

/// Prometheus exposition endpoint.
async fn render_metrics(State(state): State<AppState>) -> String {
    state.metrics.render()
}

#[derive(Serialize)]
struct WhoamiResponse {
    subject: String,
    tenant: Tenant,
    /// The tenant plane's capability probe, when the caller asked for it
    /// (CNSL-2, ADR-0058 decision 1). Absent by default: the base call is
    /// a pure task-local read that touches no database, and a screen that
    /// only wants a name should not pay for a PDP fan-out.
    #[serde(skip_serializing_if = "Option::is_none")]
    capabilities: Option<crate::capabilities::TenantCapabilities>,
}

#[derive(Deserialize)]
pub(crate) struct WhoamiParams {
    /// Ask for the tenant-plane capability block.
    #[serde(default)]
    capabilities: bool,
}

/// Introspection: who does the gateway think is calling? Returns the
/// caller's own resolution result, and — only when asked — what the caller
/// may do on the tenant plane.
///
/// The base answer names no governed asset and takes no PDP decision
/// (ADR-0008); `?capabilities=true` takes decisions **about the caller
/// only**, which is why it needs no permission of its own (ADR-0058
/// decision 3). Reads the task-local rather than a request extension: this
/// endpoint exists to prove the propagation path end to end.
async fn whoami(State(state): State<AppState>, Query(params): Query<WhoamiParams>) -> Response {
    let capabilities = if params.capabilities {
        match crate::capabilities::at_tenant(&state).await {
            Ok(block) => Some(block),
            Err(error) => return ApiError(error).into_response(),
        }
    } else {
        None
    };
    match synveda_identity::current_tenant() {
        Some(context) => Json(WhoamiResponse {
            capabilities,
            subject: context.claims.subject,
            tenant: context.tenant,
        })
        .into_response(),
        None => ApiError(Error::Internal {
            message: "authenticated route ran outside a tenant scope".to_owned(),
        })
        .into_response(),
    }
}

/// One span per request. `otel.name` gives Jaeger the `VERB /route` operation
/// name; the status code is recorded on response, and `tenant.id` by the
/// tenant-resolution middleware once resolution succeeds (TEN-1 AC).
///
/// When the caller sent a W3C `traceparent`, this span continues that trace
/// instead of starting a new one — see [`parent_context`]. When it named
/// itself in `X-Synveda-Client`, that lands on `synveda.client` — see
/// [`client_name`].
fn make_request_span(request: &Request) -> tracing::Span {
    let route = matched_route(request);
    let path = recorded_path(request, &route);
    let span = tracing::info_span!(
        "http.request",
        otel.name = %format!("{} {}", request.method(), route),
        otel.kind = "server",
        http.request.method = %request.method(),
        http.route = %route,
        url.path = %path,
        http.response.status_code = tracing::field::Empty,
        tenant.id = tracing::field::Empty,
        synveda.client = tracing::field::Empty,
    );
    if let Some(client) = client_name(request.headers()) {
        span.record("synveda.client", client);
    }
    if let Some(parent) = parent_context(request.headers())
        // Only fails when no OTel subscriber layer is installed — the unit
        // tests, and any binary that skipped `telemetry::init`. The span is
        // still a perfectly good `tracing` span; it simply has no trace to
        // join, which is the same position the gateway was in before this.
        && let Err(err) = span.set_parent(parent)
    {
        tracing::debug!(%err, "no OpenTelemetry layer to attach the caller's trace to");
    }
    span
}

/// The caller's trace context, when it sent a usable one.
///
/// ADR-0007 deferred W3C `traceparent` extraction "to Phase 1 (ADPT-1/CTX-3),
/// when external callers exist; the baseline emits new root traces per
/// request". Those callers arrived — ADPT-1's hooks have sent a `traceparent`
/// since they shipped, and ADPT-2's MCP server is a second — and nothing here
/// read it, so every trace still began at this process and the header was
/// decorative. This is that clause, landing late.
///
/// # `None` rather than an empty context, deliberately
///
/// `set_parent` with a context carrying no valid span makes the span an
/// explicit root and detaches it from whatever it would otherwise nest
/// under. That is harmless here today, because this layer is outermost — but
/// it is harmless by accident, and a future layer added outside this one
/// would silently lose its parent. Returning `None` when there is nothing to
/// join keeps the default behaviour the default.
///
/// A `traceparent` the propagator cannot parse extracts to exactly that
/// invalid context, so a bad header is ignored rather than rejected: a
/// trace is plumbing, and refusing a request over its telemetry would make
/// an observability feature into an availability one.
///
/// **One kind of malformed header is not ignored, and it is the SDK's
/// reading rather than ours.** W3C requires a version-`00` trace-id to be
/// exactly 32 hex digits; `TraceContextPropagator` checks the field parses
/// as hex and not that it is full width, so `00-4bf92f3577b34da6-…` is
/// accepted and zero-padded into a valid id. The cost is a confusing
/// Jaeger view — two callers sending the same short id share a trace — and
/// it stops there, because nothing authorises off a trace id. Left as the
/// SDK has it, and pinned by
/// `observability.rs::a_short_trace_id_is_accepted_and_padded_by_the_sdk`
/// so a tightened propagator is a failing test rather than a silent change:
/// a length check here would be the first line of a second implementation
/// of a protocol we deliberately took a library for.
///
/// # What accepting a caller's trace id does and does not mean
///
/// It means the caller chooses this request's trace id, so a client can
/// place its requests in a trace of its own — which is the entire point, and
/// how a slow session start becomes one trace from hook through plan, embed,
/// search and compose.
///
/// It also means the id is **caller-controlled and therefore not evidence**.
/// A client may reuse an id, forge one, or join a trace it guessed. ADR-0007
/// already fixes what that can cost: "traces are plumbing for the audit
/// story, not a substitute for it — AUD-1's hash-chained events remain the
/// tamper-evident record". Nothing authorises off a trace id, no audit event
/// derives from one, and the PDP never sees one. The blast radius of a
/// forged `traceparent` is a misleading Jaeger view, which is the same blast
/// radius as a client that lies in its own logs.
fn parent_context(headers: &axum::http::HeaderMap) -> Option<opentelemetry::Context> {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(headers))
    });
    context.span().span_context().is_valid().then_some(context)
}

/// What the caller says it is, from `X-Synveda-Client: <name>/<version>`.
///
/// ADR-0027's observability note promises this header from the Claude Code
/// adapter, and the adapter has sent it since it shipped — but nothing here
/// read it, so the gateway could not tell an inject from a hook, a `synveda
/// recall` from a console click, or a human's command from a model's tool
/// call. That is the attribution the tenant and the route cannot supply,
/// and now it is a span field beside them.
///
/// # Bounded, because a caller controls it
///
/// A span field is not a metric label — Prometheus cardinality is not at
/// risk here, and deliberately so: `track_http_metrics` labels by matched
/// route precisely to keep that bounded, and **this must never join it**.
/// What a span *is* at risk of is bloat, so the value is capped at
/// [`MAX_CLIENT_CHARS`] and anything outside a conservative character set
/// is refused whole rather than sanitised character by character. A caller
/// that will not name itself plainly gets no attribution, which costs it
/// nothing it was entitled to.
///
/// Absent, unreadable or refused leaves the field unset rather than
/// recording `"unknown"`: a client that literally sends `unknown` and one
/// that sends nothing are different facts, and a reader should be able to
/// tell them apart.
fn client_name(headers: &axum::http::HeaderMap) -> Option<&str> {
    let value = headers.get("x-synveda-client")?.to_str().ok()?.trim();
    let plausible = !value.is_empty()
        && value.chars().count() <= MAX_CLIENT_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+'));
    plausible.then_some(value)
}

/// Long enough for `<name>/<semver-with-build-metadata>`, short enough that
/// a caller cannot make every span in a trace expensive.
const MAX_CLIENT_CHARS: usize = 64;

/// `HeaderMap` as OTel's text-map source. Header names are already
/// lowercase-normalised by `http`, which is what the W3C keys need.
struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        // A header whose bytes are not UTF-8 is absent rather than an
        // error: the propagator's job is to find a trace, not to validate
        // somebody's HTTP.
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

fn record_response(response: &Response, _latency: Duration, span: &tracing::Span) {
    span.record("http.response.status_code", response.status().as_u16());
}

/// Counts and times every request, labelled by method/route/status. Routes
/// are matched patterns, not raw paths, to keep label cardinality bounded.
async fn track_http_metrics(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let route = matched_route(&request);
    let start = Instant::now();
    let response = next.run(request).await;
    let labels = [
        ("method", method),
        ("route", route),
        ("status", response.status().as_u16().to_string()),
    ];
    metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    metrics::histogram!(HTTP_REQUEST_DURATION_SECONDS, &labels)
        .record(start.elapsed().as_secs_f64());
    response
}

/// Routes whose **path** carries a secret.
///
/// `POST /v1/invites/{invite_token}/accept` takes an invitation token as a path
/// segment (CPR-5, ADR-0072 decision 5). A trace is an ordinary log, and the
/// seed's rule is that a secret never appears in one — so for these routes the
/// span records the matched pattern where it would otherwise record the URI.
///
/// A list rather than a heuristic, because a heuristic that decides what looks
/// like a secret is a heuristic that will one day decide wrong in the
/// permissive direction.
const SECRET_IN_PATH: [&str; 1] = ["/v1/invites/{invite_token}/accept"];

/// The path to put on the request span: the URI, unless the matched route is
/// one whose path carries a secret, in which case the pattern.
fn recorded_path(request: &Request, route: &str) -> String {
    if SECRET_IN_PATH.contains(&route) {
        return route.to_owned();
    }
    request.uri().path().to_owned()
}

fn matched_route(request: &Request) -> String {
    request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |path| path.as_str().to_owned(),
    )
}

#[cfg(test)]
mod span_tests {
    use super::*;

    fn request(uri: &str) -> Request {
        Request::builder()
            .method("POST")
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("build request")
    }

    /// The property CPR-5 needs and nothing else in the tree provides: the one
    /// route whose *path* carries a live credential must not put that path on a
    /// span. A trace is an ordinary log (seed: no secret in one).
    #[test]
    fn a_route_whose_path_carries_a_secret_records_its_pattern_instead() {
        let token = "synveda_invite_v1.00000000-0000-7000-8000-000000000000.s3cr3t";
        let uri = format!("/v1/invites/{token}/accept");
        let recorded = recorded_path(&request(&uri), "/v1/invites/{invite_token}/accept");
        assert_eq!(recorded, "/v1/invites/{invite_token}/accept");
        assert!(
            !recorded.contains("s3cr3t"),
            "the span recorded the token: {recorded}"
        );
    }

    /// And every other route still records what was actually asked for —
    /// otherwise the mitigation would cost the whole surface its traces.
    #[test]
    fn every_other_route_records_the_path_it_was_asked_for() {
        let uri = "/v1/workspaces/0199c000-0000-7000-8000-000000000000";
        assert_eq!(
            recorded_path(&request(uri), "/v1/workspaces/{workspace_id}"),
            uri
        );
    }

    /// The list names a route the router actually mounts. A stale entry here
    /// would be a mitigation that silently stopped applying.
    #[test]
    fn the_secret_bearing_route_is_one_this_gateway_serves() {
        for route in SECRET_IN_PATH {
            assert!(
                crate::openapi::declared_paths()
                    .iter()
                    .any(|path| path == route),
                "{route} is on the secret-in-path list but not on the contract"
            );
        }
    }
}
