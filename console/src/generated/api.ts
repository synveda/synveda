// GENERATED FILE — DO NOT EDIT.
//
// Written by scripts/generate-api-types.mjs from docs/api/openapi.json, which the
// gateway derives from its own request and response types (CPR-4, ADR-0071
// decision 7). Editing this file is editing the wrong end of the chain: change
// the Rust, run `cargo test -p synveda-gateway --test openapi` with
// SYNVEDA_WRITE_OPENAPI=1 to refresh the document, then
// `node scripts/generate-api-types.mjs`.
//
// `make check-api-types` fails when this file and the document disagree.
//
// Source document: Synveda 0.2.0

/**
 * The taxonomy error body, declared for the OpenAPI document.
 *
 * A schema-only mirror of `synveda_types::Error`'s serialised form, which is
 * `{"kind": "...", ...}` with a per-variant remainder. It exists because the
 * contract has to say what a 4xx body looks like and the taxonomy lives two
 * crates down, where `utoipa` deliberately does not reach — the OpenAPI
 * derive is a property of the surface, and `synveda-types` is not one.
 */
export type ApiErrorBody = {
    /**
     * `policy_denied` only: the action that was attempted.
     */
    action?: string | null;
    /**
     * `not_found` only: what was looked up.
     */
    entity?: string | null;
    /**
     * The stable machine-readable code — `invalid`, `conflict`, `not_found`,
     * `policy_denied`, …
     */
    kind: string;
    /**
     * Present on most variants; what went wrong, safe to show a caller.
     */
    message?: string | null;
    /**
     * `policy_denied` only: which policy produced the denial.
     */
    reason?: string | null;
    /**
     * `policy_denied` only: what was acted on.
     */
    resource?: string | null;
  };

/**
 * `POST /v1/projects/{project_id}/repositories`.
 *
 * The server derives `provider`, `canonical_uri`, `repository_owner` and
 * `repository_name`; a client sends what it knows and never what it
 * concluded, because two clients concluding separately is how one repository
 * becomes two rows.
 */
export type AttachRepositoryBody = {
    /**
     * Advisory default branch. Nothing in the product resolves it.
     */
    default_branch?: string | null;
    /**
     * A stable content id for a repository with no remote — a git
     * root-commit object id, 40–128 hex characters. Never a path.
     */
    local_fingerprint?: string | null;
    /**
     * Caller-supplied labelling bag; a JSON object, at most 8 KiB encoded.
     */
    metadata?: Record<string, unknown> | null;
    /**
     * What to call it. Derived from the remote when there is one; **required**
     * when there is not, because a fingerprint names nothing a human reads.
     */
    name?: string | null;
    provider?: "github" | "gitlab" | "bitbucket" | "azure_devops" | "generic_git" | "local";
    /**
     * The remote, in any form git accepts: `https://host/owner/name`,
     * `git@host:owner/name.git`, `ssh://git@host/owner/name`. **The
     * identity**, whenever it is known. A filesystem path is refused.
     */
    remote_uri?: string | null;
  };

/**
 * `POST /v1/workspaces/{workspace_id}/projects`.
 */
export type CreateProjectBody = {
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Workspace-unique handle, same grammar as a workspace slug.
     */
    slug: string;
  };

/**
 * `POST /v1/workspaces`.
 */
export type CreateWorkspaceBody = {
    /**
     * Optional prose. Blank is refused; omit it instead.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Becomes the owned
     * scope's slug too, and is immutable afterwards.
     */
    slug: string;
  };

/**
 * Everything a client needs before it renders anything.
 */
export type MeView = {
    /**
     * What this caller may do on the tenant plane, asked of the PDP.
     *
     * **A forecast, never a grant** (ADR-0058 decision 2): nothing downstream
     * reads this to decide anything, every act still takes its own decision
     * at its own seam, and a client uses this to choose what to *offer*.
     */
    capabilities: TenantCapabilities;
    /**
     * Where they are in setting up — the server's answer, not a client's
     * inference from an empty list.
     */
    onboarding: OnboardingView;
    /**
     * Who is calling.
     */
    principal: PrincipalView;
    /**
     * Every project this caller may read, by workspace then slug. Flat rather
     * than nested under its workspace: a client that wants the tree has
     * `workspace_id` on every row, and one that wants a recent-projects list
     * would otherwise have to flatten what we just nested.
     */
    projects: ProjectView[];
    /**
     * The tenant they resolved to.
     */
    tenant: TenantView;
    /**
     * Every workspace this caller may read, by slug.
     */
    workspaces: WorkspaceView[];
  };

/**
 * The onboarding vocabulary. Closed, so a client's branch is exhaustive and
 * a new state is a compile error somewhere rather than a silently unhandled
 * string.
 */
export type OnboardingState = "blocked" | "needs_workspace" | "needs_project" | "ready";

/**
 * How far along setting up this caller is.
 */
