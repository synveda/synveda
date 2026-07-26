# ADR-0038: ABAC conditions — a closed vocabulary is decidable per scope, `restricted` is a base-layer forbid whose one carve-out is a grant that cleared the compliance floor, and three of the five conditions are refused

- **Status**: Accepted
- **Date**: 2026-07-26
- **Feature(s)**: AUTHZ-5
- **Deciders**: sujitn

## Context

AUTHZ-5's text is "sensitivity, residency, channel (published/derived),
time-of-day, purpose-of-use as Cedar context", and its acceptance
criterion is "`restricted` records never injected without
compliance-granted permission, proven by leak-test suite". Seed §4.2
makes `sensitivity` a field of every record and says it "drives policy";
seed §6 makes `regulated-strict` mean "all writes classified"; and tech
plan §2.4 puts "anything `restricted` sensitivity" in the approval matrix
as "+ `compliance` role, dual approval".

Four files in the workspace name this feature in their own source, which
is the usual sign that the design was half-decided by the features that
deferred to it. `AuthzContext`'s doc comment says "AUTHZ-5 adds ABAC
attributes (sensitivity, residency, channel, ...) here". The Cedar
schema's header says "AUTHZ-5 adds ABAC context".
`regulated-strict.cedar` excludes personal scopes from content-role reads
as "the privacy floor until AUTHZ-5 classification".
`open-collaboration.cedar`'s header says the seed's own "org-wide read for
**non-restricted** content" is "AUTHZ-5's classification context; until it
lands, openness stops at other people's personal scopes".

Forces at play:

- **The product already writes records it can never read back.** The
  extraction prompt offers the model `internal`, `confidential`, or
  `restricted` (`extraction/prompt.rs`); the pipeline floors the
  proposal at `internal` and stores whatever else it is given
  (`worker.rs`, ADR-0022 decision 7). The read path then clamps:
  `allowed_sensitivities` caps the ceiling at `confidential`
  unconditionally (ADR-0024 decision 2), and every product path —
  `inject`, `ComposeRequest::new` — asks for `internal`. So a record
  classified `confidential` by an extractor is invisible to its own
  author, forever, with no surface anywhere that says so, and a
  `restricted` one is invisible to everybody. Today's AC passes
  trivially, by a clamp, which is the least interesting way for a
  security property to hold.
- **The `MemoryRead` seam decides once per scope and has no record in
  hand.** This is the constraint ADR-0037 decision 6 ran into and
  refused to paper over: seed §6's lapse example narrows to "`procedure`
  records", and a qualifier the seam cannot enforce is "a widening
  wearing a narrowing's name". The same sentence applies to sensitivity,
  and it is the thing this ADR has to answer rather than restate.
- **`restricted` already means something precise on the write side, and
  nothing at all on the read side.** The invariant approval floor
  (ADR-0032 decision 4) requires the `compliance` role and two distinct
  approvers for anything `restricted`, under every pack, unauthorable
  away. So the product has a non-negotiable rule about who may *publish*
  restricted material and no rule whatever about who may *read* it. The
  AC is asking for the mirror.
- **Nothing in the product deliberately makes a record `restricted`.**
  There is no records API and no classification surface; records are
  written by the pipeline and edited, in FLOW-6's own demo, by raw SQL.
  A tier whose only author is an uncalibrated LLM proposal cannot carry
  the meaning "compliance signed off on this".
- **`inject` is still the hot path** (p99 150ms, seed §3), and a
  per-record authorization decision is exactly the shape ADR-0024
  decision 2 refused when it made the retrieval filter mandatory and
  fail-empty: candidates are filtered *before* the index is touched,
  never fetched and then dropped.
- **Two of the five named conditions already have a home.** Channel
  (published/derived) is decided per scope by the pack's
  `CompositionConfig` (ADR-0025 decision 2) and forced to published-only
  under a lapse (ADR-0037 decision 11). Residency is a routing concern by
  seed §2.7, and OPS-3 owns it in Phase 3. Adding either to Cedar creates
  a second place where one rule is decided.

## Decision

