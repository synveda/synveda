# ADR-0073: authorisation and context resolution over governed scope anchors

- **Status**: Accepted
- **Date**: 2026-08-19
- **Feature(s)**: CPR-6
- **Deciders**: sujitn

## Context

Three prompts of the context-platform programme have now built the governed
scope model — scopes (CPR-3), workspaces and projects (CPR-4), groups, grants
and invitations (CPR-5) — and every one of them recorded the same debt in its
own ADR: **the decision point still describes the old hierarchy.** ADR-0071
decision 3 anchored twelve routes at `Resource::Tenant` because a governed
scope had no chain in the Cedar entity graph. ADR-0072 decision 9 did the same
for fourteen more, and decision 3 there went further: the role keys a grant
carries are *stored and resolved and not fed to the PDP at all*, because
`context.roles` is the old hierarchy's role-binding vocabulary.

So the product had, at the start of this prompt, a complete membership model
that decided nothing. A workspace `owner` grant was a governed record of
authority; what actually let somebody administer a workspace was a
`role_bindings` row at a node of the tree this programme is deleting.

The forces:

- **Seed §2.2 — one decision point, and no path around it.** Whatever replaces
  the hierarchy's chain has to arrive *at* the PDP, not beside it.
- **ADR-0068 decision 4 — the rank vocabulary goes.** `org`, `division`,
  `department`, `team`, `user`, `rank()`, and the strictly-increasing ladder.
  Two of those reached the PDP directly: `Scope.kind` carried the ladder's
  vocabulary and `Principal.department` carried a rung of it, read by
  `standard`'s whole sharing default.
- **A caller does not stand in one place.** The old model's chain was a single
  path from a placement node to the org root, and every rule walked it. In the
  governed model somebody can hold their own scope, a workspace shared with
  them, one project inside a *different* workspace, and a tenant-wide grant —
  four applicable places, none containing the others. A chain can only
  represent one.
- **Authority now flows the other way.** A placement chain runs upward: being
  *in* a team made you a member of the department and the org. A grant runs
  downward: being given a project must not give you the workspace above it.
- **The old hierarchy APIs stay until the prompt that deletes them.** They must
  keep deciding, and they must stop being *required* by the decision point.
- **ADR-0072 decision 2 — no permission table.** What a role key permits is the
  packs', and that has to remain true when the keys reach Cedar.
- **`/v1/me` must forecast from real decisions.** ADR-0071 decision 2 made it
  the one call a client makes first; a client that learns what to offer from an
  edition rather than from the PDP would be re-deriving policy in the browser.

## Decision

**1. One scope vocabulary at the PDP: `ScopeNode`.** The decision point takes
`{id, tenant_id, parent_id, kind, sealed}` and nothing else — no display name,
no path, no depth, no rank. `kind` is the five-shape vocabulary
(`tenant`, `org_unit`, `workspace`, `project`, `principal`), which decides only
what may parent what.

The old hierarchy's rows are **projected into it at the caller's edge**
(`ScopeNode::from_hierarchy`): an org root becomes a tenant scope, a personal
node becomes a principal scope, and division/department/team collapse into the
one shape that nests inside itself — which is exactly the collapse of the three
whose distinction *was* the rank. Nothing is written by that projection and no
`ScopeNode` is ever persisted; it is the legacy bridge, and it is deleted with
`hierarchy_nodes`.

**Options rejected.** Teaching the PDP both vocabularies: that is the ladder
surviving inside the thing meant to remove it, and every pack rule would have
to know which tree it was reading. Deciding governed-scope routes with no
entity graph and letting them fail closed: correct, useless, and the state
CPR-4 and CPR-5 were already in.

**2. A principal scope names its subject, in a column.** `scopes.principal_id`
is present exactly on a `principal`-shaped scope, unique per tenant, immutable.

The anchor resolver has to answer "the authenticated caller's own scope" and
must not do it by convention. The alternatives were a slug grammar every reader
parses and no constraint checks, or the `attributes` bag — which ADR-0070 says
in its own header is never an authorisation input, and this *is* one. It is
minted on demand by `GET /v1/me`, on ADR-0071 decision 1's argument one level
down: the thing every caller needs and nobody thinks to create is minted by the
first call that needs it. Minting one confers nothing on anybody, because
nothing above a principal scope reaches into it.

**3. Seven Cedar entities; a decision names the thing it is about.**
`Tenant`, `Scope`, `Principal`, `Group`, `ScopeGrant`, `Workspace`, `Project`.
The four new ones are parented to the scope they belong to, so every
containment rule written over scopes — the token-scope confinement, the seal,
`resource in principal.anchors` — reaches them without being restated.

