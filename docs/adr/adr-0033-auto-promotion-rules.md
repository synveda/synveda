# ADR-0033: Auto-promotion rules — evidence swept from the audit chain rather than counted on the read path, rules that ride the pack, and proposals opened under the owner's authority

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-4
- **Deciders**: sujitn

## Context

FLOW-4's text is "Rule engine: e.g. procedure recalled >N times by ≥3
members → open proposal", and its acceptance criteria are "rule fires in
soak test; proposals carry evidence (usage stats) for the reviewer". Tech
plan §2.3 states the same rule as the illustration of how tribal
knowledge climbs; §3 puts the hook at the tail of the write path —
"commit to `derived` → maybe auto-open promotion proposal".

ADR-0032 wrote this feature's first constraint before the feature
existed: reversal trigger (a), "a scope accumulating open proposals
faster than they are reviewed (the `MAX_OPEN_PROPOSALS` cap tripping in
normal use) → FLOW-4's auto-opened proposals need batching or an expiry,
before the cap is raised". The cap is 500 per scope and its doc comment
names FLOW-4 by name. Trigger (b) is aimed here too: requirement
resolution moving into a path a human is not pacing.

Forces at play:

- **The signal lives on the hot path, and the hot path has a measured
  ceiling.** Which records were composed into whose context block is
  known only at `inject`. CTX-3's inject runs p50 18.6ms / p99 24ms
  against a 150ms budget, and every inject already appends one
  `context.injected` event inside its own transaction — which means it
  already takes the per-tenant audit chain-head lock, the one contended
  resource in the product, whose ceiling `inject_latency.rs`'s
  saturation probe prints on every run. Anything FLOW-4 adds *inside*
  that transaction is added inside that lock.

- **The example rule cannot fire on the material it describes, and not
  by an oversight this ADR can fix.** A derived record lands at its
  owner's personal node (ADR-0022; `DerivedBatch` in
  `crates/synveda-ingest/src/worker.rs`, "a record lands at its owner's
  personal node, so one scope in a group has exactly one owner"). And
  composition never leaves the caller's own placement chain:
  `permitted_chain_scopes` decides `MemoryRead` once per *chain node*
  (`crates/synveda-retrieval/src/authz.rs:57`), so another member's
  personal scope is not a candidate the PDP then rejects — it is never a
  candidate. Underneath that, every non-self `MemoryRead` permit in all
  three packs carries `resource.kind != "user"`. A count of distinct
  members over one person's derived memory is therefore identically 1,
  at two independent layers, by construction of the privacy floor.
  Whatever FLOW-4 builds, "≥3 team members recalled Alice's note" is not
  a fact that can come to exist.

- **Evidence a reviewer cannot check is a number.** The AC says
  proposals carry usage stats *for the reviewer*. A count rendered from
  a table the same subsystem wrote is a claim about itself. The product
  already keeps a tamper-evident record of exactly the events the count
  summarises, and tech plan §2.5's promise is that "the auditor reads
  proposals, not database rows".

- **A rule that acts has to act as somebody.** Seed §2.2 admits no path
  from harness to storage around the PDP, and ADR-0032 decision 9
  refused auto-publishing specifically because it would run under system
  authority. Opening is a governed act (`ProposalOpen`) exactly as
  publishing is. MEM-3 already faced this and answered it: the
  extraction worker re-decides `MemoryWrite` **as the owner**, at the
  owner's current home under its current quarantine state
  (`authorize_owner`), rather than inventing a pipeline principal.

- **Automation multiplies whatever it touches.** A rule that opens one
  proposal per record fires once per record per sweep forever unless
  something stops it. The review queue is a human's attention, the
  scarcest resource in the system, and `MAX_OPEN_PROPOSALS` is the
  backstop, not the design.

- **`derived` is not a candidate list; it is everything.** Every
  extracted memory is on it. A rule engine that walks the channel walks
  the corpus. Work has to be proportional to *usage*, which is a small
  and moving subset, not to what has accumulated.

## Decision