**Sensitivity becomes a real policy attribute, decided per (scope, tier)
because the tier vocabulary is closed and ordered** — the composition
walk asks `MemoryRead` up to four times per scope and produces a
**per-scope allowed-tier set**, which is the predicate the read path
already takes. **`restricted` is forbidden in the base layer with exactly
one carve-out: a standing lapse that declared that tier** — and declaring
it is what makes the invariant floor resolve, so the only path to a
restricted read is a grant a compliance approver signed. Channel,
residency, time-of-day and purpose-of-use are **refused or deferred, each
by name and for its own reason**.

Decisions, specifically:

1. **A closed vocabulary is decidable without the record.** There are
   exactly four tiers and there will be four tomorrow, so "may this
   principal read `confidential` material at this scope" is answerable
   with nothing in hand but the scope and the tier. The plan walk asks
   that question per tier per scope and keeps the answers as a set.

   This is the whole design, and it is worth stating as a rule rather
   than a trick, because it draws the line ADR-0037 decision 6 was
   reaching for: **an attribute whose domain is small and closed can join
   the decision before the fetch; an attribute whose domain is per-record
   and open cannot, and is refused rather than stored.** Sensitivity is
   the first kind. Record content, owner, age, and confidence are the
   second.

   Record *class* (`fact`, `procedure`, …) is closed too, and is
   deliberately still refused — decision 17.

2. **`context.sensitivity` is a required `String` on `MemoryRead`, and
   packs enumerate the tiers they permit.** Required, not optional, for
   the reason `grant` and `lapsed` are (ADR-0015 decision 5, ADR-0037):
   a policy referencing a missing attribute errors, Cedar drops it, and a
   permit silently stops existing.

   Packs write `["public", "internal"].contains(context.sensitivity)`
   rather than an inequality over an integer encoding. With four values
   the enumeration *is* the documentation, it matches the
   `context.roles` idiom already in every pack, and there is no encoding
   for a stored pack to get subtly wrong — a pack that permits a tier
   says its name.

3. **The plan's output becomes a per-scope tier set, and hydration is
   where it is enforced.** `CompositionPlan` already carries a
   `ComposeScope` per allowed scope; that struct gains the tier set, and
   `SearchFilter`'s single `max_sensitivity` ceiling — a global clamp
   applied to every scope alike — becomes per-scope.

   Pushdown stays an optimization and verification stays the enforcement,
   which is ADR-0024's existing shape rather than a new one: fused
   candidates are already re-verified against current Postgres truth at
   hydration, and that check simply grows from "scope is allowed and
   sensitivity is under the ceiling" to "this (scope, tier) pair is in
   the plan". A stale sidecar can therefore miss a record; it cannot
   surface one.

4. **Three tiers, three mechanisms, in increasing order of
   deliberateness.** `public`/`internal` follow membership — the
   zero-config floor, exactly today's behaviour. `confidential` takes an
   explicit content-role binding (the "explicitly granted scopes" of the
   type's own doc comment, and ADR-0015's "explicit grant"), or the
   principal's own home. `restricted` takes a reviewed grant and nothing
   else.

   | Pack | public/internal | confidential | restricted |
   |---|---|---|---|
   | `regulated-strict` | own chain | own home; content-role subtree | lapse only |
   | `standard` | own chain; department subtree | own home; content-role subtree | lapse only |
   | `open-collaboration` | org-wide, personal scopes excluded | org-wide, personal scopes excluded | lapse only |

   `open-collaboration`'s row is seed §6's sentence — "org-wide read for
   non-restricted content" — which has been a comment deferring to this
   feature since AUTHZ-2. It is now the pack, and the deferral closes.

   This is a **widening** relative to today for `confidential`, and it is
   the feature: material an extractor classified confidential stops being
   invisible to everyone and becomes visible to readers who hold an
   explicit grant. Nothing changes at `internal`, which is where every
   existing fixture, baseline, and demo sits.

