# ADR-0054: distribution is a set where resolution was a name — an advertisement is not a demotion, so it has to earn its place a different way, and a materialisation that cannot remove is not a distribution

- **Status**: Accepted
- **Date**: 2026-08-03
- **Feature(s)**: SKIL-4
- **Deciders**: sujitn

## Context

SKIL-4's text is "skills attach to hierarchy nodes; inject index tier lists
skills available to this identity; adapter materialises them into the harness
(Claude Code plugin dir)", and its acceptance criterion is "user in team A
sees team A's skills; team B's are absent; org skills present for both".

The first clause landed with SKIL-1: a skill is authored at a scope, published
onto that scope's `skill/published` channel, and resolved **by name** up the
caller's own placement chain, nearest-first, skipping the scopes the PDP
refuses (ADR-0051 decisions 1 and 6). The registry exists. What does not exist
is the plural of it. Nothing in the product answers *"which skills may I
install?"* — `GET /v1/skills/{name}` needs a name the caller already has, and
`GET /v1/skills?scope_id=…` is one named scope's authoring shelf, drafts
included. A distribution feature is the set, and the set is the thing the
acceptance criterion is written about.

What is parked here by name:

- **ADR-0041 decision 4** built the index tier's per-`AssetKind` rendering
  seam for "PRMT-1, PRMT-2 and SKIL-1", and **ADR-0041 option 9** recorded
  the always-index threshold as a trigger for "PRMT-2/SKIL-1 land with bodies
  far exceeding their index lines".
- **ADR-0049 option 10 and ADR-0051 option 11** both deferred skills out of
  the index tier to here, in the same words: advertisement of available
  assets is SKIL-4's own acceptance criterion, and shipping half of it
  earlier would make that criterion untestable against a baseline that
  already contained it.
- **ADR-0027 decision 1** reserved `skills/` in the Claude Code plugin
  manifest "so both arrive as configuration rather than restructuring", and
  its reversal-trigger list ends with the line "SKIL-4 lands → `skills/` in
  the same manifest".
- **ADR-0051 decision 12** put the materialisation in the CLI and the receipt
  outside the bundle; **decision 13** made a rewind refuse a pinned read;
  **ADR-0053 decision 5's** rubric prices `description-states-when` joint-
  heaviest because "SKIL-4 will advertise this same line".
- **Seed §4.3** — skills are "distributed to agents by scope" — and **§2.6**,
  the harness is a guest.

Four forces, and the first two are the feature.

