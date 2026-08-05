# ADR-0059: SCIM 2.0 — directory facts, one reconciler, and the seal

- **Status**: Proposed
- **Date**: 2026-08-05
- **Feature(s)**: AUTH-4 (and the seam AUTH-5 drives)
- **Deciders**: sujitn

## Context

AUTH-4 is the feature nine earlier ones deferred to by name. ADR-0013
placed a person on first login and said so in its own consequences —
"placement is first-login-final until AUTH-4/5 own movers". Migration
0007 granted `identities` `select, insert` and nothing else, with the
comment that update and delete arrive here. ADR-0015 left role-binding
revocation explicit "until AUTH-4/5 bring mover/leaver sync". ADR-0016
and ADR-0017 built `AppState::invalidate_hierarchy` and named this
feature as the out-of-process writer that must call it. `records.rs`
says a mover's records may re-scope, and cites this ID. ADR-0055 sent
headless `init` here. The feature text is four lines
(SYNVEDA_FEATURES.md): Users+Groups endpoints; joiner/mover/leaver;
leaver seals personal scope, retention-held and unreadable by default;
AC — SCIM conformance tests, and a mover's memories re-scope per policy.

Forces at play:

- **The format belongs to somebody else.** SCIM is RFC 7643 (schema) and
  RFC 7644 (protocol), and the clients are Entra and Okta, which we do
  not ship and cannot fix. This is ADR-0051's inversion — a skill's bytes
  are read by a foreign loader — arriving on the transport side, and it
  costs the same things: our error envelope, our route conventions, and
  our freedom to answer "close enough".
- **Trust class (ADR-0013).** JIT provisioning takes no PDP decision,
  deliberately: it is a system write path driven by verified IdP claims,
  the same class as tenant admission, and ADR-0013's compliance note
  named the future SCIM sync as belonging to that class. Seed §2.2 is
  upheld there by a *reachability* argument — no governed asset is
  reachable through provisioning — not by an exemption, and AUTH-4 has to
  either make the same argument or stop making it.
- **The AC's second half touches governed material.** "A mover's memories
  re-scope per policy" is the first sentence in this epic that is about
  records rather than about people.
- **Nothing is stamped on a record (ADR-0040 decision 3, ADR-0038).** A
  record's tier, its retention horizon and its composition budget are all
  resolved from the *effective pack at its scope, at the moment of the
  read or the sweep*. So moving a node moves every record it carries into
  a different regime, retroactively, with no diff and nothing to
  reconcile. This is the fact the mover decision turns on.
- **Placement is derived, and quarantine is a place (ADR-0013 decision
  4).** There is no lifecycle column on `identities` today, and the
  reason is recorded: a second source of truth drifts the moment anything
  moves a user node. Whatever AUTH-4 adds has to survive that argument or
  answer it.
- **The identity is bound to a token subject, and SCIM does not have
  one.** `identities.subject` is `not null` and unique per tenant; a
  provisioning agent creating a user on the day they are hired knows
  their directory object id and their `userName`, and cannot know what
  `sub` their first ID token will carry.
- **The demo goal names Entra and Okta live.** Whatever this ADR decides
  about credentials has to be a thing those two products can actually be
  configured to send.

## Decision

1. **The SCIM plane is `/scim/v2`, speaks SCIM's shapes, and returns
   SCIM's errors.** `/Users`, `/Groups`, `/ServiceProviderConfig`,
   `/ResourceTypes`, `/Schemas`. Bodies are
   `urn:ietf:params:scim:schemas:core:2.0:{User,Group}` and
   `urn:ietf:params:scim:api:messages:2.0:{ListResponse,PatchOp,Error}`,
   never the product's error envelope. A provisioning agent parses SCIM
   errors and reports them to an administrator; wrapping them in our
   envelope would turn every failure into an unparseable one for the only
   audience that sees it. This is not `/v1` and takes no `/v1` bearer
   token (decision 11).