5. **`restricted` is a base-layer forbid whose one carve-out is
   `context.lapsed`.** `base.cedar` gains

   ```
   forbid (principal, action == Synveda::Action::"MemoryRead", resource)
   when { context.sensitivity == "restricted" }
   unless { context.lapsed };
   ```

   in front of ADR-0037's permit. It is in the base layer for the same
   reason that permit is — a pack that could omit it would make
   `restricted` mean different things in different tenants (ADR-0014
   decision 6) — and this direction is the easier argument: the base
   layer's job is invariants, and a forbid is what it has always held.
   ADR-0037 weakened the file's own sentence from "things no pack can
   escape" to "things no pack can change"; this decision does not weaken
   it further.

   The mirror is the point. The invariant *approval* floor already says
   nothing reaches a published channel at `restricted` without
   `compliance` and two distinct approvers. This says nothing reaches a
   reader at `restricted` without a grant, and decision 6 makes that
   grant resolve against the same floor. **One tier, one meaning, both
   directions, and the same role signs both.**

6. **A lapse declares a sensitivity ceiling, and the matrix resolves at
   the tier it declares.** `LapseTerms` gains `max_sensitivity`
   (defaulting to `internal`, which is what every existing grant means);
   `Lapse` and `policy_lapses` carry it; `authz::gather`'s filter and
   the PDP's `context.lapsed` become tier-aware — true only when a
   standing grant covers this (principal, action, scope) **at or above
   the tier being asked about**.

   ADR-0037 decision 14 wrote this feature's half of the contract in
   advance: "when AUTHZ-5 lets a lapse declare a higher ceiling, the
   matrix resolves at *that* ceiling and the floor engages by itself,
   with no lapse-specific rule anywhere". That is what happens. A lapse
   declaring `restricted` resolves the `policy` cell at `restricted`,
   the invariant floor merges in `compliance` × 1 and two distinct
   approvers, and the AC's "compliance-granted permission" is not a
   feature anybody wrote — it is the floor, reached by a grant that
   declared what it was disclosing.

   ADR-0037 decision 6's refusal ("a lapse may not narrow to a record
   type or a sensitivity") lifts for sensitivity and stays for record
   type, because the difference between them is now enforceable rather
   than rhetorical: the tier reaches the decision, and the class does
   not.

7. **There is no carve-out for the author at their own personal scope,
   and that is a sharp edge stated rather than filed off.** A
   `restricted` record at your own home is invisible to you without a
   grant. Under decision 8 the only way it got that tier is a proposal
   two people approved, one of them compliance, so what the reader loses
   is access a compliance decision deliberately removed. An owner
   carve-out would also make the AC false as written — "never injected
   without compliance-granted permission" has no "except to" clause, and
   the leak suite is generated against the sentence.

8. **The extractor's proposal is bounded above at `confidential`.**
   ADR-0022 floors it at `internal` because auto-derived content is never
   `public`; this bounds the same field from the other side, for the
   sharper reason: `restricted` is defined by the invariant floor as the
   tier that carries a compliance signature, and an uncalibrated,
   self-reported LLM judgement cannot manufacture one. A model that says
   "restricted" gets `confidential`, which is a real tier with real
   consequences and no forged provenance.

   The fail-safe reading — an LLM erring upward is the *safe* direction —
   is exactly the corrosion to avoid. If `restricted` can mean "a model
   thought so", then reading it requires a two-steward, compliance-signed
   grant, and the first three times that happens for a hallucinated tier,
   the tenant learns that the ceremony is noise.

9. **Reclassification is a third `ProposalEffect`, and it is the only
   path to `restricted`.** `ProposalEffect::Classify` joins `Published`
   and `Lapse` — AUTHZ-4's precedent, where migration 0022 widened the
   column to name the effect rather than a channel — with `POST
   /v1/proposals/{id}/classify` running it and `Action::MemoryClassify`
   deciding who may.

   Almost nothing has to be built for this, which is the usual sign it is
   the right shape. Sensitivity is already inside the memory object's
   content address (`channels.rs` asserts exactly that: a reclassified
   record has a different address), so approvals bind the tier the way
   they bind bytes, structurally, with no recheck (ADR-0032 decision 6).
   FLOW-6's renderer already refuses to show a sensitivity change as no
   change. The review surface, the CLI, the audit shape, and the matrix
   are FLOW-3's and FLOW-6's, unchanged.

   **The effect resolves the matrix at `max(current, proposed)`**, and
   this is the decision inside the decision. A proposal's stored
   sensitivity is "the maximum over its members", so a *declassification*
   — restricted → internal — would resolve at `internal` and need one
   curator, which is the one direction that actually removes a control.
   Taking the maximum of both sides means the dangerous direction costs
   what the tier costs, and `restricted` cannot be quietly taken off a
   record by the person who least wants it there.

   A new action rather than reusing `MemoryWrite`, on ADR-0036
   decision 3's separability rule: the write floor grants every principal
   `MemoryWrite` at its own home, and a pack must be able to say "you may
   write here" without saying "you may classify here". Packs grant it
   pack-uniformly to curator/steward/org-admin, plus the owner at their
   own home, and the matrix does the heavy lifting at the top tier.
   `records::update` requires an embedding (MEM-4/ADR-0023); a
   reclassification carries the record's existing vector forward, because
   the content it was computed over has not changed.