1. **An advertisement is not a demotion.** ADR-0041 decision 2's rule is that
   a candidate is offered its index line only when its body did not fit and
   only when the line is *strictly cheaper* than the body. That rule is what
   let a mechanism built for assets that did not exist yet ship against a
   corpus made entirely of assets that did. It cannot apply to a skill,
   because **a skill has no body in the block and never will** (ADR-0051
   decision 9: a skill's content never becomes a record and never composes).
   There is nothing to be cheaper than. Every safeguard CTX-4 relied on —
   the comparison, the "only when it saves", the guarantee that a demotion
   converts a silent omission into a named one and costs nothing else — is
   unavailable here. A skills index is **new content in the block**, paid for
   out of a budget that was fully spoken for, and it has to earn its place on
   an argument of its own.

2. **For the flagship client, the block is the second place this is said.**
   Claude Code reads every installed `SKILL.md`'s frontmatter and advertises
   name and description to the model at roughly 80 tokens each — the client's
   own progressive disclosure, which is the exact argument ADR-0051 decision 9
   used to keep skill *bodies* out of the block. Applied to names it says the
   same thing one level down: a block that lists what the client has already
   listed spends the memory budget doing the client's job twice. The honest
   reading is not "so don't ship it" — the feature asks for it and two ADRs
   deferred it here — but that the advertisement has to be worth its tokens
   where the client's own index is *absent or behind*: a harness with no
   skills loader at all (the generic MCP server, a LangGraph shim, an SDK
   caller), a machine where nothing has been materialised yet, and the window
   between a publication and the next time a client reads its skills folder.

3. **A materialisation that only writes is not a distribution.** FLOW-7's
   acceptance criterion is "<60s to fleet-wide effect", and ADR-0051 decision
   13 kept it true of an asset that lives on laptops by making a rewind
   refuse the pinned read. But a rewind that withdraws a skill leaves the
   directory sitting on the disk: the next session loads a bundle the
   registry no longer publishes. The same hole opens with no rewind at all —
   a user moves from team A to team B, and team A's `deploy-runbook` stays
   installed. Distribution therefore has to **reconcile**, not install, and
   the moment something prunes a directory, *who owns that directory* stops
   being a matter of taste.

4. **The read path's costs are inherited, not renegotiated.** `inject` p99
   < 150ms; the read path makes no model call and reads no clock (ADR-0024
   decision 7); CTX-2's criterion is byte-identical re-composition at the same
   instant; a second config plane was refused by ADR-0025 decision 3 and a
   separate index budget by ADR-0041 option 4. Whatever advertises skills does
   it from stored bytes, deterministically, under one budget, configured on
   the pack.

And one thing nothing parked here, found by writing the acceptance criterion
down as three clauses. **They are three different mechanisms, and only one of
them is a policy decision.** Org skills reach both readers because the org is
on both chains; team A's skills reach a team A reader because team A is on
that reader's chain; team B's skills are absent from a team A reader **because
team B is on no chain that reader has** — the same reason another tenant's
records are absent, one level down. A test that asserted all three the same
way would pass for a build that decided nothing, which is the shape of an
acceptance test that cannot fail.

## Decision

**The available set is the resolve route's chain walk applied to a whole
shelf instead of one name; the inject block gains a skills section that is an
advertisement rather than a demotion, paid for last out of the same budget and
bounded on its own terms; and the materialisation becomes a reconcile that
owns the directory it prunes — the plugin's own `skills/`, never the user's.**
No new Cedar action, no new table, no migration, and no pack version bump.

Decisions, specifically:

1. **`GET /v1/skills` with no `scope_id` is the available set.** The resolve
   route already established the convention one line up — "absent walks the
   caller's own placement chain" (ADR-0051, `ResolveParams::scope_id`) — and
   the plural reads the same way. `?scope_id=` keeps meaning the authoring
   shelf at that scope, drafts and all. Two shapes on one path, discriminated
   by a `view` field on the response so the payload says which question it
   answered rather than leaving a reader to infer it from the request.

2. **The set and the by-name resolve are the same walk, and that is a test
   rather than a comment.** Same chain, nearest-first; same `SkillRead`
   decision per `(scope, tier)`; same published-only rule; same uniform
   omission for a denial. A caller who reads the set and then installs a name
   from it must get the bundle the set described — two walks that could
   disagree would make the shadowing rule a coincidence, and the one they
   would disagree about is the interesting one (ADR-0051 decision 6: a team's
   `code-review` and the org's cannot both exist on a client's flat disk).

3. **Shadowing is applied after the PDP filter, never before.** SKIL-1's AC
   line says it: "a nearer copy nobody may read does not shadow the further
   readable one". A nearer scope that publishes the name but denies the read
   is skipped as though it published nothing, and the further readable copy
   is what the set names. Filtering after shadowing would turn a policy denial
   into a *missing skill* — the one failure mode where a governed read surface
   is worse than an ungoverned one.

4. **Only `published` is advertised.** A draft is installable by naming its
   scope (ADR-0052 recorded that seam explicitly), and it is not *available*:
   the set is what the review has carried, so an advertisement can never be
   the first way a reader hears about unreviewed executable content.

5. **A skill's index entry is an advertisement, and it is charged last.** It
   is appended after every scope section, under the same one budget, first-fit,
   and a skill that does not fit is counted rather than named. It therefore
   **never displaces a body**: force 2 says the block may be the second place
   a client hears this, and spending a memory body to repeat the client's own
   index would be paying twice for the duplication. This is the inverse of
   ADR-0041 option 5's reasoning for memory index entries — there, a trailing
   section would have broken the gradient by putting a nearer scope's name
   below a farther scope's body; here the gradient has already done its whole
   job by *choosing which* skills are in the set, and it has no opinion about
   where they sit relative to recorded material, because a capability and a
   memory are not competing to be read.

6. **The section header is the legend.** ADR-0041 decision 12 charged a
   legend line to the first demotion because `(recall <id>)` is an opaque
   marker that needs explaining, and it had to describe the marker without
   being one. A skills section needs no such sentence: `## Skills available
   (install with ...)` says what the lines are and what to do with them in the
   header the section pays for anyway. One line, not two.

7. **The line is the name and the description, elided at the pack's own
   `index_entry_chars`.** No scope path, no commit, no handle: the gradient
   already resolved which copy this is, so naming the scope inline is a fact
   the reader cannot act on, and the name *is* the handle — it is the install
   argument and the installed directory name at once (ADR-0051 decision 6). A
   tier above the working one is marked exactly as a record's line is
   (ADR-0038 decision 11): a `confidential` skill's description is
   confidential, and the guest cannot know what it is holding unless the block
   says so.

8. **The scope, the commit and the address ride the response and the chain,
   never the block.** This is ADR-0031 decision 11's precedent, taken for its
   own reason: "inject responses cite commit hashes", paid for out of the
   response instead of the budget. The `context.injected` payload gains a
   `skills` array — name, scope, commit, the `SKILL.md` object address, tier —
   which is what makes "was that agent told this capability existed" a
   question the chain answers. Never the description text, on the AUD-1
   discipline every plane has followed.

9. **The block hash covers the skills section.** `block_hash` is the block's
   identity, and two blocks that say different things must not share one. The
   hash extends over the advertised `(name, address)` pairs after the entry
   addresses — appended rather than interleaved, so a block with no skills
   hashes exactly as it did before this feature, which is the same
   byte-identity discipline `index_tier: off` keeps for CTX-4.

10. **No new Cedar action, no pack version bump.** ADR-0041 decision 1
    inherited whole: the advertised set is the permitted set rendered shallow,
    decided by the `SkillRead` the resolve route already takes, per scope and
    per tier, in the same plan walk. A name and a description are content, and
    a second, weaker "may list but not read" verdict would be a second leak
    surface across every pack × role × scope × tier cell the AUTHZ-5 suite
    covers. `PermittedTiers` gains a third set beside `memory` and
    `context_pack`; the Cedar sources do not change, so the embedded packs
    stay at `@14`.

11. **The knob rides `CompositionConfig`, and `off` restores the previous
    block exactly.** `skill_index: off | names`, resolved per candidate scope
    through the same effective-pack walk as the channel rule, the horizons and
    the index tier (ADR-0025 decision 3; ADR-0041 decision 11). The product
    default is `names` in all three embedded packs — a feature that ships off
    is a feature nobody sees — and the description width is the *existing*
    `index_entry_chars` rather than a new field, so a pack that narrowed its
    index lines narrows these too. Under `off`, a block is byte-identical to
    what it was before SKIL-4.

12. **The count is bounded by a constant, not by a knob.** At most 32 skills
    are named in one block, ordered by chain position then name, the remainder
    counted and reported. 32 is recall's id cap and for its reason (ADR-0041
    decision 7): comfortably above any plausible block, far below a corpus.
    The budget is the real bound in practice — at ~80 tokens a line, a
    1,500-token block cannot hold 20 — and the cap exists so that a scope
    publishing hundreds cannot make the read path read hundreds of objects.

