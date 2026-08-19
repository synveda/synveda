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
 * The response to redeeming an invitation.
 */
export type AcceptedInviteView = {
    /**
     * The grant it minted — or the one the acceptor already held.
     */
    grant: GrantView;
    /**
     * The scope they now hold it at.
     */
    scope_id: string;
  };

/**
 * What the caller may do at **one anchor** — a real decision at a real scope,
 * never a shape derived from an edition (CPR-6, ADR-0073 decision 8).
 */
export type AnchorCapabilities = {
    /**
     * Every operand-free scope action, decided here, by its stable machine
     * name. **A forecast, never a grant** — the whole of this module's first
     * doc section applies unchanged.
     */
    actions: Record<string, boolean>;
    /**
     * Whether a grant is written at this very scope rather than inherited
     * from an ancestor — the "why" a member list would otherwise have to be
     * read to answer.
     */
    direct: boolean;
    /**
     * Its shape: `tenant`, `org_unit`, `workspace`, `project` or `principal`.
     */
    kind: string;
    /**
     * The role keys effective here.
     */
    roles: string[];
    /**
     * The scope.
     */
    scope_id: string;
    /**
     * Why it is applicable: `principal_scope`, `selected_project`,
     * `selected_workspace`, `grant`, `org_unit` or `tenant_root`.
     */
    source: string;
  };

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
 * `POST /v1/admin/groups`.
 */
export type CreateGroupBody = {
    /**
     * Optional prose. Blank is refused; omit it instead.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Its members at creation, by principal id.
     */
    members?: string[];
    /**
     * Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Immutable.
     */
    slug: string;
  };

/**
 * `POST /v1/workspaces/{workspace_id}/invites`.
 */
