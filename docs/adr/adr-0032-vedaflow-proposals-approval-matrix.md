# ADR-0032: VedaFlow proposals — approvals that bind bytes, one matrix in front of every path across the boundary, and curator files that add requirements without granting authority

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-3
- **Deciders**: sujitn

## Context

FLOW-3 is the feature ADR-0031 wrote its own reversal trigger against:
"(c) FLOW-3 landing → publication moves behind proposals and the direct
route becomes the proposal's effect." Its text is "Proposal lifecycle;
required approvals resolved from asset×sensitivity×scope×pack; approvals
are authz-checked actions; CODEOWNERS-style curator files per scope."

The uncomfortable truth FLOW-2 shipped with, and said so: **any curator
can publish anything, alone.** `POST /v1/channels/{scope}/publish` takes
two decisions — `ChannelPublish` and `MemoryRead` — and both are answered
by one principal holding one binding. Nothing distinguishes an internal
note from a `restricted` record. Nothing requires a second pair of eyes.
`compliance` and `security-reviewer` exist in the role vocabulary with a
doc comment that says "marker until then". FLOW-3 is *then*.

Forces at play:

- **A matrix that governs one path is not a matrix.** If proposals
  enforce required approvals and the direct publish route does not, the
  direct route is the hole, and every restricted record goes through it.
  Whatever FLOW-3 decides has to be the *same* decision on both surfaces
  or the feature is decoration.
- **Cedar answers a different question than the matrix does.** "May this
  principal approve here" is authorisation and belongs to the PDP. "Have
  enough of the right people approved" is a counting rule over recorded
  acts. Conflating them would either put counting in Cedar (which cannot
  see the approvals) or authority in the counting (which would bypass the
  PDP). Both halves have to exist and stay separate.
- **Content moves under a review.** ADR-0031 decision 5 made publication
  bind bytes because "any principal with `memory.write` could rewrite the
  text under a published id". Exactly the same attack exists one layer
  up: approve a proposal, edit the record, publish. An approval that
  names only a proposal id is an approval of nothing in particular.
- **The roles that make dual approval real hold no publish grant.**
  `compliance` is not a curator. If the last approval published
  automatically, the publish would have to run under system authority
  precisely when a compliance reviewer casts the deciding vote — the one
  case the requirement exists for.
- **CODEOWNERS is a requirement mechanism, not a grant mechanism.** In
  git, listing someone in CODEOWNERS makes their review *required*; it
  does not give them commit rights they lacked. A per-scope file that
  granted approval authority would be a second authorisation system
  sitting beside the PDP, editable by anyone who can write a file, which
  is the thing seed §2.2 exists to prevent.
- **ADR-0030 and ADR-0031 both left FLOW-3 named holes.** `vedaflow_refs`
  is generic "because FLOW-3's proposal refs and FLOW-7's pins need names
  that are not channels"; `AssetKind` is inside the object address
  "because FLOW-3 resolves approvals from asset type"; `staged` "has no
  writer until FLOW-3, so its ref is genuinely absent". Two of those
  three are discharged here as written. The third is not, and this ADR
  says why.
- **`inject` is still the hot path.** Nothing FLOW-3 adds may cost the
  read path a query. The matrix is consulted when content crosses the
  boundary, never when it is composed.

## Decision

A proposal is **a VedaFlow commit plus a workflow row**: the commit is
the reviewed content, addressed member by member; the row is the
lifecycle. Required approvals resolve from **asset × maximum sensitivity
× target scope kind × effective pack**, merged with an invariant product
floor no pack can author away, and merged again with the target chain's
CODEOWNERS-style curator file, which may only *add* requirements.
**Every path across the trust boundary resolves the same matrix** — the
proposal's effect and FLOW-2's direct publish alike.

Decisions, specifically:

1. **A proposal is a commit plus a row, and it gets no ref.** The commit
   is minted exactly as `publish` mints one: a tree naming each member at
   the object address of the version proposed. The row (`vedaflow_proposals`)
   carries target, asset kind, channel, proposer, state, and the commit
   it points at. Governed history stays immutable (commits and approvals);
   workflow state is mutable — the ADR-0030 split restated, where refs
   move and history does not.

   No ref per proposal. A ref names a *moving head*, and a proposal's head
   is named by its row; minting `{asset}/staged/{id}` per proposal would
   leave one permanent row per closed proposal in a table that
   deliberately holds no DELETE grant (migration 0018), for a pointer
   nothing follows.