13. **Skills are advertised on `inject` only.** `ComposeRequest::naming` and
    `::sweeping` — the two recall forms — turn the section off with the index
    tier they already turn off. Recall names bodies (ADR-0041 decision 7) and
    a skill has none; an as-of sweep answers "what did the agent know", and
    the honest answer about a capability at a past instant would need channel
    state that ADR-0042 decision 10 deliberately does not rewind.

14. **No projection table: the descriptions are read from the published
    objects at compose time, and the cost is measured rather than assumed.**
    A published skill's description lives in its `SKILL.md` object, and the
    draft row's copy is the *draft's* — advertising that would be SKIL-3's
    own finding one plane over, a cache that describes bytes nobody published.
    The tree names `<skill>/<path>`, so identifying what a scope publishes
    costs no object read at all; only the manifests of the skills that survive
    the gradient are fetched, in one batched read, and the tier the PDP
    decides against comes out of the same object. A projection table
    maintained at the publish seam is recorded as the reversal trigger with a
    number attached, not built blind.

15. **`synveda skill sync` is a reconcile, and what it may remove is
    bounded by its own receipts.** It resolves the available set, installs
    every skill in it through the ordinary audited read route with SKIL-1's
    byte-identity check intact, and **removes every directory this product's
    own receipt says it wrote into this root and the registry no longer
    serves**. Removal is the half that makes distribution real: FLOW-7's
    sixty seconds and a leaver's revoked access both reach a laptop only
    through a delete.

    The bound is the receipt rather than the directory listing, and that is
    the decision. ADR-0051 decision 12 put a receipt beside the credentials
    to carry provenance a materialised bundle cannot; it turns out to be the
    record of *what this product wrote*, which is exactly what a destructive
    reconcile needs and what a `readdir` cannot supply. A directory with no
    receipt was not written here and is never a candidate.

    A skill already at the served commit whose files still hash to what the
    receipt recorded is left alone — so a session-start sync costs one
    listing call and no resolves — and one whose bytes have **drifted** is
    rewritten, which makes an edited governed bundle self-heal rather than
    persist.