Usage is **swept out of the audit chain into a rebuildable projection**,
never counted on the read path. Rules are **configuration carried by the
effective policy pack**, resolved at the scope whose channel would move.
A rule that fires opens an ordinary FLOW-3 proposal — same table, same
matrix, same audit action — **under the material owner's authority**,
carrying evidence that names the audit range it was computed from.

Decisions, specifically:

1. **Nothing is added to the inject transaction.** `context.injected`
   already records, per inject, every composed entry's `record_id` and
   object address under the acting subject (`crates/synveda-gateway/src/inject.rs:341`).
   That is the whole signal. FLOW-4 reads it afterwards and writes
   nothing on the read path — no counter, no queue message, no second
   statement under the chain-head lock. The read path's cost of this
   feature is zero, and that is checkable: CTX-3's latency AC and its
   saturation probe must move by nothing.

2. **A sweeper folds the chain forward from a durable per-tenant
   cursor.** `audit_log` is keyed `(tenant_id, seq)` with seq contiguous
   from 1 (a gap is a verification failure, ADR-0019), which is a cursor
   with no ambiguity in it: read `seq > watermark` in order, fold, store
   the new watermark in the same transaction. This is CTX-1's watermark
   indexer and ADPT-1's durable cursor, applied to the one log that is
   already guaranteed gapless.

3. **The projection is an index, not a source, and it is rebuildable.**
   `memory_usage`, keyed `(tenant_id, record_id, subject)` with a recall
   count and first/last timestamps. Distinct members are a `count(*)`
   over that key and total recalls a `sum` — both facts fall out of the
   one row shape, so neither needs a maintained aggregate that could
   drift from the rows under it.

   Truncating the projection and the watermark and replaying from seq 1
   must reproduce it exactly. That property is the reason the projection
   may be treated as derived state — no audit trail of its own, no
   governance story of its own — and it is an AC test, not an aspiration.

4. **Evidence cites the audit range it was computed over.** A proposal's
   evidence records, per member: recalls, distinct members, first and
   last recall, and the `[from_seq, to_seq]` window of the chain the
   counts were folded from. A reviewer who does not believe the number
   can verify it against hash-chained rows; an auditor reading the
   proposal a year later can do the same after the projection has been
   rebuilt twice. This is what makes automated evidence admissible
   rather than merely present, and it is the argument that decided
   between the audit chain and a purpose-built counter (option 2 below).

5. **"Recall" is the set of audit actions that evidence usage, and today
   that set is `context.injected`.** Said plainly because it is weaker
   than the feature text implies: an injection is the *composition
   engine* choosing to spend budget on a record for a member, not a
   human asking for it. It is real usage — the record beat everything
   else in that member's chain for a place in a 1,500-token block — but
   it is a machine's vote. CTX-5's explicit `recall` is the stronger
   signal, and it joins by being added to the swept action set: the
   projection, the rules, and the evidence shape do not change. The
   evidence names which actions it counted, so a proposal opened before
   CTX-5 and one opened after are not silently the same claim.

6. **Rules are a pack-carried configuration: `policy_packs.promotion`.**
   Beside `redaction` (0013), `composition` (0017), and `approvals`
   (0019), resolved through ADR-0014's per-node assignment, so "which
   scopes auto-promote, on what thresholds" is answered by the same
   mechanism as every other per-scope behaviour, hot-reloads on the same
   refresher, and records its pack version on everything it does. Null
   means the pack configures no rules, and nothing auto-promotes —
   silence is the safe reading here, unlike the approval matrix where
   null still resolves to the invariant floor.

   A rule is a match plus thresholds: asset kind, memory classes,
   sensitivity ceiling, minimum recalls, minimum distinct members,
   minimum age, recency window, target channel. A rule cannot lower a
   requirement, grant anything, or publish anything — its entire power
   is to cause a proposal that the ADR-0032 matrix then judges exactly
   as it judges a human's. That asymmetry is why the pack is enough and
   why promotion rules do not need the governed-asset treatment
   ADR-0032 decision 14 gave curator files: changing who must approve is
   a change to authority, changing what gets proposed is a change to
   what lands in a queue.

