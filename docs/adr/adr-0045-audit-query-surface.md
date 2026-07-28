# ADR-0045: The audit surface answers from the recorded chain and never from a replay — disclosure and authority are two lists it refuses to merge, an answer is tenant-complete or refused, and it resolves no content

- **Status**: Accepted
- **Date**: 2026-07-28
- **Feature(s)**: AUD-2 (AUD-3, AUD-4, CNSL-3 inherit)
- **Deciders**: sujitn

## Context

AUD-2 is "Search by actor/resource/time/action; auditor role read-only
incl. denials; answer 'who could see X on date D' and 'what did agent A
know at time T'", with the acceptance criterion "both questions
answerable via one API call each (uses bitemporal + refs)".

Unusually for a feature this late in the phase, most of its design was
decided by other features that needed the answer to exist. This ADR is
mostly a collection of those debts.

Forces at play:

- **Five ADRs pre-registered this surface's answers.** ADR-0042
  decision 8 drew the line — "CTX-5 answers *what was there*, AUD-2
  answers *who could see it*, from the chain that recorded the decision"
  — and ADR-0042 option 5 rejected rewinding authority for the read path
  *by naming AUD-2 as where the question belongs*. ADR-0038 decision 13
  put the permitted tier set into `context.injected`'s aggregated
  decisions specifically so "who could see X on date D" is answerable at
  tier granularity. ADR-0041 decision 9 put `tier` on every entry because
  "was that agent given the payments runbook, or only told it exists" is
  a question an auditor asks. ADR-0036 decision 9 sent "why is this
  pinned, and by whom" here rather than build a pin log. ADR-0019
  decision 7 left "user or service identity" as "the identities table's
  knowledge, joined at query time by AUD-2". ADR-0039 named this surface
  as where a curator would act on the `memory.superseded` trail.
- **The chain is already carrying the answers, deliberately.** A
  `context.injected` payload names every entry's record id, object
  address, channel, tier and staleness; its per-scope decisions carry the
  allowed tier set, `pack@version`, and the lapse that opened the scope;
  its channels list carries the commit each scope's published ref pointed
  at and whether a pin chose it. `context.recalled` carries the same
  watermark shape (ADR-0041 decision 8). No emission point needs to
  change for this feature, which is the useful consequence of ADR-0019
  decision 4's one-event-per-operation rule having been applied with the
  auditor in mind each time.
- **The table is append-only and hashed, so what may be added to it is
  narrow.** `audit_log` carries exactly one index: its primary key
  `(tenant_id, seq)`. Every query this feature needs — by actor, by
  action, by time, by a record id buried in a payload — is a sequential
  scan today. Indexes are addable and cost nothing a hash covers.
  Columns are not: a column inside the canonical form breaks
  verification for every row already written, and a column outside it is
  a field the chain does not protect, which on an audit table is worse
  than not having the field at all.
- **The `resource` column is a display string, not a key.** It is
  `Resource::to_string()` where a policy resource existed (`"scope
  <uuid>"`, `"tenant <uuid>"`), `"scope none"` where the caller was
  unplaced, and hand-built strings elsewhere (`"binding:alice@scope:…"`).
  AUD-1 specified it as "freeform but consistent per action" and meant
  it. Nothing in the schema makes it parseable, and no migration can
  retroactively make the rows already written so.
- **An auditor reads no content, and the seed says so twice.** Seed §5:
  `auditor` is "read-only including audit logs, cannot touch content".
  Every pack grants the role the read-only admin permit and nothing else;
  ADR-0021 decision 6 excluded auditor from the quarantine review plane
  on exactly that ground. The events were themselves written to hold no
  content — a task rides as a BLAKE3 hash, a record rides as an id and an
  object address, a redaction finding rides as a rule id. This surface is
  the one route by which an auditor could otherwise acquire content, and
  it is the last one.
- **Historical authority exists only on the chain.** `role_bindings`,
  `policy_pack_assignments` and `policy_lapses` are current-state tables;
  an unbound role leaves no row behind. `role.bound`/`role.unbound`,
  `policy.node.assigned`/`unassigned` and
  `policy.lapse.granted`/`revoked`/`expired` are the only record that a
  binding stood in March. So the chain is not merely the *tamper-evident*
  answer to "what governed this scope on date D" — it is the only
  answer, and one derived from the live tables would be wrong rather than
  incomplete.
