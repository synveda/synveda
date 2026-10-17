# ADR-0072: membership and access assignment — groups, scope grants and invitations

- **Status**: Accepted
- **Date**: 2026-08-18
- **Feature(s)**: CPR-5
- **Deciders**: sujitn

## Context

CPR-4 gave the context platform its first public surface: workspaces and
projects, each owning a governed scope. It left one thing conspicuously absent —
**nobody is in them**. A workspace created through `POST /v1/workspaces` had a
name, a description, a scope and no members, and the only way anybody could act
on it was a `role_bindings` row bound at a node of the *old* hierarchy, which is
a different tree that Prompt 6 deletes whole.

So the product could represent "a place to work" and could not represent "who
works here". This is that.

The forces:

- **ADR-0068 decision 1 — one domain model.** A person working alone, four
  people sharing agent context, and a company with a directory get the same
  tables. No `personal_members`, no enterprise-only access model, no edition
  conditional.
- **Seed §2.2 — policy is enforced, never advisory, and there is exactly one
  decision point.** Whatever this feature stores must not become a second
  place that decides what somebody may do.
- **ADR-0015 decision 2 — a binding may precede first login.** The PDP's
  principal is `(tenant, subject)`; the old role-binding plane is subject-keyed
  for exactly that reason, and the reason has not changed.
- **The old hierarchy is being deleted, and nothing may be synchronised with
  it** (ADR-0068 decision 3). `identities.scope_id` still has a foreign key
  into `hierarchy_nodes`, so *anything* that needs an identity row in this tree
  needs a node of the model this programme is removing.
- **The PDP has not been re-cut over generic scopes yet.** `Scope` entities are
  materialised from `hierarchy_nodes` and its closure; a generic scope has no
  row in it. CPR-4 recorded this and anchored its decisions at the tenant; this
  feature inherits the same constraint, and inherits the consequence too.
- **Invitations are how a small team actually onboards.** The product's target
  user is a person who wants to add a colleague, not an operator who runs a
  directory sync. And that person has no mail server.
- **A membership model gets audited.** "Why can this person see my project" is
  the question a customer asks in an incident, and an answer that requires
  reading the audit log is an answer nobody gets.

## Decision

**1. Four nouns: `Group`, `GroupMember`, `ScopeGrant`, `PendingInvite`.** A
grant gives a **subject** — a principal or a group — a **role key** at a
**scope**. A group is a named set of principals that grants nothing on its own;
a grant naming the group is what grants. An invitation is an expiring, one-time
token that mints a grant when somebody redeems it.

Creating a workspace or a project also mints an `owner` grant for its creator,
in the creating transaction. A collaboration space nobody is a member of is not
one, and the person who made it is the one member the product can name without
being told.

**Options rejected.** A membership row per (scope, principal) with the role as a
column would make holding two roles at one scope unrepresentable and would make
"revoke the reviewer role but keep member" an update rather than a revocation.
An ACL blob on the scope would put an authorisation input inside
`scopes.attributes`, which is the open bag ADR-0070 decision 1 refused to make
load-bearing.

**2. A role key is a key. There is no permission table, and there must not be
one.** `RoleKey` is six words — `owner`, `member`, `viewer`, `reviewer`,
`curator`, `administrator` — and nothing in the schema or in `synveda-types`
says what any of them may do. The Cedar packs decide that, as they decide
everything else.

The alternative is the one every product with roles eventually builds: a
`role_permissions` table, seeded with defaults, editable by an administrator.
It is attractive because it is inspectable and because it looks like
configuration. It is refused because it is a **second decision point**. The day
it exists, "may Robin publish here" has two answers — the pack's and the
table's — and they diverge the first time somebody edits one. Seed §2.2 permits
one, and this product sells the property that the one is auditable.

The cost is real and stated: **until the PDP is re-cut, a role key decides
nothing.** See decision 3.

