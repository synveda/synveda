# ADR-0040: Decay, TTL & staleness — retention is a pack property read at the moment somebody asks, expiry and destruction are two horizons rather than one, and staleness scores without labelling

- **Status**: Accepted
- **Date**: 2026-07-26
- **Feature(s)**: MEM-6
- **Deciders**: sujitn

## Context

MEM-6's text is "retention per policy pack; staleness scoring; pinned
exempt; Temporal sweep jobs", and its acceptance criterion is "retention
policy change re-evaluates existing records; audit trail of expiries".

Four accepted ADRs park obligations here. ADR-0020 left staging rows as
"immutable provenance whose retention/disposal lands with MEM-6/TEN-5,
which must honour the idempotency horizon". ADR-0021 said
"`observe_quarantine` and staging retention share one disposal horizon
(MEM-6/TEN-5)", and migration 0012 wrote the same sentence into the
schema: "content disposal is MEM-6/TEN-5 territory and brings its own
grants". ADR-0031 decision 6 put `valid_to` inside the content address,
"so MEM-6 and MEM-5 inherit the obligation to re-commit when they rewrite
or close a record". ADR-0039 decision 10 observed that a merge's
provenance stamp — "this fact was observed again, at this time, from this
event" — "is exactly the staleness and usage signal MEM-6 and FLOW-4
asked for", and ADR-0039 option 1 said the sweep shape "is right for decay
(MEM-6, which is about the passage of time)".

Forces at play:

- **The acceptance criterion is a statement about *when* retention is
  read, not about what it does.** "A retention policy change re-evaluates
  existing records" is trivially true of a design that stamps nothing on
  a record and reads the pack at the moment somebody asks, and is a
  backfill job — one that can silently miss rows, and whose correctness
  nobody can see — for a design that stamps an `expires_at` at write
  time. Seed §4.2 lists "`ttl` / decay policy **reference**" on the
  record, and a reference to the scope's pack is exactly what this is.
- **Nothing in the product destroys anything yet.** Every table added
  since FND-4 is append-only or update-only by grant; `records_history`
  is append-only by trigger; the staging plane has held every payload
  ever observed, pre-extraction, under RLS, since MEM-1. Seed §6 says
  `regulated-strict` means "retention enforced", and a product that can
  only *hide* material has not enforced anything.
- **But the bitemporal pair is the killer demo, and expiry must not
  break it.** Seed §9 Phase 2 names "what did the agent know on date X"
  as the regulated-industry demo, ADR-0006 built the pair for it, and
  MEM-5's AC leans on it ("superseded facts retrievable via as-of").
  A record leaving the live corpus and a record's history being destroyed
  are two different acts with two different justifications, and a design
  with one horizon has to pick which of the two demos to break.
- **A sweep's interval is an exposure window if the sweep is the only
  enforcement.** AUTHZ-4 met the same problem and answered it: a lapse's
  window closes "in the query that asks", and the sweep only writes the
  audit line (ADR-0037 decision 4). A bank that shortens its retention
  schedule on Monday morning should not still be injecting the material
  on Monday afternoon because a loop runs every five minutes.
- **Determinism is a shipped acceptance criterion.** CTX-2's AC is
  byte-identical re-composition at the same instant, and ADR-0025
  achieved it by making the valid-time instant an explicit input and
  forbidding clock reads in the engine. Any staleness score that reads a
  clock breaks a green AC.
- **Decay is the one part of this feature that can quietly lose
  information.** A wrong ranking penalty costs a true fact its slot in a
  budget-bound block; a wrong expiry costs it everything. The seed
  already fixed the guard rail: pinned records "cannot be shadowed or
  decayed" (§4.2), and it is the feature text's own "pinned exempt".
- **The composition plan is already per-scope, and already carries
  per-scope pack configuration.** ADR-0025 gave each planned scope its
  channel rule; ADR-0038 made the predicate a `(scope, tier)` pair and
  put "what the PDP decided per scope" on the plan. A per-scope retention
  horizon is the same shape, arriving through the same effective-pack
  walk, and needs no new resolution path.
