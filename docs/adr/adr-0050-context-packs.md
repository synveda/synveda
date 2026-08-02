# ADR-0050: context packs — the pack is an authored asset whose published chunks are pinned records, chunking and embedding happen at authoring so review stays network-free, and relevance ranks what the index tier then refuses to let vanish

- **Status**: Proposed
- **Date**: 2026-08-02
- **Feature(s)**: PRMT-2
- **Deciders**: sujitn

## Context

PRMT-2's text is "curated doc bundles (conventions, glossaries) pinned to
scopes; chunked+embedded on publish; composed by CTX-2 as pinned material",
and its acceptance criterion is two clauses: "pack update re-embeds
atomically; inject reflects new pack next session."

PRMT-1 landed the first authored asset and found that FLOW-1 through FLOW-7
had already built most of it. That is true again here for the *governance*
half and emphatically not true for the *read* half. A prompt is fetched by
name and composes into nothing; a context pack is the first authored asset
whose content has to enter the corpus the read path ranks — which is why
ADR-0049 option 4's third reason for refusing "prompts as memory records"
(**"a prompt is fetched by name where a record is ranked by relevance"**)
inverts for packs and stops being an argument against reuse.

What is parked on this feature by name:

- **Seed §4.3** lists context packs as the second of four managed asset
  types: "curated, versioned bundles (docs, conventions, glossaries)
  **pinned to scopes**". **§4.4** makes pinned a ranking law: pinned
  priority within each level, and "(1) pinned beats derived" as the first
  conflict rule.
- **ADR-0025's context** says "PRMT-2's context packs later arrive as
  pinned material", and its **option 5** — relevance filtering applied to
  pinned material — was **rejected**, on the grounds that "canonical
  content must not silently vanish because a task didn't mention it;
  pinned is the trust anchor PRMT-2 builds on". `compose` implements that
  today: relevance gates derived material only, and published content
  composes whether or not the task mentions it.
- **ADR-0031 decision 1** fixed the channel vocabulary as
  `{asset-kind}/{channel}` with `context-pack` reserved, and
  `ChannelMember::name` is "a record id for memories, **a path for the
  authored asset types**".
- **ADR-0036 decision 3** refuses the rewind and pin routes for asset
  kinds with no read action "by name rather than governed by memory's read
  action until PRMT-1 and SKIL-1 bring theirs". PRMT-1 shrank that refusal
  to `skill` and `context-pack`. This feature discharges the second of the
  three, leaving `skill`.
- **ADR-0041 decision 4** built the index tier's per-`AssetKind` rendering
  seam explicitly for this: "When PRMT-1, PRMT-2 and SKIL-1 land, their
  assets carry an authored name and description, and the index slot renders
  those instead". Its **option 9** — demote every candidate over a size
  threshold — was rejected *for now* as a guess, with "the right answer
  once a context pack's body is thousands of tokens" written into the
  rejection, and its measurement records that the tier's ~90-token cost
  pays back "one to two orders of magnitude for the context packs and
  skills PRMT-2 and SKIL-1 will bring". The lever, `index_entry_chars`, is
  already a pack field.
- **ADR-0035's** per-asset-kind renderer seam exists as of PRMT-1; packs
  are the second kind through it. **ADR-0032's** curator globs got their
  first real paths from prompts and get their first *bundles* here.
- **ADR-0049** is the template for everything above the content: draft
  state, name-as-id, two PDP actions, publication as an ordinary FLOW-3
  proposal, `restricted` unrepresentable, rewind and pin.

And one thing nothing parked here, found by reading the matrix this feature
is about to make resolvable for the first time. **`AssetKind::ContextPack`
is priced at one curator, at every scope kind, in all three packs** —
`approvals.rs:63`, `:93`, `:115`, its only three appearances in the
workspace outside the enum. Under `regulated-strict` a *memory* published at
a department, division or org takes a curator **and** a steward, two
distinct people; a context pack published at the org takes one curator. So
the cheapest thing to publish into every session in the company is currently
the largest one. Tech plan §2.4's table has rows for memory, prompts,
skills, `restricted`, policy lapses and the SMB collapse, and **no row for
context packs** — FLOW-3 filled a cell the specification left empty, at the
one value that turns out to invert the ordering it was modelled on. Nothing
has ever exercised it, because nothing could open a `context-pack` proposal.