16. **The adapter materialises into the plugin's own `skills/`, never the
    user's `~/.claude/skills`.** The CLI's client table keeps pointing at the
    user's directory for the by-hand install a person asks for; the *governed*
    root is `${CLAUDE_PLUGIN_ROOT}/skills`, which is the directory ADR-0027
    decision 1 reserved and the only one a reconcile may prune, because a
    prune over the user's own directory would delete skills Synveda never
    governed. Two roots is the honest cost, and the consequence is named:
    a skill installed both ways exists twice, and the client's own precedence
    decides which loads.

17. **The adapter shells out to the CLI and reimplements nothing.** ADR-0027
    decision 4 gave the credential reason — one implementation of refresh, in
    Rust, rather than a second drifting one in TypeScript — and path safety is
    the stronger case of the same rule. ADR-0051 decision 7's grammar (`..`,
    absolute forms, reserved device names, trailing dots, case-fold
    collisions) is the product's defence against a governed bundle writing
    outside its directory, and a TypeScript re-implementation of it is exactly
    the two-parsers-disagreeing failure ADR-0051 decision 4 refused for YAML —
    with a filesystem underneath instead of a frontmatter.

18. **The materialisation is off the SessionStart critical path.** A second
    `SessionStart` entry in `hooks.json`, `async: true`, running beside the
    inject hook rather than inside it: inject holds a 3-second deadline
    against a 150ms SLO (ADR-0027 decision 3), and N bundle resolves plus N
    directory writes do not belong in it. The consequence is a timing one and
    it is stated rather than hidden: **a skill materialised during a session
    is loaded by the client at the next one**, because a client reads its
    skills folder when it starts. That is precisely the gap force 2 said the
    advertisement has to be worth its tokens in — the block names what is
    available *now*, including what is not yet on disk — and it is why the two
    halves of this feature are one feature.

