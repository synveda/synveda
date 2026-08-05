# ADR-0058: an explorer that asks the PDP instead of re-deriving it — the probe is a forecast rather than a grant, a fan-out of decisions chains as one event, and the two questions that look alike are kept apart

- **Status**: Proposed
- **Date**: 2026-08-05
- **Feature(s)**: CNSL-2 (closing ADR-0056's named deferral; inheriting its toolchain and session)
- **Deciders**: sujitn

## Context

CNSL-2's text is "visualise scopes, packs, roles, active lapses", and it has
no acceptance criteria at all.

Start where ADR-0056 started, by establishing what is actually missing —
because it is less than the feature text suggests in one direction and more
in another. The nouns mostly have surfaces already:

- **scopes** — `GET /v1/hierarchy/nodes/{id}` with `/children`, `/ancestors`,
  `/descendants` and `/root` (HIER-1, ADR-0011);
- **packs** — `GET /v1/hierarchy/nodes/{id}/policy`, which serves not just the
  effective pack but **where it came from**: assigned here, assigned at an
  ancestor, the tenant default, or the embedded default (AUTHZ-2, ADR-0014);
- **roles** — `GET /v1/hierarchy/nodes/{id}/roles` and the tenant-wide
  `GET /v1/roles/bindings` (AUTHZ-3, ADR-0015);
- **lapses** — `GET /v1/lapses?scope_id=` (AUTHZ-4, ADR-0037);
- and the curator files beside them, `GET /v1/hierarchy/nodes/{id}/curators`
  (FLOW-3, ADR-0032 decision 15).

So CNSL-1's finding half-repeats: this is largely a second renderer over an
API that exists, on a toolchain and a session ADR-0056 already bought. Only
half, though, because three of the things the feature text names are not
answerable by any call this product serves, and one of them is the deferral
CNSL-1 left here **by name**.

Four forces.

1. **Policy says where it came from; roles do not.** Roles inherit downward
   (seed §5, ADR-0015), so the question a steward opening a team actually asks
   — *who holds what here* — has an answer that includes every binding at every
   ancestor plus the tenant-wide ones. `GET .../roles` returns
   `role_bindings::for_scope`: the bindings at that node and nothing else. A
   console can assemble the real answer by walking `/ancestors` and unioning,
   and that is a second implementation of an inheritance rule the PDP already
   owns — ADR-0056 decision 5's argument arriving one plane over. The pack
   surface solved exactly this problem years of decisions ago, with `origin`.
   The two admin planes disagree about how to say "this came from above", and
   only one of them says it at all.

2. **A lapse has two ends and only one of them can be listed.** The list
   handler calls `lapses::at_target`, keyed on `target_scope_id` — the
   disclosing side, which is the side ADR-0037 decision 3 makes open the
   proposal. Two consequences follow. *Active lapses* as a standing set is not
   answerable: `scope_id` is a required parameter, so a caller must already
   know which scope to ask about, which is the opposite of what an explorer is
   for. And the steward of the **grantee** scope — the team that was granted a
   read — cannot see the grant their own team holds; only the disclosing side
   can. A steward who cannot list a standing grant cannot revoke one, and
   `POST /v1/lapses/{id}/revoke` exists. The store already holds the query for
   the other end: `lapses::active_for_scopes`, unrevoked and `expires_at >
   now()` on the database's own clock, which is what the PDP itself reads on
   every request.

3. **The reader's own capabilities are on no wire.** `GET /v1/whoami` returns
   `{subject, tenant}`. ADR-0056 deferred *the set of actions offered*: which
   acts a proposal admits is a function of its state, the pack in force and the
   reader's own roles, and only the first is served — so CNSL-1's inbox offers
   approve and reject unconditionally, to everybody. That ADR named this
   feature as where it stops being avoidable, and not for scheduling reasons:
   an explorer **is** the capability question. "Visualise roles" and "what may
   this person do here" are the same screen asked twice.

4. **Ten thousand nodes and a badge each.** HIER-1's AC is a 10k-node
   hierarchy; `descendants` returns a whole subtree in one response; and an
   allowed admin-plane read chains a standalone `authz.decision` event
   (ADR-0019 decision 4). A screen that renders a tree with a capability badge
   per node is *n × k* decisions, and if each chains a row, opening the explorer
   at the org root writes more audit than the tenant's entire operating
   history. A governance product whose audit log is mostly a record of people
   looking at it has made its own chain unreadable. This force is what makes
   CNSL-2 more than a renderer.

## Decision

1. **A capability is the PDP's verdict, asked for.** `GET
   /v1/hierarchy/nodes/{id}/capabilities` — and a `capabilities` block on
   `whoami` for the tenant plane — returns, per action, what `authz::decide`
   returns for **this caller at this node**, under the pack effective there,
   carrying the pack `name@version` the answer was decided under. It is not
   derived from role bindings. A capability computed from roles is a second
   implementation of "may I"; it agrees with the PDP on the day it is written
   and is wrong in precisely the cases that matter — an active lapse,
   quarantine, a service identity's confinement, ABAC context (AUTHZ-5), the
   base layer's escalation guard.

2. **The probe is a forecast, never a grant.** This is the load-bearing
   sentence, so it is stated as an invariant rather than as a note: *nothing in
   this product reads a capability answer in order to decide anything.* The
   decisions a probe returns authorise nothing; every act still takes its own
   decision at its own seam, and if the two disagree because a pack changed
   between them, the act's decision is the one that decided. A client may use
   capabilities to choose what to **offer** and never to choose what to
   **allow**. The enforcement point is unchanged and there is no second one.

3. **The two questions that look alike are kept apart.** *What may I do here*
   is about the caller, discloses nothing about anybody else, and therefore
   needs no permission beyond the visibility the node already requires —
   uniform-404 ownership first, as everywhere. *Who may do what here* is about
   third parties and keeps `RoleRead` / `PolicyRead` and its own denial.
   Conflating them is how an explorer becomes an enumeration oracle for an
   organisation's entire role assignment, one 403 at a time. The capabilities
   route therefore takes **no `subject` parameter**: "what could Alice do" is a
   different feature with a different disclosure rule, and AUD-2 already
   answers the historical form of it ("who could see X on date D").

4. **A fan-out of decisions chains as one event.** ADR-0019 decision 4 already
   settled this shape for CTX-2's per-candidate `MemoryRead` sweep: aggregate
   into the request-level event with the candidate decisions summarised, and
   keep per-call detail in the structured decision log and traces. A capability
   probe is that shape on the admin plane — **one** `authz.decision` per probe
   request, naming the node set, the action set and the outcome summary, not
   one row per (node, action) pair. This is not a new rule and not an
   exemption; it is the second sentence of a decision whose first sentence
   would otherwise price a tree render at thousands of rows.

5. **The tree is lazy and the probe is bounded.** Children on expand via
   `/children`, never `descendants` from the root; capabilities probed only for
   the nodes actually rendered, batched into one request under a maximum the
   **API** declares. A bound a screen can exceed is a bound the screen works
   around, so the response names what it did not answer rather than truncating
   silently — SKIL-4's discipline, and EVAL-5's "no silent caps".

6. **Roles gain the origin packs already serve.** `GET .../roles?effective=true`
   returns the inherited union with, per binding, the node it was bound at, in
   the same `origin` vocabulary `EffectiveResponse` uses. Same walk, same
   `RoleRead`, one new query. The local form stays the default, because "what
   is bound *here*" is the question the mutation surfaces are about and the one
   `PUT`/`DELETE` operate on.

7. **The lapse list gains the grantee side and a scope-free form, and the
   scope-free form is a union of per-scope decisions rather than a new grant.**
   `GET /v1/lapses` with no `scope_id` returns the standing set the caller may
   see; each lapse is visible if the caller holds `PolicyRead` at **either**
   end, decided per scope exactly as it is decided today. No tenant-level
   permission is invented — `GET /v1/roles/bindings`'s tenant-wide `RoleRead`
   is the shape deliberately *not* taken, because it is held by org-admins and
   the person who needs this view is a team steward. This is SKIL-4 decision
   2's rule one plane over: the plural of a walk is the same walk.

   Two sub-decisions. `active` defaults to **true** for the scope-free form and
   **false** for the scoped one, which preserves `?scope_id=`'s deliberate
   inclusion of expired and revoked rows — "who could read what, when" is a
   question about history — while making the standing set what the word
   *active* in the feature text asks for. And the standing predicate is
   `active_for_scopes`' own, so the explorer and the PDP cannot disagree about
   which grants are live.

8. **Every surface here is the CLI's too — and one of them is a pre-existing
   gap this feature closes by choice rather than by rule.** ADR-0056 decision 9
   — the console gets no endpoint the CLI does not have — is a standing
   decision, so the three surfaces CNSL-2 *adds* reach both clients:
   `synveda lapse list`, `synveda whoami --capabilities`, and `--effective` on
   `synveda role list`. `synveda policy show` is **not** in that set, and the
   distinction is worth keeping honest: `GET .../policy` has existed since
   AUTHZ-2 with no CLI read verb, so adding one closes a gap that predates this
   feature rather than honouring decision 9. It lands here anyway, deliberately,
   because the alternative is a feature whose own screen renders a pack origin
   that no terminal can print — and because `synveda policy` today offers
   `apply` and `clear`, which is a verb set that can change a subtree's
   governance and not one that can show it.

   The lapse verb is the one that matters most and the only one with no
   predecessor at all. There is **no `lapse` verb whatsoever**, so a product
   whose lapse machinery is its entire answer to "strict by default, relaxable
   by design" (seed §2.3) has no terminal in which to ask what is currently
   relaxed.

9. **The explorer is read-only, and the asymmetry it surfaces gets a feature ID
   rather than a footnote.** Content mutates only through proposals — CNSL-4's
   rule, adopted early by ADR-0056 decision 9 — but `PUT .../policy` and
   `PUT .../roles` are direct routes: the admin plane was never proposal-gated.
   Building this screen is what makes that visible, because the screen's whole
   job is to put a pack, its origin, the roles under it and the grants over it
   on one page, and the next question a reader asks is who changed any of it and
   under what review.

   The answer is sharper than it first looks, and bounded in two directions that
   matter. All three packs grant `PolicyAssign` to `steward` and `org-admin`
   over the bound subtree, and the decision deliberately skips the node's own
   assignment (ADR-0014 decision 4) so that a restrictive pack cannot seal its
   own node — which together mean a steward bound at a team can replace that
   team's pack with **one call and one signature**. What that cannot do is widen
   anybody's candidate universe: the universe is the caller's placement chain
   and it widens by lapse and by nothing else (ADR-0037 decision 13), which is
   exactly why EVAL-5's relaxation demo had to be a lapse and not a pack flip.
   Nor can it reach below the invariant floor (ADR-0032 decision 4, ADR-0051
   decision 18, ADR-0052 decision 3). What it *can* do is lower the approval
   counts, the sensitivity ceiling, the scan threshold and the quality bar for
   an entire subtree — permanently, on one signature — while the **lapse** that
   relaxes far less demands a reasoned, time-boxed, dual-approved proposal that
   expires on its own. Seed §2.3 has controls relaxed "through explicit,
   audited, time-boxable policy relaxations". A pack assignment is explicit and
   audited (`policy.node.assigned`); it is the one relaxation path in this
   product that is neither time-boxed nor second-signed.

   This feature does not settle that, and declines to add a **second** direct
   mutation surface while it is open. It also declines to park the question in
   CNSL-4, which is the *memory browser*: that feature's "no direct-mutation
   path exists — everything is a proposal" is a rule about records, so an
   admin-plane governance question filed inside a Phase 4 screen about memory is
   one nobody would look for. It becomes **AUTHZ-7**, in the epic that owns how
   policy changes are authorised.

10. **Parity is the corpus again, extended rather than re-invented.** CNSL-1's
    `console/fixtures/` corpus and its `.facts.json` discipline take four new
    cases — effective roles with mixed origins, a pack inherited from two levels
    up, a standing lapse beside an expired one, and a capability set with at
    least one denial — asserted by both renderers and checked for teeth the way
    CNSL-1's was: delete each fact, name which case fails and confirm nothing
    else does.

## Options considered

**Where a capability comes from.**

1. **Ask the PDP (chosen)** — one implementation of "may I", correct by
   construction under lapses and context. Cost: a `gather` per node.
2. **Derive from role bindings in the client** — no new endpoint, and wrong
   whenever anything other than a role decides. It is ADR-0056 decision 5's
   "two implementations that agree on the day they are written", with the
   difference that these two would disagree **immediately**, not eventually.
3. **Serve the role×action matrix as data and let clients index it** — the
   AUTHZ-3 golden matrix over the wire. Tempting because it is static and
   cacheable, and rejected for the same property: a static matrix cannot
   express a lapse, a quarantine or any per-request context, so it answers a
   question adjacent to the one asked and looks like it answered the real one.
4. **An `_actions` block on every existing response**, REST-hypermedia style —
   rejected because it prices a decision fan-out into read paths that never
   needed one (`inject` and `recall` included, against a 150ms p99), and it
   makes force 4's audit question harder rather than easier.

**Audit of a probe.**

1. **One summarised event per probe request (chosen)** — ADR-0019 decision 4's
   own shape, already argued for a larger fan-out.
2. **One event per (node, action)** — the most faithful reading of "every authz
   decision", and it turns a tree render into a denial-of-service against the
   tenant's own chain. It also degrades what the chain is *for*: an auditor
   asking what happened would have to filter out what was merely looked at.
3. **No event at all** — defensible on decision 2's own logic, since a probe
   decides nothing, and rejected because somebody systematically probing an
   organisation's admin surface is exactly the reconnaissance an audit log
   should show. One summarised row per request shows it; zero rows do not.

**The lapse view.**

1. **Union of per-scope `PolicyRead` decisions (chosen).**
2. **A tenant-wide `LapseRead`** — one decision and one query, and it hands the
   standing-relaxation view to org-admins only, which is precisely the
   population that does not need it.
3. **Target side only, as today** — the honest minimum, and it leaves a steward
   able to revoke a grant they cannot list.

**Tree loading.** Lazy children (chosen) · the whole subtree in one call
(simple, and force 4) · a server-paginated flat list with a path column
(rejected: it renders a hierarchy as a table, which is the one thing this
screen exists not to do).

## Consequences

- **Positive.** The deferral CNSL-1 named is closed at its named place, and
  closed by asking the only component that knows the answer rather than by
  teaching a second one the rule. The roles plane stops being the admin surface
  that will not say where an inherited thing came from. `synveda lapse list`
  gives the lapse machinery a terminal it has never had, which matters more
  than a console screen for the operators who run this product over SSH.
  Nothing new is invented for the audit chain — decision 4 is an existing
  decision's second sentence — and no table, column or `AuditAction` is added
  by the whole feature.

- **Negative / accepted.** `capabilities` is the first read surface whose
  *answer is a decision*, and somebody will eventually mistake it for a
  permission cache; decision 2 is written as an invariant for that reason and
  the AC asserts the disagreement case directly. The probe costs one `gather`
  per node — identity, chains, assignments, lapses — even though the decisions
  themselves are µs-level (AUTHZ-1), so decision 5's bound is doing real work
  and a 200-node render is a real query fan-out. And decision 9 leaves a
  genuine asymmetry standing — direct mutation on the admin plane, proposals
  everywhere else — in a product that sells governance. It leaves it standing
  with an ID and a written finding rather than with a footnote, which is the
  most this feature can honestly do: closing it means deciding whether a pack
  assignment needs an approval-matrix cell of its own, and that is a policy
  decision rather than a console one.

- **Reversal triggers.** If any client is found gating an action on a
  capability answer rather than offering on it, decision 2 has failed as
  documentation and the field should be renamed to something that cannot be
  read as authority (`offers` rather than `capabilities`). If an auditor's real
  investigation wants the summarised probe event expanded — which nodes did
  this person probe — decision 4 grows the node list into the payload rather
  than growing rows. If the per-node `gather` makes a bounded probe slower than
  the tree render it annotates, capabilities move to one request per subtree
  with the chains gathered once. And if AUTHZ-7 decides the admin plane should
  be proposal-gated, decision 9's asymmetry closes and this screen gains the
  write path it declined — which is the one reversal here that would make the
  explorer bigger rather than smaller.

## Compliance notes

- **PDP.** No bypass, no new decision point, and one new *caller* of an
  existing one. A capability answer is `authz::decide`'s own return value; the
  route adds no rule and cannot widen one. Seed §2.2 holds here in the
  strongest form available to any surface in this product — the response's
  entire content is the PDP's output.

- **Disclosure.** Uniform-404 ownership runs first, as everywhere, so the probe
  annotates a set the caller could already enumerate and never widens it. The
  route takes no `subject`, so it cannot become an oracle about anybody but its
  own caller (decision 3). The scope-free lapse list is bounded by the scopes
  the caller may read rather than by a tenant-wide predicate (decision 7), and
  a lapse's `reason` — free text a steward wrote about an incident — is served
  only on the scoped read paths that already serve it and never in a probe
  payload.

- **Multi-tenancy.** No new table, no new column, nothing stored at rest. Every
  query runs inside `rls::begin_tenant_tx` like its neighbours; the effective
  roles walk and the lapse union both resolve through the tenant's own chain
  resolver.

- **Audit.** No new `AuditAction`. Probes chain `authz.decision` with an `op`
  of `capabilities` and a summarised payload (decision 4); the acts they
  forecast chain their own semantic events, unchanged and indistinguishable
  from a CLI caller's (ADR-0056 decision 9). A denial of the probe itself
  chains at the `respond` seam like every other denial.