7. **The distinct-member threshold is the only thing that distinguishes
   personal promotion from shared promotion, and it needs no scope-kind
   clause.** `min_distinct_members: 1` at a personal scope promotes a
   user's own heavily-used memory to their own `published` channel — not
   a triviality: under bank mode (published-only composition, ADR-0031)
   a scope's derived material does not compose at all, so promotion is
   the difference between a record existing and a record counting.
   `min_distinct_members: 3` at a team scope is the tech plan's rule, on
   the material it can actually apply to (decision 8). One knob, two
   products, no special case in the engine.

8. **FLOW-4 is same-scope, and the tech plan's illustration is FLOW-5's
   to deliver.** ADR-0032 decision 17 requires `source_scope_id ==
   target_scope_id`, and this ADR keeps it, but the reason it binds here
   is stronger than "FLOW-3 said so": as the second force above
   establishes, a personal record's distinct-member count cannot exceed
   1, so a personal→team climb could never be *triggered* by this rule
   even if the proposal were representable. The two facts are the same
   fact — the privacy floor — seen from the trigger side and the target
   side.

   The reach of that is wider than it first looks, and worth stating
   exactly: **a service identity is "placed like a user"** (ADR-0018
   decision 2 — registration creates a `ScopeKind::User` personal leaf
   under the anchor and points the identity row at it), so a
   team-anchored agent's extracted memories land on a user-kind leaf too,
   not on the team node. Every record the write path produces, human or
   agent, lands somewhere only its owner can read. `distinct_members` is
   therefore 1 for **everything `observe` → extraction can produce
   today**, without exception.

   So the rules that can fire today are the `min_distinct_members: 1`
   ones, and they are worth having on their own terms (decision 7): under
   bank mode a scope's derived material does not compose at all, so
   promoting a member's own well-used memory to their own published
   channel is the difference between a record existing and a record
   counting. That is what the soak test exercises, over the real product
   path — `observe` → extraction → repeated `inject` → sweep → proposal.

   A rule wanting two or more distinct members needs material at a
   *shared* scope, and nothing writes one yet: `MemoryWrite`'s floor is
   the owner's own home, and the pipeline commits there. Shared-scope
   material arrives with the first authoring path (PRMT-1's prompts,
   context packs) or with FLOW-5's climb. The engine needs no change for
   either — the threshold is already a number in a pack.

   A rule therefore names its `target_channel` and inherits its target
   scope from the material, and the same-scope constraint lives in one
   place. FLOW-5 relaxes exactly that constraint and these rules light
   up against ancestors unchanged. The *other* signal tribal knowledge
   needs — three people independently recording the same procedure, which
   is a similarity fact and not a usage fact — is MEM-5's, and it will
   feed the same proposal machinery from a different projection.

9. **The rule engine opens as the material's owner, never as itself.**
   ADR-0022's `authorize_owner`, generalised: resolve the owner
   identity, its current chain, its current quarantine state, the
   effective pack and role bindings on that chain, and decide
   `ProposalOpen` at the target scope with explicit context instead of
   task-locals. An owner who has since been quarantined, moved, or
   deleted stops having material proposed on their behalf, with no
   special case — the same property MEM-3 tests at commit. The proposal
   row records that owner as `proposer_id`/`proposer_subject`, which is
   true: it is their authority the act rode on.

   The audit event's actor is `system:promotion` — ADR-0022 decision 5's
   actor kind, component name as subject — and its payload names the
   owner whose authority was decided. Who acted and whose authority it
   was are two different facts, and a trail that recorded only one of
   them would be lying by omission in whichever direction it chose.

10. **One proposal per (scope, rule, sensitivity tier) per sweep.**
    ADR-0032 reversal trigger (a), discharged in the batched form it
    asked for. A proposal is already a set — its tree names many members
    and `MAX_PROPOSAL_MEMBERS` is 200 — so the natural shape of an
    automated promotion is one reviewable batch, not one proposal per
    record.

    Batching splits by sensitivity tier because ADR-0032 decision 3
    governs a proposal by the **maximum** sensitivity over its members: a
    single `confidential` record in a batch of 200 internal ones would
    drag the whole batch to the stricter requirement and make a routine
    review a compliance event. Tiers are batched apart so each batch
    faces the requirement its own contents earn.

    At the cap, the sweeper stops opening at that scope, emits one audit
    event saying so, and raises a metric. It does not raise the cap, drop
    the candidates silently, or spread them over other scopes.