2. **SCIM carries directory facts, never product instructions.** A SCIM
   request names a user, their `active` flag, and their groups. It never
   names a hierarchy scope, a record, a role, a pack or a channel — those
   are not in the wire format and will not be added to it as extensions.
   Everything downstream is the product's own resolver plus the effective
   pack. This is what preserves ADR-0013's reachability argument
   unchanged: no governed asset is reachable from this plane, so seed
   §2.2 binds exactly where it bound before, and the material effects
   (decision 9) are the *pack's* decisions rather than the caller's.

3. **A directory mirror, then a reconciler.** `scim_users`,
   `scim_groups`, `scim_group_members` (migration 0036) are the SCIM
   resource of record: every attribute a conformant GET must echo,
   `externalId`, `active`, and the `meta.version` ETag. `identities` and
   hierarchy placement are a *projection* of that mirror, written by one
   function — `scim::reconcile(tenant, scim_user_id)` — which is the only
   writer of the projection anywhere in the product.

   The mirror is not redundancy. A conformant server must return
   attributes it has no product meaning for (`name.givenName`,
   `phoneNumbers`, `title`), must answer `GET /Users/{id}` for a user who
   has never logged in, and must let a PATCH of one member of one group
   be applied without the agent resending the world. Projecting straight
   into `identities` would mean either storing product-meaningless
   columns there or lying to the client about what it sent us.

   AUTH-5 drives the same `reconcile`, which is the whole reason that
   feature is an M: a pull sync writes the mirror from a directory read
   and calls the same projection. ADR-0013's reversal trigger anticipated
   exactly this and is hereby discharged in the direction it named.

4. **The correspondence between a SCIM user and a token subject is a
   per-issuer claim, with an ordered fallback, and it never produces two
   identities for one person.** `IssuerConfig` gains `external_id_claim`
   (default `sub`). Reconciliation matches, in order:

   1. `identities.external_id` = the mirror row's `externalId`;
   2. `identities.subject` = the mirror row's `externalId` (the case
      where the directory's anchor *is* the token subject);
   3. `identities.email`, case-folded, = the mirror row's `userName`.

   A SCIM POST for somebody who has already logged in **adopts** the JIT
   identity and stamps its `external_id`; a login for somebody SCIM
   already created adopts that row rather than provisioning a second one.
   Both directions are the same rule and both are tested.

   This decision exists because the obvious implementation is wrong in a
   way that is invisible until it has cost a customer their memory. Entra
   issues a pairwise `sub` — unique per (application, user) — so an Entra
   tenant's `sub` will *never* equal the directory object id its
   provisioning agent sends, and a server that joins on `sub` alone gives
   every Entra user two identities, two personal scopes, and half their
   memory in each. `external_id_claim` is set to `oid` for Entra, beside
   `groups_claim`, which is the per-issuer seam ADR-0010 already built
   for exactly this class of vendor difference.

   The fallback exists because `externalId` is *the customer's attribute
   mapping*, not a protocol constant, and the Entra default for a custom
   application is a mutable attribute rather than the object id. We
   document mapping it to an immutable one, we cannot enforce it, and so
   correctness does not depend on it: a changed `externalId` that still
   matches on `userName` is a **re-anchor** of the existing identity, not
   a new person — except where decision 12 forbids it.

5. **A pre-login identity has no subject.** `identities.subject` becomes
   nullable, with a partial unique index on `(tenant_id, subject) where
   subject is not null`, and a new partial unique index on `(tenant_id,
   external_id)`. First login binds the subject to the row the directory
   already created. A row with no subject authenticates nothing — it is a
   placement and a personal scope waiting for a person — and every seam
   that resolves a caller keys on `subject`, so a null one is
   unreachable rather than special-cased.

6. **Joining is ADR-0013's resolver, called from somewhere else.** The
   `group_mappings` override table, then the `synveda-{dept}-{team}`
   convention, groups in lexicographic order, first resolution wins,
   nothing resolves → quarantine. Not one line of that changes. What
   changes is where the group list comes from — `scim_group_members`
   rather than a token claim — and when it runs. The audit event stays
   `identity.provisioned` with its existing payload shape, so a chain
   consumer cannot tell the two doors apart, which is correct: they
   produce the same thing.