2. **`staged` stays unwritten, and this amends ADR-0031's deferral.**
   ADR-0031 recorded "`staged` has no writer until FLOW-3". FLOW-3 is not
   its writer either, for a reason that is now known rather than pending:
   `staged` is a *set* channel, and a set channel cannot express
   withdrawal — retraction is FLOW-7's `force_update_ref` by name. "What
   is open here" is one indexed query over `vedaflow_proposals`, and that
   query is correct where a set would drift the first time a proposal was
   withdrawn. The channel vocabulary keeps `Channel::Staged` for a
   future set-shaped view (FLOW-8's export, the CNSL-1 inbox) if one is
   ever wanted.

3. **Requirements resolve live, from asset × max sensitivity × scope kind
   × pack.** An `ApprovalMatrix` is a list of rules; a rule matches when
   its asset (or `any`), its minimum sensitivity, and its scope kinds (or
   `any`) all match the request. Matching rules combine by taking the
   **maximum count per role** and the **maximum distinct-approver count** —
   never a sum, so a rule appearing twice in two forms does not silently
   double a requirement.

   Sensitivity is the **maximum over the proposal's members**: a set is
   reviewed as a set and is governed by its most sensitive element.

   Resolution is **live at every decision point**, not frozen at open
   time. That is ADR-0014 decision 3's doctrine — a pack switch governs
   the very next request — and freezing would create a second, staler
   answer to a question the product answers one way everywhere else. The
   requirement *as resolved* is recorded in the audit event at each act,
   so the trail shows what was asked for at the moment it was asked.

4. **A product floor is merged into every matrix, embedded or stored.**
   The `base.cedar` pattern (ADR-0014 decision 2) applied to
   configuration: two rules are prepended to every matrix and cannot be
   authored away by a stored pack —

   - anything at `restricted` sensitivity requires the `compliance` role
     and **two distinct approvers** (tech plan §2.4, and seed §4.2's own
     definition of the tier);
   - any `skill` requires `security-reviewer`, because a skill is
     executable and "skills are treated like code because they are".

   A stored pack may add requirements above the floor. It may not go
   below it. This is what makes "restricted asset requires compliance +
   dual approval" a property of the product rather than of a
   configuration someone remembered to write.

5. **The matrix counts approvals; Cedar decides who may cast one.** Two
   layers, never conflated. `ProposalReview` (a new Cedar action) decides
   whether this principal may vote at this scope at all. The matrix
   decides whether the votes so far are enough. An approval is recorded
   with the effective roles the approver held **at the target scope** at
   the moment they cast it, resolved by the PDP from bindings on the
   target's chain (ADR-0015 decision 3) — so an approval is evidence of
   the authority that existed then, not a claim re-derived later against
   bindings that may have changed.

   An approval that satisfies no outstanding requirement is **refused,
   not recorded**: a vote that governs nothing is noise in a log a
   reviewer and an auditor both read.

6. **Approvals bind the commit, and publication rechecks the bytes.**
   Every approval names the proposal commit it approved. Publishing
   recomputes each member's content address from the record as it stands
   now and requires it to equal what the approved commit named; a
   mismatch is `Conflict`, naming the record that moved. ADR-0031
   decision 5 one layer up: publication binds what was published,
   approval binds what was approved. Without it, "approve, edit, publish"
   launders unreviewed text through a completed review.

7. **Distinct approvers are counted by identity, and the proposer is not
   special-cased.**

   > **Superseded for recorded proposal review by ADR-0091.** The common
   > context-platform proposal now has an explicit author and its live matrix
   > may forbid author review or require a separate effect actor. The direct
   > authored-channel route remains the single-actor case described below.

   There is no universal self-approval ban. A proposer's approval
   counts as one identity if they hold a role the matrix asked for. What
   forbids unilateral action is `distinct_approvers >= 2`, and it forbids
   it identically on both surfaces.

   A self-approval ban was the obvious rule and it is the wrong one: on
   the direct publish route the acting principal is necessarily the only
   approver, so a ban would have made the two paths disagree about the
   same matrix. One person holding both `curator` and `compliance`
   satisfies both role requirements and still counts as one identity, so
   dual approval binds them exactly as intended.

8. **The direct publish route resolves the same matrix, satisfied by the
   acting principal alone.** `POST /v1/channels/{scope}/publish` stands
   and gains the gate: the publisher counts as one approver holding their
   effective roles, and the call proceeds only if that single approval
   satisfies the resolved requirement. A curator publishing internal
   memory under `regulated-strict` still works — the matrix asks for one
   curator and one curator acted. A `restricted` record refuses, names
   the missing role and the outstanding approver count, and points at the
   proposal route.

   This is ADR-0031's reversal trigger (c) discharged in the form that
   keeps *one* matrix rather than two paths: the direct route did not
   become a hole to close, it became the degenerate case where one
   approval is enough.

9. **Publishing a proposal is a separate act, under `ChannelPublish`.**
   The deciding approval does not publish. `POST /v1/proposals/{id}/publish`
   takes `ChannelPublish` and `MemoryRead` at the target exactly as the
   direct route does, and additionally requires the proposal open, the
   requirement satisfied, and the bytes unchanged.

   Auto-publishing on the final approval would have to run under system
   authority in precisely the case the requirement exists for — a
   `compliance` reviewer casting the deciding vote, holding no publish
   grant in any pack. That is a PDP bypass however it is spelled, and
   seed §2.2 has no exception for convenient ones. This is the seam
   ADR-0031 named: approvals go *in front of* `ChannelPublish`; they do
   not replace it.

10. **The published commit is a merge.** Its parents are `[channel head,
    proposal commit]`, first-parent mainline as in git. Tech plan §2.5's
    promise — "every published sentence of context traces to an author or
    a source session, through an approval" — becomes a fact about the
    commit graph rather than a join between two tables, and FLOW-8's
    export carries it into a real repository for free.
    `update_ref`'s fast-forward check passes unchanged, because the head
    is the first parent.

11. **The stored state is a fact; `approved` is a rendering.** The
    lifecycle column holds only what happened: `open`, `rejected`,
    `withdrawn`, `published`. Whether an open proposal is *approved* is
    computed live from its approvals against the live requirement
    (decision 3), so there is no state machine to drift, no background
    job to re-evaluate proposals when a pack changes, and no stored
    `approved` that a lowered requirement could contradict. The API
    reports the tech plan's five-state vocabulary, with `approved`
    rendered from `open && satisfied`.

12. **There is no revise verb.** A revision is a new proposal. The
    machinery for revising in place exists — approvals name a commit, so
    moving the commit invalidates them — but the surface does not,
    because a proposal whose content changes under its approvals is a
    review nobody consented to, and "withdraw and open a new one" says
    that plainly in the audit trail. A rejection is terminal and carries
    a reason (FLOW-5 inherits that reason for its per-level denials).

13. **Curator files add requirements; they never grant authority.** A
    per-scope CODEOWNERS-shaped file maps `{asset-kind}/{glob}` patterns
    to approvers — `@<subject>` for a named principal, `role:<role>` for
    a role. Every pattern matching *any* member of a proposal contributes
    its approvers to the requirement.

    The file cannot grant: a named subject still has to pass
    `ProposalReview` at the target scope, so a file naming someone the
    pack denies makes the proposal unsatisfiable rather than making that
    person an approver. This is `CompositionConfig`'s rule (ADR-0025
    decision 2, "the config never grants") applied to the other side of
    the boundary, and it is what keeps the file from becoming a second
    authorisation system editable by whoever can write a file.

