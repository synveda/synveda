# ADR-0035: The CLI review flow — the reviewer is a governed principal, not an operator, and the diff is of the effect on the channel

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-6
- **Deciders**: sujitn

## Context

FLOW-6's text is "`synveda proposal list/show/review/approve/reject`; diff
rendering for text assets", and its acceptance criterion is one sentence:
**full review possible without console**. Tech plan §2.4 says the same
thing from the other side — "Reviews happen in the admin console or via a
CLI (`synveda proposal review 142 --approve`)" — and the console does not
exist (CNSL-1..4 are unstarted), so today the CLI is not an alternative to
the console, it is the only review surface there is.

Everything the flow needs already exists. FLOW-3 (ADR-0032) built
`/v1/proposals` with `list`, `get`, `open`, `approve`, `reject`,
`withdraw`, and `publish`, all behind the PDP and the approval matrix.
FLOW-5 (ADR-0034) added the climb without adding a route. So this feature
is not a new capability; it is the question of whether a person holding
`curator` at a team can do their job from a terminal, and of what they
have to be shown to do it honestly.

Forces at play:

- **This CLI already has two halves, and they are not interchangeable.**
  `db migrate`, `tenant create`, `token issue`, `policy apply`, `role
  bind`, `service register` connect to `DATABASE_URL` and write rows.
  They exist for the moment before a gateway is usable, they audit
  themselves as break-glass with OS-user attribution (ADR-0019 decision
  7), and every one of them is documented as dev plumbing. `login` and
  `auth token` are the other half: gateway clients that hold a bearer.
  A review belongs to the second half and could be mistaken for the
  first, because a reviewer with a database URL is *technically* able to
  insert an approval row.

- **CLAUDE.md forbids a code path that bypasses the PDP, in tests
  included.** Approvals are counted against a requirement the PDP has no
  view of, and the PDP decides who may cast one (`ProposalReview`) and who
  may run the effect (`ChannelPublish` + `MemoryRead`). Those two layers
  are kept apart on purpose (ADR-0032): "the PDP cannot see stored
  approvals, and a counting rule must never be authority". A CLI that
  wrote `vedaflow_proposal_approvals` directly would be the counting rule
  acting as authority, from a laptop, with no `ProposalReview` decision
  anywhere in the trail.

- **A review surface must show what changes.** Publishing is additive and
  keyed by record id: a scope's published tree maps record → address, so
  proposing an *edited* record that the channel already names replaces its
  entry. FLOW-3's own AC exercises exactly this shape — approvals bind
  bytes, so the way to republish an edited record is to open a new
  proposal — which means the interesting review is precisely the one where
  the channel already holds an older version and nothing in
  `GET /v1/proposals/{id}` says so today. It returns the record's current
  content and whether it still matches the proposed address. It does not
  return what is being replaced.