10. **Reviewers still see what they review, and that is not a leak.**
    ADR-0035 decision 8 deliberately shows a reviewer both sides of a
    change regardless of `MemoryRead` — a `compliance` reviewer composes
    nothing at that scope from `inject` and is shown the content anyway,
    because approving what you cannot see is not review. Nothing here
    changes it, and the leak suite asserts the distinction rather than
    tripping over it: the surface that discloses restricted content to
    the person the floor requires is the review surface, once, audited,
    under a proposal — never `inject`, never `recall`.

11. **The block labels every tier above `internal`.** A composed entry at
    `confidential` or `restricted` is marked the way a lapsed section is
    marked (ADR-0037 decision 12) and a pinned commit is (ADR-0036
    decision 10). The harness is a guest (seed §2.6) and cannot know what
    it is holding unless the block says; EVAL-5's own framing —
    "content is data, wrapped and labelled" — is the same rule arriving
    from the security side.

12. **The caller may narrow the ceiling and never widen it.** `inject`'s
    body gains `max_sensitivity`, treated exactly as `budget_tokens` is
    (ADR-0026): the request narrows what policy allows, never the
    reverse. An agent that knows it is about to paste into a PR can ask
    for `internal` and get a block it can be careless with.

13. **The audit event carries what was permitted, not just what was
    read.** `context.injected`'s aggregated per-scope decisions gain the
    tier set (ADR-0019 decision 4's shape, one event, no row per
    candidate). This is what makes AUD-2's "who could see X on date D"
    answerable at tier granularity, which is the question a regulator
    actually asks about a `restricted` record.

    The cost is named rather than discovered: `synveda_authz_decisions_total`
    grows by up to 4× per inject, because the walk asks per tier. The
    metric's meaning does not change — it has always counted PDP calls —
    but any dashboard reading it as "injects" was already wrong and will
    now be wrong by a bigger factor.

14. **Channel stays configuration; it does not become Cedar context.**
    The pack's `CompositionConfig.channels` is resolved from the same
    effective-pack read, at the same per-scope seam, in the same walk —
    it is already an attribute condition in every sense except which
    engine evaluates it. Adding `context.channel` would let a pack permit
    in Cedar what its own config withholds, with no defined resolution
    order between them. One rule, one place.

    What Cedar could express and config cannot is a *principal-dependent*
    channel rule ("curators see derived, everyone else published-only").
    Nobody has asked for one; reversal trigger (b) records the shape it
    takes when they do, and it is small, because the plan already carries
    `include_derived` per scope and would carry a set instead.

15. **Residency is refused here and belongs to OPS-3.** There is one
    region. An attribute that no data plane routes on is decoration, and
    ADR-0037 decision 6's rule — refuse what the seam cannot enforce —
    applies with full force to a residency tag that nothing enforces.

    One finding worth recording so OPS-3 does not have to rediscover it:
    seed §6's residency rule is "cross-region `inject` returns only
    metadata-safe summaries unless policy allows replication", which is a
    **degradation, not a denial** — CTX-3's `X-Synveda-Degraded` ladder,
    not a permit. Residency therefore lands as a scope attribute plus a
    composition rule, and the Cedar half may turn out to be empty.