export type OnboardingView = {
    /**
     * How many projects this caller can see.
     */
    project_count: number;
    /**
     * The single word a client branches on.
     */
    state: OnboardingState;
    /**
     * The tenant's root scope, once anything has needed one. Absent on a
     * deployment where nobody has created a workspace yet: the root is minted
     * by the first thing that needs a parent, so that nobody is asked to
     * declare an organisation before they can hold a record.
     */
    tenant_scope_id?: string | null;
    /**
     * How many workspaces this caller can see.
     */
    workspace_count: number;
  };

/**
 * The authenticated principal.
 */
export type PrincipalView = {
    /**
     * The IdP's `name` claim at provisioning time, if any.
     */
    display_name?: string | null;
    /**
     * The identity row, when this subject has provisioned one. Absent for a
     * dev token and for a service client that never completed login.
     */
    identity_id?: string | null;
    /**
     * `human` or `service`.
     */
    kind?: string | null;
    /**
     * Whether the base layer forbids this caller everything (AUTH-2,
     * ADR-0013 decision 5). True also for an IdP subject that never
     * provisioned — fail closed.
     */
    quarantined: boolean;
    /**
     * The verified token's `sub` claim — the name every audit event, role
     * binding and idempotency key is keyed by.
     */
    subject: string;
  };

/**
 * The project listing.
 */
export type ProjectList = {
    /**
     * The workspace's projects, by slug. Archived ones included.
     */
    projects: ProjectView[];
  };

/**
 * A project, as the API serves it.
 */
export type ProjectView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * The project's stable id.
     */
    id: string;
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * The governed scope this project owns, beneath the workspace's.
     */
    scope_id: string;
    /**
     * Workspace-unique handle, identical to the scope's slug. Immutable.
     */
    slug: string;
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
    /**
     * The workspace it belongs to. Immutable.
     */
    workspace_id: string;
  };

/**
 * The repository listing.
 */
export type RepositoryList = {
    /**
     * What the project is about, oldest attachment first.
     */
    repositories: RepositoryView[];
  };

/**
 * A repository attached to a project, as the API serves it.
 */