14. **A curator file is a VedaFlow asset, stored under a `curators`
    ref.** An `AssetKind::Policy` object holding the file's exact bytes,
    committed to a ref named `curators` at the scope — content-addressed,
    immutable history, every change recording the pack in force and the
    identity that made it. So "who changed who must approve, and when" is
    the same kind of question as "who published this", answered from the
    same tables. The name is not a channel and does not parse as one
    (ADR-0031 decision 1 reserved exactly this), so `GET /v1/channels`
    skips it.

    Resolution is **nearest-ancestor-first** over the target's chain —
    the first scope carrying a file wins outright, no union — matching
    pack assignment (ADR-0014 decision 3). A union up the chain would
    make an org-level file impossible to narrow at a team, which is the
    direction the hierarchy is supposed to work.

15. **Curator files are written through `PolicyAssign`, not a new
    action.** `PUT`/`GET`/`DELETE /v1/hierarchy/nodes/{id}/curators`,
    beside `/policy` and `/roles`, under `PolicyAssign` and `PolicyRead`.
    The steward who can swap the entire pack — and with it the entire
    matrix — can obviously edit the file that pack's matrix composes
    with; a separate action would imply a separable authority that does
    not exist, and would be the fourth place to look when asking who can
    change approval requirements.

16. **Three new Cedar actions, on the quarantine plane's shape.**
    `ProposalRead`, `ProposalOpen`, `ProposalReview` — pack-uniform
    permits, like the channel plane and for the same recorded reason: how
    content crosses the trust boundary does not loosen per pack. What
    *does* vary per pack is the matrix, which is the thing the feature
    text says varies.

    `ProposalRead` mirrors `MemoryRead`'s shape in each pack (the
    membership floor plus content roles, personal scopes excluded) and
    adds the review and audit roles. `ProposalOpen` floors on membership —
    a placed principal may propose at a scope it belongs to, which is how
    tribal knowledge climbs without needing a grant first — plus
    contributor and above. `ProposalReview` is role-only: curator,
    steward, org-admin, compliance, security-reviewer. Auditor reviews
    nothing, by name.