19. **`sync` adds no audit action.** Every bundle it writes is served by the
    resolve route, which chains `skill.resolved` with the commit and the
    per-file addresses (ADR-0051 decision 16), so the chain already records
    exactly what reached the laptop and under what decision. A removal is a
    client-side act on a directory the server never knew about — the same
    reason there is no `skill.installed`, and an event the server cannot
    verify is a fact an auditor has to reconcile.

## Options considered

1. **The available set as `GET /v1/skills` with no scope, a trailing skills
   section in the block, and a reconciling CLI sync into the plugin's own
   root (chosen)** — inherits the walk, the PDP decision, the audit chain,
   the byte-identity check and the receipt; adds one response shape, one
   render, one subcommand and one hook entry. Con: two response shapes on one
   route, and two skills roots per client.
2. **A separate `GET /v1/skills/available` route** — unambiguous, and what an
   API reviewer would draw first. Rejected on a fact of this product's own
   grammar: `available` is a *valid skill name* (one lower-case hyphenated
   segment, ADR-0051 decision 6), so the route would either shadow a skill
   somebody may legitimately publish or require reserving names for routes.
   The scope-absent convention already exists one line up and costs nothing.
3. **Skills as index-tier demotions of pseudo-candidates** — the literal
   reading of "inject index tier lists skills", and it would reuse every line
   of CTX-4's assembly. Rejected: it needs a `RecordId` a skill does not have,
   it would put a `(recall <id>)` handle on a body that must never compose
   (ADR-0051 decision 9), and ADR-0041 decision 2's cheaper-than comparison
   has no second operand. Reusing the machinery would have meant lying to it.
4. **A projection table of published skill names and descriptions,
   maintained at the publish seam** — one indexed read per scope on the hot
   path instead of a batched object read, and the obvious answer at scale.
   Deferred with a trigger (decision 14): it is a second place the truth
   lives, it has to be rebuilt on a FLOW-7 rewind as well as a publication,
   and SKIL-3's finding was precisely that a cache beside a governed number is
   only safe when nothing gates on it. Revisit when the measurement below
   binds.
5. **A separate budget for the skills section** — clean, and the section
   could never be squeezed out. Rejected twice over: it is ADR-0025 decision
   3's second config plane and ADR-0041 option 4's own refusal, and here it
   would be worse, because a reserved allocation is spent whether or not the
   caller's client already has an index of its own.
6. **Advertising drafts as well as published skills** — an author would see
   their own work in their own block. Rejected on decision 4: the block is a
   trust surface, and the first place a reader hears about executable content
   must not be one that skipped the review.
7. **Naming the scope path on every skill line** — an agent could tell a team
   capability from an org one. Rejected on price and on actionability: the
   gradient has already chosen, so the scope changes nothing the agent can
   do, and it is in the response and on the chain for the reader who *is*
   asking that question.
8. **Suppressing the advertisement for clients that materialise** — the
   sharpest answer to force 2, and the one that would make the tokens
   unambiguously worth spending. Rejected: the gateway cannot know what is on
   a caller's disk, and a client-declared "I already hold these" would be an
   unverifiable claim from a guest shaping a governed read. The pack knob
   (decision 11) is the supported way to say it, per scope, by somebody who
   knows.
9. **Materialising into `~/.claude/skills`** — one root per client, and the
   CLI's existing table already points there. Rejected on decision 15: a
   reconcile prunes, and pruning a directory a person also writes into by
   hand would delete their own skills. The plugin root is a directory this
   product created and may therefore own.
10. **Materialising in TypeScript inside the adapter** — no process spawn, no
    CLI dependency, and the adapter is already a HTTP client. Rejected on
    decision 17: it duplicates a filesystem-safety grammar whose whole value
    is that there is one of it.
11. **Syncing synchronously inside the SessionStart inject hook** — the
    skills would be on disk before the block that names them. Rejected on
    ADR-0027 decision 3's posture: a memory system must never break the
    session it serves, and the hook's budget is 5 seconds for a call whose
    SLO is 150ms. It would also not buy what it looks like it buys — the
    client has already read its skills folder by the time a SessionStart hook
    runs.