export type RepositoryView = {
    /**
     * **The identity**: canonical, credential-free, and never a filesystem
     * path. Two clients that describe one repository differently are served
     * the same value here, which is what makes it an identity.
     */
    canonical_uri: string;
    /**
     * When it was attached.
     */
    created_at: string;
    /**
     * Who attached it.
     */
    created_by?: string | null;
    /**
     * The advisory default branch.
     */
    default_branch?: string | null;
    /**
     * The attachment's id — the handle `DELETE` takes.
     */
    id: string;
    /**
     * The stable content fingerprint of a local checkout, when one was given.
     */
    local_fingerprint?: string | null;
    /**
     * The caller's labelling bag, echoed back.
     */
    metadata: Record<string, unknown>;
    /**
     * The project it belongs to.
     */
    project_id: string;
    provider: "github" | "gitlab" | "bitbucket" | "azure_devops" | "generic_git" | "local";
    /**
     * The repository's own name.
     */
    repository_name: string;
    /**
     * The owning path on the host, when the remote had one.
     */
    repository_owner?: string | null;
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * What the caller may do on the **tenant** plane — `whoami`'s block, and
 * since CPR-4 `/v1/me`'s.
 *
 * Carries a `ToSchema` because `/v1/me` embeds it and that route is on the
 * OpenAPI contract. The three fields are declared to the document by
 * `value_type` rather than derived: `Role` and the `&'static str` map keys
 * live in `synveda-types`, where `utoipa` deliberately does not reach — a
 * contract is a property of the surface, and no crate below the gateway has
 * one.
 */
export type TenantCapabilities = {
    /**
     * Every operand-free tenant-plane action, by its stable machine name.
     */
    actions: Record<string, boolean>;
    /**
     * `RoleAssign` per role, because it fails closed without
     * `context.grant`.
     */
    role_assign: Record<string, boolean>;
    /**
     * The caller's tenant-wide effective roles. Node bindings are absent
     * by construction: [`effective_roles_at`] keeps only the tenant-wide
     * rows for a tenant resource, which is the same rule the decisions
     * beside it ran under.
     */
    roles: string[];
  };

/**
 * The tenant, as this plane serves it.
 */
export type TenantView = {
    /**
     * The isolation key.
     */
    id: string;
    /**
     * Display name.
     */
    name: string;
    /**
     * Human-stable handle.
     */
    slug: string;
    /**
     * `active` or `suspended`.
     */
    status: string;
  };

/**
 * `PATCH /v1/workspaces/{workspace_id}` and
 * `PATCH /v1/projects/{project_id}`.
 *
 * `description` has three cases and the wire says them apart: absent leaves
 * it alone, `null` clears it, a string replaces it.
 */
export type UpdateBody = {
    /**
     * New description; `null` clears it.
     */
    description?: string | null;
    /**
     * New display name.
     */
    display_name?: string | null;
    /**
     * The revision the caller last saw. Required: an update without a
     * precondition is a last-writer-wins update, which is the failure this
     * field exists to remove rather than a convenience it offers.
     */
    expected_revision: number;
    status?: "active" | "archived";
  };

/**
 * The workspace listing.
 *
 * An envelope rather than a bare array, so that paging can arrive without
 * breaking every client — and so that a future `not_answered` has somewhere
 * to live, which is the shape the capability probe already uses for the same
 * reason (ADR-0058 decision 5).
 */
export type WorkspaceList = {
    /**
     * Every workspace the caller may read, by slug. Archived ones included:
     * a listing that silently omitted them would make an archived workspace
     * indistinguishable from one that never existed.
     */
    workspaces: WorkspaceView[];
  };

/**
 * A workspace, as the API serves it.
 *
 * A view rather than `synveda_types::workspace::Workspace` itself, because
 * this is the **contract** and the domain type is not: the two agree today
 * and the day they need to differ — a computed field, a withheld one — the
 * contract must be able to say so without the storage type moving. `tenant_id`
 * is deliberately absent: every `/v1` response is already scoped to the
 * caller's tenant, and echoing it invites a client to key on it.
 */
export type WorkspaceView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it; absent when the deployment did.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * The workspace's stable id.
     */
    id: string;
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * The governed scope this workspace owns — what policy, role bindings and
     * every asset attach to.
     */
    scope_id: string;
    /**
     * Tenant-unique handle, identical to the scope's slug. Immutable.
     */
    slug: string;
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * Every operation the contract declares, keyed by its operation id.
 *
 * `body` is present exactly when the operation takes a request body;
 * `response` is the union of its 2xx bodies (`void` for a 204). Error
 * bodies are {@link ApiErrorBody} on every operation and are not repeated
 * here.
 */
export type Operations = {
  /**
   * `GET /v1/me`.
   */
  readonly get_me: {
    readonly path: "/v1/me";
    readonly method: "GET";
    readonly response: MeView;
  };
  /**
   * `GET /v1/projects/{project_id}`.
   */
  readonly get_project: {
    readonly path: "/v1/projects/{project_id}";
    readonly method: "GET";
    readonly response: ProjectView;
  };
  /**
   * `PATCH /v1/projects/{project_id}`.
   */
  readonly update_project: {
    readonly path: "/v1/projects/{project_id}";
    readonly method: "PATCH";
    readonly body: UpdateBody;
    readonly response: ProjectView;
  };
  /**
   * `GET /v1/projects/{project_id}/repositories`.
   */
  readonly list_repositories: {
    readonly path: "/v1/projects/{project_id}/repositories";
    readonly method: "GET";
    readonly response: RepositoryList;
  };
  /**
   * `POST /v1/projects/{project_id}/repositories` — attach a repository.
   */
  readonly attach_repository: {
    readonly path: "/v1/projects/{project_id}/repositories";
    readonly method: "POST";
    readonly body: AttachRepositoryBody;
    readonly response: RepositoryView;
  };
  /**
   * `DELETE /v1/projects/{project_id}/repositories/{repository_id}`.
   */
  readonly detach_repository: {
    readonly path: "/v1/projects/{project_id}/repositories/{repository_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/workspaces` — the tenant's workspaces.
   */
  readonly list_workspaces: {
    readonly path: "/v1/workspaces";
    readonly method: "GET";
    readonly response: WorkspaceList;
  };
  /**
   * `POST /v1/workspaces` — create a workspace and the governed scope it owns.
   */
  readonly create_workspace: {
    readonly path: "/v1/workspaces";
    readonly method: "POST";
    readonly body: CreateWorkspaceBody;
    readonly response: WorkspaceView;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}`.
   */
  readonly get_workspace: {
    readonly path: "/v1/workspaces/{workspace_id}";
    readonly method: "GET";
    readonly response: WorkspaceView;
  };
  /**
   * `PATCH /v1/workspaces/{workspace_id}` — rename, re-describe or retire.
   */
  readonly update_workspace: {
    readonly path: "/v1/workspaces/{workspace_id}";
    readonly method: "PATCH";
    readonly body: UpdateBody;
    readonly response: WorkspaceView;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}/projects`.
   */
  readonly list_projects: {
    readonly path: "/v1/workspaces/{workspace_id}/projects";
    readonly method: "GET";
    readonly response: ProjectList;
  };
  /**
   * `POST /v1/workspaces/{workspace_id}/projects`.
   */
  readonly create_project: {
    readonly path: "/v1/workspaces/{workspace_id}/projects";
    readonly method: "POST";
    readonly body: CreateProjectBody;
    readonly response: ProjectView;
  };
};

/** An operation id. */
export type OperationId = keyof Operations;