- **There is no seed SLO for the admin plane.** §10 budgets `inject`
  (p99 < 150ms) and `observe` (ack < 20ms, lag < 60s); ADR-0029 derived
  recall's 300ms from the seed's own "richer and slower". An audit query
  is an interactive human read over a table that grows forever and is
  never pruned before TEN-5. Nothing pre-registers a number, so this ADR
  does.

## Decision

**The audit surface answers from the chain as it was recorded and never
from a replay of the state that produced it: it reports disclosure and
authority as two lists it refuses to merge, an answer covers the whole
tenant or is refused, and it resolves no content.** One new action, four
routes, one migration of indexes, and no change to any emission point.

Decisions, specifically:

1. **One action, `AuditRead`, and it joins the permit that has been
   waiting for it.** The read-only admin permit in all three packs —
   `HierarchyRead`, `PolicyRead`, `RoleRead`, `ServiceIdentityRead`,
   granted to `steward`, `org-admin` and `auditor` — carries the comment
   "including audit logs when AUD-2 lands" and has since AUTHZ-2.
   `AuditRead` joins that list; packs bump to `@11`. Verification gets no
   second action: `verify` returns a verdict and a sequence number and no
   event content, and a principal who may read the chain may check it.

2. **The resource is the tenant, and a subtree-bound auditor is refused
   rather than served a subset.** There is one chain per tenant, and an
   event's `resource` is a scope for some actions, a binding or a tenant
   or nothing for others. A surface that returned "the events we could
   attribute to your subtree" would silently omit events it could not
   parse — the one property an audit answer must never have. So the
   decision is asked with `Resource::Tenant`, a team-bound auditor is
   denied with the requirement named (the FLOW-6 refusal shape, which
   names `--scope` rather than saying no), and a scope *filter* on the
   search route narrows what the caller may already read and can never
   widen it. **An audit answer is complete over what it claims to cover,
   or it refuses.**

3. **Four routes; the two AC questions get one call each.**
   - `GET /v1/audit/events` — the search: `actor`, `action`, `outcome`,
     `resource` (exact match on the string the chain recorded, described
     as that), `from`/`to`, `scope`, and a `seq` cursor. Denials are not
     a mode: `outcome=deny` is a filter value like any other, which is
     what the AC's "read-only incl. denials" asks for.
   - `GET /v1/audit/disclosures?record=<id>&on=<date>` — "who could see X
     on date D".
   - `GET /v1/audit/knowledge?subject=<A>&at=<T>` — "what did agent A
     know at time T".
   - `GET /v1/audit/verify` — the chain check the CLI has had since
     AUD-1, now reachable by an auditor who holds no `DATABASE_URL`.

4. **"Who could see X on date D" answers with two lists, and the surface
   never merges them.**
   - `disclosed` — every subject the chain records the record being
     *served* to that day: the `context.injected` and `context.recalled`
     events whose entries name it, each with the object address, channel,
     tier and staleness that reader actually got, and the seq that proves
     it. This is evidence.
   - `authority` — the state that governed the record's scope that day,
     reconstructed from the events that opened and closed it: the pack in
     force, the role bindings standing, the lapses open with their
     windows and their declared ceilings, the published channel commit,
     and the classification in force. These are inputs.

   Collapsing the two into a single "could see" set would require
   *deciding* — running the PDP over reconstructed inputs — and ADR-0042
   option 5 rejected exactly that for the read path, on the grounds that
   it answers from a reconstruction rather than from the record. The
   objection is stronger here, where being evidence is the whole value of
   the answer. Two lists is the honest shape: what happened, and what
   permitted it.

5. **"What did agent A know at time T" is what A was served, not what A
   could have asked for.** Every `context.injected` and
   `context.recalled` event with actor A and `occurred_at <= T`, folded
   to the record ids last delivered, each carrying its object address,
   channel, tier, staleness and the channel commit that was live when it
   composed. That is the AC's "uses bitemporal + refs" precisely: the
   address resolves to exact bytes in the VedaFlow object store, and the
   id resolves to its version in the bitemporal pair as-of T. The answer
   is therefore *checkable* rather than displayed — the FLOW-4 discipline,
   where evidence is re-derivable from the chain that produced it.

6. **The surface resolves nothing.** Record ids, object addresses,
   channels, tiers, staleness, commits, pack versions, seq numbers —
   never a body, never a task string, never text a redaction removed. A
   caller who *also* holds `MemoryRead` resolves an address through
   CTX-5's `recall`: a different call, a different decision, a different
   audit event. Two authorities do not become one because they arrived in
   the same session, and an auditor holding only `AuditRead` gets the
   shape of what was known and not the knowing of it.