17. **FLOW-3 is same-scope; FLOW-5 is the climb.** A proposal's source
    and target scope are both recorded and both must be the same node:
    this feature moves content from a scope's derived material to that
    scope's published channel. Cross-scope is refused by name, as FLOW-2
    refused it. The disclosure question that comes with a climb — a
    reviewer reading proposal content they could not read at its source —
    is FLOW-5's to answer, which is why `ProposalRead` is deliberately
    shaped like `MemoryRead` now: the boundary is already in the place
    FLOW-5 will have to reason about.

18. **Four new audit actions; the effect reuses `ChannelPublished`.**
    `ProposalOpened`, `ProposalApproved`, `ProposalRejected`,
    `ProposalWithdrawn` each record one governed act with the requirement
    as resolved, the roles counted, and the commit. Publishing a proposal
    emits `ChannelPublished` carrying the proposal id — it is the same
    governed act with the same consequence as a direct publish, and a
    second action asserting it would be a fact an auditor has to
    reconcile (ADR-0019 decision 4). Payloads carry ids, addresses,
    roles, and requirement summaries; never record content.

## Options considered

1. **One matrix in front of both paths, approvals binding commits,
   curator files additive (chosen)** — a single answer to "what does it
   take to publish this here", enforced identically wherever content
   crosses. Con: the direct publish route gains a resolution step it did
   not have, and a `restricted` record can no longer be published by the
   route FLOW-2 shipped.
2. **Close the direct publish route; proposals only** — the tidiest
   reading of ADR-0031's reversal trigger, and one surface to reason
   about. Rejected: it forces a two-step ceremony on the case the matrix
   itself says needs one approval, and it would have rewritten FLOW-2's
   acceptance demo to prove something FLOW-2 already proved.
3. **Leave the direct route ungated and enforce only on proposals** —
   no churn at all in FLOW-2's tests. Rejected outright: it is the hole.
   A matrix that any curator can walk around is a description of good
   intentions.
4. **Frozen requirements, snapshotted at open time** — a review whose
   goalposts cannot move, which is arguably fairer to the proposer.
   Rejected: it contradicts ADR-0014 decision 3 (a pack governs the very
   next request) and creates a second answer to "what does this need",
   which is exactly the drift a stored `approved` state would cause.
   The audit event records the requirement at each act, so the trail
   still shows what was asked when.
5. **Stored `approved` state, transitioned by the deciding approval** —
   a queryable state machine and a cheaper list endpoint. Rejected: the
   moment requirements resolve live (decision 3), a stored `approved` can
   contradict the matrix, and the fix is either a background re-evaluator
   or a lie. Computing it costs one already-loaded approvals list.
6. **Auto-publish on the deciding approval** — one fewer step, and the
   obvious product ergonomics. Rejected: it must run under system
   authority exactly when `compliance` casts the deciding vote, and no
   spelling of that is not a PDP bypass. FLOW-6's CLI can chain the two
   calls; the *authority* stays two decisions.
7. **Self-approval forbidden** — the PR-culture default and the
   quarantine plane's own precedent (an owner cannot release their own
   event). Rejected because it would make the two publish paths disagree:
   the direct route's actor is necessarily its only approver.
   `distinct_approvers` expresses separation of duties precisely, at both
   surfaces, and expresses it as a number the matrix can set per rule.
8. **Curator files as grants — listing someone makes them an approver** —
   what a naive reading of CODEOWNERS suggests, and it would make
   delegation easy. Rejected: it is a second authorisation system with no
   PDP in it, writable per scope. Additive-only requirements keep one
   authority (Cedar) and one counting rule (the matrix).
9. **Curator files as rows in a table** — queryable, no object store
   involvement, no parser. Rejected: the file is a governed asset by the
   feature's own text, and the whole point of VedaFlow is that changing
   who must approve is itself reviewable history with a pack recorded
   against it. A table would have needed its own audit story and its own
   versioning to reach the same place.
10. **A union of curator files up the chain** — every ancestor's owners
    always apply, which is arguably safer. Rejected: it makes an
    org-wide file impossible to narrow anywhere, inverting how the
    hierarchy works everywhere else, and it disagrees with pack
    resolution for no stated reason. The floor (decision 4) is where
    org-wide non-negotiables belong.