16. **Time-of-day is refused, because the product has one clock.**
    AUTHZ-4 put time in a row read at decision time and made a point of
    it: expiry is a property of the decision, not of a job, and the
    `expires_at` predicate exists in exactly one query. A `context.now`
    would put a second clock in the policy language, and "when may this
    be read" would then be answered in two places that can disagree.

    There is a second objection, and it is the one that would need
    answering first: a time-based denial produces a *smaller block*, not
    an error, and the reader cannot tell it from "there was no memory".
    Under decision 11's own rule — a response that deliberately differs
    has to say so — a time rule needs a marker for material it withheld,
    which means the block would have to describe what is not in it. That
    is a real design question and it is not this feature's.

17. **Purpose-of-use is refused as a widening, permanently.** A
    caller-asserted purpose that unlocks material is the reader
    authorising their own read. This product's answer to that request is
    older than this ADR: a disclosure is always initiated on the
    disclosing side (ADR-0037 decision 3), and the thing purpose-of-use
    is reaching for already exists in a better form — a lapse carries a
    mandatory reason that a *reviewer at the target* consented to, rather
    than a string the reader typed.

    Purpose as a **narrowing** — the caller declaring "public demo", the
    block coming back smaller and the declaration landing on the audit
    event — is unobjectionable and unasked-for; reversal trigger (e).

    Record class is refused on the same axis for a different reason.
    Class is closed and would be decidable under decision 1, so this is a
    product judgement, not a mechanical limit: class is descriptive
    everywhere else in the product, an extractor assigns it with
    uncalibrated confidence (EVAL-2 has not measured it yet), and a
    disclosure narrowed by a label a model chose is a control that only
    looks like one. Sensitivity earns its place because the seed makes it
    policy-driving, the matrix already keys on it, and decision 8 puts a
    human behind its top tier.

18. **One migration, no new table, no new vocabulary.** Migration 0023
    adds `max_sensitivity` to `policy_lapses`, widens the proposal
    effect's CHECK to admit `classify`, and nothing else.
    `Sensitivity` gains no variant — a per-tenant taxonomy is
    option 9 — and no table is added, because a classification is a
    column on a record that already exists and a ceiling is a column on
    a grant that already exists. Embedded packs bump to `@10`.

19. **The leak suite is the AC, and EVAL-5 scales it.**
    `crates/synveda-gateway/tests/leak.rs` generates query variants
    across seeded material at all four tiers and several scopes, and
    asserts on the product surfaces that no `restricted` content appears
    in any block without a standing grant that declared it, that it does
    appear — exactly the target's published members, marked — while one
    stands, and that it stops appearing the moment the grant expires. It
    asserts the same at the engine seam, where a stale sidecar and a
    warm index are the interesting adversaries.

    EVAL-5 owns what it grows into: 10k generated variants nightly,
    cross-tenant fuzz (TEN-6), and the prompt-injection-via-memory
    suite. The boundary is deliberate — this feature ships the suite that
    proves its own AC, and the zero-tolerance nightly gate is EVAL-5's.

## Options considered

1. **Per-(scope, tier) decisions producing a per-scope tier set, with
   `restricted` forbidden in the base layer and lifted only by a
   tier-declaring lapse (chosen)** — ABAC before the fetch, no
   record-level decisions, the AC's "compliance-granted" falls out of the
   floor that already existed, and the read path's shape is unchanged.
   Con: up to 4× the `MemoryRead` decisions per inject, and a
   classification surface this feature did not originally set out to
   build.
2. **Decide per record after retrieval** — the obvious reading of "ABAC",
   and the only one that could enforce a per-record qualifier of any
   kind. Rejected on three counts: it puts N decisions on the hot path
   where N is candidate count rather than chain depth; it fetches
   material and then drops it, which is the fetch-then-filter shape
   ADR-0024 decision 2 refused when it made the filter mandatory and
   fail-empty; and it moves enforcement after the index, where a bug
   leaks instead of returning nothing.
3. **Sensitivity as pack configuration (a `SensitivityConfig` beside
   `CompositionConfig`)** — no Cedar change at all, hot-reloadable, and
   consistent with how redaction, composition, approvals and lapses are
   carried. Rejected: config is per-scope and the rules here are
   per-*principal* (own home, content-role bindings), which configuration
   cannot express without becoming a policy language; and the AC's floor
   must be invariant, which configuration by definition is not.