12. **A `SkillList` Cedar action, weaker than `SkillRead`** — the honest
    model for a browse-without-open UI, and it would let a scope advertise
    what it will not serve. Rejected on ADR-0041 decision 1's ground, which
    has not changed: a description is content, and the second verdict doubles
    the matrix the leak suite covers. CNSL-2's explorer is where that case
    gets made if it ever does.
13. **Doing nothing until a harness without a skills loader exists** — force
    2's strongest form. Rejected: the generic MCP server is ADPT-2, one
    feature away; the AC asks for the listing today; and the pack knob makes
    the cost opt-out-able for the tenants who disagree.

## Consequences

- **Positive**: "which skills may I install" is answerable for the first
  time, by the same walk that answers "give me this one", so the two cannot
  drift; the acceptance criterion's three clauses are three distinct
  mechanisms and the test asserts them as three; a rewind or a team move now
  reaches a laptop, because the materialisation removes as well as writes; the
  adapter grows one hook entry and no new logic, so ADR-0027's reserved
  `skills/` arrives as configuration exactly as it promised; no migration, no
  Cedar change, no pack bump, and `skill_index: off` restores the previous
  block byte for byte.
- **Negative / accepted trade-offs**: the block now spends tokens on
  something a materialising client may already be advertising to the same
  model — named in force 2, bounded by decision 5's placement, and opt-out per
  scope, but real; the compose path reads one object per advertised skill,
  which is the first read-path cost this product has added without a cache
  behind it; a client sees a newly published skill one session late, because a
  loader reads its folder at start; two skills roots exist per client, and a
  skill installed both ways exists twice with the client's own precedence
  deciding; and `sync` deletes directories, which is the first destructive act
  the CLI performs on a user's machine — bounded to a root this product owns
  and to names its own receipts record, and the reason decision 16 is not a
  matter of taste. A caller's `max_sensitivity` now narrows pack chunks too,
  which is a fix and also a behaviour change: a caller who asked for
  `internal` and was quietly receiving `confidential` conventions receives
  fewer of them.
- **Reversal triggers**: the measured per-inject cost of the manifest reads
  binds the p99 → option 4's projection table, with the rewind path built in
  the same commit; a tenant's corpus makes 32 the wrong cap → it becomes a
  `CompositionConfig` field beside `skill_index`, which is a knob rather than
  a redesign; the block's skill lines measured as pure duplication for every
  client anybody uses → flip the product default to `off` and keep the
  mechanism for ADPT-2's harnesses; a client ecosystem where plugin-owned
  skills are not loaded, or not loadable, from the plugin root → the governed
  root moves to configuration and `sync --root` is already the seam; anybody
  needing the set at a scope they are not placed under → the widened universe
  ADR-0042 decision 2 built for recall applies unchanged.

## What the acceptance work turned up

Four things, recorded because they are behaviour rather than notes.

**A caller's `max_sensitivity` was not narrowing anything but memory.**
ADR-0038 decision 12's promise — "an agent that knows it is about to paste
into a pull request asks for `internal` and gets a block it can be careless
with" — is about a *block*, and `ComposeRequest::narrowed_to` trimmed
`sensitivities` alone. Since PRMT-2 that meant a `confidential` pack chunk
still composed for a caller who had asked for `internal`, from any scope that
also had readable memory; SKIL-4 would have added `confidential` skill
descriptions to the same hole. Found by writing the ceiling into the third
tier set and asking what the other two did. It also dropped a scope whose
*only* readable material was packs, undoing ADR-0050 decision 8 at the one
seam that narrows. Both halves are fixed here, with a unit test per half,
because the alternative was shipping a third asset kind through a promise
that had quietly stopped being true.