11. **The content address is the idempotency key, and rejection binds
    bytes exactly as approval does.** A member is not proposed if it is
    already in an open proposal at that scope, already published at its
    current address, or was in a *rejected* proposal at its current
    address. The third is the one that matters and it costs no new state:
    ADR-0032 decision 6 made approvals bind bytes, and the same commit
    tree that records what was approved records what was refused. So a
    human's "no" suppresses re-proposal of those exact bytes forever,
    while an edited record is new material that may be proposed again —
    which is the behaviour a reviewer would expect and the only one that
    does not need a cooldown timer, a suppression table, or a
    configuration key nobody knows how to set.

12. **Evidence is a frozen column on the proposal row, a line in the
    commit message, and a block in the audit payload — three renderings
    of one fact, each where its reader is.** The column
    (`vedaflow_proposals.evidence jsonb`, added to migration 0019's
    transition trigger so it is immutable like every other non-closure
    column) is what CNSL-1's inbox and FLOW-6's `proposal show` read in
    one row read. The commit message carries the one-line human summary a
    commit needs anyway, so FLOW-8's git export shows why a promotion
    happened without a joined table. The audit payload carries the
    structure under the hash chain. Manually opened proposals leave the
    column null, which is the honest value: no rule fired.

13. **The evaluator is a second loop in `synveda-ingest`, spawned by the
    gateway binary.** Where the extraction worker already lives
    (`worker::spawn` / `run_once`, driven from `main.rs`), with the
    crate dependencies FLOW-4 needs already present — policy, store,
    audit, vedaflow. Its cadence is minutes, not the extraction worker's
    second: a promotion is not a hot path, and a sweep that runs while a
    reviewer is asleep is exactly as useful as one that runs now. It
    enumerates active tenants as the pack refresher does, and exposes
    `run_once` so demos and the AC test drain deterministically instead
    of waiting on a background loop — the MEM-3 discipline.

14. **Work is proportional to changed usage.** The sweep touches the
    audit rows since the watermark. Evaluation considers only records
    whose counters moved in that fold, resolves the effective pack once
    per distinct scope among them, and applies that scope's rules. The
    `derived` channel is never walked, the record corpus is never
    scanned, and a tenant with no injections since the last sweep costs
    two indexed reads.

## Options considered

1. **Sweep the audit chain into a rebuildable projection; rules on the
   pack; proposals under the owner's authority (chosen)** — zero
   hot-path cost, evidence that is verifiable against a hash chain, and
   configuration in the place every other per-scope behaviour already
   lives. Con: usage is visible to rules only after a sweep, so a
   threshold crossed at 09:00 fires at the next cadence rather than
   instantly; and the projection is a table whose only justification is
   that it can be thrown away.
2. **A usage counter written on the inject path** — exact, instant, no
   sweeper, no watermark, no rebuild story. Rejected on where the write
   lands: inject's transaction already holds the per-tenant chain-head
   lock, so an upsert per composed entry lengthens the one critical
   section that sets the tenant's inject ceiling, to make a background
   feature marginally fresher. It also produces evidence that is a
   number in a table this subsystem wrote, where the chosen option
   produces evidence a reviewer can check (decision 4).
3. **A PGMQ usage message enqueued by inject** — MEM-1's content-free
   work signal, reused; it decouples rules from audit payload shapes,
   which is a real benefit. Rejected: still a write inside the inject
   transaction, for a signal that is already being written one row over.
   Two records of the same event is the duplication ADR-0019 decision 4
   warns about, and the audit copy is the one with the hash chain on it.
4. **Query the audit log at evaluation time; no projection at all** —
   no new table, no watermark, nothing to rebuild, and the source is the
   only copy. Rejected: it is an unbounded jsonb scan over all history
   per evaluation, needing a GIN index on `audit_log.payload` that
   exists for no other reason, and it makes AUD-3's WORM export a
   behaviour dependency — archiving old audit rows would silently retire
   usage history. The projection is the same read done once, forward.