4. **Keep the structural clamp; make a lapse the only door** — the
   smallest change that passes the AC, and it needs no classification
   surface. Rejected: it leaves `confidential` permanently unreachable,
   so the product keeps writing records nobody can read, and the AC
   passes for the reason it passes today — a clamp — rather than because
   policy decided anything.
5. **Encode tiers as `Long` so packs write `context.level <= 2`** —
   compact, and ordering comparisons read naturally. Rejected: it puts a
   numbering between the author and the meaning, and a stored pack that
   gets the encoding wrong is wrong silently. Four names enumerated in a
   set cannot be off by one.
6. **Ask top-down and short-circuit at the first allowed tier** — one to
   three decisions per scope instead of four. Rejected *for now*: it is
   only sound if permits are monotone in tier, which Cedar does not
   guarantee and a stored pack could violate by writing a permit at
   `restricted` with none at `internal`. Reversal trigger (a) records the
   upgrade honestly: validate monotonicity at pack-install time — the
   ADR-0032 discipline of refusing at install rather than discovering at
   review — and the short-circuit becomes sound rather than assumed.
7. **Let the extractor mint `restricted`** — no classification surface
   needed, and erring upward is the fail-safe direction. Rejected: it
   makes the tier mean "a model thought so" while the invariant floor
   says it means "compliance signed off", and a ceremony that fires on
   hallucinations trains tenants to route around it. Decision 8.
8. **Reclassification as a direct route (`PATCH /v1/records/{id}`)** —
   ADR-0032 decision 8 kept the direct publish route for exactly this
   ergonomic reason, and most reclassification is mundane. Rejected: the
   mundane direction (raising your own material) is not the one that
   needs governing, and one call that both sets a tier and removes one is
   the shape the trail exists to make impossible. The proposal path costs
   one extra call and produces a reviewer, a reason, and a matrix.
9. **A per-tenant classification taxonomy** — banks have more than four
   labels, and a tenant that wants `pci` as a tier will ask. Rejected:
   the invariant floor, the approval matrix, the tech plan's §2.4 table
   and this ADR's base-layer forbid are all written against a fixed
   ordered vocabulary; a tenant-defined tier has no defined position in
   that order and no defined floor. The extension point when it comes is
   labels *beside* the tier, not instead of it.
10. **Ship all five conditions from the feature text** — it is what the
    line says. Rejected per condition rather than in bulk: channel would
    be a second engine deciding a rule the pack config already decides
    (decision 14), residency would be an attribute nothing routes on
    (15), time-of-day would be a second clock and an unexplained smaller
    block (16), and purpose-of-use as a widening is the reader
    authorising their own read (17). What is left is sensitivity, which
    is also the only one the AC names.
11. **Purpose-of-use as audited break-glass — declare a purpose, get the
    material, answer for it afterwards** — how healthcare actually does
    this, and the audit chain here is unusually well suited to it.
    Rejected: the product already has break-glass, it is the CLI's, and
    it is bounded to operations rather than content. A break-glass that
    hands over another team's restricted memory on a typed string is a
    different product, and it is not one seed §2.3 describes.
12. **Put the `restricted` forbid in each pack rather than the base
    layer** — consistent with how the channel, proposal, and quarantine
    planes were added, all of them pack-uniform. Rejected on ADR-0037
    decision 7's own axis, in the safer direction: those planes are
    restrictions a pack omitting them cannot loosen, and this is a
    restriction a pack omitting it would delete. A custom pack that
    forgot the rule would silently publish every tenant's restricted
    memory into its readers' blocks.

## Consequences

- **Positive**: the AC holds because policy decided it, not because a
  constant clamped it, and the sentence it holds by is the one the
  product already enforces on the write side — `restricted` needs
  compliance, in both directions, through the same matrix. The read path
  keeps its shape: decisions still precede the fetch, the filter is still
  mandatory and fail-empty, hydration still verifies against current
  truth, and no index, sidecar, or store query learns anything new beyond
  a pair check. `confidential` stops being a black hole. Four deferrals
  close: AUTHZ-2's "non-restricted content" qualifier becomes
  `open-collaboration`'s actual rule, ADR-0015's privacy-floor comment
  stops pointing forward, ADR-0024's clamp becomes a decision, and
  ADR-0037 decision 14's promised ceiling arrives with the matrix
  resolving at it and no lapse-specific rule anywhere.
