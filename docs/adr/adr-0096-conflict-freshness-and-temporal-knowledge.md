# ADR-0096: unresolved Knowledge is transitional and freshness is governed configuration

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-37
- **Deciders**: autonomous context-platform continuation

## Context

The immutable Knowledge aggregate already preserves transaction history, valid
time, explicit relations and exact provenance. Capture can suggest duplicates,
conflicts and possible supersession, while ContextRun excludes revisions whose
explicit or configured freshness date has passed. These pieces do not yet form
one conflict/freshness contract: a direct write can publish a second opposing
active item, capture classifications are not durable resolution addresses,
the browser cannot ask what was valid versus what was known, and freshness
defaults are evaluated in only one reader.

CPR-30 also made freshness defaults part of an immutable governed
Configuration version. A separately mutable `FreshnessPolicy` table would be a
second authority and would let runtime behaviour diverge from the exact
configuration evidence already frozen on sessions and context runs.

## Decision

1. **Matching is bounded and write-side.** One deterministic classifier is
   shared by capture and Knowledge mutation. It compares content hashes and a
   bounded lexical candidate set and returns the closed classifications
   `duplicate`, `support`, `contradiction`, `supersession` and `transition`.
   Generative reconciliation is not run on ordinary reads. Every candidate
   item is individually admitted through `KnowledgeRead` before its address or
   classification may be retained.

2. **A conflict is a durable fact, not another content aggregate.** A
   `ConflictSet` owns ordered `ConflictMember` evidence citing exact immutable
   Knowledge revisions or one reviewable capture candidate. Both tables are
   tenant-leading, enabled and forced-RLS. They store ids, classifications,
   bounded reason codes and scores; proposed capture content remains in the
   capture aggregate and Knowledge content remains in immutable revisions.

3. **Unresolved writes are transitional.** An otherwise-applied create or edit
   that has a duplicate, contradiction, supersession or transition match
   retains its immutable revision and provenance but moves its head to the new
   `transitional` lifecycle. Ordinary listing, search and context planning
   exclude it. Supporting evidence may remain active because it does not
   create mutually exclusive current truth. A capture match creates a conflict
   set but remains an unreviewed candidate and therefore never enters current
   Knowledge.

4. **Resolution is a Knowledge/VedaFlow command.** Activating separate truth,
   recording support/duplication, superseding existing heads, scheduling a
   future transition or archiving the challenger uses a typed, revision-aware
   `resolve_conflict` Knowledge command. It opens the same Knowledge/apply
   VedaFlow proposal, repeats ownership and Cedar decisions over every member,
   and emits the ordinary change audit plus conflict-specific opened/resolved
   chain evidence. A merge continues to use the existing governed merge
   command; the conflict set records its exact change when the result is
   attached. No handler updates current state directly.

5. **`FreshnessPolicy` is an evaluated immutable view.** The public/domain
   object cites the exact effective Configuration artifact, binding, version
   and hash and exposes the type-specific trigger and default interval. It is
   computed from CPR-30 Configuration rather than persisted as a second
   mutable policy. An explicit revision `stale_after` wins. Otherwise facts,
   entities, procedures, conventions, warnings and references use their
   configured interval; decisions require explicit supersession; episodes are
   retained history; preferences remain owner/scope-sensitive. Repository
   change, failed-use feedback and source freshness are additional visible
   verification signals for conventions, procedures and references.

6. **Staleness is a read-time state until governed action changes the head.**
   A due item is excluded from ContextRun and appears in the staleness queue,
   but database time does not silently append a revision or change lifecycle.
   Verification appends an immutable revision and derives its next due date
   from the exact effective policy at application time. Supersession,
   archival and verification remain VedaFlow actions.

7. **Valid and transaction time stay orthogonal.** Public Knowledge queries
   accept `as_of` for valid time and `as_known_at` for transaction time.
   `include_history` admits non-current lifecycle heads; `include_transitional`
   admits unresolved or future-valid heads. Defaults remain current, active,
   non-transitional Knowledge. Cursor digests include every temporal selector,
   and every historical row is hydrated from the selected head version then
   independently PDP-decided.

8. **Disclosure is conflict-member scoped.** Conflict counts and members are
   formed only after per-item decisions. A denied item contributes no id,
   title, relation, count or reason; an aggregate omission marker may say that
   policy hid members. Audit payloads carry ids, hashes, classifications,
   transition times and decisions, never Knowledge bodies or capture payloads.

## Options considered

1. **Persist a new mutable freshness table.** Rejected: it would compete with
   governed Configuration and make an effective session policy unanswerable.
2. **Publish both items and resolve conflicts later.** Rejected: retrieval
   would silently present two unresolved current truths.
3. **Run a model reconciliation pass on every query.** Rejected: it is
   expensive, non-deterministic and too late to protect current-state
   semantics.
4. **Make conflict review a second proposal system.** Rejected: VedaFlow is
   already the one governed mutation and approval engine.

## Threat model and abuse cases

- A conflict is itself sensitive evidence. A caller who cannot read every
  exact member receives no set id, member id, classification, cardinality,
  title, edge or reason. The collection may report only that policy excluded
  some evidence, without a count.
- Transaction-time queries rewind content state, never authority. Historical
  heads still pass the caller's current Cedar decision and forced-RLS tenant
  boundary before hydration; a guessed foreign id therefore remains absent.
- Freshness signals run after the exact Knowledge decision. Repository,
  feedback and source rows collapse to bounded booleans, so they cannot expose
  a denied repository name, user, source address, count or feedback body.
- A stale conflict precondition closes its real VedaFlow change as rejected
  and leaves or reopens the set for a new review. It cannot apply an obsolete
  resolution or wedge a pending set permanently.
- Future transitions do not rewrite immutable history. The current projection
  keeps the predecessor until the authorised `transitions_to` successor's
  valid time begins, then excludes it from ordinary truth while explicit
  historical queries retain both.

## Consequences

- Positive: current retrieval has one explicit answer about unresolved writes;
  conflict evidence is durable and policy-safe; freshness and temporal queries
  cite the same immutable configuration/history already used by runtime work.
- Negative / accepted: a direct conflicting write may be applied yet not
  visible as current until its second governed resolution completes; policy
  filtering can make a conflict set appear to have fewer members than its
  database row count; deterministic lexical classification remains a proposal,
  not a claim of semantic certainty.
- Reversal trigger: a validated reconciliation model can add a versioned
  write-side classifier behind the same bounded contract; it may not move
  reconciliation into reads or bypass conflict/VedaFlow state.

## Compliance notes

- **PDP/VedaFlow/audit:** matching decisions before retention, exact-member
  reads and every resolution are PDP-governed; current-state mutations remain
  typed Knowledge/apply changes; conflict transitions are hash-chained.
- **RLS:** both new tenant tables are forced-RLS and enter the completeness
  inventory.
- **No parallel model:** the aggregate, revisions, capture candidates,
  Configuration and VedaFlow remain the only authorities for their respective
  content, runtime policy and mutation state.