Four forces bound the design, and only the first is inherited:

1. **The governance path must stay network-free.** Nothing in a proposal's
   approval or effect has ever made a network call. Embedding is a call to
   TEI (ADR-0023 decision 6), so "chunked+embedded **on publish**" read
   literally puts a dependency failure on the review path: a curator's
   approval would fail because a model server is down.
2. **Published content is not relevance-gated, and a pack is large.**
   ADR-0025 option 5's concern was a handful of hand-pinned records. A
   20,000-token glossary composing in full against seed §4.4's 1,500-token
   default budget is not "the trust anchor"; it is the block. Whatever this
   feature does, `compose` cannot keep treating "published" as "always
   composes in full" once a single asset exceeds the budget by an order of
   magnitude.
3. **`records` already carries the behaviour a pack chunk needs, and it is
   behaviour nobody should write twice.** ADR-0040 decision 8 exempts
   pinned material from expiry, destruction and staleness **by law**;
   ADR-0039 closes supersession windows only on `derived` records, so
   canonical content cannot be superseded by a pipeline guess; CTX-1 ranks
   `records` on two legs; CTX-4 tiers them; CTX-5 recalls them; ADR-0009
   isolates them. A second content table re-earns all of it or silently
   loses it.
4. **A pack is bulk external text, which prompts were not.** PRMT-1 does
   not scan prompt bodies for secrets: a prompt is short and hand-written.
   The first thing a customer will do with a context pack is upload an
   existing runbook, and runbooks carry connection strings. MEM-2's scanner
   (`ingest::redaction::scan`) already exists and already has per-pack
   modes.

## Decision

**A context pack is an authored asset whose published chunks are pinned
records.** The pack — its identity, its documents, its versions, its review
— is governed exactly as a prompt is, through the channel and proposal
machinery that already exists. Its *content* enters the corpus the read path
already ranks, as `records` rows with `kind = Pinned`, so retrieval,
tiering, recall, retention exemption and supersession exemption are
inherited rather than rebuilt. Chunking, scanning and embedding happen at
**authoring**; publication moves a ref.

Decisions, specifically:

1. **The pack is a row, its documents are objects, its publication is an
   ordinary FLOW-3 proposal whose asset is `context-pack`.** ADR-0049
   decisions 1, 2, 6 and 7 apply unchanged and for the same reasons: the
   draft is a row because a set channel cannot express withdrawal, the name
   is the id because it is what a scope's override is expressed in, and the
   direct publish route takes packs too, refusing on the matrix's own
   arithmetic rather than on a second rule. `AssetKind::ContextPack` has
   been in the vocabulary since FLOW-1 and in the approval matrix since
   FLOW-3; this is the first feature that can resolve one of its cells.

2. **A pack's published content composes as `records` rows with
   `kind = Pinned`.** This is the decision the rest hangs on, and it is
   taken because of force 3: it is the only shape in which seed §4.4's
   "pinned beats derived", ADR-0040's pinned exemption, ADR-0039's
   derived-only supersession, CTX-1's two retrieval legs, CTX-4's tiering
   and CTX-5's recall all apply to pack content without a second
   implementation of each. Seed §4.3's own words — packs are "pinned to
   scopes" — describe the storage rather than only the intent.

3. **Published-ness comes from `context-pack/published`, never from the
   memory channel, and the tree names documents.** One entry per document,
   its name the path `pack-name/document-name` — ADR-0031's reserved "a
   path for the authored asset types", and ADR-0032's curator globs get
   their first bundle to glob over — and its object the document's content
   address. A chunk row carries the address of the document it was cut
   from, so **a chunk composes as published when the scope's pack channel
   names its document at exactly the address the chunk was cut from**:
   ADR-0031 decision 5's rule reaching chunks through their document, so
   editing a published document demotes every chunk of it rather than
   laundering the edit through chunks the tree still appears to name.
   Composition reads a second ref per planned scope; `ChannelWatermark` is
   already a `Vec` on the block, so citing two refs per scope is the shape
   the watermark was built for, and `synveda channel show
   memory/published` keeps meaning what it says.