- **Negative / accepted trade-offs**: the composition walk takes up to
  four times as many PDP decisions, on the hot path, and the decision
  counter's per-inject rate grows with it — mitigated by HIER-3's cached
  entity fragments, since the four asks at one scope differ in a context
  attribute and share the entity graph, but it is real and the latency AC
  has to measure it. `confidential` material becomes visible to
  content-role holders who could not see it yesterday: a widening, chosen
  deliberately, and the reason it is safe is that it was unreachable by
  *everyone* including its author, which is not a security property but
  an accident. An author can lose read access to their own restricted
  record (decision 7). Reclassification adds a proposal effect, an
  action, and a route to a feature whose AC is about reads — accepted
  because without it `restricted` has no honest author and the AC has no
  product path to demonstrate. And three of the five conditions the
  feature text names do not ship, which is a scope reduction stated in
  the ADR rather than discovered in the demo.
- **Reversal triggers**: (a) the per-tier walk showing in the inject
  budget (p99 > 150ms, seed §3) → validate tier-monotonicity at pack
  install and short-circuit top-down (option 6), which trades an
  install-time refusal for three-quarters of the decisions; (b) a tenant
  needing a principal-dependent channel rule → `context.channel` joins
  the walk as a second predicate dimension and `ComposeScope` carries a
  channel set instead of a bool (decision 14); (c) a tenant needing more
  than four tiers → labels beside the tier, never instead of it, and a
  new ADR because the floor and the matrix key on the order (option 9);
  (d) declassification proposals appearing in bulk (a retention sweep, a
  merger) → a batch effect over many members, which the proposal shape
  already admits and the matrix already resolves at the maximum;
  (e) a caller wanting to declare purpose → it lands as a *narrowing* on
  the inject body and a field on the audit event, and it may never widen
  (decision 17); (f) a time-of-day rule genuinely required by a tenant →
  answer the two-clocks question and the withheld-material marker first,
  then `context.now` (decision 16).

## Compliance notes

The PDP remains the one enforcement seam. `MemoryRead` gains one required
context attribute (`context.sensitivity`); the base layer gains a forbid
whose only `unless` is `context.lapsed`, which the PDP resolves from
grant rows and a caller can never assert. Every existing forbid still
overrides every permit, so a quarantined principal reads nothing at any
tier and a service identity is still confined to its anchor (ADR-0018
decision 4 carves out own-chain `MemoryRead` only). One new action,
`MemoryClassify`, joins the vocabulary and the schema; packs bump to
`@10`.

**The floor is now enforced on both sides of the same tier.** Nothing
reaches a published channel at `restricted` without `compliance` and two
distinct approvers (ADR-0032 decision 4), nothing becomes `restricted`
without a proposal resolved at `max(current, proposed)` (decision 9), and
nothing reaches a reader at `restricted` without a lapse that declared
the tier — which resolves against that same floor and therefore carries
the same compliance signature (decisions 5 and 6). The one deliberate
disclosure outside that path is the review surface, which shows a
reviewer both sides of what they are deciding (ADR-0035 decision 8): the
leak suite asserts that boundary rather than assuming it.

Migration 0023 adds a column to `policy_lapses` and widens one CHECK; no
table is added, so the forced-RLS set, the ADR-0009 completeness guard,
and the adversarial suite change only where they already cover those
tables. The adversarial cases that matter are a `max_sensitivity` raised
by UPDATE after approval — the tier equivalent of ADR-0037's
pushed-forward `expires_at`, and the same attack: it would turn an
`internal` grant two stewards approved into a `restricted` one nobody
did — and a record's `sensitivity` lowered directly in the records table,
bypassing the proposal that the matrix would have resolved at the higher
tier.

The trail answers the regulator's question in one place. A restricted
record's life reads as `proposal.opened` → `proposal.approved` × 2 (one
of them compliance) → `memory.classified` on one chain; the grant that
disclosed it reads as ADR-0037's sequence with the declared ceiling on
the `policy.lapse.granted` event; and every `context.injected` in between
names the scope, the lapse, and the tier set that was permitted.
Payloads carry ids, scopes, tiers and reasons — never record content, and
never the material the tier was protecting.