**3. Grants are stored and resolved; they are not yet a PDP input.** The
resolution — inheritance through `scope_closure`, group expansion, principal
privacy — is implemented, tested and served by the members routes. What does
*not* happen is feeding the resolved role keys to Cedar as `context.roles`.

This is deliberate rather than unfinished. `context.roles` today carries the old
role-binding vocabulary (`steward`, `org-admin`, …) resolved from
`role_bindings` over the old hierarchy. Feeding a grant written on a *generic*
scope into a decision taken at the **tenant** would do two unacceptable things:
it would be a translation between the two models, which ADR-0068 decision 3
forbids outright; and it would silently widen every tenant-wide decision by
every project-level grant, because a decision anchored at the tenant cannot tell
which scope a role came from.

So this feature ships the record of authority and the next prompt makes it the
authority. The four new Cedar actions already apply to `[Tenant, Scope]` where
the model admits it, so re-anchoring is a route change rather than a contract
change — the same shape CPR-4 left behind, and this ADR is the second time the
same debt is written down rather than the first time it is hidden.

**Options rejected.** Wiring grants into `context.roles` now, at the tenant:
rejected above. Blocking this feature until the PDP re-cut: rejected because it
would mean two more prompts of infrastructure with no surface, and because the
storage model is the harder half and is what the re-cut needs to exist.

**4. A principal is a verified token subject, not an identity row.**
`scope_grants.principal_id` and `group_members.principal_id` are `text`.

ADR-0015 decision 2 already argued the general case: a grant that cannot precede
first login cannot be pre-assigned. The case that *decided* it here is narrower
and harder: an `identities` row in this tree requires a `hierarchy_nodes` node,
because `identities_scope_fk` points there. A membership model that needed a row
in the model it replaces would be a synchronisation between the two, which this
programme forbids.

What this costs: the member list carries subjects rather than names. Display
names belong to the identity plane, and the identity plane is re-cut by a later
prompt; joining to it here would reach into the old model for cosmetics.

**5. An invitation is a bearer credential, minted and hashed like the
provisioning credential.** `synveda_invite_v1.<tenant-uuid>.<43-char base64url
secret>`, 256 bits of entropy, SHA-256 of the **whole string** stored, the
plaintext returned exactly once in the creation response and never again.

The shape is ADR-0059 decision 13's, and the reasons carry over: hashing the
whole string means a secret lifted behind another tenant's prefix hashes to
nothing, and a versioned greppable prefix means one leaked into a chat window or
a support ticket can be found.

Three consequences, each a decision of its own:

- **`POST /v1/invites/{token}/accept` puts a secret in a URL path.** A trace is
  an ordinary log, and seed says a secret never appears in one — so
  `make_request_span` records the matched *route pattern* rather than the URI
  for this route, from an explicit list rather than a heuristic. The route shape
  itself is the prompt's, and the mitigation is the honest one.
- **A replayed invitation creation is a 409, not a 200.** Every other creation
  on this plane replays with the original resource; this one cannot, because the
  original resource included a token that no longer exists anywhere. Answering
  200 with the token field missing would serve a body that looks successful and
  is unusable. The refusal names the invitation and points at the listing.
- **Redeeming takes no `Idempotency-Key`.** The token is one-time by
  construction, so it *is* the key. A retry by the principal who already
  redeemed it replays (`200`, the same grant); by anybody else it is a `409`.
  Requiring a header from a person clicking a link would be hostile, and the
  guarantee is already there.

**6. `PATCH /v1/admin/groups/{id}` replaces membership wholesale, under
`expected_revision`.** Not a delta. A membership list has no precondition of its
own, so add/remove pairs race: two callers each removing one person can both
succeed and leave a list neither intended. A replacement under a revision
precondition cannot — the second caller is refused and re-reads. It is also the
shape a directory sync sends, so the enterprise path is the same code rather
than a second one.

**7. Four Cedar actions: `MembershipRead`, `MembershipGrant`, `GroupManage`,
`InviteAccept`.**

