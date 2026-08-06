# ADR-0060: Directory pull sync — absence is a hypothesis, and the first secret we have to keep

- **Status**: Proposed
- **Date**: 2026-08-06
- **Feature(s)**: AUTH-5
- **Deciders**: sujitn

## Context

The feature text is two lines (SYNVEDA_FEATURES.md:196): scheduled pull sync
(Temporal) for IdPs without SCIM push; AC — drift converges ≤ sync interval,
deletions handled as leavers.

Most of the work this feature would have needed is already done. ADR-0059
decision 3 built the seam and named this feature in it: the SCIM mirror is
the directory resource of record, `identities` and hierarchy placement are a
projection of it, and `scim::reconcile` is the only writer of that
projection anywhere in the product. Its module doc says so in the second
paragraph — "AUTH-5's scheduled pull sync writes the same mirror rows from a
directory read and calls `reconcile`, which is the whole reason that feature
is an M rather than a second implementation of joiner/mover/leaver". So
joiner, mover, leaver, the correspondence rule, the seal's three layers, the
mover's pack question and the rehire rule are **not this ADR's to decide**,
and it does not re-open any of them.

What is left is the part that inverts.

Forces at play:

- **The direction reverses, and every property AUTH-4 got from being the
  server reverses with it.** On the push plane the directory authenticates
  to us, names its own tenant, and states facts as acts: `active: false` is
  something a provisioning agent *did*. On the pull plane we authenticate to
  the directory, we choose what to ask, and we receive a snapshot. Nothing
  in a snapshot is an act.
- **Absence is the only leaver signal a pull has, and it is not a signal.**
  ADR-0059 decision 11 is emphatic that only an explicit deactivation seals,
  because the difference between a misconfigured group and a person losing
  their memory is that one is reversible and the other is not. A pull sync
  has no explicit deactivation to wait for at the IdPs this feature exists
  for. It has a user who was on page 3 last hour and is on no page now — and
  a throttled response, a truncated page, an expired token, a narrowed group
  filter, an administrator fixing a scoping rule, and a resignation all
  produce that same nothing.
- **The seal does not lift (ADR-0059 decision 12).** That decision's
  sentence is "a hold that the directory can release is not a hold". Its
  mirror image is this ADR's problem: a hold that the directory's *outage*
  can impose is not a lifecycle event. AUTH-4 could afford to be strict
  because it was reacting to an act; reacting to an inference with the same
  irreversible mechanism is how one bad afternoon at a vendor permanently
  deprovisions a tenant.
- **Temporal is already answered.** ADR-0022 refused the community Temporal
  Rust SDK (git-distributed, outside the licence-clean dependency graph) and
  settled on "a simple Rust worker now, Temporal-shaped so enterprise can
  host it later". Every periodic job in the product since is an in-process
  ticker: `retention::spawn`, `promotion`, `authz::spawn_pack_refresher`,
  `lapses::spawn_expiry_sweep`, the indexer.
- **The reconciler is in the gateway crate and takes `&AppState`.** The
  crate dependency rule (seed §8) is that nothing imports upward, so a sync
  job that reuses `scim::reconcile` — which decision 3 requires it to —
  cannot live in `synveda-ingest` beside the retention sweep. Where it runs
  is decided by where the seam already is.
- **Every secret this product stores is a hash.** `scim_credentials.token_hash`
  is a SHA-256 of the whole presented string (migration 0036);
  `console_sessions.token_hash` the same (migration 0034). To call Microsoft
  Graph or the Okta Users API we have to *present* a credential, which means
  keeping one we can read back. That is the first recoverable secret in the
  product, and TEN-4 — per-tenant encryption keys — is unbuilt and sits
  behind this feature in the phase.
- **ADR-0013's reachability argument has to survive.** ADR-0059 decision 2 —
  a SCIM request carries directory facts, never product instructions — is
  what keeps seed §2.2 binding exactly where it bound before, by there being
  no governed asset reachable from the plane rather than by an exemption. A
  connector that could express a scope, a role or a pack would break that
  argument on the read side, where nobody is watching a wire format.

## Decision