- **Temporal is still deferred.** ADR-0022 decision 1 hosts the pipeline
  as a PGMQ-polling worker with Temporal-shaped stages because the SDK's
  licence graph fails `cargo-deny`; FLOW-4 and AUTHZ-4 both added plain
  `tokio` loops on that precedent. "Temporal sweep jobs" in the feature
  text is a hosting decision that has already been taken twice.

## Decision

**Retention is a property of the policy pack governing a scope, evaluated
at the moment somebody asks — never a timestamp stamped on a record.**
The read path stops serving material past its scope's horizon in the query
that asks; a background sweep then **expires** it out of the live corpus
(a temporal delete, so `records_history` keeps answering "what did the
agent know on date X") and, at a second and longer horizon, **destroys**
it, along with the staging plane MEM-1 and MEM-2 have been accumulating
since they landed. **Staleness is a ranking signal** the composition
engine computes from the instant it is already given, so nothing about
CTX-2's determinism changes. **Pinned material is exempt from all of it**,
by seed §4.2, and not by a switch a pack can flip.

Decisions, specifically:

1. **Nothing is stamped on a record.** No `expires_at` column, no
   per-record TTL, no retention state machine. A record's fate is a
   function of facts it already carries — its class, its kind, its valid
   time — and the pack in force at its scope *now*. The AC's "retention
   policy change re-evaluates existing records" is therefore structural:
   there is nothing to re-evaluate, because nothing was ever decided in
   advance. Seed §4.2's "`ttl` / decay policy reference" is read as the
   reference it says it is, and the referent is the scope's pack.

2. **Two enforcement points, and the earlier one is the read path.** The
   composition plan carries each planned scope's retention horizon beside
   its channel rule and its tier set, and `compose_candidates` refuses
   material older than it in SQL. The sweep is *disposal*, not
   enforcement. This is ADR-0037 decision 4's shape applied to a second
   feature, and for the same reason: the interval between a policy change
   and the next sweep must not be a window in which expired material is
   still injectable.

3. **The retention clock is the record's `valid_from`.** It is the one
   stamp that moves only when the fact does. `tx_from` is the current
   *version's* stamp, so a reclassification (ADR-0038 decision 9) or a
   MEM-5 merge would reset a five-year-old record's retention clock — a
   retention schedule that can be reset by re-reading or re-tagging is not
   a schedule. A last-use clock is worse: retention would be defeated by
   the very act retention exists to bound. And a stored `expires_at` is
   decision 1.

   The consequence is deliberate and is the correct one: an observation
   asserting an `occurred_at` older than the horizon (MEM-3 decision 7 sets
   `valid_from = occurred_at`) lands *already* expired and leaves on the
   next sweep. A retention schedule is about the age of the information,
   not about when a harness got round to sending it.

4. **Horizons are per record class, because that is how retention
   schedules are written.** "Session episodes 30 days, decisions seven
   years" is the shape of every real schedule, and `RecordClass` is a
   closed six-value vocabulary (seed §4.2), so the config is a fixed
   struct rather than a map: a pack cannot name a class that does not
   exist, every class is answered, and the config stays `Copy + Eq + Hash`
   like every other pack config (ADR-0039's note on `PackConfig`). Zero
   means keep — the absence of a horizon, not a horizon of zero length.

5. **Two horizons: expire, then destroy.** *Expire* is
   `records::delete` — the temporal delete FND-4 built: the current
   version archives into `records_history` with its transaction period
   closed, the record stops existing going forward, and the CTX-1 sidecar
   drops its document through the change feed it already tails
   (`records_history.tx_to` is half of that feed — ADR-0024 decision 4).
   *Destroy* is the history rows themselves, past a second and longer
   horizon measured from when each version closed.

   One horizon cannot serve both: a product that only expires keeps every
   payload forever behind an as-of query, which is not retention; a
   product that only destroys cannot answer what the agent knew last
   March, which is the demo the seed leads with. Two horizons is also what
   a retention schedule actually says — a period of use and a period of
   preservation — so this is the honest model rather than a compromise.

6. **The destruction path is a named flag on an append-only trigger, not
   a superuser and not a new role.** Migration 0001 says in its own
   comment that `records_history`'s append-only trigger is "not a security
   boundary (a superuser can drop triggers) — defence in depth against
   application bugs, complementary to the AUD-1 hash chain". So the purge
   is a `set local synveda.retention_purge = 'on'` that the trigger
   honours, plus a DELETE grant, inside the sweep's own tenant
   transaction. RLS still forces `tenant_id`, so the boundary that *is* a
   boundary is untouched, and the adversarial suite gets the attack: a
   purge cannot reach another tenant's history however the flag is set.

   A `SECURITY DEFINER` function was the alternative and is worse: it
   would run as the owner and therefore *bypass* RLS, trading a
   defence-in-depth trigger for a hole in the actual isolation boundary.

7. **The staging plane is disposed on its own horizon, and the
   idempotency guarantee is exactly as long.** `observe_quarantine` rows
   go first, then `observe_events` — migration 0013's FK is deliberate and
   its comment already says "disposal (MEM-6/TEN-5) retires both at the
   same horizon". This discharges ADR-0020's and ADR-0021's parked
   obligation, and it is the disposal that matters most: staging holds
   whole redacted payloads, pre-extraction, and it has grown without
   bound since MEM-1.

   Disposal frees `(tenant_id, idempotency_key)`, so MEM-1's
   first-writer-wins admission gate stops covering a disposed key. That is
   named here rather than discovered later: **idempotency holds for the
   staging horizon** — days, with a validated floor of one day, against
   adapters that retry in seconds and a pipeline whose lag SLO is 60s.
   A pending quarantine row aged out is disposed like any other and
   counted separately in the audit payload, because "three reviews nobody
   ever did" is a fact an auditor should be told rather than one they have
   to notice.

8. **Pinned is exempt from expiry, from destruction, and from staleness —
   by law, not by config.** Seed §4.2: pinned records "cannot be shadowed
   or decayed". No pack can turn that off, and the exemption is one clause
   in the SQL predicate and one branch in the scorer rather than a
   configuration surface. Disposing of pinned material stays a human act
   through the existing surfaces, and TEN-5's erasure is where a
   subject-rights deletion that must reach pinned content belongs.

9. **Published material is not exempt, and the sweep still never writes a
   channel.** Retention is the org's own decision about its own data and
   it outranks curation — a scope cannot buy immortality for a record by
   publishing it. But a commit is an *authored* act (FLOW-3's whole
   thesis; FLOW-7 decision 9 makes even a rollback a governed one with a
   mandatory reason), and a sweep has no author. So the published tree
   keeps naming an id whose record is gone, and nothing composes: ADR-0031
   decision 9's read already documents that "an id the predicate rejects —
   deleted, re-classified above the ceiling, or outside its valid window —
   simply does not come back". A dangling published member is inert by
   construction, and the alternative is a system principal rewriting a
   trust boundary at 3am.

10. **A scope's pack decides what that scope *serves* and what that scope
    *keeps*.** For material that lives where it is read — all derived
    material, and published material that has not climbed — those are the
    same number. They diverge only for a FLOW-5 cross-scope publication,
    where a department's published tree names a record living at a team
    below it, and there each answer is right for its own question: the
    department's pack decides what the department circulates, the team's
    pack decides what the team retains. Neither can widen anything: a
    horizon only ever removes.

    Two resolution points follow from this and are worth stating, because
    they are where the rule meets a table. Records and their history
    resolve **per scope**, from the scope the record lives at — the sweep
    visits the scopes that hold material rather than every hierarchy node,
    and the destruction stage additionally visits scopes that hold only
    *closed* versions, because a record that has already expired leaves no
    live row to be found by. The **staging plane resolves once, at the org
    root**: its rows deliberately carry no scope foreign key (ADR-0020
    calls them provenance records, and a service identity's revocation
    must not destroy them), so the buffer is a tenant-level object and its
    horizon is a tenant-level number.

11. **The derived channel is a log, so an expiry re-commits nothing.**
    ADR-0031's inherited obligation is about a record whose *address*
    changes — MEM-5 discharged it by re-committing closed records, because
    `valid_to` is inside the address. A record that ceases to exist
    changes no address, and a log channel's tree "is exactly this write's
    members" (ADR-0031 decision 4). Rewriting the log to erase what the
    pipeline once wrote would be falsifying a log to record a disposal.
    The obligation is discharged with nothing to do, and this is where
    that is written down.

12. **Staleness scores; it does not label.** The score is exponential
    decay with a pack-configured half-life over *time since last
    assertion* — `valid_from`, or MEM-5's
    `provenance.merged.last_observed_at` when a restatement reinforced the
    record, which is the reinforcement signal ADR-0039 decision 10 said
    this feature asked for. It is computed in the composition engine, in
    Rust, from the instant CTX-2 already takes as an explicit input, so
    there is no clock read and the determinism AC is untouched.

    It reorders *within* a gradient position and never across one: seed
    §4.4's order (pinned beats derived, nearer scope beats further, newer
    valid time beats older) is law, and a freshness heuristic does not get
    to overturn a scope boundary. It rides the inject response as a number
    per entry — auditable, and free — and the rendered block gets no
    `[stale]` marker: the labels there (`[unreviewed]`, `[lapse]`,
    `[restricted]`) are trust statements, an age is not one, and every
    label spends budget on every entry that carries it.

    **Retention runs from first assertion, staleness from last.** The two
    clocks answer two questions — how long we have held this, and how long
    since anyone confirmed it — and a restatement that resets the second
    must not reset the first (decision 3).

13. **The mode is two-valued, and the product default expires nothing.**
    `off` gives the pre-MEM-6 product back exactly, which is the shape
    MEM-5 shipped for dedup and the shape its AC test uses. `enforce` is
    the default: the machinery is on, and every record horizon is unset,
    so nothing expires until a pack names a number.

    **No embedded pack names a record TTL.** An upgrade that silently
    deletes a tenant's memory is the one surprise this product must never
    spring, and it is ADR-0033 decision 6's fail-safe restated — an absent
    trigger must not fire. What `regulated-strict` differs on is the plane
    where indefinite retention is the *risk* rather than the service: it
    disposes of staging at 7 days against the relaxed packs' 30, and
    carries the shorter staleness half-life. That is seed §6's "retention
    enforced" in the one place a schedule can be a product default without
    destroying something a tenant expected to keep.

14. **The sweep is the third background loop, on the lapse sweep's
    shape.** Per tenant, per stage, batched and bounded, resumable, and
    everything it does is audited under `actor_kind=system` (migration
    0014's actor kind, which ADR-0022 introduced for exactly this class of
    writer). Its stages are Temporal-shaped in ADR-0022's sense —
    serializable inputs, orchestration split from transport — so OPS-2 can
    host them under the SDK when its licence graph stops failing
    `cargo-deny`. "Temporal sweep jobs" is a hosting decision that
    ADR-0022 decision 1, FLOW-4 and AUTHZ-4 have now each taken the same
    way.

15. **Two audit actions, because two different things happen.**
    `memory.expired` says material stopped being available: the pack and
    version that decided, the horizon, the class, the ids, the ages —
    never content. `memory.disposed` says content was destroyed, per
    plane, with counts and the horizon that authorised it. Splitting them
    is ADR-0019 decision 4's rule (one event per audited operation,
    asserting one fact) and it is also the auditor's own split: "show me
    what we stopped using" and "show me what we destroyed" are different
    questions, and the second is the one with a legal answer.

    The expiry event is *not* bookkeeping in the sense `policy.lapse.expired`
    is: a lapse expires whether or not its sweep runs, but a record is
    destroyed only because this loop ran. The event and the act commit in
    one transaction.