7. **Leaving seals, and the seal is stored once, on the identity.**
   `identities.status` — `active` | `departed`, with `departed_at`. The
   scope's sealed-ness is *derived* through the 1:1
   `identities_scope_unique`: a user-kind node is sealed iff the identity
   that owns it is departed. There is no `sealed` column on
   `hierarchy_nodes` and there must not be one.

   This is ADR-0013 decision 4 applied rather than contradicted. That
   decision refused a `quarantined` column because placement *already*
   answered the question and a second copy would drift. Departure is
   answered by nothing else in the schema, so it is stored — once — and
   everything else derives from it.

8. **The seal is three things at three layers, and deliberately not a
   fourth.**

   - **The principal stops authenticating.** `authz::require` refuses a
     departed identity the way it refuses a quarantined one: at the
     enforcement seam, fail closed. An access token minted before
     departure stops working on the next request, without waiting for
     AUTH-6's revocation list and without depending on the IdP having
     revoked anything.
   - **The scope stops being readable.** A base-layer forbid on
     `resource.sealed` (decision 9) — so no pack, no role and no future
     carve-out reaches it. The lapse permit already cannot: its
     `resource.kind != "user"` privacy floor (ADR-0015 decision 4) means
     no personal scope is disclosable by a lapse, sealed or live.
   - **The material stops being disposable.** The MEM-6 sweep skips
     sealed scopes at both the expire and destroy stages. This is the
     "retention-held" half and the only one that changes an existing
     loop: a retention hold whose whole purpose is to survive a schedule
     must not be implemented as a schedule.

   What the seal is **not** is a reader. This feature adds no path by
   which anybody sees a departed person's personal memory — not
   compliance, not legal, not an org-admin. "Unreadable by default" is
   satisfied by there being no default and no non-default; inventing an
   e-discovery disclosure inside a lifecycle feature would be adding the
   product's first "read somebody's private material" surface as a side
   effect of somebody resigning. It belongs to the export plane, with its
   own ADR and its own approval matrix, and it is recorded as a deferral
   with a trigger below.

9. **`sealed` is a Cedar attribute on `Scope`, resolved where the chain
   is built.** The schema gains `sealed: Bool` on the `Scope` entity;
   HIER-3's entity materialisation resolves it by joining the owning
   identity's status for user-kind nodes; freshness is inherited from the
   HIER-2 chain cache, invalidated through `AppState::invalidate_hierarchy`
   — the single seam ADR-0016 decision 5 and ADR-0017 decision 5 built
   for out-of-process writers and named this feature in.

   Cost is bounded by shape: user nodes are leaves, so at most one node
   in any chain can carry a true value, and the join rides the query the
   chain fragment already runs.

