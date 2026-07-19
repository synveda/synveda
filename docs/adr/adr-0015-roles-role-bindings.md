# ADR-0015: Roles & role bindings — vocabulary, subject-keyed bindings, roles as decision context

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: AUTHZ-3
- **Deciders**: sujitn

## Context

AUTHZ-3 lands the product's roles (seed §5): `viewer`, `contributor`,
`curator`, `steward`, `org-admin`, `auditor`, `security-reviewer`,
`compliance` — bound per hierarchy node, inherited downward, with the
full role×action matrix golden-tested.

What the predecessors left for this feature to discharge:

- **The admin planes are tenant-wide.** Every product pack permits
  hierarchy admin, `PolicyRead`, and `PolicyAssign` to any principal of
  the resource's tenant — "until AUTHZ-3 narrows them with roles"
  (ADR-0012 decision 3, ADR-0014 decisions 1 and 4).
- **Dev subjects are tenant admins.** An out-of-band (HS256) subject
  carries tenant-wide admin semantics "until AUTHZ-3 lands roles"
  (ADR-0013 decision 6, STATUS deferral).
- **Who may assign packs is tenant-wide** (ADR-0014 decision 4).

Forces:

- **Zero-config** (seed §2.1): a JIT-provisioned user with no bindings
  must keep composing their own chain; roles must be derivable from IdP
  groups, not YAML.
- **Strict by default** (seed §2.3): the moment roles exist, holding no
  role must mean holding no administrative power — including for dev
  tokens.
- **Layering** (seed §2.4): policy never touches storage; binding rows
  are caller-supplied data, like hierarchy rows and pack assignments.
- **Sequencing.** The actions several roles exist for — proposal
  approvals (`curator`, FLOW-3), executable-skill review
  (`security-reviewer`, SKIL-2), restricted-sensitivity approvals
  (`compliance`, FLOW-3/AUTHZ-5), audit queries (`auditor`, AUD-2),
  content writes (`contributor`, MEM-1) — land later. The role
  *vocabulary* and *binding machinery* must be final now; the matrix
  grows a column per new action, never a reshape.
- **HIER-3 will sync entities.** Whatever roles add to a decision must
  not become a resource-dependent entity attribute, or a synced entity
  store cannot hold it.
- **Escalation resistance.** A role system that lets a subtree steward
  mint an org-admin has no floor; the guard must survive custom packs.

## Decision

Roles are a closed product vocabulary; bindings are subject-keyed rows
resolved per decision into a role set that reaches Cedar as *request
context*; the product packs (now `@2`) gate the admin planes on that
context; and the org-admin escalation guard joins the invariant base
layer.

1. **A closed, typed vocabulary.** `synveda_types::Role` enumerates the
   eight product roles. Free-form role strings would make the matrix
   untestable and let a typo become a silent never-matches; new roles
   are a product decision (a type change), not tenant data. Custom
   *packs* may still interpret the vocabulary however they like — what
   is closed is the set of names, so `steward` means the same thing in
   every tenant, like pack names (ADR-0014 decision 6).
2. **Bindings are subject-keyed, per node, inherited downward;
   `scope_id null` is tenant-wide.** New tenant-scoped table
   `role_bindings (tenant_id, subject, scope_id?, role)` (forced RLS +
   grants per ADR-0009). Subject-keyed — not an `identities` FK —
   because the PDP's principal *is* `(tenant, subject)` (ADR-0012), a
   binding may precede first login (pre-binding a steward before they
   ever sign in), and dev subjects, which never provision, stay
   bindable. A null scope binds at the top of the inheritance chain —
   the tenant itself — which is what makes a fresh tenant governable at
   all: tenant-level actions (creating the org root, the tenant default
   pack, tenant-wide bindings) would otherwise be decidable by nobody
   until a hierarchy exists to bind at.