1. **The sync is a gateway-hosted ticker, Temporal-shaped, and not a
   Temporal workflow.** ADR-0022's decision is applied rather than
   re-litigated: a pass is a sequence of idempotent activities over a
   durable cursor in Postgres, so hosting it under a real workflow engine
   later is a change of caller and nothing else. It runs in the gateway
   process because that is where `scim::reconcile` and `AppState` are, and
   `spawn_pack_refresher`/`spawn_expiry_sweep` are the shape it copies —
   one immediate pass, then one per interval, `MissedTickBehavior::Delay`,
   pass-level failures logged and retried next tick, never fatal to the
   gateway.

2. **A pull writes the mirror and calls `reconcile`. It has no lifecycle of
   its own.** The connector's entire output is the same `UserAttributes` and
   group membership a SCIM `PATCH` writes. Not one line of joiner, mover,
   leaver, correspondence or seal is re-implemented or re-decided, and the
   chain cannot tell the two doors apart — ADR-0059 decision 6's rule for
   the joiner, applied to the whole lifecycle. The `source` in the event
   payload is the only distinction, so "which door did this come through"
   stays answerable without making them two mechanisms.

3. **Absence is a hypothesis, and it takes three things to become a leaver.**
   This is the decision the feature exists for, and it is the one place this
   ADR is deliberately less eager than its AC.

   1. **A pass concludes nothing about absence unless it completed.** Every
      page fetched, no HTTP error, no partial result. An incomplete pass
      still writes *presence* — seeing somebody is not conditional on seeing
      everybody — but it may not conclude that anyone is gone. This alone
      separates a truncated page from a departure, and it is why decision 6
      refuses to make a delta feed the authority.
   2. **Absence must persist across `N` consecutive complete passes**
      (default 2) before it is offered to the reconciler. `scim_users` gains
      `missing_since`; the count, not the wall-clock, is the condition, so a
      long interval does not become a long exposure and a short one does not
      become a hair trigger.
   3. **A bulk-change circuit breaker.** If a pass would seal more than a
      configured fraction of the tenant's live users (default 10%, with a
      small absolute floor so a 6-person tenant does not trip on one
      leaver), it seals **none of them**, chains one event, raises a metric,
      and leaves `missing_since` standing so the next pass re-evaluates. A
      directory that really did deactivate a third of the company is
      precisely the case where a person should push the button, and a
      directory that merely changed its assignment filter looks identical
      from here.

   An explicitly deactivated user — `active: false` present in a complete
   pass — seals **immediately**, on the first pass, exactly as the push
   plane does. That is an act, and it gets the treatment an act gets. Only
   *absence* takes the slow path, and the asymmetry is the whole content of
   this decision.

4. **Drift is defined, its bound is stated in two halves, and the honest
   half is the wider one.** Drift is the interval between a change in the
   directory and the product state that reflects it. Joiners, movers, group
   changes and explicit deactivations converge in ≤ one interval plus one
   pass duration, which is the AC as written. **Absence-derived leavers
   converge in ≤ (N+1) intervals by decision 3**, which is wider than the AC
   asks for, and this ADR widens it on purpose rather than meeting the
   number by sealing on a dropped packet. The AC test asserts both bounds
   separately, so the widening is a measured property rather than a caveat
   in prose.

5. **The two planes are mutually exclusive per tenant, and the pull yields.**
   A tenant with a live SCIM credential does not get pulled: the sync skips
   it, says so once per startup with a metric, and takes over on the next
   tick if the credential is revoked. Two authorities for one fact is the
   mistake ADR-0013 decision 4 refused for placement and ADR-0059 decision
   4's implementation note refused for the anchor; at the plane level it is
   worse, because the authority that infers from absence would be
   deprovisioning people the authority that *knows* never deprovisioned. The
   push plane wins because it carries acts.

6. **The first cut enumerates fully; delta is an optimisation for presence
   and never the authority for absence.** Graph's `delta` and Okta's
   `lastUpdated gt` cursor both answer "what changed", and neither answers
   "what still exists" — which is the only question decision 3.1's
   completeness proof is made of. A leaver who arrives as a removed
   membership is visible in a delta feed; a leaver who arrives as *nothing*
   is not, and that is the shape this AC is about. Full enumeration per pass
   is therefore the authority, with delta available later to make presence
   cheap between full passes, on a documented slower cadence for the full
   one. The reversal trigger is a real tenant's user count, below.