`Resource` gains `Workspace`, `Project`, `Group` and `Grant`, and the
twenty-six routes CPR-4 and CPR-5 anchored at the tenant now name them: a read
or an update names the workspace or the project, a project creation names the
workspace it would land in, curating a group names the group, and **revoking
names the grant** — which is what makes "who may take away a directory-managed
grant" a sentence a pack can write. The two that stay on the tenant plane stay
for reasons rather than for want of a resource: creating a group has no group to
name, and redeeming an invitation must work for somebody who holds nothing
anywhere. Both are decided at the tenant *root scope* when there is one.

The consequence, stated because it changes behaviour: the ownership check now
runs **before** the decision on every per-object route, because deciding about
a workspace requires having fetched it. That is ADR-0012 decision 7's order and
the hierarchy plane's, so it is a convergence rather than a new rule.

**4. Anchors are an ordered set, and they reach *downward*.**
`synveda_store::anchors::resolve` answers "where does this request stand" from
six inputs — the caller's own scope, the selected project, the selected
workspace, the organisation-unit relationships above them, the tenant root, and
every scope a direct or group grant names — and returns them most-specific
first, merged per scope, each carrying the role keys effective there.

Ordering is `(source precedence, depth descending, id)`. Depth is a structural
fact about the tree and is a **tie-break for readers**, never an authorisation
input: nothing grants more because an anchor sorted earlier, and no comparison
anywhere asks whether one *kind* outranks another.

The held anchors become `principal.anchors`, a `Set<Scope>` on the principal
entity. They are deliberately **not** entity parents of the principal: an
entity parent is the upward direction, and a project grant that made its holder
a member of the workspace above it would be the placement chain returning under
another name. `principal in resource` stays what it always was — the caller's
own chain — and `resource in principal.anchors` is the new, downward one.

**5. `context.roles` carries both vocabularies, and nothing translates.** A
grant's role keys reach a resource when the grant's scope is on that resource's
chain, subject to principal privacy; the old hierarchy's binding roles reach it
by the rule ADR-0015 decision 3 set. Both arrive as one `Set<String>`, which is
what a pack reads.

They cannot widen each other, and the reason is structural rather than careful
naming: **the two trees are disjoint**, so a grant's scope is never a node of a
hierarchy chain and a node binding is never at a governed scope. What reaches
both is a tenant-wide binding and a tenant-root grant, and the two words the
vocabularies share there — `viewer` and `curator` — are priced identically by
every shipped pack. No translation table exists and none may be added:
translating would be the synchronisation ADR-0068 decision 3 forbids.

**6. Personal principal-scope privacy is a base-layer forbid, with a
governance carve-out.** Nothing reaches a `principal`-shaped scope unless it is
in `principal.private` — the caller's own, plus any principal scope somebody
wrote a grant **directly at**. An inherited grant is not in it, because an
inherited grant did not reach there: `synveda_types::access::inherits_into` is
applied while the anchor set is built, and this forbid restates the same floor
where no pack can drop it.

That door is what makes "share my own notes with you" sayable at all. The packs
used to carry `resource.kind != "user"` on every permit, which was the same
rule stated many times and strictly stronger — it refused even a deliberate,
direct grant. Those clauses are deleted; the floor is now in one place.

The carve-out is short and closed: `PolicyRead`, `PolicyAssign`, `RoleRead`,
`HierarchyRead`, `HierarchyUpdate`, `HierarchyDelete`, `ServiceIdentityRead`,
`ServiceIdentityManage` — governance that discloses no material and confers no
access to it. `RoleAssign` and `MembershipGrant` are deliberately **absent**: a
grant written at a private scope puts that scope in the grantee's `private`
set, so an administrator who could write one could grant themselves everybody's
notes. The shape is "forbid everything, then name the exceptions" because the
failure modes are not symmetric — a content action forgotten in a deny-list
becomes silently readable, a governance action forgotten in an allow-list is
merely refused until somebody notices.

**7. `principal.department` is deleted; `standard` shares a neighbourhood.**
The attribute named the nearest department-kind ancestor of a placement and was
the last rung of the rank vocabulary inside the PDP. What replaces it in
`standard`'s sharing default is `principal.ambit`: the **parent** of every
scope this caller holds, minus the tenant root.

That reproduces the rule's *shape* — a grant at one team shares the teams beside
it — without asking what kind of thing the parent is, and it keeps the pack's
identity between its two neighbours. Excluding the tenant root is what keeps
`standard` from meaning `open-collaboration`. A caller who holds nothing gets
exactly `regulated-strict`'s read surface, which is the same collapse the
missing-`department` case used to produce.