3. **Effective roles are decision context, not entity attributes.** The
   caller supplies the principal's binding rows for the resource's
   chain plus its tenant-wide rows (`AuthzContext.role_bindings`, read
   in the caller's own transaction like assignments — a binding is in
   force on the very next request). The PDP resolves the effective set:
   tenant-wide bindings always apply; node bindings apply iff the bound
   node is on the resource's chain — that one rule *is* "inherited
   downward". For `Resource::Tenant`, only tenant-wide bindings apply:
   a root-scoped steward manages nodes, never the tenant's own plane.
   The set reaches Cedar as required request context
   (`context.roles: Set<String>`) on every action — never a principal
   attribute, because the effective set varies with the resource and
   HIER-3's synced entity store could not hold it.
4. **The product packs, `@2`, narrow the planes.** All three packs
   replace the tenant-wide admin permit with role conditions (identical
   across packs — packs differ on composition, not on who
   administers): `steward`/`org-admin` for the mutating plane
   (Hierarchy create/update/delete, `PolicyAssign`, `RoleAssign`),
   plus `auditor` for the read plane (`HierarchyRead`, `PolicyRead`,
   `RoleRead`). The content roles (`viewer`, `contributor`, `curator`)
   gain a `MemoryRead` permit over the bound subtree, personal
   (user-kind) scopes excluded — under `regulated-strict` this *is*
   the seed's "explicit grant" for cross-team read; AUTHZ-4 lapses add
   the time-boxed variant. Roles are strictly additive: the
   role-free membership floor (own chain composes) survives unchanged —
   zero-config — and no role subtracts access, so binding `auditor` to
   someone never strips their own memories. Admin and audit roles grant
   no content read (least privilege: an org-admin who wants to read a
   team's memories binds themselves `viewer` there, visibly).
   `security-reviewer` and `compliance` are marker roles today — their
   power arrives with the approval actions (SKIL-2, FLOW-3, AUTHZ-5).
   This ends dev-token tenant-wide admin (ADR-0013): an unbound subject
   — dev or provisioned — holds no administrative power.
5. **The escalation guard is a base-layer invariant.** `RoleAssign`
   carries the role being granted *or revoked* as **required** context
   (`context.grant`); `base.cedar` forbids granting or revoking
   `org-admin` without holding `org-admin`. Required — not optional —
   context, so a grant-less `RoleAssign` request fails schema
   validation at request-build time and denies, rather than silently
   skipping an erroring forbid (Cedar drops a policy that errors; an
   optional attribute would turn a caller bug into an escalation path).
   In the base layer — not the packs — because a custom pack that
   forgot the guard would otherwise let any of its permitted principals
   mint org-admins; invariants don't travel by convention (ADR-0014
   decision 2). Stewards may grant and revoke every other role in their
   subtree — delegation downward is the point of stewardship.
6. **Bootstrap: the `synveda-admins` convention group.** At every login
   completion, a subject whose IdP groups contain `synveda-admins`
   (case-insensitive, same convention family as ADR-0013 decision 3)
   gets a tenant-wide `org-admin` binding upserted. Every login — not
   first-login-final like placement — so adding someone to the group
   works on their next login; *removal* of bindings stays explicit
   (API/CLI) until AUTH-4/5 bring mover/leaver sync, and is recorded as
   a deferral. An admin-group subject whose groups resolve no team
   placement is placed under the *org root*, never quarantine — the
   quarantine base forbid would otherwise nullify the very binding that
   makes the tenant governable (provisioning outcome `admin`). Richer
   group→role mapping rules (the override table growing a role column)
   are deferred with it. The store-level CLI
   (`synveda role bind/unbind/list`) is the dev path and the
   break-glass — a tenant that revokes its last org-admin recovers the
   same way as ADR-0014's sealed-tenant hazard.
7. **The surface.** `/v1/roles/bindings` (GET: the tenant's bindings;
   PUT/DELETE: tenant-wide bindings — `Resource::Tenant`) and
   `/v1/hierarchy/nodes/{id}/roles` (GET/PUT/DELETE: the node's
   bindings — uniform-404 ownership first, then the PDP). New actions
   `RoleRead`/`RoleAssign` join the vocabulary and the matrix. Binding
   mutations are AUD-1 emission points; until then: traces plus
   `synveda_role_operations_total{op, outcome}`.