7. **The outbound credential stays in deployment configuration, beside the
   issuer it belongs to, and the product's "no recoverable secret in the
   database" property survives one more feature.** The connector is
   configured per issuer in the same environment JSON that already carries
   `SYNVEDA_OIDC_ISSUERS` — the issuer this connector syncs is the issuer
   whose tokens that entry already verifies, so it adds no configuration
   surface and inherits the deployment-level scope the issuer list already
   has, rather than introducing a new limitation of its own.

   The alternative is a per-tenant table holding a secret we can read back,
   and that table wants TEN-4's per-tenant encryption keys, which are
   unbuilt and sit behind this feature. Shipping a plaintext outbound
   credential in tenant data now, to be encrypted in a later feature, is the
   version of this that is hard to walk back: the rows outlive the decision.
   Recorded as a deferral with TEN-4 as its explicit trigger.

   The secret is presented outward, so it reaches nothing inward: never a
   chained payload, never a span field, never an error message, never a log
   line. AUTH-4's demo already sweeps the chain for credential material and
   that sweep extends here.

8. **The pull takes no PDP decision, and the connector has no vocabulary for
   one.** ADR-0059 decision 2's reachability argument is preserved on the
   read side by the connector's *output type*: `UserAttributes` plus group
   names, with no field for a scope, a role, a pack, a channel or a record,
   and none to be added. The job runs as `ActorKind::System` named by
   component — the kind ADR-0022 minted for "sweeps and AUTH-4/5 sync jobs",
   which this discharges in the direction it named.

9. **A pass that changes nothing chains nothing.** ADR-0059 decision 14
   bounded ADR-0019's chain-every-admin-read rule because a provisioning
   agent polls; a pull sync polls harder, on our own schedule, and chaining
   passes would make a quiet tenant's chain a record of the product reading
   a directory that had not changed. What chains: the per-person events
   `reconcile` already chains, one aggregated event per pass that changed
   something (CNSL-2's fan-out aggregation precedent, counts in the
   payload), and the circuit breaker's refusal — which is the one thing here
   an auditor must not have to notice. Every pass is traced and metered
   regardless.

## Options considered

1. **Gateway-hosted full-enumeration ticker; absence confirmed over N
   passes plus a circuit breaker; deployment-level credential (chosen)** —
   reuses the reconciler as ADR-0059 designed it, and puts the feature's
   whole weight on the one question a pull has that a push does not. Con:
   absence-derived leavers converge slower than the AC's literal bound, and
   full enumeration costs the most of the three read strategies.
2. **Temporal Rust SDK, as the feature text says** — highest fidelity to the
   backlog line. Rejected by ADR-0022 already, on the licence-clean
   dependency graph and the SMB profile's footprint; nothing has changed and
   re-deciding it inside an M feature would be the wrong place.
3. **Seal on first absence** — the literal reading of "deletions handled as
   leavers", and it meets the AC's bound exactly. Rejected: the seal is
   three layers and does not lift, so this converts any single failed or
   filtered response into permanent deprovisioning of everyone it omitted.
   The AC's number is worth less than the property ADR-0059 decision 12
   spent a decision protecting.
4. **A delta feed as the authority** — cheapest, and what both vendors
   advertise. Rejected: a change feed states what changed and never what
   still exists, so the leaver who arrives as nothing is invisible to it,
   and the completeness proof decision 3.1 needs cannot be built from one.
5. **Absence never seals; the pull is presence-only and only an explicit
   deactivation ever seals** — the safest option, and consistent with
   ADR-0059 decision 11 read strictly. Rejected: it fails the AC outright,
   and the IdPs this feature exists for are exactly the ones with no
   deactivation to send. Recorded because it is the tempting answer and
   because decision 5 adopts it for the one case where the push plane is
   also live.
6. **A per-tenant encrypted credential table now** — the right long-term
   shape, and it would let one deployment pull two tenants from two
   directories. Rejected: it needs TEN-4, and the interim — a recoverable
   secret in tenant data with no key management — is worse than the
   deferral, because the rows outlive the decision.
7. **A second reconciler for the pull path** — simpler to write, no shared
   invariants to preserve. Rejected by ADR-0059 decision 3: two
   implementations of joiner/mover/leaver is what the mirror exists to
   prevent, and it is what makes this feature an M.
8. **Both planes live at once with merge rules** — attractive for a
   migration from pull to push. Rejected (decision 5): the merge rule that
   matters is what happens when the pull infers a departure the push never
   stated, and every version of that rule is either "the pull cannot seal"
   — which is decision 5 with more code — or a directory quietly
   deprovisioning people.