- **FLOW-1 already chose the format for this.** `MemoryAsset::
  canonical_bytes` is canonical JSON with sorted keys, and its doc comment
  says why: "Human-readable on purpose — FLOW-6 renders diffs of it and
  FLOW-8 exports it into a real git repository, where a length-prefixed
  binary blob would be worthless" (`crates/synveda-vedaflow/src/
  channels.rs`). The bytes to diff were decided two features ago.

- **Content roles and review roles are disjoint, deliberately.**
  `compliance` and `security-reviewer` hold no `MemoryRead` in any pack.
  ADR-0034 decision 1 settled what that means for content a proposal
  carries: the reviewer sees it because the *proposer* disclosed it, once,
  under their own read, recorded under their name. Any new content the
  review surface shows has to be answered against that same rule rather
  than assumed into it.

- **A listing of UUIDs is not a review surface.** `ProposalSummary`
  carries `target_scope_id` and `source_scope_id` and no names. A console
  would render "Platform"; a terminal today renders
  `0198f0aa-…-…`. A reviewer who cannot tell which team a proposal is for
  cannot review it, and for a climb the *source* is half of what they are
  judging.

## Decision

`synveda proposal` is **a gateway client under the reviewer's own
bearer** — no database connection, no store dependency, no break-glass —
and what it renders is **the proposal's effect on the target's published
channel**: per record, whether publication would add, replace, or change
nothing, with a field-wise diff and a line diff of the text for the
records it would replace. The gateway gains no route, no Cedar action, no
audit action, and no migration; it gains three additive fields on the
member view of `GET /v1/proposals/{id}` and the scope paths the summary
was missing.

Decisions, specifically:

1. **Every `synveda proposal` verb is an HTTP call to `/v1/proposals`
   under the profile's bearer.** The CLI gains no dependency
   (`synveda-cli` still links types/store/identity/policy/audit, and none
   of the new code touches the store), issues no SQL, and has no
   `--database-url` escape. A reviewer with no login is told to run
   `synveda login`; there is no fallback, because the fallback is the
   bypass.

   This is the one decision the feature is really about. Approving is not
   an operator action that happens to be recorded — it is a governed act
   whose authority, whose count, and whose audit event all live behind the
   PDP. The break-glass half of this CLI exists because a store with no
   usable gateway still has to be bootstrapped; a review has no such
   moment, and inventing one would mean an approval with no
   `ProposalReview` decision, no pack version, and no chained event
   attributable to anyone but whoever held the password.

2. **The bearer is resolved exactly as `synveda auth token` resolves
   it**, refresh, skew, and the ADPT-1 fallback included, with
   `SYNVEDA_TOKEN` + `SYNVEDA_GATEWAY` as the explicit override recorded
   for CI and demos (ADR-0027). One implementation of expiry and refresh
   in this binary, not a second one drifting beside it; a demo and a
   human reach the same code path.

3. **The named verbs are `list`, `show`, `review`, `approve`, `reject` —
   and `publish` and `withdraw` are added, because the flow cannot
   conclude without them.** The AC is *full* review without a console. A
   curator who can approve but not run the effect still has to leave the
   terminal, and ADR-0032 decision 9 is emphatic that the deciding
   approval must not publish by itself: publishing is a separate governed
   act taking `ChannelPublish` and `MemoryRead` at the target. If the CLI
   omits it, the product's answer to "how do I complete a review without a
   console" is `curl`. `withdraw` is the proposer's matching act — the one
   way a closed lifecycle is reachable from the same surface. Neither adds
   a route or an authority; both are routes FLOW-3 shipped.

4. **`review` is the interactive walkthrough; `approve`/`reject` are the
   scriptable verdicts.** The tech plan's sketch was one verb with verdict
   flags (`proposal review 142 --approve`); it is split here on purpose.
   A `--approve` flag on the command whose job is to render the diff is an
   invitation to approve without reading it, and a script that casts a
   verdict should say `approve` in the trail it leaves in someone's shell
   history. `review` walks the queue **oldest first** — a review queue
   that starves its oldest entry is how a proposal quietly never gets
   read — renders each proposal in full, and prompts for a verdict per
   proposal.

   Its fail-safe direction is silence: EOF on stdin ends the queue having
   cast nothing, so `synveda proposal review < /dev/null` is a no-op
   rather than a blind approval, and an unattended invocation cannot
   approve anything.

5. **What is diffed is the effect on the target's published channel, per
   record.** Three outcomes, and every member is exactly one of them:

   - **add** — the target's `memory/published` tree names no version of
     this record; publication admits it.
   - **update** — the tree names it at a *different* address; publication
     replaces that version with this one.
   - **none** — the tree already names it at exactly this address;
     publication changes nothing about this member.

   Membership in the target's tree is the predicate, which is the same
   sense of "the channel holds this" that ADR-0034 decision 3 used one
   scope over and that composition reads. A climb's baseline is therefore
   the *ancestor's* channel — correctly, because the ancestor's channel is
   what the proposal moves.

6. **The two sides are object bytes, not record rows.** The new side is
   the object at the address the proposal's commit names; the old side is
   the object at the address the target's tree currently names. Both come
   from `vedaflow_objects`, which is append-only and immutable, so both
   are always readable and neither can be rewritten under the review.

   Reading the *proposed* side from the object rather than re-deriving it
   from the record matters in exactly the case that matters: when a record
   has been edited since the proposal opened, the record row is no longer
   what anyone approved. Approvals bind bytes (ADR-0032 decision 6), so
   the review surface shows the bytes. The existing `content` field —
   the record as it stands now — stays, and beside `unchanged` it is what
   makes drift visible rather than confusing.

7. **The diff is field-wise, with a line diff for the text.** A memory
   object is canonical JSON with sorted keys, so a raw byte diff renders a
   multi-line content edit as one enormous escaped line — the worst
   rendering of the most important case. Instead each governed field
   renders as `field: old → new`, and `content` renders as a line-level
   unified diff. That is what "diff rendering for text assets" asks for,
   and it keeps the governed metadata visible: a proposal that changes
   only `sensitivity` or closes `valid_to` is a real change to what
   crosses the boundary, and a content-only diff would render it as empty.

   The renderer is CLI-side presentation and nothing else depends on it.
   The API ships bytes.

8. **Showing the old side is a disclosure, and it is the one the review
   already makes.** Rendering `update` means showing a `ProposalRead`
   holder the content the target's published channel currently holds for
   the records this proposal names — which for `compliance`, who holds no
   `MemoryRead` in any pack, is content they could not otherwise read.

   It is admitted for the reason ADR-0034 decision 1 admitted the proposed
   side: a review of a change that hides one side of the change is not a
   review, and refusing it would make `compliance` — the role the
   invariant floor requires on everything `restricted` — the one reviewer
   who must approve replacements sight unseen. It is bounded three ways
   and deliberately not bounded by role: only records the proposal names,
   only the version the target's own channel currently publishes, and only
   at the target scope the reviewer is already reviewing for. It is
   strictly the thing being replaced by the thing they are already shown.

9. **The summary gains the scope paths it needed to be readable.**
   `target_scope_path` on every proposal (the listing already loads the
   target node to resolve the requirement, so it costs nothing), and
   `source_scope_path` beside it — the same node when source and target
   are equal, one extra read when they are not, so only climbs pay for it.
   A reviewer reads "acme/eng/platform → acme/eng", which is the sentence
   FLOW-5 exists to make true, instead of two UUIDs.

10. **No route, no action, no event, no migration.** `GET /v1/proposals/
    {id}` grows `effect`, `proposed`, and `baseline` on each member;
    `ProposalSummary` grows the two paths. The PDP decides exactly what it
    decided before, at the same seams, and every audit event keeps its
    shape — the CLI's acts chain as the gateway's, under the reviewer's
    IdP-authenticated identity, because they *are* the gateway's acts.
    The one thing added below the gateway is a batched object read
    (`vedaflow::read_objects`), so the detail route's statement count stays
    constant rather than growing with the member set.

## Options considered

1. **A gateway client under the reviewer's bearer** (chosen) — the
   reviewer is the same principal the console would authenticate; every
   decision, count, and event is unchanged. Costs a login before a review,
   which is the point.

2. **A store-backed CLI like `policy apply` and `role bind`** — rejected.
   It is the PDP bypass CLAUDE.md forbids, wearing the clothes of the
   break-glass commands that legitimately need one. Those exist because a
   database with no gateway must still be bootstrapped; there is no
   corresponding moment for an approval. It would also have to invent an
   identity: `vedaflow_proposal_approvals` names an `IdentityId` and the
   roles held at the target, and the break-glass actor has neither.

3. **A read-only CLI (list/show only), verdicts left to the console** —
   rejected: it fails the acceptance criterion by construction, and the
   console does not exist.

4. **`review --approve` as the tech plan sketched it** — rejected as the
   only spelling, for decision 4's reason. It survives in effect as
   `approve`, which is the same act named for what it is.

5. **Diff the raw canonical bytes as text** — rejected for decision 7's
   reason: correct, and unreadable exactly where it matters.

6. **Diff the `content` field only** — rejected: it renders a
   sensitivity change or a closed validity window as no change at all,
   and those are changes to what crosses the trust boundary.

7. **Compute the baseline client-side from `GET /v1/channels/{scope}`** —
   rejected. That route returns a commit and an entry count, not a
   membership, so the CLI cannot resolve record → address from it; adding
   membership there would put the object bytes behind `ChannelRead`
   instead of `ProposalRead`, which is a *wider* disclosure than decision
   8 (every `ChannelRead` holder, for every record, at any time) reached
   by a surface nobody asked for.

8. **Gate the diff fields behind `?diff=true`** — rejected. The default
   answer to "what does this proposal do" should not be the incomplete
   one. The response does grow (see the trade-off below); the bound is
   `MAX_PROPOSAL_MEMBERS`, which already bounds the `content` the route
   returns today.

9. **A TUI (full-screen, cursor-addressed)** — rejected for this feature.
   A line-oriented prompt is scriptable, pipeable, testable in a demo, and
   works over SSH and in CI; a TUI is a bigger dependency surface and a
   worse fit for the one criterion being met.

10. **Do nothing** — the review flow requires a console that does not
    exist, or `curl` with a hand-assembled bearer. That is the status quo
    FLOW-6 exists to end.

## Consequences

- Positive: a curator can drain a review queue from a terminal, and every
  act in it is the same governed act the console would have made — same
  PDP decisions, same approval counting, same chained events, same
  identity. The product's review story does not depend on CNSL landing.
- Positive: the diff makes the *replacement* case legible for the first
  time. "Approvals bind bytes" was previously visible only as a 409 at
  publish time; a reviewer can now see what a proposal would overwrite
  before voting on it.
- Positive: the interactive queue and the scriptable verdicts serve two
  different users (a human draining a queue, a pipeline acting on a
  decision already made) without either pretending to be the other.
- Negative / accepted trade-offs:
  - `GET /v1/proposals/{id}` now returns up to three texts per member
    (current record content, proposed bytes, baseline bytes) where it
    returned one. Bounded by `MAX_PROPOSAL_MEMBERS` (200) and
    `MAX_OBJECT_BYTES`, the same bounds the route already carried; no
    pagination is added, and a proposal large enough to make this hurt is
    already a review nobody can perform.
  - The old side is disclosed to reviewers who hold no `MemoryRead`
    (decision 8). Recorded as a deliberate widening of ADR-0034 decision
    1's carve-out, not an accident of implementation.
  - The diff renderer is hand-written rather than a dependency. Line-level
    LCS over short records is a boring, testable ~100 lines, and the core
    path's licence rule (MIT/Apache-2.0/PostgreSQL, cargo-deny enforced)
    makes a casual dependency a reviewed diff. The cost is that its
    correctness is ours; unit tests are the mitigation.
  - `synveda proposal list` with no `--scope` takes a *tenant*-resource
    decision, which the packs grant to tenant-wide review and admin roles
    only. A curator bound at one team is denied it and must pass
    `--scope`. That is FLOW-3's boundary, not this feature's, and the CLI
    names the flag in the refusal rather than hiding it — but it is the
    one place the terminal flow is less discoverable than a console would
    be.
- Reversal trigger:
  - CNSL-1 landing does **not** retire this surface; if the console ever
    duplicates the diff computation rather than reading these fields,
    that duplication is the bug.
  - If a review ever needs to show more than one record's worth of
    surrounding context — a proposal against a prompt template or a skill
    bundle (PRMT-1, SKIL-1), whose objects are not one text field — the
    field-wise renderer needs a per-asset-kind renderer behind the same
    seam, and that is a new ADR rather than a widened `match`.
  - If the response size ever becomes the reason someone cannot review,
    option 8's `?diff=` param is the recorded first move, before
    pagination.

## Compliance notes

- **No new authority anywhere.** No Cedar action, no permit, no pack
  version bump. `ProposalRead`, `ProposalReview`, `ProposalOpen`,
  `ChannelPublish`, and `MemoryRead` decide exactly what they decided
  before ADR-0035, at the same seams.
- **No new audit action, and no new emission point.** Every event a CLI
  review produces is emitted by the gateway handler that already emitted
  it, inside the same tenant transaction, chained under the reviewer's
  own IdP-authenticated subject. A CLI review and a console review are
  indistinguishable in the chain, which is correct: they are the same act.
- **The break-glass actor never appears.** `Actor::break_glass` is not
  reachable from any `synveda proposal` verb, because none of them opens
  a database connection.
- **Tenant isolation is unchanged**: the CLI carries a bearer whose `tid`
  the gateway resolves, and every read the new fields require runs inside
  the request's existing `begin_tenant_tx` under forced RLS. The baseline
  read is `vedaflow_objects` and `vedaflow_tree_entries`, both already in
  the adversarial RLS suite (FLOW-1).
- **Content disclosure**: decision 8 is the one content-visibility change
  in this ADR and it is bounded by the proposal's own member set at the
  target the reviewer already holds `ProposalRead` on. The record text
  itself continues to live in `records`; nothing new is written anywhere,
  and no audit payload gains content — the trail keeps carrying ids and
  addresses only.
