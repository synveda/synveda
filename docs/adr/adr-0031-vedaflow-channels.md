# ADR-0031: VedaFlow channels — published as a content-addressed set, derived as a log, and the trust boundary composition actually reads

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-2
- **Deciders**: sujitn

## Context

FLOW-2 is the feature three landed ADRs deferred work to by name:
"derived/staged/published refs per scope per asset type; inject reads
published (+ derived per policy). AC: 'bank mode' switch
(published-only) flips composition instantly."

ADR-0030 built the substrate and stopped there: "FLOW-2 turns `refs`
rows into the derived/staged/published channels the inject path reads",
and its decision 14 deferred the first audit action, route, and CLI verb
to whichever feature produced a governed action — this one. ADR-0022
recorded "FLOW-1/2 replace the direct records insert with the
derived-channel commit". ADR-0025 shipped the composition engine with
two explicitly transitional decisions and named their reversal trigger:
"FLOW-1/2 land → commit hashes replace version hashes and channel refs
replace the kind stand-in".

So the shape of the work is fixed. What is not fixed is the thing that
decides whether any of it is honest: **what a channel contains, and how
a record gets onto one.**

Forces at play:

- **`RecordKind` is currently doing a job it cannot do.** ADR-0025
  decision 2 made `pinned` stand in for the published channel, and its
  own consequences section calls this out: "`kind` conflates authorship
  with channel until FLOW-2 (a pinned record is *treated as* published
  without a review having happened)". Today an author can write a
  `pinned` record and it composes in bank mode. That is the opposite of
  what bank mode is for. FLOW-2 either fixes it or the AC is theatre.
- **Published and derived have opposite shapes.** The published channel
  is small, curated, and changes rarely — a *set* somebody stands behind.
  The derived channel is the extraction firehose: every observe event at
  every scope, continuously. A git-faithful "full membership tree per
  commit" costs O(corpus) `vedaflow_tree_entries` rows *per commit*. At
  10k derived records that is 10k rows per extraction batch. One model
  cannot serve both without sharding machinery neither channel's read
  path wants.
- **`inject` is the hot path.** Seed §10 fixes p99 < 150ms. Whatever
  composition reads to learn a record's channel has to be one indexed
  query for the whole scope chain, not a walk.
- **The content lives in `records`, not in the objects.** Records are
  bitemporal (ADR-0006), embedded (ADR-0023), and indexed (ADR-0024).
  Composition, retrieval, and conflict resolution all read them. A
  channel cannot become a second source of record text without forking
  the read path.
- **Publishing is a governed act with no home yet.** Nothing in the
  product writes a `published` ref. FLOW-3 owns proposals and the
  approval matrix; FLOW-5 owns cross-scope climbs. But a channel nobody
  can write to is not a channel, and "inject reads published" cannot be
  demonstrated against an empty ref.
- **Concurrency is now the normal case.** ADR-0030 predicted it: "Two
  ingestion workers committing to the same scope's derived channel in
  the same second is the normal case once FLOW-2 lands." The pipeline's
  write transaction already holds an archive-lock and takes the audit
  chain-head lock last (ADR-0019 decision 1); a ref compare-and-swap has
  to fit between them without inverting that order.

## Decision

Channels are `vedaflow_refs` rows named `{asset-kind}/{channel}` per
scope. **The published and staged channels are content-addressed sets;
the derived channel is an append-only log.** Composition resolves a
record's channel by looking its id up in its scope's published tree and
comparing content addresses, retiring the `RecordKind` stand-in. One new
PDP action publishes; one new audit action records it.

Decisions, specifically:

1. **A channel is a ref named `{asset-kind}/{channel}`.** `memory/derived`,
   `memory/published`, `memory/staged`, and the same three for `prompt`,
   `skill`, `context-pack`, and `policy` as those asset types arrive.
   FLOW-1 deliberately left the ref vocabulary open ("a CHECK constraint
   written now would have to be guessed and migrated then"); FLOW-2 fixes
   it in code, in `ChannelRef`, without a migration. `vedaflow_refs` stays
   generic on purpose — FLOW-3's proposal refs and FLOW-7's pins need
   names that are not channels, and a two-column schema would have had to
   grow a third.

   The name is parsed, not typed by the database. That is the accepted
   cost: `ChannelRef` is the only constructor the codebase has, both
   halves parse through the existing `AssetKind`/`Channel` vocabularies,
   and an unparseable ref name is simply not a channel.

2. **Refs materialise on first write; there is no bootstrap.** No
   per-scope initialisation, no backfill migration, no empty root commit
   manufactured so that three refs exist everywhere. A scope with nothing
   published has no `memory/published` ref, and reading it returns an
   empty set — which is the true answer, and the same answer the
   bootstrap would have produced at 100× the rows. `staged` therefore
   costs nothing until FLOW-3 writes it.

3. **Published and staged are sets; derived is a log.** A publish commit's
   tree is the channel's *entire* membership, one entry per member, so
   "what is published here" is one read of one tree. A derived commit's
   tree holds only the records that commit added, and the channel's
   history is the parent chain.

   The asymmetry is forced by cost, and it is safe because of decision 4.
   A full-membership tree per derived commit is O(corpus) rows per
   extraction batch; git's answer is sharded subtrees, which bounds the
   *write* at ~500 rows per commit but turns the *read* into a recursive
   descent over the whole corpus — the wrong trade for a channel whose
   membership composition never enumerates. Published sets are curated
   and small, so the flat tree is both affordable to write and free to
   read. `MAX_CHANNEL_MEMBERS` (10,000) is a reviewed constant on the
   `MAX_OBJECT_BYTES` precedent, and subtree sharding is the recorded
   upgrade if a scope ever approaches it.

4. **Derived is the complement, not an enumeration.** A record at a scope
   composes as derived material unless it is published there. Nothing
   needs to enumerate the derived channel to decide that, which is what
   makes decision 3 affordable: the derived ref's job is history,
   provenance, and FLOW-3's proposal source — not membership. This is
   also the honest reading of tech plan §2.2: derived is where everything
   lands, and published is the boundary drawn through it.

5. **Publication binds bytes, not ids.** A tree entry is
   `<record-id> → object hash of the exact version that was published`.
   Composition recomputes that address from the record version it is
   about to compose and requires it to match. A record edited after
   publication no longer matches its published object, so it composes as
   *derived* — unreviewed — until someone publishes it again.

   This is the decision that makes "published" mean something. The
   alternative — membership by id — would let any writer with
   `memory.write` change the text under a published id and have it
   compose as reviewed content, which is exactly the attack the review
   boundary exists to stop. It costs one hash comparison per candidate
   and no extra query, because the address is computed from the record,
   not fetched.

6. **A memory's object is canonical JSON over its governed fields.**
   `{class, content, id, kind, owner, scope, sensitivity, valid_from,
   valid_to}`, keys sorted bytewise, timestamps in the ADR-0019 canonical
   rendering. Human-readable because FLOW-6 renders diffs of it and
   FLOW-8 exports it into a real git repository, where a length-prefixed
   binary blob would be worthless.

   `provenance` is deliberately outside the object, for the reason
   ADR-0030 decision 8 gives about `PolicySnapshot`: it carries
   `confidence`, a float, and a float has no canonical form worth
   hashing. `tx_from` is outside too — the object is a *content* address,
   so the same content at the same scope is the same object however many
   times the bitemporal pair rewrites around it. `valid_to` is inside,
   so closing a record's window produces a different object, which is
   what makes a superseded record fall off its published set (MEM-6 and
   MEM-5 inherit the obligation to re-commit when they rewrite).

7. **`RecordKind` goes back to meaning what seed §4.2 says.** Pinned is
   authored/canonical — "cannot be shadowed or decayed", the Shruti/Smriti
   split — and nothing more. It stays tier 1 of seed §4.4's conflict order
   and stays the pinned-before-derived ordering inside a scope section. It
   no longer decides whether a record is trusted, and a pinned record that
   nobody published does not survive bank mode. That is a behaviour change
   and it is the point of the feature: **authorship is not review.**

8. **Channel is tier 0 of conflict resolution.** Seed §4.4 orders
   conflicts pinned-before-derived, then nearer scope, then newer
   valid-time. That list predates channels. When the identical content
   exists published at one scope and unpublished at another, the published
   copy wins regardless of distance — otherwise the block would render the
   unreviewed copy, drop the reviewed one, and watermark the result with
   the address of the version nobody approved. Tech plan §2.2's trust
   boundary is the more specific statement and it wins. Everything below
   tier 0 is seed §4.4 unchanged.

9. **Published material composes without regard to the task; derived
   still needs relevance.** ADR-0025 decision 5 said this of pinned
   material ("canonical content composes regardless of relevance input");
   the property was always about the *trusted* channel, and now it can
   attach to it. The candidate fetch follows: published members are
   fetched by id (the ids are already in hand from the tree) through the
   scope/sensitivity/valid-time predicate, and the capped
   per-`(scope, kind)` sweep supplies derived candidates. Without the
   split, a scope with 64+ derived records could crowd its own published
   set out of the fetch — and the promoted-extraction case, where a
   published record still has `kind = derived`, is the *normal* lifecycle
   per tech plan §2.3.

10. **Bank mode skips the derived query entirely.** `published-only` at a
    scope means composition reads that scope's published set and issues no
    candidate sweep for it. The switch is not a filter applied after the
    fact; it removes the read. Faster, and it makes "no derived material
    reached the block" structural rather than assertional.

11. **Watermarks become object addresses, and the block carries its
    channel commits.** ADR-0025 decision 7's `blake3(record_id ‖ tx_from ‖
    content)` is replaced in place by the decision-6 object address — a
    real VedaFlow address, equal to the stored object's when the record
    was committed, recomputable by an auditor from the record alone. The
    `ComposedBlock` additionally carries one `(scope, ref, commit)` triple
    per channel it read, and the inject audit event records them: that is
    tech plan §2.5's "inject responses cite commit hashes".

    They do **not** go in the rendered text. ADR-0025 decision 7 already
    spends ~10 tokens per record on the in-text watermark; ~25 more per
    scope for a hash the audit event carries anyway would be ~8% of the
    default budget bought for nothing. The text keeps the block hash and
    record ids; the response and the audit event carry the commits.

12. **Publishing is additive, same-scope, and takes two decisions.**
    `ChannelPublish` on a Scope resource, requiring `curator` (or steward
    / org-admin above it) in every pack — seed §5 already defines curator
    as the role that "can pin/approve". `POST /v1/channels/{scope}/publish`
    unions the named records into the scope's published set and
    fast-forwards the ref; `GET /v1/channels/{scope}` lists the scope's
    channels under a paired `ChannelRead`.

    The route additionally decides `MemoryRead` at the scope: **nobody
    publishes material they cannot read.** That second decision is what
    protects personal scopes, and it does the job a `resource.kind !=
    "user"` clause in the pack would only approximate. A curator at a team
    holds `ChannelPublish` on a teammate's personal leaf (their binding is
    on that leaf's chain) but no `MemoryRead` there, because the privacy
    floor (ADR-0015 decision 4) excludes personal scopes from the
    content-role grant — so the publish is refused. The same rule lets a
    user who holds a curator binding publish *their own* memories to
    *their own* channel, which is the one place the membership floor
    grants the read.

    That has a consequence worth stating plainly, because it decides what
    FLOW-3 is for. The pipeline lands every extracted record at its
    owner's personal scope (MEM-1, ADR-0020 decision 3), and a team
    curator cannot read a personal scope. **So the user→team climb the
    tech plan §2.3 lifecycle describes cannot be a curator reaching into
    someone's personal scope — it has to be a proposal the owner opens,
    where the curator reviews content the proposal shows them.** FLOW-2
    deliberately ships no way around that, and FLOW-3 is where the climb
    belongs.

    Additive because retraction is a rewind, and rewinds are FLOW-7's
    `force_update_ref` by name. Same-scope because climbing to a higher
    scope's channel needs that scope's approvers, not this one's. And a
    direct action because FLOW-3 does not replace this mechanism — it puts
    *required approvals* in front of it. That is the seam, recorded as a
    forward obligation rather than guessed at now.

13. **The pipeline's derived commit rides the existing write transaction,
    between the record inserts and the audit append.** One commit per
    `(group, scope)`, its tree naming that batch's records, parented on
    the scope's current derived head, compare-and-swapped with a bounded
    retry. Lock order is preserved — archive-lock, records, ref, chain
    head last (ADR-0019 decision 1) — so no new deadlock edge appears. A
    lost race re-reads the head, re-parents, and re-mints only the commit:
    the objects and the tree are content-addressed and parent-independent,
    so they survive the retry untouched.

    The pack recorded in `policy_snapshot` is the one `authorize_owner`
    already resolved for that owner. Vedaflow cannot ask the PDP anything
    (ADR-0030 decision 1); the caller passes what it has, which it has
    because it just decided `MemoryWrite` with it.

14. **One new audit action, and the pipeline does not get one.**
    `vedaflow.channel.published` records the governed act: actor, scope,
    asset kind, the records added, the prior and new commit, and the pack.
    The pipeline's derived commits ride the existing
    `memory.extracted` group event as per-scope commit hashes in its
    payload — ADR-0019 decision 4's aggregation doctrine says one event
    per group, and a second event per group asserting the same fact would
    be noise an auditor has to reconcile.

15. **The gateway takes the `synveda-vedaflow` dependency and describes
    its counters.** ADR-0030 decision 14 held the dependency back because
    "the gateway does not depend on this crate yet because nothing in it
    calls this crate. FLOW-2 adds the dependency for real work and
    describes the counters in the same diff." This is that diff. One new
    counter joins them: `synveda_composed_entries_total{channel}` — the
    production evidence that bank mode does what the AC says.

16. **Signing stays `Unsigned`.** FLOW-1 shipped the seam with an honest
    default and ADR-0030 put key *management* at TEN-4. Channels do not
    change that calculus, and wiring a key-shaped environment variable
    here would be key management arriving through the side door.

## Options considered

1. **Published/staged as sets, derived as a log, membership by content
   address (chosen)** — one indexed read per inject, a write cost
   independent of corpus size, and a review boundary that survives an
   edit. Con: the two channels have different shapes, which has to be
   explained (this ADR) and is a rule a future asset type must be told.
2. **Full membership trees on every channel, git-faithful** — one model,
   one explanation, and the derived ref becomes a genuine snapshot chain
   that FLOW-8 could export directly. Rejected on write cost: O(corpus)
   tree-entry rows per extraction batch, which is the pipeline's steady
   state, not its peak.
3. **Sharded subtrees on every channel** (git's own answer: `ab/cd/<id>`)
   — bounds the write at ~500 rows per commit regardless of corpus size,
   and keeps one model. Rejected because it moves the cost to the read
   that matters: enumerating a channel becomes a recursive descent, on
   the p99-bounded inject path, to answer a question decision 4 shows we
   never have to ask.
4. **A `channel` column on `records`, with commits as a side log** — the
   fastest possible read and no tree work at all. Rejected because it
   makes the column the truth and the commit a copy: FLOW-7's rollback
   would have to re-derive the column from history that no longer
   determines it, and the ref would be documentation rather than
   governance. This is the shape FLOW-1 was built to avoid.
5. **Membership by record id, ignoring content drift** — simpler, one
   fewer comparison. Rejected: any principal with `memory.write` could
   then rewrite the text under a published id and have it compose as
   reviewed material. Publication has to bind what was reviewed.
6. **Keep `RecordKind` as the channel and let refs shadow it** — no
   behaviour change, no test churn, and the AC keeps passing. Rejected;
   this is the reversal trigger ADR-0025 recorded against itself, and
   leaving it would mean shipping a bank mode that trusts unreviewed
   authored content.
7. **Compose the published object's bytes rather than the record row** —
   true git semantics: the channel holds the content. Rejected because
   record text is what retrieval ranked, what the sidecar indexed, and
   what the vector was computed over; forking the read path so the block
   text comes from a different table than the relevance decision is a
   correctness hazard well beyond the drift case decision 5 handles.
8. **Defer the publish surface to FLOW-3** — the smallest FLOW-2, and
   arguably the tidier feature boundary. Rejected: "inject reads
   published" is half the feature text and would be demonstrable only
   against an empty channel, which proves the filter and not the
   channel.
9. **Per-scope channel commit hashes in the rendered watermark line** —
   the block would self-describe completely. Rejected on budget (~25
   tokens per scope, ~8% of the default 1,500) for a fact the response
   and the audit event already carry.

## Consequences

- **Positive**: the trust boundary is real — nothing composes as
  reviewed material that a `curator` did not publish, and publication
  binds bytes, so editing published content demotes it rather than
  laundering the edit. Bank mode removes a query instead of filtering its
  results. The pipeline's records and their derived commit land in one
  transaction, so ADR-0003's central claim ("a commit, the records it
  describes, and the audit event attesting to it either all land or none
  do") is now true of the product's only write path, not just of the
  substrate's tests. Watermarks are VedaFlow addresses an auditor can
  recompute from an API response. Three transitional stand-ins
  (ADR-0025 decisions 2 and 7, ADR-0022's records-insert obligation) are
  discharged.
- **Negative / accepted trade-offs**: pinned records that were never
  published stop composing under `published-only` — a deliberate
  behaviour change, and the reason several CTX-2/CTX-3 tests change in
  this diff. The two channel shapes are a rule, not a symmetry. Published
  sets are capped at 10,000 members per scope with sharding deferred.
  Retraction has no surface until FLOW-7. The derived ref is a log, so
  FLOW-8's export story covers `published` cleanly and would need the
  snapshot question reopened if anyone ever wants `derived` mirrored.
  Records written before this feature have no derived commit and are
  simply unpublished material; no backfill is attempted.
- **Reversal triggers**: (a) a published set approaching
  `MAX_CHANNEL_MEMBERS` at a real scope → subtree sharding for the set
  channels, read path unchanged in shape; (b) ref contention on
  `memory/derived` showing up in FLOW-4's soak test → the generation
  number ADR-0030 already recorded, before any cache; (c) FLOW-3 landing
  → publication moves behind proposals and the direct route becomes the
  proposal's effect; (d) if drift demotion (decision 5) proves noisy once
  MEM-5 rewrites records routinely, the fix is MEM-5 re-committing on
  supersession, not relaxing the comparison.

## Compliance notes

The PDP stays unbypassable. Publishing is a Cedar `ChannelPublish`
decision at the target scope before any ref moves, and the pipeline's
derived commit rides inside the `MemoryWrite` decision that already
gated the record insert — no new path to storage exists that does not
pass a decision first. Composition gained no grant: channels only narrow
which of the already-`MemoryRead`-allowed scopes' material is treated as
trusted, exactly as ADR-0025 decision 2 argued of the config it replaces.

Every channel read and write runs inside the caller's
`rls::begin_tenant_tx` transaction against the six forced-RLS tables
migration 0018 created; no new tables, grants, or policies are
introduced, so ADR-0009's completeness guard has nothing new to check.

`vedaflow.channel.published` is the first VedaFlow audit action, and it
carries what an auditor needs to reconstruct the act without reading
database rows: who, at which scope, which asset kind, which records,
which commit the channel moved from and to, and the pack that governed
it. Together with `policy_snapshot_hash` on the commit itself (ADR-0030
decision 8) this closes ADR-0003's compliance claim for the published
channel: which pack, at which version, with which configuration,
governed an asset at the moment it became trusted.