## Options considered

1. **Stamp `expires_at` at write time** — seed §4.2 read literally, one
   indexed column, a trivially cheap sweep. Rejected: the AC becomes a
   backfill over every record in every tenant on every policy change, and
   a backfill that misses rows fails silently and looks identical to
   success. It also puts a policy decision in the record, where a later
   pack change cannot reach it.
2. **Sweep-only enforcement, no read cut** — one enforcement point,
   simplest to explain. Rejected: the sweep interval becomes an exposure
   window, and "retention enforced" would mean "retention enforced within
   five minutes, usually". ADR-0037 already refused this shape for lapses.
3. **Read cut only, no sweep** — no destruction, no grants, no trigger
   flag, no new failure mode. Rejected: nothing would ever be destroyed,
   the store would grow without bound, and a regulator asking "show me
   that it is gone" would be shown a query predicate.
4. **Expiry as a closed valid window rather than a temporal delete** — it
   reuses MEM-5's exact machinery and keeps the row readable. Rejected: it
   asserts the fact stopped being *true*, which is a different claim and a
   false one, and it leaves the content in the live table forever, so it
   is expiry in name only. `valid_to` is MEM-5's; `tx_to` is this
   feature's.
5. **One horizon: destroy on expiry** — simplest possible model, and the
   strictest reading of "retention". Rejected: it breaks the bitemporal
   demo for every record that ever expires, and it collapses two decisions
   an org genuinely takes separately (stop using / destroy) into one
   number nobody can set correctly.