**The measurement's honest reading is not the section's own number.** The
A/B below composes a corpus that is *entirely skills*, so the `off` arm
composes the **empty block** — no preamble, no watermark, zero tokens — and
turning the section on makes the block exist. The section costs 65 tokens;
the reader is charged 118, because a block that exists pays its preamble and
its watermark. "The advertisement costs N" is only true for a reader who was
already being given something, and a reader with nothing else now gets a
block where they previously got none. That is the right behaviour and the
wrong sentence to leave unqualified.

**A rewind rewrites bundles whose bytes never changed.** A receipt is keyed
by commit, so when a rollback moves a scope's channel every skill that
channel serves is "not current" and is resolved and rewritten — even the ones
whose files are identical. Correct, because the commit is what a receipt pins
to and what a reinstall reproduces, and cheap at these sizes; noted because
the first person to watch a sync after a rollback will see more writes than
they expected.

**The fixtures had to clear SKIL-3's bar to publish at all.** A one-file
bundle with a description and a body scores 50–55 against
`regulated-strict`'s 70, so every skill this feature's tests and demo publish
carries a section and a worked example. That is the previous feature's gate
working on the next one's fixtures, and taking the override instead would
have demonstrated the wrong thing: what a fleet installs is what a review
passed.

### The measurement (decision 14), as taken

`crates/synveda-gateway/tests/skills.rs::the_skill_index_tiers_token_cost_is_measured`
composes one corpus twice over `POST /v1/inject` — two published org skills,
one reader with no memory records at all, the same block with
`skill_index: off` and then with it on:

| | skills named | block tokens | section cost |
|---|---|---|---|
| `off` | 0 | 0 | 0 |
| `names` | 2 | 118 | 65 |

Read against ADR-0041's own table, which is why it is in the same shape. The
index tier there cost 122 tokens to name **one** record; this names two
skills for 65, because a skill's index line is an authored description rather
than 320 characters of elided body — decision 4's per-`AssetKind` rendering
seam paying for itself. At the seed §4.4 default of 1,500 tokens the section
is 4% of a block per skill, and the remaining 53 tokens of the 118 are the
preamble and the watermark that only a non-empty block pays.

The number to watch is not this one. It is `skills_omitted`, which is zero
here and is what turns non-zero when a fleet's shelf outgrows its budget —
and the day it does, the answer is `index_entry_chars` or a pack saying
`off`, not a bigger cap.

## Compliance notes

- **PDP**: no skill reaches a block or a laptop without `SkillRead` at the
  scope that publishes it, at the tier that version carries, decided at
  request time under the live pack. The advertised set's only scope producer
  is `composition_plan` — the same walk inject already runs — and an empty
  plan advertises nothing; the available-set route decides the same action per
  scope; `sync` performs no authorization of its own, because it installs
  bytes an authorized resolve returned. There is no path that lists a skill a
  caller could not resolve (seed §2.2).
- **Tenancy**: no new table and no new query shape — the section reads
  `refs`, `commits`, `trees` and `objects`, all of them per tenant and already
  covered by the adversarial suite, inside `rls::begin_tenant_tx` (ADR-0009).
- **Audit**: no new action type. `context.injected` gains a `skills` array
  carrying names, scopes, commits, addresses and tiers and never a
  description; every bundle `sync` writes is chained by the resolve route's
  own `skill.resolved`; the available-set listing chains the `AuthzDecision`
  event the per-scope listing already does, with its own `op`.
- **Determinism**: the set is ordered by chain position then name, elided at a
  character boundary, and read from stored bytes under the caller's explicit
  instant — no clock, no model, no map iteration order. CTX-2's byte-identical
  re-composition holds with the section on, and is asserted with it on.
- **Observability**: `synveda_skill_index_tokens` beside
  `synveda_index_tier_tokens`, the advertised and omitted counts on the
  `gateway.inject` span, and `skills.available` in the skill-operations
  counter's `op` label (DoD #3).