4. **Chunk, scan, embed, commit — at authoring.** ADR-0023 decision 5's
   order applied to authored bulk: the scanner runs before the embedder, so
   a secret that never reaches `content` never reaches vector space; the
   chunker is deterministic, so identical bytes chunk identically and
   re-authoring an unchanged document re-embeds nothing. The AC's phrase is
   "chunked+embedded **on publish**", and this is a deliberate departure
   with force 1 as its reason: publication would otherwise fail when TEI is
   down, which makes a governance act hostage to a model server.

5. **"Re-embeds atomically" is the existing one-statement API inside one
   transaction, and the ref move is a separate, cheaper atom.** Every chunk
   row lands with its embedding or none do (ADR-0023 decision 2), with the
   deferred constraint trigger (decision 4) as the backstop against raw
   SQL. A publication cannot move the ref to a commit whose chunks are not
   all embedded, because the commit names addresses that only exist once
   the authoring transaction committed. There is no window in which half a
   pack is live.

6. **A version swap is a ref move, not a rewrite.** The previous version's
   chunk rows stay addressable; which chunks compose is decided by the
   commit the pack channel *serves*. So FLOW-7's rewind restores a previous
   pack version with no re-embedding at all and no half-swapped state, and
   ADR-0036's pin freezes a pack exactly as it freezes a memory channel.
   The cost is chunk rows that no live commit names — ADR-0030's open GC
   question, not worsened in kind, and the same one PRMT-1 accepted for
   draft objects.

7. **`ContextPackRead` and `ContextPackWrite`**, mirroring ADR-0049
   decision 4: both scope actions, `ContextPackRead` carrying
   `context.sensitivity` and no `lapsed` attribute, `ContextPackWrite` the
   authoring seam with the home-scope floor role-free. This is what
   discharges ADR-0036 decision 3 for packs.

8. **`ContextPackRead` is taken per scope inside the existing plan walk,
   and it is what admits pack chunks — `MemoryRead` never does.** ADR-0025
   decision 1's rule is one PDP *walk*, not one decision per scope; the
   walk already takes tiers and channel rules per scope. Adding this
   decision there keeps composition's single authorization path and buys
   the case the feature exists for: a scope may distribute conventions and
   glossaries to readers who hold no readable memory there at all. A
   memory is never admitted by `ContextPackRead`, and a chunk is never
   admitted by `MemoryRead`.

9. **Relevance ranks pack chunks, and the index tier is what keeps
   ADR-0025 option 5's concern true.** Option 5 was rejected because
   canonical content must not *silently vanish*; it was decided about
   hand-pinned records, where "compose it all" costs tens of tokens. A pack
   is orders of magnitude larger (force 2), so composing it all is not
   fidelity to that decision but a repudiation of the budget. The
   resolution keeps both halves: pack chunks are ranked like retrieved
   material, and what does not fit is **named in the index tier rather than
   dropped** — the pack, the document, the section, with a recall handle.
   Canonical content never silently vanishes, because nothing about it is
   silent; the budget survives, because a named chunk costs ~90 tokens
   against a body that ADR-0041's own measurement puts one to two orders of
   magnitude higher. This fires ADR-0041 option 9's recorded trigger, and
   the knob it fires through — `index_entry_chars` — is already a pack
   field.

10. **The index line renders the pack's authored name and the document's
    title, which is what ADR-0041 decision 4 reserved the seam for.** A
    memory record has no name, so its index line truncates a body; a pack
    chunk has `pack/document § heading`, which is a better description than
    any truncation and is the reason the seam is per-`AssetKind` rather
    than a memory special case.

11. **MEM-2's scanner runs at pack authoring, with the authoring scope's
    effective redaction config.** Force 4: this is the first surface where
    bulk external documents enter the product. The disposition ladder is
    MEM-2's own — `redaction::scan` and `ScanOutcome::disposition` — so a
    pack carrying a live credential is quarantined for review under the
    machinery that already reviews quarantined observe events, rather than
    under a second review queue.

12. **`restricted` packs are unrepresentable, exactly as `restricted`
    prompts are** (ADR-0049 decision 5): the only mechanism that mints the
    tier is a classification proposal over records, and this feature ships
    no classify effect for authored assets. Sensitivity is declared per
    document rather than per pack — a glossary of public terms and an
    internal runbook are plausibly the same bundle — and each chunk
    inherits its document's tier, which is what CTX-4 and ADR-0038's
    per-scope tier check then apply per entry.

