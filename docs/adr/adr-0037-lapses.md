# ADR-0037: Lapses — expiry is a property of the decision rather than of a job, the permit is invariant, and what a lapse discloses is what the target scope already stands behind

- **Status**: Accepted
- **Date**: 2026-07-26
- **Feature(s)**: AUTHZ-4
- **Deciders**: sujitn

## Context

AUTHZ-4's text is "lapse = time-boxed policy override proposal; reason
mandatory; dual approval under regulated-strict; Temporal timer
auto-revert; all transitions audited", and its acceptance criterion is
"E2E — lapse grants cross-team read, expiry restores denial, audit shows
the full story". Seed §6 states the product claim it serves: "a steward
may apply a scoped, reasoned, time-boxed override ('allow team X to read
team Y's `procedure` records for 30 days — reason: joint incident
review'). Lapses require a second approver in `regulated-strict`, are
fully audited, and auto-expire. **This is the mechanism that lets one
product serve both an SMB and a bank.**"

Unusually for a feature this size, most of the machinery already exists
and two earlier features wrote this one's name into their own source.
`regulated-strict.cedar`'s header says a content-role binding "*is* the
seed's 'explicit grant' for cross-team read; **AUTHZ-4 lapses add the
time-boxed variant**". `AssetKind::Policy`'s doc comment says "a policy
pack or lapse, flowing through the same propose/review/approve path as
everything else it governs". And every embedded pack has carried a
`policy` approval rule since FLOW-3, `regulated-strict`'s written
straight off tech plan §2.4's lapse row — 2 × steward, two distinct
approvers — under a comment that quotes it. That cell has been rendered
into a 300-cell golden for two features without a single proposal ever
resolving against it. This feature is what it was written for.

Forces at play:

- **A lapse is an override, so the mechanism that carries it cannot be
  optional.** If a pack can neutralise lapses by not mentioning them,
  "lapse" means different things in different tenants, which is the exact
  trustworthiness rule ADR-0014 decision 6 reserved the product pack
  names for.
- **A timer that *is* the expiry fails open.** The feature text names a
  Temporal timer, and nothing in the workspace depends on Temporal —
  MEM-3 chose PGMQ (ADR-0022) and FLOW-4 a gateway background loop. That
  is a sequencing accident. The load-bearing objection is different: if
  a job is what ends a grant, then a job that does not run leaves a
  cross-team read standing, and there is no worse shape for the one
  feature whose entire promise is that access ends by itself.
- **The `MemoryRead` seam decides once per scope and cannot see a
  record.** Seed §6's own example narrows to "`procedure` records". The
  composition seam has no record in hand when it decides
  (`permitted_chain_scopes` asks once per candidate scope), so a
  record-type qualifier is not enforceable at the seam a lapse widens.
  A narrowing that nothing applies is a widening wearing a narrowing's
  name.
- **The inject candidate universe is the chain, and a permit that is
  never asked grants nothing.** ADR-0024 fixed it there deliberately —
  "scopes packs permit *beyond* the chain — bound subtrees, `standard`'s
  department subtree — are recall's deep-query surface; CTX-5 owns
  enumerating a broader universe". The consequence, which this feature
  is the first to run into, is that ADR-0015's "explicit grant" for
  cross-team read has never reached `inject`: the PDP would permit it and
  nothing asks. A lapse that only produced a permit would satisfy the
  letter of its AC at the PDP seam and change nothing a reader sees.
- **`inject` is still the hot path** (p99 150ms, seed §3). Whatever the
  read path learns about lapses, it pays for per request.
- **Whatever is disclosed, somebody has to have consented to it
  specifically.** Two stewards approving "team X may read team Y" have
  to be able to know what that means before they approve, or the dual
  approval is ceremony.

## Decision

A lapse is **an ordinary FLOW-3 proposal whose asset is `policy` and
whose effect is a grant row**, granting one action from one scope to one
scope until an instant recorded on the row. **Expiry is a property of
the decision, not of a job**: the read that assembles every authorization
context selects only unexpired, unrevoked rows, so nothing has to run for
a lapse to end. What the grant admits to the reader is **the target
scope's published channel and nothing else**.

Decisions, specifically:

1. **A lapse is a proposal, and it is the `policy` cell the matrix has
   already held.** No new asset kind (`AssetKind::Policy` names lapses in
   its own doc comment), no new approval rule (every embedded pack has
   one, and `regulated-strict`'s is tech plan §2.4's lapse row), no new
   proposal actions, and no new review surface: FLOW-6's `synveda
   proposal list/show/review/approve/reject` reviews lapses the day this
   lands, because a reviewer's verbs never knew what asset they were
   reviewing.

   The proposal's commit names exactly one member: an `AssetKind::Policy`
   object holding the lapse's terms in canonical form, entry-named by the
   lapse id. ADR-0032 decision 6 — approvals bind bytes — therefore holds
   *structurally* here rather than by a recheck: the object is the only
   copy of the terms, so there is no row to edit under an approval and
   nothing for the publish-time address comparison to catch.

2. **A lapse's terms are grantee scope, target scope, action, duration,
   and reason. Nothing else.** The grantee is a *scope* because seed §6's
   example is "team X", and a scope-shaped grantee subsumes a
   subject-shaped one: a single person is their own personal scope, so
   "let Dana in for a day" needs no second mechanism. `principal in
   grantee` is the same membership test every pack already writes.

   The action vocabulary is a closed subset of `Action` containing
   `MemoryRead`. A lapse naming anything else is refused by name at open
   time. Widening the admin plane on a timer is a different product, and
   the CLI's break-glass already covers the case it would serve.

3. **The disclosing side opens it.** The target is the scope whose
   material is disclosed; requirements resolve there (FLOW-3's rule,
   unchanged), and `ProposalOpen` at the target is floored on membership
   plus contributor-and-above on the target's chain. A steward of the
   team that *wants* the access therefore cannot open the proposal that
   grants it unless they are also bound above the target. That is
   correct and it is the product being opinionated: a disclosure is
   always initiated on the disclosing side, and asking is a conversation,
   not an API call.

4. **Expiry is a property of the decision, not of a job.** The grant row
   carries `expires_at` and `revoked_at`; `authz::gather` — the one seam
   that assembles a decision context — reads only rows that are neither.
   A sweep that never runs cannot leave a grant standing, because nothing
   consults the sweep.

   Temporal is not used, and the timer the feature text names becomes
   **bookkeeping**: a pass on FLOW-4's background-loop pattern chains one
   `policy.lapse.expired` per lapse, idempotently. If it is down the
   trail is still complete — the grant event records the window it opened
   — and the grant is still gone.

   Duration is **seconds on the wire**, and there is no minimum. That is
   what lets the acceptance test and the demo observe a *real* expiry
   rather than a simulated one, which is the difference between
   demonstrating the AC and asserting it. The clock starts when the
   effect runs, not when the proposal opens: a proposal that sits in a
   queue for a week must not spend the window it was approved for.

5. **The ceiling is pack configuration under a product maximum, and zero
   means no lapses.** `LapseConfig` joins `RedactionConfig`,
   `CompositionConfig`, `ApprovalMatrix`, and `PromotionConfig` on
   `PackConfig`: `regulated-strict` 30 days (seed §6's own example), the
   relaxed packs 90, and 90 is the product maximum no pack can raise. A
   pack setting `max_duration_secs: 0` admits no lapse at all — the
   config narrows and never grants, which is `CompositionConfig`'s rule
   (ADR-0025 decision 2) and makes "this tenant does not do lapses" a
   configuration rather than a Cedar exercise. "Auto-expiry mandatory"
   needs no validation: a duration-less lapse is unrepresentable.

   **The zero case is enforced at decision time as well as at grant
   time**, so a pack that admits no lapses admits none on the very next
   request, standing grants included (ADR-0014 decision 3's doctrine).
   A *shortened* ceiling is not: it changes what may be granted, not what
   stands, because the grant that stands cleared the matrix under the
   rules in force when it was approved. Nothing is deleted either way —
   an unresolved grant keeps its row and its window, so restoring the
   pack restores it for the remainder and the expiry sweep still closes
   it on time.

6. **A qualifier the seam cannot enforce is refused, not stored.** A
   lapse may not narrow to a record type or a sensitivity, and one that
   tries is refused at open time naming the reason. The alternative —
   store it and apply it later — would mean a lapse that reads "procedure
   records only" grants everything, which is the single most dangerous
   thing this feature could ship. AUTHZ-5 brings per-record context to
   the decision; the field arrives with it, the matrix then resolves at
   the declared ceiling instead of the fixed one (decision 14), and the
   refusal lifts without a format change.

7. **The permit lives in the base layer — the product's first base-layer
   permit.** `base.cedar` has held only forbids until now, and the
   asymmetry is worth naming rather than sliding past: this is an
   *override* mechanism, and one a pack can neutralise by omission is one
   that means different things in different tenants. A pack that wants no
   lapses says so with decision 5's ceiling, visibly, or writes a forbid,
   which Cedar's semantics honour over any permit.

   What still forbids over it is the point of putting it there: a
   quarantined principal is unaffected, and **a service identity cannot
   be widened past its anchor by a lapse**, because ADR-0018's
   confinement forbid carves out only own-chain `MemoryRead`. Neither
   needed a clause in this feature.

8. **The permit is `resource == target`, and never a user-kind scope.**
   Not the target's subtree: composition asks about the target and only
   the target, and material living below the target reaches the reader
   through what the target *published* — the set the approvers could
   inspect. Granting the subtree would grant scopes nobody reviewed, on
   the strength of an approval given about a different one.

   A user-kind target is refused at open time by name *and* excluded in
   the permit — loud and safe, the ADR-0032 discipline of refusing at
   install rather than discovering at review. This is ADR-0015
   decision 4's privacy floor restated where it cannot be forgotten:
   nobody's personal memory is disclosed by a lapse, and an
   investigation that genuinely needs one person's corpus is a different
   feature with a different name.

9. **The match resolves in Rust; the permit is Cedar.** `AuthzContext`
   gains `lapses`, the PDP filters them exactly as `effective_roles`
   filters bindings — grantee contains the principal, target is the
   resource, action matches — and sets `context.lapsed` on `MemoryRead`
   requests. Two layers, the same two AUTHZ-3 established: resolution is
   data handling, authority is a policy. One seam reads the rows, so the
   `expires_at` predicate exists in exactly one query.

10. **The read path's change is which scopes the plan contains, plus
    decision 12's marker.** A lapse-named target enters
    `composition_plan`'s output as an ordinary planned scope, **after**
    the chain and with the derived channel off; `ComposeScope` carries
    the lapse it arrived by, which is what the marker and the audit event
    render. Everything else downstream already works: the
    gradient position falls out of plan order (so a lapsed scope loses
    every conflict against the reader's own material), the section header
    is the target's path, and FLOW-5's fetch-published-members-by-id
    handles residence.

    **Hybrid retrieval does not change at all.** Published members are
    fetched by id and uncapped (ADR-0031 decision 9), so no scope
    predicate, index, or sidecar has to learn what a lapse is. That is
    not luck — it is the same property that let FLOW-5 admit a record
    living below the scope that published it, and it is the strongest
    available argument for decision 11.

    What it does cost is honest and worth stating: a lapsed scope is not
    on the caller's chain, so deciding `MemoryRead` there needs the
    *target's* chain and its assignment rows — the effective pack is a
    property of the resource (ADR-0014 decision 3), and a decision taken
    without them would silently fall back to the tenant default and
    materialise an entity graph with no ancestry. HIER-2's cache makes
    the chain warm; the assignments are one more indexed read per lapsed
    scope, paid only by callers who actually hold one.

11. **A lapse admits the target's published channel and nothing else.**
    Derived material is unreviewed extraction output that nobody at the
    target has looked at; disclosing *that* to another team on an
    override is precisely the accident this product exists to prevent,
    and "what the target scope stands behind" already has a name. It also
    makes dual approval mean something: two stewards can read the channel
    and see exactly what they are consenting to, which they cannot do for
    a corpus that grows while they deliberate.

    It is a narrowing of what the pack would otherwise permit, and
    narrowings are always available to this product. The cost is
    honest and stated: **a lapse over a scope that has published nothing
    discloses nothing**, and the remedy is to publish, which is a review.

12. **The block says so.** A section composed from a lapsed scope is
    marked, and the `context.injected` event names the lapse beside the
    scope in its aggregated decisions. This is CTX-3's degradation-header
    and ADR-0036 decision 10's `pinned` flag as a standing rule: a
    response that deliberately differs from the expected one has to say
    that it does.

13. **The universe widens by lapse and by nothing else.** Content-role
    bindings and `standard`'s department sharing stay off the inject
    path, exactly where ADR-0024 put them. The asymmetry is deliberate
    and the reason is not "smaller diff": a lapse **enumerates** (a row
    naming one target, which is what the chain-only universe lacks), is
    bounded in time, carries a mandatory reason, cleared a dual-approval
    matrix, and admits only reviewed material. A binding is durable,
    needs no approval, and would bring the derived channel with it —
    putting another team's unreviewed extraction output into every bound
    reader's block, which is a much larger decision than this feature's
    and belongs to whoever makes it.

14. **A lapse resolves the matrix at `internal`.** That is the tier
    `inject` composes: the read path clamps below `restricted`
    unconditionally (ADR-0024 decision 2) and requests default to
    `internal`. So the invariant floor does not engage, and the pack
    rules decide — `regulated-strict` asks its two stewards, `standard`
    and `open-collaboration` one, which is tech plan §2.4's SMB collapse
    landing where it was written to land. When AUTHZ-5 lets a lapse
    declare a higher ceiling, the matrix resolves at *that* ceiling and
    the floor engages by itself, with no lapse-specific rule anywhere.

15. **Revoking takes no matrix, and it is its own action.** `LapseRevoke`
    ends a grant early with a mandatory reason and resolves no approvals.
    ADR-0036 decision 3 made `ChannelRollback` matrix-free because a
    rewind can only install a state the matrix already cleared; a
    revocation is the simpler case of the same family — it installs
    nothing and can only narrow, and a product whose answer to "that
    grant was a mistake" is "convene the two stewards again" has not
    shipped revocation.

    It is a second action rather than a mode of the first for the other
    half of that decision: separability. A pack must be able to grant one
    broadly and the other narrowly — the responder who should be able to
    end a lapse at 3am is not the steward who should be able to open
    one — and two acts sharing an action share a grant forever.

16. **One new table and one migration.** `policy_lapses` (migration 0022)
    is the granted proposal's projection in typed columns, because the
    read path reads it per request and parsing an object per decision is
    not a read path. SELECT/INSERT/UPDATE and no DELETE, with a trigger
    admitting exactly the two transitions a grant has (revoked, and
    expiry recorded), forced RLS, the completeness guard, and the
    adversarial suite — ADR-0032's shape for `vedaflow_proposals`,
    unchanged.

    The proposal row needs one column widened and nothing else.
    `source_scope_id` equals `target_scope_id` (nothing moves, so FLOW-5's
    direction rule keeps its meaning) and `sensitivity` is decision 14's;
    but `target_channel` carries `check (target_channel = 'published')`
    from migration 0019, and a lapse has no target channel. Migration 0022
    widens it to `in ('published', 'lapse')`, which makes the column name
    the proposal's **effect** rather than always a channel — a mild
    tension, named here rather than papered over by storing `published`
    on a row that publishes nothing.

    `Channel` deliberately does not gain a variant. `lapse` is a literal
    in one CHECK, not the second half of a ref name: no scope has a
    `policy/lapse` ref, nothing writes one, and `GET /v1/channels` has
    nothing new to skip.

17. **Three new audit actions; the review half reuses FLOW-3's four
    unchanged.** `policy.lapse.granted` carries the proposal, both
    scopes, the action, the window, the reason, and the requirement as
    resolved; `policy.lapse.revoked` carries the actor, the reason, and
    the window it cut short; `policy.lapse.expired` is the sweep's, under
    `actor_kind=system` (migration 0014). `proposal.opened`,
    `.approved`, `.rejected`, `.withdrawn` already carry asset kind and
    target, so the review half of a lapse's life is auditable today.

## Options considered

1. **A proposal whose effect is a grant row, expiry read at decision
   time, published-channel-only (chosen)** — reuses the review
   machinery, the CLI, and the matrix cell wholesale; nothing has to run
   for access to end. Con: the read path gains a per-request query, and
   a lapse over a scope that has published nothing is inert.
2. **A lapse as a time-boxed role binding** — the tidiest reading of
   `regulated-strict.cedar`'s own comment ("the time-boxed variant"),
   and Cedar would not change at all. Rejected on the grantee:
   `role_bindings` is subject-keyed, so "team X" is N rows that do not
   track membership, and a person joining X during a joint incident
   would need a second act nobody would remember to perform.
3. **A lapse as a Cedar policy, generated per grant and compiled into the
   tenant's pack** — one language, one engine, and the decision log names
   the lapse's own policy id for free. Rejected: it makes granting and
   *expiring* both recompiles, so the timer becomes the mechanism again
   (the objection in Context), and a tenant with a thousand grants
   carries a thousand policies through every evaluation.
4. **Free-form Cedar in the lapse's terms** — maximum expressiveness, and
   the seed's "scoped" would need no vocabulary. Rejected: it is a
   policy-authoring surface behind a two-steward approval, and the two
   stewards would be reviewing Cedar. A typed vocabulary is what lets the
   review show what the lapse does.
5. **The permit per-pack, pack-uniform, like the channel and proposal
   planes** — consistent with how every other cross-boundary plane was
   added. Rejected: those planes are *restrictions* that a pack omitting
   them cannot loosen, and this is a relaxation a pack omitting it
   silently deletes. Same shape, opposite failure mode.
6. **Active lapses as a fourth VedaFlow channel, one ref per scope whose
   tree is the standing set** — governed history for free, `Channel`
   grows one variant, and FLOW-7's rewind becomes revocation. Rejected on
   the same fact ADR-0032 decision 2 rejected `staged` for: a set channel
   cannot express withdrawal, and expiry is withdrawal on a schedule — a
   background job rewriting governed history every time a grant lapses.
   It would also put a tree read on the inject path.
7. **`PolicyAssign` as the effect's action** — a steward who can assign
   `open-collaboration` at the target can already open it tenant-wide, so
   a separate action implies a separable authority that does not exist
   (ADR-0032 decision 15's argument for curator files). Rejected on a
   concrete coupling: `PolicyAssign` is the one action decided with
   `skip_self` (ADR-0014 decision 4), so a lapse at a node would be
   authorized under the node's *parent's* pack while its matrix resolved
   under the node's own — two packs in one act, which nobody would choose
   deliberately.
8. **A post-decision override: if the PDP denies, consult lapses** —
   no schema change to Cedar, no context attribute. Rejected: it is a
   second authorisation engine, `determining` stops explaining outcomes,
   and seed §2.2's one seam becomes one seam with an appendix.
9. **Widen the universe for role bindings too, so ADR-0015's explicit
   grant finally reaches inject** — arguably the bug fix this feature
   walked into. Rejected here, not forever: it changes what every
   existing tenant's inject composes, it would admit the derived channel,
   and it moves an EVAL-1 baseline for a reason unrelated to this AC.
   Decision 13 records the shape it would take.
10. **A direct route (`POST /v1/lapses`), the way ADR-0032 decision 8
    kept the direct publish route** — under `standard` the matrix asks
    for one steward, so a proposal is a two-step ceremony for a one-step
    requirement. Rejected: publishing is a routine curatorial act whose
    matrix legitimately says "one curator", and a lapse is by
    construction an exception with a mandatory reason. One call that both
    writes the reason and enacts it is the shape the trail exists to make
    impossible. The cost is one extra call, once per exception.
11. **A fake clock so the AC can advance time** — the usual way to test
    expiry. Rejected: durations in seconds with no minimum make a real
    expiry observable in a test that takes seconds, and an AC that
    asserts against a clock it controls has demonstrated the clock.
12. **Compose the lapsed scope under its own pack's channel rule, like
    any planned scope** — one rule everywhere, no special case, and the
    seed's example does not say "published". Rejected: under the default
    config that admits the target's derived material, which is the one
    thing the approvers cannot inspect in advance and the one thing
    nobody at the target has reviewed.

## Consequences

- **Positive**: nothing has to run for a grant to end, so the failure
  mode of the expiry mechanism is a missing audit line rather than
  standing access. The feature adds no asset kind, no approval rule, no
  proposal action, and no review surface — the matrix cell, the CLI, and
  the audit shape were all written by FLOW-3 and FLOW-6, and this is the
  first thing to use them. The read path gains one plan entry per lapse
  and one section marker; hybrid retrieval, the published-member fetch,
  conflict resolution, the budget, and the watermark are untouched, and
  no index, sidecar, or scope predicate learns what a lapse is.
  `compliance` and
  `security-reviewer` keep their meaning, and the invariant floor engages
  by itself the day a lapse can declare a higher ceiling. Seed §6's
  closing claim — the mechanism that lets one product serve an SMB and a
  bank — is now a configuration difference (30 days and two stewards
  versus 90 and one) rather than a paragraph.
- **Negative / accepted trade-offs**: every governed request pays one
  more indexed read, on the path ADR-0016 deliberately left
  per-request for freshness — and a lapse is the worst possible thing to
  cache, so it stays there. A lapse over a scope with an empty published
  channel discloses nothing, and the product's answer is "publish first",
  which is another review. The seed's `procedure`-records qualifier is
  refused rather than approximated until AUTHZ-5. `base.cedar` now
  contains a permit, so reading it is no longer "these are the things no
  pack can escape" but "these are the things no pack can change" — a
  weaker sentence, and the file says so. And the limit no technical
  control reaches: a person who legitimately read material during a lapse
  can re-observe it into their own scope afterwards. The chain records
  the read and the write; nothing prevents them, and no ADR should
  pretend otherwise.
- **Reversal triggers**: (a) the per-request lapse read showing in the
  inject budget (p99 > 150ms, seed §3) → it moves into HIER-2's cache
  keyed by the caller's chain, and the cache TTL becomes the resolution
  of expiry — a trade named here rather than made here, because it turns
  decision 4's guarantee into a bounded one; (b) a tenant needing a lapse
  over derived material (a joint incident whose reviewed set is genuinely
  empty) → `LapseConfig` grows a channel rule, and it is a new ADR
  because it changes what an approver is consenting to, not a
  configuration key; (c) role bindings needing to reach inject →
  decision 13's second enumerable source, with the derived-channel
  question as the thing that makes it its own ADR; (d) approvers
  routinely approving lapses without reading the target's channel → the
  review surface grows "this would disclose N records at these
  addresses", behind FLOW-6's renderer seam and ahead of any change to
  the matrix; (e) a lapse-shaped need for `MemoryWrite` (a seconded
  engineer contributing to another team) → the action vocabulary is the
  place, and the write floor's own shape (MEM-1, ADR-0020 decision 3)
  is the thing to reason about, not this ADR's.

## Compliance notes

The PDP stays the one enforcement seam and gains two actions
(`LapseGrant`, `LapseRevoke`) plus one context attribute
(`context.lapsed`, declared on `MemoryRead`). The base layer gains its
first permit, conditioned on that attribute; every forbid still overrides
it, so a quarantined principal is unaffected and a service identity
cannot be widened past its anchor (ADR-0018 decision 4 carves out only
own-chain `MemoryRead`). The permit is bounded to `resource == target`
and excludes user-kind scopes, which keeps ADR-0015 decision 4's privacy
floor intact with no clause about personal scopes anywhere in the lapse
code. Embedded packs bump to `@9`.

**Lapse laundering is already refused, and by a rule written for another
reason.** The obvious attack — read another scope's material under a
lapse, publish it onto your own channel, keep it after expiry — fails on
FLOW-5's direction rule: a proposal's target must be its source or a
strict ancestor of it, and a sibling team is neither, so sideways is
refused by name (ADR-0034 decision 2). Nothing in this feature relaxes
it, and the acceptance suite asserts the refusal rather than assuming it.

`policy_lapses` joins the forced-RLS set in migration 0022 with
least-privilege grants (SELECT/INSERT/UPDATE, no DELETE) and a trigger
admitting only revocation and expiry-recording, the ADR-0032 shape. It
joins the ADR-0009 completeness guard and the adversarial RLS suite,
where the cases that matter are a forged grant naming another tenant's
scopes, a revoked row resurrected by UPDATE, and an `expires_at` pushed
forward after the fact — the last being the one that would turn a
30-day grant into a permanent one without a second approval.

The trail carries the whole story on one chain: `proposal.opened` with
the requirement as resolved, one `proposal.approved` per steward with
the roles each counted under, `policy.lapse.granted` with both scopes and
the window, every `context.injected` that composed under it naming the
lapse beside the scope in its aggregated decisions, and
`policy.lapse.expired` or `policy.lapse.revoked` closing it. Payloads
carry ids, scopes, addresses, and the mandatory reason — never record
content, and never the material the lapse disclosed.