11. **A ref per proposal, `{asset}/staged/{id}`** — git-faithful, and it
    would put open proposals on a channel FLOW-8 could export. Rejected
    on the table it lands in: `vedaflow_refs` has no DELETE grant by
    design, so every closed proposal would leave a permanent pointer
    nothing follows.
12. **Requirements as Cedar policies** — one language, one engine, no
    second configuration format. Rejected: Cedar decides one request at a
    time against a principal, and "have two distinct people with these
    roles already approved" is a count over stored acts that the PDP
    cannot see and must not be given storage to see (seed §2.4).

## Consequences

- **Positive**: `restricted` content cannot reach a published channel on
  one person's say-so, by any route, under any pack — the floor is
  compiled in and merged into every matrix. Approvals bind bytes, so
  approve-edit-publish is a `Conflict` naming the record that moved.
  Lineage is in the commit graph: a published commit's second parent is
  the proposal, whose approvals are append-only rows naming the roles
  that were held. `compliance` and `security-reviewer` stop being
  markers. The two publish paths share one resolution function, so there
  is one place to read to know what it takes to publish anything
  anywhere. Curator files make per-scope review requirements a governed,
  diffable asset without adding an authority.
- **Negative / accepted trade-offs**: publishing a `restricted` record
  now takes two people and three calls — deliberate, and the reason the
  feature exists. The direct publish route gained a matrix resolution
  (one pack read, one curator-file read up the chain, both off the
  already-resolved chain) on a path that previously took none; it is not
  the inject path and the cost is a governed write's, not a read's.
  `staged` remains a name with no writer. Rejected and withdrawn
  proposals leave unreferenced commits, which is exactly the packing/GC
  question ADR-0030 left open and does not worsen it in kind. The curator
  file's glob language is deliberately minimal (`*` only, matching entry
  names that are record ids today), and will need real path semantics
  when SKIL-1 and PRMT-1 bring path-named entries — the parser accepts
  the shape now so that growth is not a format change.
- **Reversal triggers**: (a) a scope accumulating open proposals faster
  than they are reviewed (the `MAX_OPEN_PROPOSALS` cap tripping in
  normal use) → FLOW-4's auto-opened proposals need batching or an
  expiry, before the cap is raised; (b) requirement resolution showing up
  in the publish path's latency once FLOW-4 opens proposals
  automatically → cache the curator file per scope on the pack
  refresher's cadence, not on the request; (c) a tenant needing an
  approver set that the additive-only rule cannot express (a *narrower*
  requirement at a child scope) → nearest-ancestor-first already allows
  it for the file; if the *floor* is what needs narrowing, that is a
  product decision and a new ADR, not a configuration key; (d) FLOW-5
  landing → `source_scope_id` stops being required to equal
  `target_scope_id`, and the disclosure rule this ADR defers becomes
  decision 1 of that ADR.

## Compliance notes

The PDP stays unbypassable and gains three actions. Opening, reading,
reviewing, and publishing a proposal are each a Cedar decision at the
target scope before anything is written; publishing takes the same two
decisions FLOW-2's route takes (`ChannelPublish` + `MemoryRead`) and adds
the matrix on top. The matrix is a counting rule over recorded approvals
and never authorises anything by itself: an approval it would have
counted is still refused if `ProposalReview` denies, and a curator file
naming a principal the pack denies makes a proposal unsatisfiable rather
than making that principal an approver.

Two new tables (`vedaflow_proposals`, `vedaflow_proposal_approvals`) join
the forced-RLS set in migration 0019 with least-privilege grants:
proposals take SELECT/INSERT/UPDATE (the lifecycle column moves) and no
DELETE; approvals take SELECT/INSERT only, with the ADR-0019
update/delete/truncate triggers, because a review log that can be edited
is not one. `policy_packs` gains an `approvals` column beside `redaction`
and `composition`. Every read and write runs inside the caller's
`rls::begin_tenant_tx`, and both tables join the ADR-0009 completeness
guard and the adversarial RLS suite.

The audit trail gains four actions and one enriched payload. Each
records the requirement **as resolved at that moment** — roles, counts,
distinct-approver threshold, named subjects, and where each came from
(floor, pack, or which scope's curator file) — so an auditor reading the
chain can reconstruct why a proposal needed what it needed without
reading the pack that has since changed. `ChannelPublished` carries the
proposal id when a publish had one, which is what makes "the auditor
reads proposals, not database rows" (tech plan §2.5) true for the
published channel: the event names the proposal, the proposal names the
commit, the commit's tree names the bytes, and the approvals name who
stood behind them under which roles.