9. **Do nothing** — joiner/mover/leaver stays manual for every IdP without
   SCIM push. Fails the AC, and leaves ADR-0059 decision 3's "AUTH-5 drives
   the same reconcile" as an untested claim about a seam.

## Consequences

- Positive: joiner, mover and leaver work for any IdP with a readable
  directory API rather than only the two with provisioning agents; ADR-0059
  decision 3's "AUTH-5 is a loop around a function that already exists and
  is already tested" is discharged as written, and the reconciler acquires
  the second caller that proves it was a seam rather than an internal
  function; ADR-0022's `ActorKind::System` "AUTH-4/5 sync jobs" naming is
  used by the job it named.
- The finding this ADR is built on: **AUTH-4's leaver rule does not survive
  the transport change unmodified.** ADR-0059 decision 11 could say "only an
  explicit deactivation seals" because a push plane always has one. Carried
  onto a pull plane the same sentence either means "nothing ever seals" —
  failing the AC — or is quietly reinterpreted as "absence seals", which is
  the same sentence with its safety property removed. Decision 3 is what the
  rule becomes when the signal stops being an act, and the asymmetry between
  an observed deactivation and an inferred one is the whole of it.
- The second finding: **this is the product's first credential it has to be
  able to read back.** Every stored secret to date is a SHA-256 of something
  a caller presents, which is a property nobody wrote down because nothing
  had tested it. Naming it in decision 7 is what makes TEN-4 the place it
  gets fixed rather than a thing discovered during TEN-4.
- Negative / accepted trade-offs: absence-derived leavers converge slower
  than the AC's bound, deliberately and measurably (decision 4); the gateway
  process gains an outbound network dependency on a vendor API, on a loop,
  with the failure taxonomy that implies; a recoverable secret exists in the
  deployment environment; a genuine mass departure needs an administrator to
  clear the circuit breaker, which is friction on the worst day a customer's
  HR department has; and one deployment cannot pull two tenants from two
  directories until TEN-4.
- Reversal triggers:
  - Full enumeration costing more than the interval at a real tenant's user
    count → delta lands for presence, with full enumeration retained as the
    absence authority on a slower cadence (decision 6 is written to be
    extended in exactly this direction).
  - The circuit breaker firing on a legitimate reorganisation more than once
    → the threshold is wrong, or it wants a pre-authorisation from an
    administrator rather than a refusal after the fact.
  - A customer needing two directories in one deployment → the credential
    moves to a per-tenant table, and that is TEN-4's trigger, not this
    feature's.
  - `N = 2` proving too slow for a customer's offboarding SLA → the count is
    configurable, but a request to set it to 1 is a request to delete
    decision 3.2, and the answer is the push plane.

## Compliance notes

- **Audit.** One new aggregated action per changed pass plus the circuit
  breaker's refusal (decision 9); the per-person lifecycle events are
  `reconcile`'s existing ones, unchanged, so "who deprovisioned this person"
  is answerable from the chain in the same shape for both planes, with the
  source in the payload. Quiet passes are deliberately unchained, with the
  reason recorded here rather than left as an omission — the same bounding
  ADR-0059 decision 14 applied, for the same reason, one plane over.
- **Multi-tenancy.** The mirror tables are already tenant-scoped with forced
  RLS. The new per-tenant sync state — cursor, last complete pass, counts,
  breaker status — ships in migration 0037 with forced RLS, policies and
  least-privilege grants in its own migration per the ADR-0009 structural
  rule, and joins the TEN-2 adversarial suite and its completeness guard.
  `scim_users.missing_since` is a column on a table already in that suite.
  The sync loop iterates tenants and opens one tenant transaction per pass,
  so a pass cannot see across the boundary any more than a request can.
- **Policy enforcement.** Seed §2.2 binds exactly where it bound before, by
  ADR-0059 decision 2's reachability argument extended to the read side and
  enforced by the connector's output type (decision 8): the pull reaches
  identities, placements and lifecycle state, and no governed asset. Every
  material effect of a synced change — what follows a mover, what a seal
  makes unreadable — remains the effective pack's, decided by code this
  feature does not touch.
- **Credential handling.** The outbound secret never enters the chain, a
  span, an error or a log (decision 7); the acceptance demo's existing
  credential-leak sweep covers it, and the connector's error taxonomy is
  asserted to carry the endpoint and the status without the authorization
  header.