13. **Two new audit actions, and no third.** `context_pack.authored` for a
    draft write and `context_pack.quarantined` where decision 11 fires.
    Publication is `vedaflow.channel.published` as it always is (ADR-0019
    decision 4), and a *served* pack chunk chains nothing of its own: it
    composes inside `context.injected` with its object address like every
    other entry. PRMT-1's `prompt.resolved` exists because a prompt fetch
    is its own route; a chunk arrives through a route that already chains
    an event.

14. **A pack climbs exactly as a prompt does** (ADR-0049 decision 16), and
    the deletion story is the same: no DELETE grant, retraction is FLOW-7's
    rewind, replacing a draft is an overwrite.

15. **`regulated-strict` re-prices the `context-pack` cell to match its own
    memory rule: one curator locally, a curator and a steward — two
    distinct people — at org, division and department.** The inversion in
    the Context section is not defensible in the direction it currently
    points, and decision 9 is what makes that concrete: pack content
    composes into every session at and below the publishing scope, which is
    a wider blast radius than the single memory record the same pack
    already prices at two people. The change is one `rule(...)` line gaining
    the `SHARED`/`LOCAL` split memory has had since FLOW-3, it lands as a
    pack version bump with the role×action golden re-recorded, and it is
    deliberately **not** applied to `standard` or `open-collaboration`,
    whose whole content is that the same publication is cheaper there.

    Left alone, and recorded rather than fixed: `regulated-strict` prices a
    prompt at a steward and a curator *at every scope kind* including a
    user's own leaf, where memory and — after this — packs ask for one
    curator. That asymmetry is at least in the safe direction, it is
    straight off tech plan §2.4's own row, and re-deciding a cell this
    feature does not touch would be PRMT-2 quietly re-opening PRMT-1.

## Options considered

1. **Chunks as pinned records governed by a `context-pack` channel
   (chosen)** — inherits retrieval, tiering, recall, the retention
   exemption and the supersession exemption; adds one channel read and one
   PDP decision per scope. Con: `records` holds rows that are not memories,
   so every memory-shaped sweep must be re-read with that in mind — which
   is the work this ADR's compliance notes bound.
2. **A `context_pack_chunks` table with its own embedding sidecar and its
   own retrieval lane** — perfect separation, and `records` stays exactly
   what seed §4.2 says it is. Rejected on force 3: it re-earns the dense
   leg, the sparse sidecar, the tier check, recall, RLS and the retention
   and supersession exemptions, and every one of those is a place to lose
   the behaviour silently rather than loudly.