10. **A move is only a policy question when it changes the policy.** When
    the directory moves somebody, the personal node moves under the newly
    resolved parent (`hierarchy::move_node`, the audited path
    release-from-quarantine has used since ADR-0013). If the source and
    destination scopes resolve the **same effective pack**, the material
    follows and nothing is asked: nothing is re-priced, so there is no
    decision to take. If they resolve **different** packs, the *source*
    scope's pack decides, because authority over material belongs where
    the material is — ADR-0037's rule for lapses, applied to a move.

    `PackConfig` gains `mover: Option<MoverConfig>` with one field today:
    `personal_memory: Follows | SealsAndRestarts`. `SealsAndRestarts`
    seals the old personal scope in place (decision 8's seal, exactly)
    and creates a fresh personal scope under the new parent.
    `regulated-strict` configures `SealsAndRestarts`; `standard` and
    `open-collaboration` configure `Follows`. An unconfigured stored pack
    resolves to `SealsAndRestarts`.

    **The hazard a move carries is disposal, not disclosure, and that is
    the finding this decision exists for.** The instinct is that moving
    somebody's notes into a looser department discloses them — it does
    not: every embedded pack excludes user-kind scopes from every
    content-role grant, and a lapse cannot target one, so a personal
    scope is readable by its owner and nobody else no matter where it
    hangs. What actually changes is the **retention regime**. ADR-0040
    decision 10 resolves a record's horizons at the scope it lives at,
    per sweep, with nothing stamped on the record — so moving a node from
    a department that keeps material for seven years into one that keeps
    it for ninety days is a *bulk destruction* that nobody approved, that
    no diff shows, and that happens on a background loop's next pass.

    That is also why the unconfigured default is the sealing one, against
    the instinct that the friendlier default is better: ADR-0040 decision
    13's sentence is "a pack that configures nothing must not start
    destroying memory", and between the two options only one of them can.
    This is not ADR-0053's fail-safe (a gate nobody opted into must not
    start refusing things), because nothing here refuses: the move always
    succeeds. What varies is only whether material crosses a regime
    boundary, and the unconfigured answer is the one that cannot lose it.

    With today's embedded packs no record horizon is set at all
    (`ClassTtl::KEEP`, `destroy_after_days: 0`), so no move destroys
    anything yet. The config exists because the moment a customer sets a
    horizon — which is precisely the moment they have started caring
    about retention — a move silently becomes a disposal decision.
    Naming it now costs one optional field; finding it later costs
    somebody's data.

11. **Losing every group is quarantine; only an explicit deactivation
    seals.** A PATCH that removes a user from the last group mapping to
    anything re-resolves to nothing, which ADR-0013 already answers:
    quarantine. `active: false`, and `DELETE`, are the leaver signal and
    the only ones. The distinction is the difference between a
    misconfigured group and a person losing their memory: quarantine is
    reversible by fixing the mapping, and a seal is not (decision 12).

    `DELETE /Users/{id}` therefore **seals and does not delete**. It
    answers `204`, the resource then answers `404` on `GET` as RFC 7644
    requires, and the mirror row is retained and marked. `userName`
    uniqueness is enforced over live rows only, so a rehire is not `409`d
    by a departed row holding their old address.

12. **The seal does not lift, and a rehire is a new person.** Nothing in
    the SCIM plane reactivates a departed identity: `active: true` on a
    departed user creates a *new* identity with a *new* personal scope
    and leaves the sealed one sealed. Decision 4's `userName` re-anchor
    is refused against a departed row for the same reason.

    A lifting path would mean the retention hold — the one thing on this
    plane that a regulator is relying on — could be undone by whoever
    holds the provisioning credential, which after a directory compromise
    is the attacker. A hold that the directory can release is not a hold.

13. **The credential is a static bearer token, tenant-bound, hashed,
    expiring, rotatable in pairs, and confined to this plane.** Issued by
    `synveda scim token issue` behind a PDP decision at the tenant
    resource; stored as SHA-256 with an expiry (required, capped, AUTH-3's
    lifetime-cap doctrine) and `last_used_at`; two may be live at once so
    that rotation does not stop provisioning.

    A static secret is not the credential this product would choose —
    AUTH-3 exists because short-lived scoped tokens are better — and it
    is the credential Entra can be configured to send for a non-gallery
    application, which the phase demo goal names. Confinement does the
    work instead: a SCIM token is refused by the `/v1` router, a `/v1`
    bearer token is refused here, and the plane it does reach holds no
    governed asset by decision 2.

    **This does not close ADR-0055's headless-`init` deferral**, and the
    reason is worth recording rather than discovering later: issuing the
    credential is itself PDP-gated at `org-admin`, so it presupposes the
    identity a headless install does not yet have. AUTH-4 gives that
    deferral a joiner path; it does not give it a bootstrap.