6. **A `SECURITY DEFINER` purge function** — the textbook way to make a
   narrow hole in an append-only surface. Rejected in decision 6: it runs
   as the owner and bypasses RLS, which trades a trigger that was never a
   security boundary for a hole in the one that is.
7. **A `[stale]` marker in the rendered block** — visible, honest, and
   symmetrical with `[unreviewed]`. Rejected: it spends budget on every
   entry to state a number the response already carries, and it reads as a
   trust label when it is an age. Recorded as a reversal trigger if EVAL-4
   shows agents acting on stale material they were shown the score for.
8. **Staleness computed from `now()`** — the obvious implementation, and
   what every decay model in the literature does. Rejected: CTX-2's
   determinism AC is byte-identical re-composition at the same instant,
   and a clock read in the engine fails it. The instant is already an
   input; using it costs nothing.
9. **Staleness as a retrieval-time penalty inside RRF fusion** — it would
   reach recall as well as composition. Rejected for now: RRF is
   rank-based by construction (ADR-0024's stopword note makes the same
   argument), the fused list is re-verified against Postgres truth anyway,
   and a score that changes ranking *before* the scope gradient is applied
   can move material across a boundary the gradient exists to hold.
   Recorded as EVAL-4's to measure.
10. **Retention as a Cedar condition** — packs already carry policy, and
    ABAC conditions landed in AUTHZ-5. Rejected: retention is not an
    access decision. It does not answer "who may see this", it answers
    "does this still exist", and modelling it as a permit would make
    disposal look like a denial to every consumer of the decision log.
    Pack *configuration* is the right plane, and it is where redaction,
    composition, promotion, lapse ceilings and dedup already live.