8. **The AC.** The full role×action matrix — nine principals (eight
   roles plus role-free) × the nine-action vocabulary × in-subtree,
   out-of-subtree, and tenant-resource targets × all three product
   packs — is golden-tested in `synveda-policy/tests/roles.rs`,
   through the same assignment-resolution and binding-resolution paths
   production uses. Gateway integration tests prove the routes, the
   next-request effect, and the escalation guard end to end.

## Options considered

1. **Roles as request context from caller-supplied rows (chosen)** —
   survives HIER-3, per-resource semantics fall out of one chain rule,
   bindings take effect next request. Con: one more indexed read per
   governed request (the subject's bindings for the chain); accepted
   like the assignment reads, absorbed by HIER-2/3 caching.
2. **Roles as Cedar entity parents (`principal in Role::"steward"`)** —
   loses the node dimension: a role entity is global, so "steward *of
   eng*" needs per-binding synthesized entities or template-linked
   policies, which turns bindings back into policy-set mutations and
   fights the packs-are-static model. Rejected.
3. **Cedar policy templates, one linked policy per binding** — the
   textbook Cedar RBAC shape, but bindings become per-tenant PolicySet
   state with reload semantics, versioning questions, and a decision
   log that can no longer name one pack@version per decision. Rejected.
4. **Identity-FK bindings** — referential integrity, but no
   pre-binding, no dev-subject bindings (every test and dev flow must
   provision first), and the PDP principal would need an identity id it
   deliberately does not carry. Rejected; a typo'd subject binding
   nobody is the accepted cost, mitigated by the routes echoing the
   binding back.
5. **First-provisioned-user-becomes-admin bootstrap** — zero-config but
   nondeterministic in any org with more than one person; a race
   decides who owns the tenant. Rejected for the convention group.
6. **Role vocabulary as tenant data** — maximally flexible, but the
   matrix becomes untestable, packs can't name roles portably, and
   `steward` stops being a promise. Rejected (same grounds as reserved
   pack names).

## Consequences

- Positive: the admin planes are least-privilege at last; the matrix is
  pinned before the actions multiply (FLOW/SKIL/AUD extend a tested
  table, not a convention); bindings are explicit, immediate,
  tenant-isolated grants — under `regulated-strict`, *the* sanctioned
  durable cross-team grant; the vocabulary and machinery are final for
  AUTHZ-4/5 to build on.
- Negative / accepted trade-offs: governed requests pay one more
  indexed read until HIER-2/3 cache the chain and its rows; every
  existing dev flow and test that leaned on tenant-wide admin must now
  seed a binding (the cost of closing the gap ADR-0013 recorded); a
  tenant can revoke its last org-admin — the CLI is the documented
  break-glass (ADR-0014 precedent); group-driven bindings are additive
  only until AUTH-4/5 (revocation is explicit); embedded pack versions
  bump to `@2`, so decision logs change shape for auditors tracking
  pack versions.
- Reversal trigger: if effective-role resolution outgrows the flat
  chain rule (negative grants, role parameters, per-role expiry),
  bindings graduate to first-class Cedar entities under HIER-3's synced
  store; the vocabulary and the matrix tests survive that move.

## Compliance notes

Seed §2.2 holds: roles change what `Pdp::authorize` decides, never
where it is called; there is still exactly one enforcement seam, and no
caller interprets bindings itself. Every decision keeps logging pack
name@version; `RoleAssign` decisions additionally carry the granted
role in the trace. Binding mutations and the JIT admin-group binding
are new audited action types (AUD-1 emission points, tracked in
STATUS.md). Tests bind roles through the same store rows and resolution
path production uses — never a PDP bypass. `role_bindings` is
tenant-scoped with forced RLS (ADR-0009); the RLS completeness guard
covers it.