14. **Reads on this plane are traced and metered; state changes are
    chained.** ADR-0019 decision 4's first sentence — every allowed
    admin-plane read chains one event — does not extend here, and this is
    the second time it has been bounded rather than the first (CNSL-2's
    fan-out aggregation was the first). A provisioning agent polls: Entra
    re-reads its assigned users every cycle, so chaining reads would make
    the tenant's audit chain mostly a record of a directory reading its
    own copy back, and the events that matter would be needles in it.
    Nothing on this plane is governed content, so a read discloses
    nothing the directory did not send us.

    New chained actions: `identity.moved`, `identity.sealed`,
    `scim.credential.issued`, `scim.credential.revoked`. The actor is
    `ActorKind::System` named by component — the kind ADR-0022 already
    minted for "sweeps and AUTH-4/5 sync jobs" — with the credential id
    in the payload, so the chain answers *which* credential sealed an
    identity.

15. **Conformance is two suites, and each says what it can stand
    behind.** A **protocol** suite built from the RFCs' own examples (RFC
    7643 §8 resources; RFC 7644's ListResponse, PatchOp, filter and Error
    shapes), with `/ServiceProviderConfig` advertising exactly what is
    implemented and the suite asserting the advertisement matches the
    behaviour. And a **vendor** corpus of Entra and Okta request frames,
    each fixture labelled by provenance — replayed from a live tenant, or
    transcribed from the vendor's published compliance table — in
    ADPT-2's idiom, so that "spec-compliant" never means more than what
    was actually exercised.

    The filter grammar is implemented as the subset those two clients
    send (`eq` on `userName`, `externalId`, `id`, `displayName`), and
    everything else answers `501` with `scimType: "invalidFilter"`, which
    RFC 7644 §3.4.2.2 provides for. The bound is not laziness: filters
    compile to sqlx compile-time-checked queries (CLAUDE.md), so the
    grammar we can accept is exactly the grammar we can express without
    building SQL from strings, and a parser that accepted more than it
    could run would be a rejection at evaluation time wearing a
    conformance badge.

## Options considered

1. **Mirror + one reconciler (chosen)** — the SCIM resource of record is
   separate from the product's identity, and one function projects. Con:
   two tables to keep consistent, and a reconciler that must be
   idempotent. Pro: conformant GETs, AUTH-5 for free, and no
   product-meaningless columns on `identities`.
2. **SCIM writes `identities` directly** — fewer tables. Rejected: it
   cannot answer a conformant GET without storing attributes the product
   has no meaning for, and it gives AUTH-5 nothing to reuse, so the pull
   sync would be a second implementation of the same lifecycle.
3. **Sealing as a placement** — move the departed node under a reserved
   `former` scope, deriving sealed-ness the way quarantine derives.
   Rejected twice over: a move re-prices the material (decision 10) so a
   retention hold would depend on where the node hangs, and it would make
   the leaver and the mover the same physical act, which is the one
   confusion this feature cannot afford.
4. **A `sealed` column on `hierarchy_nodes`** — one less join at entity
   materialisation. Rejected: a second truth beside the identity's
   status, drifting exactly the way ADR-0013 decision 4 described, on a
   table that holds no other lifecycle state.
5. **Deleting the leaver's material** — the "right to be forgotten"
   reading. Rejected: the feature text says retention-held, and
   destruction in this product is ADR-0040 decision 13's deliberate,
   flagged act that no embedded pack configures. An erasure obligation is
   a real requirement and it is a different feature, with a different
   approval matrix.
6. **A compliance reader for sealed scopes** — the obvious other half of
   "unreadable by default". Rejected here (decision 8): it is the
   product's first surface for reading somebody's private material, and
   it must not arrive as a side effect of a lifecycle feature.
7. **Ranking packs and letting material follow into a stricter regime
   automatically** — attractive, and it would make decision 10's config
   unnecessary in the safe direction. Rejected: it needs a total order
   over packs, which this product has never had and cannot have, because
   a stored custom pack's position in it would be the customer's own
   claim about their own policy.
8. **The full SCIM filter grammar** — maximal conformance. Rejected:
   string-built SQL is forbidden (CLAUDE.md), and the honest alternative
   — parse everything, reject most of it at evaluation — is worse than a
   documented subset plus the `501` the RFC provides.