3. **Chunks named on `memory/published`** — no second channel read, no
   second PDP decision, composition unchanged. Rejected: it makes the
   memory channel's contents a lie, it hands pack content to `MemoryRead`
   (destroying decision 8's case), and it would make a pack publication
   indistinguishable from a memory publication on the chain.
4. **Compose the pack as one pinned blob, unchunked** — the literal reading
   of "pinned to scopes", and the simplest thing that could work. Rejected
   by the AC itself ("chunked+embedded") and by arithmetic: one document
   exceeds the default budget.
5. **Chunk and embed at publish time**, the AC's literal reading —
   reviewers approve exactly what gets embedded, and there is no authored
   state carrying vectors nobody approved. Rejected on force 1: a curator's
   approval would fail when TEI is down. Recorded as the reversal trigger
   for decision 4 if the authoring-time vectors ever diverge from what a
   reviewer read — they cannot today, because both are a function of the
   same content address.
6. **Pack chunks compose unranked, as published material does today** —
   faithful to ADR-0025 option 5 as written. Rejected: at any realistic
   pack size the first-fit budget makes "which chunks an agent gets"
   a function of the comparator's tie-breaks rather than of the task, which
   is a worse failure than the one option 5 was protecting against, and it
   is not what option 5 was deciding about.
7. **A separate token budget lane for pack material** — packs cannot
   crowd out memory, and vice versa. Deferred rather than rejected: it is a
   real answer to a real risk, it is ADR-0041 option 4's shape, and it
   should be fired by EVAL-4 measuring displacement rather than by
   anticipation. Decision 9's ranking plus the index tier is the cheaper
   thing that might be enough.
8. **Packs as plain pinned memory records with no pack asset at all** —
   the "just write them as memories" reading. Rejected for ADR-0049 option
   4's two surviving reasons: the approval matrix prices `context-pack`
   differently from `memory` and would stop being able to, and the asset
   kind is inside the object address by ADR-0030 decision 4 precisely so
   that identical bytes governed differently are different objects.
9. **A model-driven chunker** (semantic splitting) — better chunk
   boundaries than any structural rule. Rejected outright for now: it puts
   a network call and a nondeterminism on an authoring path whose output is
   content-addressed, so the same bytes would produce different addresses
   on different days.
10. **Leaving the `context-pack` approval cell as FLOW-3 set it** (decision
    15) — a matrix is a product decision, and a read-path feature quietly
    re-pricing governance is exactly the kind of change that should not
    ride along. Rejected on the narrow ground that this is the feature that
    makes the cell resolvable at all: shipping the first `context-pack`
    publication under a rule that is weaker than the one governing a single
    memory record at the same scope would set the precedent rather than
    inherit it. Recorded as a decision of its own, with its own line, so it
    can be refused on its own.
11. **Pricing the pack cell by sensitivity rather than by scope kind** —
    an `internal` glossary is not a `confidential` runbook. Deferred: the
    matrix already supports `min_sensitivity`, and decision 12 puts
    sensitivity on the *document* rather than the pack, so the cell has no
    single value to test until a pack-level tier exists. The scope-kind
    split is the part that is wrong today.

## Consequences

- **Positive**: the approval matrix's `context-pack` cells resolve for the
  first time since FLOW-3 wrote them — and one of them is corrected in the
  same breath, before any tenant has published under it (decision 15).
  Rewind, pin, climb, the review CLI,
  the audit chain, the retention exemption and the supersession exemption
  all extend to pack content with no new concepts. The index tier gets the
  assets ADR-0041 decision 4 built its rendering seam for, and its
  `index_entry_chars` knob gets its first real use. `inject` reflects a new
  pack on the very next call, not merely the next session, because the pack
  channel is read live — the AC's second clause is satisfied more strongly
  than it asks.
- **Negative / accepted trade-offs**: `records` now holds rows that are not
  memories, and every memory-shaped sweep is a place that must say so
  deliberately. Chunk rows accumulate per published version (ADR-0030's GC
  question). A pack cannot be `restricted`. A published pack's chunks are
  ranked rather than composed whole, which is a real departure from
  ADR-0025 option 5 and is written down as one rather than folded in.
  Authoring a large pack is now the slowest write in the product — it
  scans, chunks and embeds a bundle in one request — and needs a bound
  before it is a route anyone can call twice.
- **Reversal triggers**: (a) EVAL-4 showing pack material displacing memory
  that mattered → option 7's separate budget lane, behind the pack config
  that already carries the budget; (b) authoring latency on a large bundle
  exceeding what a request can hold → the embed stage moves to a PGMQ
  worker with the draft unpublishable until it completes, which is MEM-3's
  shape and not a change to any decision here; (c) a reviewer needing to
  see chunk boundaries rather than documents → the diff renderer gains a
  chunk view, which is ADR-0035's seam doing its job; (d) anyone needing a
  `restricted` pack → the same classify-effect ADR that ADR-0049 reversal
  trigger (b) names.

## Compliance notes

- **PDP**: no path reaches pack content without `ContextPackRead` at the
  scope whose channel admitted it, decided at the tier the document
  carries, at request time, under the live pack — in the same walk that
  already decides `MemoryRead`, never a second authorization path
  (ADR-0025 decision 1). Authoring takes `ContextPackWrite`; publication
  takes `ChannelPublish` plus the approval matrix plus `ContextPackRead`,
  the three-part rule memory publication has obeyed since ADR-0031
  decision 12.
- **Tenancy**: the pack table and the pack-chunk mapping arrive with forced
  RLS, tenant-isolation policies and least-privilege grants in their own
  migration (0030), and join the adversarial suite and its completeness
  guard (ADR-0009). Chunk rows are `records` and are already covered.
- **Audit**: authoring chains in the caller's own transaction; publication
  chains the event it always did; a served chunk is watermarked inside
  `context.injected` with its object address, so a block naming pack
  content is exactly as recomputable as one naming memory. No payload
  carries document text — the discipline every plane has followed since
  AUD-1.
- **Redaction**: decision 11 puts authored bulk through the same scanner
  and the same per-pack modes as the observe path, so the guarantee MEM-2
  makes about ingested content now covers the one surface that would
  otherwise have been the easy way around it.