11. **A per-record retention override set by an author** — the escape
    hatch a curator will eventually want ("keep this incident review
    forever"). Not built: `kind = pinned` already means exactly that and
    is already exempt, so the override exists and is spelled "pin it". A
    second mechanism would need its own audit trail and its own abuse
    story.
12. **Disposing of `pgmq.a_observe`** — the archive grows with every
    processed signal. Not in scope: it holds `{tenant_id, event_id}` and
    no content, it is MEM-3's dead-letter/completion record, and its
    disposal is operational rather than a retention obligation. TEN-5.
13. **Audit-row retention** — the same argument would apply to
    `audit_log`. Refused here on purpose: an append-only hash chain that
    a background loop can delete from is not tamper-evident, and STATUS
    already assigns audit retention and erasure to TEN-5, where the
    export/anchoring story (AUD-3) can be a precondition for it.
14. **Do nothing until Temporal lands** — the feature text does say
    "Temporal sweep jobs". Rejected: ADR-0022 decision 1 already made the
    hosting call, twice-followed since, and the stages here are shaped so
    that adopting the SDK is a transport change.

## Consequences

- Positive: the product can destroy data, which is the half of "retention
  enforced" it has never had; a retention policy change is visible in the
  very next inject, with nobody acting and nothing restarted, which is the
  AC demonstrated rather than asserted; the staging plane stops growing
  without bound and ADR-0020/0021's oldest parked obligation closes;
  "what did the agent know on date X" keeps working *because* expiry and
  destruction are separate horizons; staleness gives the composition
  engine a freshness signal without a clock, a label, or a new query; and
  a tenant that wants none of it writes `mode = off` and gets the
  pre-MEM-6 product back exactly.
- Negative / accepted trade-offs: expiry is not reversible through any
  product surface — a record removed by a horizon that was set wrong comes
  back only from `records_history` by hand, and once the destroy horizon
  passes, not at all (that is what destruction means, and it is why no
  embedded pack sets one); a published tree can name a record that no
  longer exists, which is inert but visible in `synveda channel history`;
  disposal of a staging row ends MEM-1's idempotency guarantee for that
  key; the read cut adds a per-scope predicate to the composition sweep on
  the p99-budgeted inject path; the staleness score is an unvalidated
  heuristic until EVAL-4 measures it, and its half-life default is a
  product guess; and `records_history` growth is now bounded by a horizon
  most tenants will never set.
- Reversal triggers: (a) the retention predicate showing up in CTX-3's
  latency budget → the horizon moves from the SQL predicate to a
  post-hydration filter, or to a partial index per shipped horizon;
  (b) EVAL-4 showing agents acting on material whose staleness score was
  low → the `[stale]` marker of option 7 lands, budget cost accepted;
  (c) a tenant needing a record-level retention override → it is a pin,
  and if that proves wrong the override is a governed proposal, never a
  field; (d) TEN-5 landing subject erasure → the purge flag and this
  sweep's stages are what it extends, and audit-row retention is decided
  there against AUD-3's anchoring; (e) OPS-2 adopting the Temporal SDK →
  these stages move to activities unchanged.

## Compliance notes

- **The PDP stays unbypassable.** Retention adds no authorization path and
  no Cedar action: the read cut narrows what a caller already had
  `MemoryRead` on, and the sweep authorises nothing at all — it acts on
  the tenant's own material under the pack the tenant's own scopes
  resolve to. No pack version bumps, because the action vocabulary is
  unchanged. A horizon can only ever remove, so no configuration in this
  feature can widen what a session sees.
- **Tenant isolation.** Every stage of the sweep runs inside
  `rls::begin_tenant_tx` for one tenant at a time; the purge flag changes
  the append-only trigger's behaviour and nothing about RLS, so a purge
  statement for another tenant's rows matches nothing. Both new DELETE
  grants (`records_history`, the staging pair) are least-privilege
  additions in the migration that needs them, and all three tables are
  already in the adversarial suite, which gains the purge attack.
- **Audit.** Two actions added (`memory.expired`, `memory.disposed`),
  chained in the same tenant transaction as the deletions they describe,
  so a destroyed row and the record that it was destroyed are one commit.
  Neither payload carries record content — which matters more here than
  anywhere else, since the point of the operation is that the content
  should stop existing, and an audit trail that quoted it would defeat the
  act it records.
- **What an auditor can prove afterwards.** After destruction the content
  is gone from `records`, `records_history`, the embedding and signature
  sidecars (by cascade), and the CTX-1 sidecar index (by the change feed
  the expiry already drove). What remains is the chain: which pack, which
  horizon, how many rows, which ids, and when. That is the correct
  residue — enough to prove the disposal happened and to reconstruct the
  policy that ordered it, and not enough to reconstruct what was
  disposed.