7. **Indexes only; no new column on `audit_log`.** Migration 0028 adds
   `(tenant_id, occurred_at, seq)`, `(tenant_id, action, seq)`,
   `(tenant_id, actor_subject, seq)`, and a `gin (payload
   jsonb_path_ops)` for the containment query that finds a record id
   inside an `entries` array. An index changes no byte any hash covers,
   so every row written since AUD-1 still verifies unchanged. The
   migration header states the rejected alternative rather than leaving
   it to be rediscovered: a `scope_id` column inside the canonical form
   would invalidate the existing chain, and outside it would be a field
   the chain does not protect.

8. **A query is itself audited, and it appears in the next query's
   results.** An allowed admin-plane read chains a standalone
   `authz.decision` (ADR-0019 decision 4), and `AuditRead` is no
   exception. That the audit log records reads of the audit log is a
   property rather than an accident — "who has been reading the trail"
   is a question a regulator asks — and it is why the events route is
   cursor-paginated on `seq` rather than offset-paginated: the chain
   grows underneath a reader who is reading it.

9. **Every answer states its own completeness.** Each response carries
   the seq range it covered and the chain head at the moment it was
   taken, so an answer can be re-derived exactly, and one taken before an
   append can be told from one taken after. A page that hit its limit
   says so rather than ending quietly.

10. **`actor_kind` is joined, never duplicated.** ADR-0019 decision 7
    deliberately kept "user or service identity" out of the event row and
    left it to query time; the events route resolves each subject against
    `identities` and `service_identities` and labels it. A subject that
    resolves to neither — a `break_glass` OS user, a `system` component,
    an identity since deleted — is labelled as exactly that. The join
    never invents an attribution the chain did not record.

11. **The CLI splits along the authority line it already uses.**
    `synveda audit verify` and `synveda audit tail` stay as they are:
    direct-to-store break-glass, for the operator who has lost the
    gateway. The new `synveda audit events`, `disclosures` and
    `knowledge` go through the gateway under the PDP, like `synveda
    recall` and `synveda proposal` — which is what lets the demo run them
    with `DATABASE_URL` unset and have that mean something.

12. **The performance budget is pre-registered here, because nothing else
    covers the admin plane.** Median ≤ 200ms for each of the three query
    routes over a chain of 1M events in a single tenant, with tails
    reported and not asserted — the HIER-1/MEM-1/CTX-1 discipline for
    ACs that cross IO on dev hardware. EVAL-6 owns percentile SLO
    enforcement on production-shaped IO, as it does for every other
    measured path in this product.

## Options considered

1. **Replay the PDP over reconstructed historical packs, bindings,
   placements and lapses** — the most literal reading of "who could see
   X on date D", and it would produce the single set decision 4 declines
   to produce. Rejected, and this is the second time: ADR-0042 option 5
   rejected it for the read path because it lets a revoked reader read
   and makes every historical misconfiguration permanently exploitable,
   and named AUD-2 as the home of the question "from the audit chain,
   which recorded the real decisions rather than a replay of them". The
   objection is stronger on this side of the line. A replay's answer is
   only as good as the reconstruction, the reconstruction is only as good
   as the current tables, and the tables do not hold history — so the
   feature would compute a confident answer out of inputs the chain
   already holds more reliably. Worse, it would be *indistinguishable in
   the response* from the recorded answer.
2. **Add `scope_id` (or `record_id`) columns to `audit_log`** — makes
   the queries trivial and the indexes obvious. Rejected on both possible
   placements: inside the canonical form it breaks verification for every
   row written since AUD-1, and outside it produces an audit column the
   audit chain does not protect, which an attacker with the app role
   could set freely while `verify` stayed green.
3. **A materialised projection folded from the chain, FLOW-4's shape** —
   a `audit_disclosures` table with a watermark, rebuildable from seq 1.
   Rejected *for now*, and it is the recorded reversal trigger. FLOW-4's
   projection exists because auto-promotion sweeps continuously and must
   be cheap on every cycle; an audit query is interactive and rare. A
   projection is a second thing to keep honest, and the first version of
   this feature should not ship one to solve a latency problem nobody has
   measured yet.
4. **Subtree-scoped audit reads, filtered by parsing `resource`** —
   would let a team-bound auditor read their own subtree's trail.
   Rejected under decision 2: the column is a display string by
   specification, the parse would succeed for some actions and fail for
   others, and the failure mode is an audit answer that quietly omits
   rows. Refusing is the smaller harm and the honest one.