5. **Rules as a VedaFlow asset under a `promotion` ref, like curator
   files** — governed, diffable, versioned history of what the org
   decided to automate. Rejected for the asymmetry in decision 6: a
   curator file changes *who must approve*, which is authority, and had
   to be reviewable history; a promotion rule changes only *what gets
   proposed*, and everything it proposes still faces the whole matrix.
   Paying the asset cost for a knob that cannot lower a requirement
   would say the two are the same kind of thing.
6. **Rules in a dedicated table with their own admin API** — queryable,
   no pack coupling, and rules could be edited without a pack revision.
   Rejected: it is a fourth place to look when asking "what governs this
   scope", with its own audit story and its own versioning to build, to
   avoid adding a column beside three columns that already do this.
7. **The tech plan's hook: evaluate at the end of the observe
   pipeline** (§3, "commit to `derived` → maybe auto-open promotion
   proposal") — no loop, no cadence, no watermark; promotion happens
   where the material appears. Rejected as a misfit with the trigger
   this feature actually has: extraction is a *write* event and every
   threshold here is a *usage* fact accumulated by reads. Evaluating on
   write means evaluating a record at the one moment its usage is
   provably zero. The hook is kept for rules that are write-shaped when
   MEM-5's similarity signal arrives; the usage rules run on their own
   cadence.
8. **One proposal per record** — the simplest engine, the clearest
   review unit, and every proposal's sensitivity is exactly its own.
   Rejected against ADR-0032's cap and its recorded trigger: a scope
   with fifty qualifying records produces fifty reviews, and 500 is ten
   such days. Batching by tier keeps the sensitivity property that made
   per-record attractive (decision 10).
9. **A cooldown or expiry on auto-opened proposals** — the other half of
   ADR-0032 trigger (a)'s suggestion, and it bounds the queue by time
   rather than by count. Rejected as the primary mechanism: an expiring
   proposal is a review that silently did not happen, which is the one
   outcome a governance product must not produce quietly. Content-address
   idempotency (decision 11) suppresses the actual cause of pile-up —
   re-proposing what a human already refused — without ever discarding an
   open review. Expiry stays available if the cap trips for a reason
   idempotency does not cover.
10. **Auto-approve what the rule proposes, under `standard`/SMB packs** —
    tech plan §2.4 does say "SMB `standard` pack: most of the above
    collapses to single-approver or auto-approve". Rejected here, and it
    is not this ADR's call to make: an approval is a recorded act by a
    principal that Cedar permitted (ADR-0032 decision 5), and a rule
    engine casting one would be exactly the system-authority vote
    ADR-0032 decision 9 refused. If a pack wants zero-human promotion, it
    is the *matrix* that has to say a rule-opened proposal needs no
    approvals — a change to the approval floor, therefore a product
    decision and its own ADR.

## Consequences

- **Positive**: the read path pays nothing — the feature's entire cost is
  a background sweep proportional to usage. Evidence in a proposal is
  verifiable against the hash chain rather than trusted, which is the
  property that makes an automated proposal reviewable at all. The
  projection is rebuildable from seq 1, so it needs no governance story
  of its own and a bug in the fold is recoverable by truncation. Rules
  ride the pack, so a bank and an SMB differ by assignment, and every
  automated act records the pack version that caused it. An auto-opened
  proposal is an ordinary proposal — same table, same matrix, same
  `ProposalOpened` action, same publish path — so nothing downstream
  (FLOW-6's CLI, CNSL-1's inbox, FLOW-8's export, the audit story) needs
  to know rules exist. Rejection binding bytes means a human's "no" is
  durable against an engine that would otherwise ask again forever.
- **Negative / accepted trade-offs**: rules see usage one cadence late,
  which is invisible for a feature whose output is a review queue but
  makes the AC test drive `run_once` rather than assert on wall-clock
  timing. The projection grows as records × members who saw them, which
  is bounded by real usage but unbounded in time — no decay is applied
  here, so "recalls" means "ever", and a genuine windowed count would
  need per-period buckets (MEM-6's decay work is where that belongs, if
  the recency window proves insufficient). Evidence is duplicated in
  three renderings, deliberately, and they must be written from one
  computed value or they will drift. Auto-opened proposals make
  `MAX_OPEN_PROPOSALS` a live limit rather than a theoretical one for the
  first time. And the honest one: on today's material only
  `min_distinct_members: 1` rules can fire at all, because every record
  the write path produces lands on a user-kind leaf — a service
  identity's included — so FLOW-4 ships a rule engine whose
  multi-member case is waiting on a shared-scope writer (PRMT-1) or on
  FLOW-5's climb.
- **Reversal triggers**: (a) the sweep falling behind — cursor lag
  against the chain head growing across cadences — → the fold is doing
  per-row work that belongs in a set-based statement, before the cadence
  is shortened; (b) `MAX_OPEN_PROPOSALS` tripping at a scope *despite*
  batching and content-address idempotency → expiry becomes necessary
  after all (option 9), and it is a product decision about silently
  unreviewed material, not a config default; (c) the idempotency check
  against rejected proposals' trees showing up in the sweep's cost as
  closed proposals accumulate → a suppression index keyed by
  `(scope, object_hash)`, derived and rebuildable exactly as the usage
  projection is; ~~(d) CTX-5 landing → the swept action set gains explicit
  recall, and the evidence's action list is what keeps old proposals
  honest about which signal they counted~~ **(d — ADR-0042 decision 16:
  `context.recalled` joins the swept set, and because the evidence already
  names the actions it counted, a proposal opened before CTX-5 and one
  opened after stay distinguishable rather than silently the same claim)**;
  (e) FLOW-5 landing → the
  same-scope constraint in decision 8 moves to a rule's target
  expression, and the disclosure rule FLOW-5 decides governs what a
  rule may propose upward; (f) MEM-5 landing → similarity-triggered
  proposals join through the write-shaped hook option 7 kept open,
  feeding the same proposal machinery from a different projection.

## Compliance notes

The PDP stays unbypassable and gains no actions. An auto-opened proposal
takes the same `ProposalOpen` decision at the same target scope that a
human's does, decided against the material owner's *current* placement,
quarantine state, effective pack, and role bindings (ADR-0022's
`authorize_owner` discipline, with explicit context instead of
task-locals). There is no system principal, no standing grant, and no
rule-engine bypass: a rule whose owner is quarantined proposes nothing,
and a rule cannot cause anything to reach a published channel — the
ADR-0032 matrix and a human `ChannelPublish` still stand between the
proposal and the trust boundary, unchanged.

Two new tables (`memory_usage`, and the sweeper's per-tenant watermark)
join the forced-RLS set in migration 0020 with least-privilege grants:
the watermark takes select/insert/update, and the projection additionally
takes delete — the grant that makes decision 3's rebuild something the
app role can actually perform. Neither carries the append-only triggers
the governed-history tables carry (`audit_log`, the VedaFlow objects, the
approval log), and that difference is the whole point of decision 3:
these hold no governed facts, only a re-derivable summary of facts held
under a hash chain elsewhere. Both join the ADR-0009 completeness guard
and the adversarial RLS suite, and every read and write runs inside
`rls::begin_tenant_tx`. The sweeper
reads `audit_log` under the same tenant transaction as everything else:
it is a reader of the chain, never a writer, and it holds no capability
to alter, reorder, or truncate what it reads. `policy_packs` gains a
`promotion` column beside `redaction`, `composition`, and `approvals`.

The audit trail gains no action. `ProposalOpened` is emitted for a
rule-opened proposal exactly as for a human's, distinguished by
`actor_kind = 'system'` with subject `promotion` and by an evidence block
in the payload naming the counts, the swept actions, the chain range they
were folded from, and the owner identity whose authority was decided. A
second action asserting "a rule opened this" would be a fact an auditor
has to reconcile against the first (ADR-0019 decision 4, ADR-0032
decision 18). One further event is emitted where nothing else would
record it: a scope at `MAX_OPEN_PROPOSALS` whose candidates were not
proposed, because material a rule declined to raise is exactly the kind
of silence a governance log exists to break.