export type CreateInviteBody = {
    /**
     * Who it is meant for. Optional: an invitation with no address is a link
     * the inviter copies, redeemable once by whoever presents it first.
     */
    email?: string | null;
    /**
     * How long it stands, in seconds. Defaults to seven days and is capped at
     * thirty — an invitation that never expires is a key left under the mat.
     */
    expires_in_secs?: number | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
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
 * The response to creating an invitation — the **one and only** place the
 * token exists.
 *
 * It is not stored (only its SHA-256 is), not logged, not in the audit
 * payload and not on any other route. An inviter who loses it withdraws the
 * invitation and issues another, which is one action; the alternative is a
 * product that can show you somebody else's live credential.
 */
export type CreatedInviteView = {
    /**
     * The URL the recipient posts to, with their own credential, to redeem
     * it. Enough for a local or demo deployment to invite by copying a link —
     * email delivery is deliberately not a requirement of this feature.
     */
    accept_url: string;
    /**
     * The invitation.
     */
    invite: InviteView;
    /**
     * The token. **Shown once.**
     */
    token: string;
  };

/**
 * The grant listing.
 */
export type GrantList = {
    /**
     * The grants this filter selected, oldest first. These are the **rows**,
     * not the authority in force anywhere: a workspace grant appears once
     * here and reaches every project inside it. `GET /v1/projects/{id}/members`
     * is the other question.
     */
    grants: GrantView[];
  };

/**
 * `POST /v1/projects/{project_id}/members` and `POST /v1/admin/grants`.
 *
 * Exactly one of `principal_id` and `group_id`. Two flat fields rather than a
 * tagged union, for [`GrantView`]'s reason.
 */
export type GrantSubjectBody = {
    /**
     * The group.
     */
    group_id?: string | null;
    /**
     * The principal, by verified token subject.
     */
    principal_id?: string | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope. Required on `/v1/admin/grants`, where the caller chooses;
     * **refused** on the project route, where the path already says which
     * scope — a body that could name a different one would make the path a
     * suggestion.
     */
    scope_id?: string | null;
  };

/**
 * A grant, as the API serves it.
 *
 * The subject is **flattened** into `subject_kind` plus one of two id fields
 * rather than nested as a tagged union. A tagged union is the tidier model and
 * the worse contract: it renders as a `oneOf` that the frontend generator
 * would have to discriminate, and this document's whole point is that the
 * types on both ends are derived rather than hand-reconciled.
 */
export type GrantView = {
    /**
     * When it was made.
     */
    created_at: string;
    /**
     * Whether a directory manages it, and it therefore cannot be revoked
     * here.
     */
    directory_managed: boolean;
    /**
     * Who granted it, when a caller did.
     */
    granted_by?: string | null;
    /**
     * The group, for a `group` grant.
     */
    group_id?: string | null;
    /**
     * Stable id — what a revocation names.
     */
    id: string;
    /**
     * The invitation that produced it, for an `invite` grant.
     */
    invite_id?: string | null;
    /**
     * The principal, for a `principal` grant.
     */
    principal_id?: string | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope the grant is at. Its subtree inherits it.
     */
    scope_id: string;
    source: "owner" | "direct" | "invite" | "directory" | "automation";
    subject_kind: "principal" | "group";
  };

/**
 * The group listing.
 */
export type GroupList = {
    /**
     * The tenant's groups, by slug. Archived ones included: a listing that
     * omitted them would make an archived group indistinguishable from one
     * that never existed.
     */
    groups: GroupView[];
  };

/**
 * A group, named enough to render without a second call.
 */
export type GroupRefView = {
    /**
     * The group's id.
     */
    id: string;
    /**
     * Its handle.
     */
    slug: string;
  };

/**
 * A group, as the API serves it.
 */
export type GroupView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it, when a caller did.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * The external id a directory knows it by, when one does.
     */
    directory_ref?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Stable id.
     */
    id: string;
    /**
     * Its members, by principal id.
     */
    members: string[];
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * Tenant-unique handle. Immutable.
     */
    slug: string;
    source: "direct" | "directory";
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * The invitation listing.
 */
export type InviteList = {
    /**
     * Every invitation issued at this scope, newest first — redeemed and
     * withdrawn ones included, because "who was invited here and what
     * happened" is the question this answers.
     */
    invites: InviteView[];
  };

/**
 * An invitation, as the API serves it.
 *
 * The token is **not here**, on any route, ever. It appears once, in
 * [`CreatedInviteView`], in the response to the request that minted it.
 */
export type InviteView = {
    /**
     * When it was redeemed.
     */
    accepted_at?: string | null;
    /**
     * The principal that redeemed it, when somebody has.
     */
    accepted_by?: string | null;
    /**
     * When.
     */
    created_at: string;
    /**
     * Who issued it.
     */
    created_by?: string | null;
    /**
     * Who it was meant for, when the inviter said.
     */
    email?: string | null;
    /**
     * When it stops being redeemable.
     */
    expires_at: string;
    /**
     * Stable id — what a withdrawal names.
     */
    id: string;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope it grants at.
     */
    scope_id: string;
    status: "pending" | "accepted" | "revoked" | "expired";
  };

/**
 * Everything a client needs before it renders anything.
 */
export type MeView = {
    /**
     * Where this caller stands, most specific first, and what they may do at
     * each — **from real policy decisions** (CPR-6, ADR-0073 decision 8).
     *
     * Their own scope, the tenant root, and every scope a direct or group
     * grant reaches them at. Nothing here is derived from a plan, an edition
     * or a shape: each entry is `Action::PROBED_AT_SCOPE` decided at that
     * scope, under that scope's own effective profile, by the same PDP the
     * act itself will pass through. A personal deployment and an enterprise
     * one differ in the rows this reads, never in the code that reads them.
     */
    anchors: AnchorCapabilities[];
    /**
     * How many anchors the response bound dropped. Named rather than hidden:
     * a truncated answer presented as a complete one is the one failure a
     * capability surface cannot afford (ADR-0058 decision 5).
     */
    anchors_not_answered?: number;
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
 * The member listing.
 */
export type MemberList = {
    /**
     * Everybody who holds a role here, nearest grant first. One entry per
     * (principal, role): somebody holding two roles appears twice, because
     * the two came from different grants and are revoked separately.
     */
    members: MemberView[];
  };

/**
 * One principal's one role at one scope, with everything a reader needs to
 * answer **why**.
 *
 * The `source`, `scope_id`, `inherited` and `via_group` fields together are
 * the whole of "access-source visibility": a person looking at a project's
 * member list can see that Robin is there because somebody granted them
 * `member` at the workspace, or because they are in the `engineering` group,
 * or because a directory said so — without reading an audit log.
 */
export type MemberView = {
    /**
     * Whether a directory manages it, and it therefore cannot be edited here.
     */
    directory_managed: boolean;
    /**
     * The grant this came from — what a revocation names.
     */
    grant_id: string;
    /**
     * When the grant was made.
     */
    granted_at: string;
    /**
     * Whether it was inherited from an ancestor scope rather than written
     * here. A client that offers "remove" on an inherited row is offering
     * something the API will refuse, so this is the field that decides.
     */
    inherited: boolean;
    /**
     * The principal, by verified token subject.
     */
    principal_id: string;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope the grant is actually written at — **not** necessarily the
     * one that was asked about.
     */
    scope_id: string;
    source: "owner" | "direct" | "invite" | "directory" | "automation";
    via_group?: unknown | null | GroupRefView;
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
     * The caller's role keys at the **tenant root scope** — the grants that
     * reach the whole boundary (CPR-6, ADR-0073 decision 5). Separate from
     * `roles` rather than merged into it because the two are different closed
     * vocabularies over different trees, and a client that displayed them as
     * one list would be inventing a translation nothing in this product has.
     */
    role_keys: string[];
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
 * `PATCH /v1/admin/groups/{group_id}`.
 *
 * `members` is a **full replacement**, not a delta: a membership list has no
 * precondition of its own, so add/remove pairs race — two callers each
 * removing one person can both succeed and leave a list neither intended. A
 * replacement under `expected_revision` cannot.
 */
export type UpdateGroupBody = {
    /**
     * New description; `null` clears it.
     */
    description?: string | null;
    /**
     * New display name.
     */
    display_name?: string | null;
    /**
     * The revision the caller last saw. Required, for the reason the
     * workspace plane's is: an update without a precondition is a
     * last-writer-wins update.
     */
    expected_revision: number;
    /**
     * The complete membership after this update.
     */
    members?: string[] | null;
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
   * `GET /v1/admin/grants`.
   */
  readonly list_grants: {
    readonly path: "/v1/admin/grants";
    readonly method: "GET";
    readonly response: GrantList;
  };
  /**
   * `POST /v1/admin/grants` — grant at any scope the caller names.
   */
  readonly create_grant: {
    readonly path: "/v1/admin/grants";
    readonly method: "POST";
    readonly body: GrantSubjectBody;
    readonly response: GrantView;
  };
  /**
   * `DELETE /v1/admin/grants/{grant_id}` — revoke one.
   */
  readonly revoke_grant: {
    readonly path: "/v1/admin/grants/{grant_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/admin/groups`.
   */
  readonly list_groups: {
    readonly path: "/v1/admin/groups";
    readonly method: "GET";
    readonly response: GroupList;
  };
  /**
   * `POST /v1/admin/groups`.
   */
  readonly create_group: {
    readonly path: "/v1/admin/groups";
    readonly method: "POST";
    readonly body: CreateGroupBody;
    readonly response: GroupView;
  };
  /**
   * `PATCH /v1/admin/groups/{group_id}`.
   */
  readonly update_group: {
    readonly path: "/v1/admin/groups/{group_id}";
    readonly method: "PATCH";
    readonly body: UpdateGroupBody;
    readonly response: GroupView;
  };
  /**
   * `POST /v1/invites/{invite_token}/accept` — redeem one.
   */
  readonly accept_invite: {
    readonly path: "/v1/invites/{invite_token}/accept";
    readonly method: "POST";
    readonly response: AcceptedInviteView;
  };
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
   * `GET /v1/projects/{project_id}/members` — who may act here, **including
   * what the workspace above it grants**.
   */
  readonly list_project_members: {
    readonly path: "/v1/projects/{project_id}/members";
    readonly method: "GET";
    readonly response: MemberList;
  };
  /**
   * `POST /v1/projects/{project_id}/members` — a **project-only** grant.
   */
  readonly add_project_member: {
    readonly path: "/v1/projects/{project_id}/members";
    readonly method: "POST";
    readonly body: GrantSubjectBody;
    readonly response: GrantView;
  };
  /**
   * `DELETE /v1/projects/{project_id}/members/{principal_id}`.
   */
  readonly remove_project_member: {
    readonly path: "/v1/projects/{project_id}/members/{principal_id}";
    readonly method: "DELETE";
    readonly response: void;
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
   * `GET /v1/workspaces/{workspace_id}/invites`.
   */
  readonly list_workspace_invites: {
    readonly path: "/v1/workspaces/{workspace_id}/invites";
    readonly method: "GET";
    readonly response: InviteList;
  };
  /**
   * `POST /v1/workspaces/{workspace_id}/invites` — issue one.
   */
  readonly create_workspace_invite: {
    readonly path: "/v1/workspaces/{workspace_id}/invites";
    readonly method: "POST";
    readonly body: CreateInviteBody;
    readonly response: CreatedInviteView;
  };
  /**
   * `DELETE /v1/workspaces/{workspace_id}/invites/{invite_id}` — withdraw one.
   */
  readonly revoke_workspace_invite: {
    readonly path: "/v1/workspaces/{workspace_id}/invites/{invite_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}/members` — who may act here.
   */
  readonly list_workspace_members: {
    readonly path: "/v1/workspaces/{workspace_id}/members";
    readonly method: "GET";
    readonly response: MemberList;
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