5. **Resolve content inline for callers who hold `MemoryRead` as well** —
   convenient, and it would make the console's job easier later.
   Rejected: it makes one response the product of two authorities, so a
   later reader of that response cannot tell which one produced which
   field, and it puts the content path behind a route whose whole
   contract is that it has none.
6. **One `/v1/audit/query` endpoint with a `question` parameter** —
   fewer routes, and it reads well in a feature list. Rejected: the two
   questions have different answers, different shapes and different
   honesty caveats, and folding them into one endpoint would either
   flatten the caveats or attach them to a response shape that varies by
   parameter.
7. **Do nothing — leave `synveda audit verify`/`tail` as the surface** —
   fails the AC (one API call each, and neither question is answerable by
   tailing), and leaves `auditor` what it has been since AUTHZ-3: a
   marker row in the golden role matrix with no live action behind it.

## Consequences

- Positive: the auditor role stops being a marker and gets its first live
  action, closing the last of AUTHZ-3's deferred role obligations
  alongside FLOW-3's `compliance` and MEM-2's `security-reviewer`. Five
  ADRs' forward obligations close at once, and none of them needs an
  emission change to do it — the payload fields they each added turn out
  to be exactly sufficient, which is worth recording as evidence that
  ADR-0019 decision 4's discipline paid.
- Positive: the answers are recomputable. A disclosure names an object
  address and a seq, a knowledge answer names versions and commits, and
  every response names the chain head it was taken against, so an
  auditor's finding can be re-derived by someone who does not trust the
  auditor.
- Positive: `audit_log` gains query performance without gaining a single
  byte that verification does not cover.
- Negative / accepted trade-off: "who could see X on date D" does not
  return the set a regulator may have in mind. It returns who *was told*
  and what *stood*, and an auditor who wants the counterfactual must
  reason over the second list themselves. This is stated in the response
  itself, not only in this ADR — the two fields are named `disclosed` and
  `authority` precisely so nobody reads one as the other.
- Negative / accepted trade-off: a team-bound auditor can read nothing.
  Audit is a tenant-level function in this product until someone builds
  the attribution that would make a subtree answer complete, and that is
  a schema question, not a query question.
- Negative / accepted trade-off: the chain grows on read. An estate with
  heavy audit use will find `authz.decision` events for `AuditRead`
  making up a visible share of the trail, and the cursor pagination
  exists because of it.
- Negative / accepted trade-off: the GIN index on `payload` is over every
  payload in the chain, not only the disclosure ones. It is the largest
  index in the schema on the largest table in the schema, and TEN-5's
  disposal work inherits it.
- Reversal trigger: if the disclosure or knowledge route's **median**
  exceeds the 200ms budget at 1M events on production-shaped IO — or if
  the GIN index's size or write amplification shows up in the
  `audit.append` histogram that every mutation pays — option 3's folded
  projection is the recorded upgrade, with FLOW-4's watermark and
  rebuild-from-seq-1 machinery already built and tested to copy.
- Reversal trigger: if a real deployment needs subtree-scoped audit
  reads, the change is a structural attribution on new events (a scope
  the emission seam knows and the canonical form covers), not a parse of
  the existing `resource` column — and it answers only for events written
  after it lands, which is the fact that would have to be lived with.

## Compliance notes

Seed §2.5's "every access, decision and mutation is recorded
tamper-evidently" gains its reading surface; seed §5's `auditor`
("read-only including audit logs, cannot touch content") is implemented
as written, with the no-content rule enforced by the route having no
content path rather than by a filter it could forget to apply. Seed §2.2
is untouched: this surface makes no authorization decision of its own and
introduces no PDP bypass — it reads decisions others took, and its own
read is gated by `AuditRead` like every governed act.

Tenant isolation: all reads go through `synveda_store::rls::begin_tenant_tx`
against the same forced-RLS `audit_log` policy AUD-1 installed; a chain
cannot be read across tenants because the genesis hash binds it to its
tenant and the policy refuses the rows besides. The new indexes are
tenant-leading, so no query plan reaches another tenant's rows even
before the policy applies.

The surface is deliberately not an existence oracle: a record id that
does not exist, belongs to another tenant, or has been disposed of under
MEM-6 returns the same empty disclosure answer, in the shape ADR-0041
decision 6 established for `recall` and for the same reason. Answers
carry no record content, no task text and no redacted material, so an
`AuditRead` grant discloses the trail's structure and never the corpus.
AUD-3's WORM export and AUD-4's SIEM stream consume the same reads;
CNSL-3 surfaces "what did the agent know at T" as a console query over
this API rather than over the store.