The three shipped packs bump to `@17`. Their role lists gain the grant keys
beside the binding roles they already named, so a workspace `owner` administers
their workspace with no legacy binding minted anywhere.

**8. `/v1/me` forecasts from real decisions, per anchor.** The route answers
`Action::PROBED_AT_SCOPE` at each of the caller's anchors, each under **its
own** chain and assignments, because the effective pack is a property of the
resource. Bounded at 32 anchors, with the remainder *named* rather than
truncated silently.

Nothing here is derived from a plan, an edition or a deployment size. A
personal deployment and an enterprise one differ in the rows this reads, never
in the code that reads them — which is ADR-0068 decisions 1 and 2 made
checkable rather than asserted.

**9. The SCIM boundary projects onto the same four tables.** A directory group
becomes a `groups` row with `source = 'directory'`, keyed by the external id
the directory knows it by; its members become `group_members` rows keyed by
each member's **token subject**. Deleting a directory group *archives* the
governed one, which resolves to nobody on the next request.

**No enterprise membership table**, and that is the whole point: a directory
group and a group somebody typed are the same row shape in the same table,
differing in one column that decides only whether the product refuses to edit
it. The projection writes **no grants** — a directory says who is in a group,
never what the group may do — and skips a directory user who has not
provisioned an identity yet, because a subject is what a verified token
carries.

## Consequences

**What this buys.**

- Grants decide. A workspace `owner` administers their workspace, a group's
  grant reaches its members, and a revocation is refused on the very next
  request — no invalidation, because the anchors are resolved per request.
- A project-only grant reaches the project and stops. That sentence was
  unsayable in the placement model, where membership ran upward by
  construction.
- The rank vocabulary is gone from the decision point. `depth_is_not_authority`
  asserts it directly: nesting the same tree four levels deeper changes no
  verdict.
- A person's own scope is theirs, structurally, and they can share it
  deliberately. Both halves are new.

**What it costs.**

- **Six extra indexed reads per request.** The anchor resolution runs on every
  `/v1` call, including the hierarchy planes where it can never contribute,
  because the tenant plane is decided at the tenant root and a tenant-wide
  grant is written there. It is inside a transaction the request already
  opened. The obvious optimisation — resolving lazily, or caching per request —
  is deferred rather than taken, because a cache in front of the thing that
  makes revocation immediate is exactly the wrong first optimisation.
- **`standard`'s golden matrix moved.** A caller with no grant now reads their
  own chain, where before they read their department. That is the removal of
  the rank rather than a regression, and it is recorded in the golden tests
  rather than accommodated.
- **Two role vocabularies still coexist**, and will until the hierarchy is
  deleted. They cannot be confused into widening each other, but a reader of
  `context.roles` sees both, and the packs name both.
- **The legacy bridge is real code.** `ScopeNode::from_hierarchy` is a mapping
  from a vocabulary this programme is deleting, and it lives in the crate that
  is supposed to no longer know about it. It is one function, it writes
  nothing, and it goes with `hierarchy_nodes`.

**What it does not do.**

- **The packs are not renamed to `personal`/`team`/`enterprise`** (ADR-0068
  decision 2). That is a separate cut with its own defaults; this one re-anchors
  the mechanism and leaves the three names alone.
- **The runtime planes still decide over the old hierarchy.** Observe, inject,
  recall, prompts, context packs, skills, channels, proposals and quarantine
  all resolve chains from `hierarchy_closure` and supply no anchors — which
  changes no verdict, because an anchor's scope is a governed scope and is
  never a node of a hierarchy chain. Prompts 7–18 re-cut those planes.
- **Nothing mints a principal scope except `/v1/me`.** A read path must not
  write, and a caller who has never called `/v1/me` has no own scope and
  therefore no private anchor.

## References

- ADR-0068 (context-platform domain and epoch), decisions 1, 2, 3, 4
- ADR-0070 (generic governed scopes), decisions 1, 2
- ADR-0071 (workspaces, projects and repository identity), decisions 1, 2, 3
- ADR-0072 (groups, grants and invitations), decisions 2, 3, 4, 7, 8, 9
- ADR-0012 (embedded Cedar PDP), decisions 1, 4, 7
- ADR-0014 (policy packs), decisions 3, 4, 5
- ADR-0015 (role bindings), decisions 2, 3, 4, 5
- ADR-0017 (Cedar entity store), decisions 2, 3, 4, 5
- ADR-0018 (service identities), decision 4
- ADR-0038 (sensitivity as a policy attribute), decisions 2, 4, 5
- ADR-0058 (capability probes), decisions 2, 3, 4, 5
- ADR-0059 (directory mirror and seals), decisions 8, 9
- seed §2.2, §2.4, §6