- **One read authority over the whole plane**, on `DirectoryManage`'s argument:
  "who may act here" and "who has been invited to act here" are one disclosure
  seen from two ends, and splitting them would create a role whose only power is
  reconnaissance over the other half.
- **Read is not `WorkspaceRead`.** CPR-4 widened the packs so content roles can
  see a workspace, arguing that a name discloses nothing. Membership discloses
  who works on what, which is a different fact — so it is a different action,
  and the packs grade it: `regulated-strict` keeps it to the admin roles,
  `standard` adds `curator` (a reviewer who cannot see whose work it is cannot
  review it), `open-collaboration` adds every content role.
- **Issuing an invitation takes `MembershipGrant`**, because an invitation *is*
  deferred granting. An authority to invite weaker than the authority to grant
  would be a way around the second.
- **`GroupManage` is separate from `MembershipGrant`** on ADR-0036 decision 3's
  separability rule: a group says who exists together, a grant says what they
  may do, and a group with no grant naming it confers nothing. A deployment must
  be able to let somebody maintain the first without conferring the second.
- **`GroupManage` and `InviteAccept` are tenant-only.** A group is not anchored
  anywhere in the scope tree, and redeeming happens at the tenant plane —
  which also means AUTH-3's confinement forbids both to every service identity
  for free. An agent must not redeem a person's invitation.

**8. Every shipped pack permits `InviteAccept` to any principal the base layer
has not already forbidden.** The token is the authority. A person holding a
valid invitation who is refused for want of a role is a person this product
invited and then turned away.

It is an *action* rather than an exemption so the invariant floor still runs
over it — a quarantined principal is refused, a sealed scope is refused — and so
a deployment that wants the mechanism off says so by deleting a permit in its
own pack, visibly, in configuration.

**9. Inheritance is the scope tree; a `principal`-shaped scope inherits
nothing.** A grant at a workspace's scope is in force at every project inside
it, resolved through `scope_closure` at read time. Nothing materialises a
per-project row: a derived set is a set that can be stale, and it would have to
be repaired every time a scope moved.

The one stop is a `principal`-shaped scope, which is somebody's own. No
ancestor reaches into it — not the tenant root, not a workspace owner — so a
person's own material stays theirs. The rule lives in
`synveda_types::access::inherits_into` and is applied in the resolution SQL, so
no caller can forget it. The scope tree already makes a principal scope a leaf
(`ScopeKind::permits_parent` admits no child under one); the rule is written as
a property of the walk anyway, so it is already in the right place if that
changes.

**10. A directory-managed grant or group cannot be edited here, and the refusal
names the directory.** `source = 'directory'` marks rows a directory owns. The
refusal is a `409` whose message says to change it there, because the failure it
prevents is not "you lack permission" — it is a person deleting a grant,
watching it return on the next sync, and concluding that revocation in this
product is unreliable.

This is the one rule on the plane that is **not** a database fact, and that is
stated rather than glossed: the directory adapter and a person hold the same
database role, so no constraint can tell them apart. It lives in
`synveda_store::access` with a test.

## Consequences

**What holds now.**

- A person creating their first workspace is its owner, with no configuration
  and no directory.
- A workspace-level grant reaches that workspace's projects, and a project-only
  grant stays where it was written.
- Somebody's own scope is theirs: the widest grant a tenant can write does not
  reach into it.
- A colleague joins by clicking a link that expires, works once, and is
  revocable — with no mail server anywhere in the product.
- "Why can this person see my project" is answerable from one listing: the
  source, the scope the grant is actually at, whether it was inherited, and the
  group it came through.
- Every table is tenant-bound with forced RLS; every route takes a PDP decision;
  every mutation chains an audit event; no invitation token reaches a response
  twice, a log, or the chain.

**What does not hold yet, and is somebody's next prompt.**