9. **Client-credentials only, no static token** — the credential AUTH-3
   would want. Rejected: Entra's non-gallery SCIM provisioning is
   configured with a static secret token, so this would make the phase
   demo goal ("Entra live") unreachable on the product's own terms.
10. **Re-resolve placement on every login instead of a SCIM plane** —
    cheap, and it makes movers work with no new surface. Rejected: it
    makes the token's claims and the directory's push two authorities for
    one fact, and it cannot see a leaver at all, because a leaver is
    exactly the person who stops logging in.
11. **Do nothing** — joiner/mover/leaver stays manual. Fails the AC
    outright, leaves ADR-0013's first-login-final placement permanent,
    and leaves nine recorded deferrals standing.

## Consequences

- Positive: a person's arrival, movement and departure are the
  directory's to state and the product's to enforce; ADR-0013's
  placement-is-final consequence is discharged; ADR-0015's
  revocation-stays-explicit note gets its automatic path; the sweep gains
  a hold that a schedule cannot age out; AUTH-5 is a loop around a
  function that already exists and is already tested; and a departed
  person's token stops working at the next request rather than at the
  next AUTH-6 feature.
- Negative / accepted trade-offs: a nullable `subject` on `identities`
  is a schema loosening on the product's most security-relevant table,
  bounded by two partial unique indexes and by every caller seam keying
  on it; the base layer gains a forbid and the `Scope` entity an
  attribute, so the embedded packs go to `@15` and every golden matrix
  is re-recorded (the diff must be *only* the sealed rows, which is what
  makes it checkable); a static bearer credential exists in a product
  that had eliminated them; sealed material is unreadable by everyone
  including the people who will eventually need to read it, until the
  export plane lands; and `mover.personal_memory` is a policy field whose
  effect is invisible under the embedded packs today, which makes it
  exactly the kind of field that is easy to get wrong later and hard to
  notice.
- Reversal trigger: if a customer's directory sends a mover signal for
  somebody whose personal scope is under a *stored* pack and the
  seal-and-restart default surprises them into a support ticket, the
  default is wrong and the fix is the pack's, not the code's — but two
  such tickets means the default should be `Follows` with a loud warning
  at move time. If Entra's assigned-user re-read cycle turns out to cost
  more than the polling budget at a real tenant's user count, `/Users`
  gains ETag-conditional responses before it gains anything else. And if
  a legal hold is ever asserted over a *live* person's scope, the seal's
  three layers separate: the retention hold is the only one wanted, and
  it stops being derivable from departure.

## Compliance notes

- **Audit.** Four new chained actions (decision 14) plus the existing
  `identity.provisioned` and the hierarchy events a move already chains.
  Every seal, move and credential lifecycle event carries the credential
  id, so "who deprovisioned this person" is answerable from the chain
  alone. Reads on the plane are deliberately unchained, with the reason
  recorded in decision 14 rather than left as an omission.
- **Multi-tenancy.** Three new tables and one new credential table, all
  tenant-scoped, all shipping forced RLS + policies + least-privilege
  grants in their own migration per the ADR-0009 structural rule, all
  joining the TEN-2 adversarial suite and its completeness guard. The
  SCIM credential resolves the tenant *before* any query runs — a
  credential is bound to one tenant at issuance and there is no
  tenant-selecting parameter on the wire, so a cross-tenant SCIM request
  is unrepresentable rather than denied.
- **Policy enforcement.** Seed §2.2 holds where it binds, by decision
  2's reachability argument rather than by exemption: the plane reaches
  identities, placements and lifecycle state, and no governed asset. The
  material effects — what follows a mover, what a seal makes unreadable
  — are the PDP's and the effective pack's, never the caller's, and the
  caller has no vocabulary to express them in. The new base-layer forbid
  is an invariant no pack can drop, which is the same guarantee
  quarantine has had since ADR-0013 decision 5.
- **`identities` grants.** Migration 0007 granted `select, insert` and
  said update and delete would arrive with this feature. They arrive as
  `update` only: nothing in AUTH-4 deletes an identity row, because
  every lifecycle end in this ADR is a seal.