- **A role key decides nothing.** The PDP does not read grants; access is still
  enforced through the old role bindings until the PDP re-cut. This is decision
  3, and it is the largest thing this feature defers.
- **Every decision on this plane is anchored at `Resource::Tenant`**, inherited
  from ADR-0071 decision 3 and for the same reason. A grant is written at a
  scope and decided at the tenant, which is coarser than the model.
- **Nothing creates `principal`-shaped scopes.** The isolation rule is
  implemented and tested against scopes a test creates; minting one per identity
  is the identity plane's re-cut.
- **No directory writes `source = 'directory'` yet.** The column, the CHECK, the
  refusals and their tests exist so that when the adapter lands (Prompt 29) a
  person's group and a directory's are the same row shape. Until then that path
  is exercised only by tests.
- **No console screen and no CLI command.** The console shell is Prompt 20; the
  CLI re-cut is Prompt 24. The generated TypeScript types exist and typecheck
  without a screen consuming them.
- **`GET /v1/admin/grants` does not page.** It filters by scope and principal
  and returns an envelope with room for a cursor. A tenant with tens of
  thousands of grants will need one.

## Reversal trigger

Reverse decision 2 — and add a permission matrix — if a customer requirement
arrives that cannot be expressed as a Cedar pack *and* that a pack cannot be
generated from. Reverse decision 4 — and key grants by identity — once
`identities` no longer depends on the old hierarchy and the product has a reason
to require an identity row before access can be assigned, which today it does
not.

## Amendment: browser invitation links and replay of an already-held grant (2026-10-17)

The creation response's `accept_url` named the redemption operation, and People
told inviters to share it. A browser opening it sends `GET`, which the
catalogue did not route, so recipients met a 405, and the console had no page
on which to accept. Decision 5's bearer-token shape, one-time redemption and
`InviteAccept` permit (decision 8) are unchanged; three surfaces change.

- **`invite_url` is the link to share.** It is `/console/invite` on the
  configured public origin with the token in the URL fragment
  (`#token=<token>`). A fragment is not sent in any request, so the token
  reaches no proxy access log, gateway span, metrics label or `Referer`. The
  console moves it out of the address bar before its first render and into
  that tab's `sessionStorage` for at most 20 minutes, so it survives one OIDC
  round trip without entering OAuth state or identity-provider parameters and
  is never written to persistent storage. The tab forgets it on acceptance, a
  terminal refusal, cancellation, sign-out or expiry; a different account
  signed in later in the same tab must open the link again. `accept_url`
  keeps its meaning as the authenticated `POST` target for API clients.
- **An API-shaped link still opens.** `GET`/`HEAD` on
  `/v1/invites/{invite_token}/accept` is mounted beside the unauthenticated
  auth plane, not in the bearer catalogue, because its recipient is usually
  signed out. It reads nothing, decides nothing, consumes nothing and chains no
  event: it answers `303` to the console page with the token in the fragment,
  `no-store` and `no-referrer`, or to the page without a token when the path is
  not token-shaped. The pattern is the existing `SECRET_IN_PATH` route, so spans
  still record the pattern. `POST` is unchanged.
- **Accepting is explicit.** The page names the signed-in account and sends the
  generated `accept_invite` call only when the recipient selects Accept. Session
  and network failures keep the invitation for a retry; expired, withdrawn,
  already-used, not-found and refused answers are terminal and shown in the
  gateway's own words. Where the recipient lands comes from a fresh `/v1/me`,
  never from the acceptance response.

Redemption for access the recipient already held (decision 5's replay rule)
returned the held grant on the first call, but the same recipient's retry
looked the grant up by `invite_id`, which a held grant does not carry, and
reported it revoked. The replay now falls back to the exact held
(scope, principal, role) grant only when that grant existed at acceptance
time. A grant revoked since is still reported, never recreated, and a grant
written after the revocation is not presented as the invitation's result.
Provenance is unchanged and no row is written; no migration is needed.
