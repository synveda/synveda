# ADR-0034: Cross-scope promotion — the climb is a proposal against the higher scope, disclosure is the proposer's read at the source, and a published channel admits content that lives below it

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-5
- **Deciders**: sujitn

## Context

FLOW-5's text is "Team→dept→org climbs with each level's approvers", and
its acceptance criteria are "E2E of knowledge climbing two levels with
distinct approver sets; denial at any level audited with reason". Tech
plan §2.3 states it once: "**Cross-scope promotion** (team → department →
org) is a proposal against the higher scope, requiring that scope's
approvers. This is how tribal knowledge climbs the org gradient with
governance at each step."

Two ADRs wrote this one's first constraint before it existed. ADR-0032
decision 17 made FLOW-3 same-scope on purpose and named what it was
deferring: "The disclosure question that comes with a climb — a reviewer
reading proposal content they could not read at its source — is FLOW-5's
to answer, which is why `ProposalRead` is deliberately shaped like
`MemoryRead` now: the boundary is already in the place FLOW-5 will have
to reason about." Its reversal trigger (d) says the same thing as an
instruction: "`source_scope_id` stops being required to equal
`target_scope_id`, and the disclosure rule this ADR defers becomes
decision 1 of that ADR." ADR-0033 decision 8 leaned on the same
constraint from the trigger side and recorded that FLOW-4's multi-member
case is waiting on "a shared-scope writer (PRMT-1) or on FLOW-5's climb".

Forces at play:

- **A climb that changes nothing an agent reads is decoration.** The
  composition engine's candidate universe is records whose `scope_id` is
  on the caller's permitted chain, and a record composes as *published*
  when **its own scope's** tree names it
  (`crates/synveda-retrieval/src/compose.rs`, `channel_of`). Publishing a
  team's record onto the department's channel under those rules would
  compose for nobody: a sibling team has the department on its chain but
  not the team, so the record is never a candidate. Whatever FLOW-5
  decides about proposals, the read path has to learn the same thing or
  the feature is a row in a table.

- **The privacy floor is load-bearing in both directions.** No pack
  permits `MemoryRead` on another principal's personal scope — every
  non-self content permit carries `resource.kind != "user"` — which is
  what makes "nobody can climb someone else's private material" free.
  The same floor is why a rule that asks a *later* actor to read the
  source cannot work: nobody but the owner reads a personal scope, so no
  curator anywhere could publish a climb out of one.

- **`compliance` reads no memory in any pack, by design.** The content
  roles are `viewer`, `contributor`, `curator`; `compliance` and
  `security-reviewer` are review roles and hold no content read
  ("Admin and audit roles deliberately grant no content read (least
  privilege)"). ADR-0032 decision 4's floor requires `compliance` on
  anything `restricted`. So any rule that makes reviewing a climb
  conditional on reading its source makes the product's own invariant
  floor unsatisfiable for climbs.

- **Role bindings inherit downward and never upward.** A curator bound at
  a team holds nothing at the department (ADR-0015 decision 3: bindings
  on the resource's chain apply, and a team is not on a department's
  chain). That is what makes "each level's approvers" a real distinction —
  and it is also why any design that gives the *source* a vote has to
  invent a second review authority rather than reuse the one that exists.

- **A proposal's requirement resolves at exactly one scope.** ADR-0032
  decision 3 resolves from asset × maximum sensitivity × target scope
  kind × effective pack, merged with the floor and the nearest curator
  file on the target's chain. Everything about the matrix assumes one
  target. A climb must not become a second kind of proposal.

- **`inject` is still the hot path.** ADR-0032 and ADR-0033 both paid
  their costs on governed writes and background sweeps. FLOW-5 has to
  touch the read path — it is the read path the feature exists to change —
  so the change has to be a *substitution* in the composition query, not
  an addition to it.

## Decision

A climb is **an ordinary proposal whose target is a strict ancestor of
its source**: same table, same matrix, same lifecycle, same audit
actions. The material's location is defined the way ADR-0031 defined
trust — a scope holds material either because it **lives** there or
because that scope **published** it — so a second hop starts from where
the first one landed without anything new being stored. Disclosure is
answered once, at open time, by **the proposer's `MemoryRead` at the
source**; every later act decides at the target exactly as FLOW-3 does.
And publication at a scope **admits content that lives below it**, on the
read path as well as in the tree, because the channel is the trust
boundary and residence is not.

Decisions, specifically:

1. **The disclosure rule: one read decision, at the source, at open
   time, and it is the proposer's.** Opening a climb takes `ProposalOpen`
   at the target (may this principal propose here) and `MemoryRead` at
   the **source** (may they read what they are proposing). Reading,
   reviewing, and publishing decide at the target, unchanged from FLOW-3.

   A reviewer at the target therefore sees content they may not be able
   to read at its source. That is not a leak that slipped through; it is
   the disclosure the proposer made, under a read they already held,
   recorded under their name in `ProposalOpened`. The alternative —
   requiring every reviewer to hold `MemoryRead` at the source — is
   refused for two independent reasons, either of which is decisive:
   `compliance` holds no content read in any pack, so a `restricted`
   climb could never be reviewed by the role the floor requires; and
   nobody but the owner reads a personal scope, so a user's own memory
   could never climb to their team, which is the sentence the feature
   exists to make true.

   The floor does the rest with no special case. Bob cannot climb
   Alice's personal material because no pack permits him `MemoryRead`
   there. Alice can climb her own because the self permit does. A
   sibling team's material is unreachable for the same reason it is
   unreadable.

2. **`source_scope_id` stops having to equal `target_scope_id`; the
   target must be a strict ancestor of the source.** Checked against the
   source's own chain (the HIER-2 cache, one warm resolve), so a climb
   walks *up* the chain that composition walks *down*. Sideways is
   refused by name: a peer scope is not on the source's chain, has no
   authority over it, and admitting it would turn the approval matrix
   into a cross-team transfer mechanism with the target's curator as the
   only party. Same-scope proposals are unchanged — FLOW-3's case is a
   climb of zero levels, and it takes the same two decisions, because a
   principal that passes `ProposalOpen`'s membership floor at a scope
   passes `MemoryRead`'s membership floor there too.

3. **"At a scope" means living there or published there.** A climb's
   members must be current records that either live at the source or are
   named by the source's `memory/published` tree **at exactly their
   current address**. Not a precondition bolted onto the feature — the
   definition of where material is, taken from ADR-0031 decision 5,
   which already says a scope's published tree names content that scope
   stands behind, and says it byte-exactly.

   This is what makes the tech plan's ladder expressible without
   mandating it: hop one is team → department with the records living at
   the team; hop two is department → org, where the department holds the
   material because hop one published it there. Nothing new is stored to
   make the second hop possible, and an edited record drops out of both
   senses at once, because the address moves with the content.

4. **Requirements resolve at the target and only at the target.**
   ADR-0032 decision 3 unchanged, curator file included:
   nearest-ancestor-first over the **target's** chain. The source's
   curator file is deliberately not merged, and the reason is
   mechanical rather than philosophical — a curator named in a team's
   file holds no `ProposalReview` at the department, because bindings
   inherit downward and never upward, so merging the source's file would
   make climbs *unsatisfiable* rather than consented-to (ADR-0032
   decision 13's rule, hitting the case it was written for). "Each
   level's approvers" is true because each level's proposal resolves at
   that level, not because a proposal accumulates levels.

5. **No ladder is enforced.** A team may target the org directly if the
   org's approvers agree. What the product guarantees is a property of
   where content *lands* — everything on a scope's published channel was
   approved under that scope's matrix by principals that scope's pack
   permitted to review — not a sequence of scopes it had to visit first.
   Requiring the immediate parent would let a department veto a decision
   the org's own stewards made, and would have to be enforced against
   channel state that FLOW-7's rewind can move underneath it.

6. **A published channel admits content that lives below it, and
   composition reads it that way.** The read-path half, without which
   decisions 1–5 change nothing an agent sees. Two substitutions in
   `compose`:

   - the published fetch is by **id** — membership in a planned scope's
     published tree is the predicate — where FLOW-2 additionally
     required the record to live at a planned scope;
   - an entry takes the gradient position of the **nearest planned scope
     whose published tree names it at its current address**, where
     FLOW-2 took the position of the scope the record's row carries.

   A record no planned scope's tree names composes as derived at its own
   scope's position, or not at all, exactly as before; the derived sweep
   keeps its scope predicate untouched, because derived material has
   crossed no boundary and never leaves the scope it was extracted at.
   When source and target are the same scope — every proposal FLOW-3 and
   FLOW-4 can produce — both substitutions are identities, which is why
   FLOW-2's and CTX-2's suites pass unchanged.

   The dropped `scope_id = any(planned)` predicate is a guard relocated,
   not weakened: "this reviewed set names the record at the address it
   currently holds" is a stronger statement than "the record lives at a
   scope you may read", and the PDP still decides `MemoryRead` once per
   planned scope before any of this runs. What the read path stops
   asserting is that content may only reach a reader from the scope it
   was written at — which is the assertion a promotion exists to retire.

7. **Publication of a climb binds bytes at the source.** ADR-0032
   decision 6, one scope over: every member's address is recomputed from
   the record as it stands now, required to equal what the approved
   commit named, and required to still be held by the source in
   decision 3's sense. A record that moved, was deleted, or was rewound
   off the source's channel between approval and publication is a
   `Conflict` naming the record, not a silent partial publish.

8. **Nothing new is stored and the PDP gains no actions.**
   `vedaflow_proposals` has carried `source_scope_id` since migration
   0019 and migration 0019's transition trigger already makes it
   immutable, so there is no migration in this feature. Cedar gains no
   action and no permit: a climb is `ProposalOpen` and `MemoryRead`, both
   of which every pack already answers, decided at two scopes instead of
   one. That is the strongest available evidence that FLOW-3 put the
   boundary in the right place — ADR-0032 decision 16 shaped
   `ProposalRead` like `MemoryRead` for this exact moment, and the shape
   held.

9. **The audit trail gains no action and two fields.** `ProposalOpened`,
   `ProposalApproved`, `ProposalRejected`, `ProposalWithdrawn`, and
   `ChannelPublished` each carry the source scope beside the target, and
   `ProposalOpened` additionally records the source-read decision beside
   the open decision — two governed decisions, two recorded contexts.
   "Denial at any level audited with reason" needs nothing new: a
   rejection is `ProposalRejected` with the mandatory reason ADR-0032
   decision 12 already requires, and the *level* is the target scope on
   the event. A PDP denial at a level — a team curator who cannot review
   at the department — chains as `authz.decision` at the respond seam
   with the pack that decided (ADR-0019 decision 5).

10. **`MAX_OPEN_PROPOSALS` counts at the target, unchanged.** A climb
    spends the review queue of the scope whose approvers must read it,
    which is where the attention it costs actually lives. No second cap,
    and no per-source accounting: a scope that is the source of a
    thousand climbs has cost its ancestors a thousand reviews, and the
    ancestors' caps are what say so.

11. **Rules do not climb yet, and the reason is decision 1.** ADR-0033
    reversal trigger (e) is half discharged here: the same-scope
    constraint leaves the proposal surface, so it is now a property of
    the rule engine rather than of the table. The other half — a rule's
    target expression — is not this feature's, and not for want of
    plumbing. A rule acts under the material owner's authority
    (ADR-0033 decision 9), and an owner who configured no target has
    decided nothing about disclosing their material upward; decision 1
    says a climb is a disclosure and names the principal who made it. A
    rule that climbed on a threshold would have no such principal. When
    the rule vocabulary gains an explicit target, the org that wrote it
    is that principal, the owner's `MemoryRead` at the source is decided
    exactly as it is here, and the engine needs nothing else.

12. **A climb costs one gather, not two.** Decision input is gathered at
    the **source** — the deeper node, whose chain contains the target's
    chain as a suffix — and the target's decision runs against that
    suffix. This is `permitted_chain_scopes`' own pattern
    (`crates/synveda-retrieval/src/authz.rs`), and it is why a climb
    reads no more pack assignments or role bindings than the same-scope
    proposal FLOW-3 shipped, despite deciding at two scopes.

13. **The direct publish route stays same-scope.** `POST
    /v1/channels/{scope}/publish` continues to require its records to
    live at the scope. Not a hole in ADR-0032 decision 8's "one matrix in
    front of every path" — a restriction never is, and the direct route
    can only ever do *less* than the proposal route. What a climb needs
    beyond a single call is exactly what a proposal is: a recorded
    proposer, a recorded disclosure decision, and a review other people
    can read before the content crosses.

## Options considered

1. **A climb is an ordinary proposal at a strict ancestor; disclosure is
   the proposer's source read; publication admits content living below
   (chosen)** — one proposal shape, one matrix, no new tables, no new
   Cedar actions, and the read path learns one rule. Con: the composition
   query stops filtering published candidates by residence, so a record
   can compose for a reader who cannot read the scope it lives at — which
   is the feature, stated as its own cost.
2. **Reviewers must hold `MemoryRead` at the source** — the cautious
   reading, and it would make a climb disclose nothing to anyone who
   could not already see it. Rejected twice over: `compliance` holds no
   content read in any pack, so the invariant floor's own role could
   never review a `restricted` climb; and the privacy floor means no
   curator could ever publish a climb out of a personal scope, killing
   the headline case. A rule that makes the product's floor unsatisfiable
   is not a safe rule.
3. **The climb copies the records to the target scope** — composition
   needs no change at all, and every scope's material lives where it
   composes from. Rejected: a copy is a second row that drifts from the
   first the moment either is edited, doubles the corpus at every level
   it climbs, and gives FLOW-4's usage projection two records where the
   product has one fact.
4. **The climb moves the records to the target scope** — no duplication,
   and residence keeps meaning what it meant. Rejected: it takes the
   record out of its owner's write reach (`MemoryWrite`'s floor is the
   owner's own home), silently invalidates the source's own published
   tree, and answers a question about *trust* by editing a row about
   *residence* — the exact conflation ADR-0031 decision 5 separated.
5. **Mandatory ladder: the target must be the immediate parent** — the
   most literal reading of "team → department → org … governance at each
   step". Rejected: it gives a department a veto over an org decision
   the org's own stewards made, and it can only be enforced against
   channel state, which FLOW-7's rewind moves — so a rollback at the
   department would retroactively invalidate a climb the org approved.
   Decision 3 gets the ladder as an available path without making it a
   rule.
6. **Merge the source's curator file into the requirement** — the
   source's stewards get a say in what leaves their scope, through
   machinery that already exists. Rejected on mechanics: a curator named
   in a team's file holds no `ProposalReview` at the department, so the
   merge produces unsatisfiable proposals rather than consented-to ones.
   Giving the source a real vote means a second review authority
   evaluated at a second scope, which is a different feature and a
   different ADR.
7. **A multi-stage proposal: one row that walks the chain, gathering
   each level's approvals in turn** — one object for "this knowledge is
   climbing", and the AC's "two levels" reads directly off it. Rejected:
   a requirement resolves at one scope (ADR-0032 decision 3), so a row
   with N stages is N reviews pretending to be one; a rejection at stage
   two would have to un-publish stage one's effect, which is a rewind
   (FLOW-7) triggered by a review; and every downstream consumer —
   FLOW-6's CLI, CNSL-1's inbox, FLOW-8's export — would need to learn a
   second proposal shape. Two proposals say the same thing and say it in
   the vocabulary that already exists.
8. **Cross-scope on the direct publish route as well** — symmetry with
   ADR-0032 decision 8, and one fewer refusal to explain. Rejected: the
   direct route's whole justification is that the acting principal *is*
   the review, and a climb's disclosure needs a proposer who is not the
   approver on the record. Keeping the route same-scope is a
   restriction, and a restriction is never the hole a relaxation would
   be.
9. **Composition re-decides `MemoryRead` at the record's home scope
   before admitting a climbed entry** — defence in depth on the read
   path, and the strictest available reading of the privacy floor.
   Rejected: it denies exactly the case the climb exists for. A sibling
   team member can never read team T, so the department's publication
   would admit the record to the tree and then refuse it to every reader —
   publication at the department would mean precisely nothing.
10. **A `staged` ref per climbing proposal, so a climb is visible as a
    channel** — FLOW-8 could export it, and CNSL-1 could read it as a
    set. Rejected for ADR-0032 decision 2's reason, unchanged: a set
    channel cannot express withdrawal, and `vedaflow_refs` holds no
    DELETE grant, so every closed climb would leave a permanent pointer
    nothing follows.

## Consequences

- **Positive**: tribal knowledge climbs, and the climb changes what
  agents read — a department publication composes for every team under
  it, at the department's position in the gradient, under the
  department's `MemoryRead` decision. Each level's approvers are real and
  distinct in the direction that matters: a team curator holds nothing at
  the department, so the department's climb needs the department's
  people. The feature adds no table, no migration, no Cedar action, no
  audit action, and no second proposal shape — FLOW-6's CLI, CNSL-1's
  inbox and FLOW-8's export inherit climbs without knowing they exist.
  ADR-0033's recorded gap closes: material at a shared scope now exists,
  so a `min_distinct_members: 3` rule has something it can fire on. And
  the disclosure question ADR-0032 deferred has one answer in one place,
  rather than a rule per surface.
- **Negative / accepted trade-offs**: a record can now compose for a
  reader who cannot read the scope it lives at. That is the feature, and
  it means the published tree is doing authorisation work that the record
  row used to do — so a bug in tree membership is a disclosure bug, where
  before it was a trust-label bug. The address check is what keeps that
  narrow (a record whose content moved falls out of the tree by
  arithmetic, not by a job), and the PDP decision per planned scope is
  unchanged. Opening a climb costs one extra PDP decision and one extra
  channel read on a governed write path; nothing is added to `inject`,
  which substitutes two predicates and gains no query. Everything that
  climbs accumulates at the org root, where `MAX_CHANNEL_MEMBERS`
  (10,000) is the standing bound. And FLOW-4 still cannot climb, so
  automated promotion remains same-scope until a rule can name a target.
- **Reversal triggers**: (a) a tenant needing publication *without*
  disclosure — a scope that wants a record on its channel for lineage but
  not in its readers' context — → that is a channel-visibility rule and a
  new ADR, not a filter bolted onto composition; (b) an org root's
  published set approaching `MAX_CHANNEL_MEMBERS` because climbs
  accumulate there → subtree sharding, which ADR-0031 reversal trigger
  (a) already names, arrives before the cap is raised; (c) FLOW-7 landing
  → a rewind at the source does not un-publish at the target, so a
  climbed record survives its source's rollback; whether that is right is
  FLOW-7's decision and this ADR does not presume it; (d) a rule gaining
  an explicit target (ADR-0033 trigger (e)'s other half) → decision 11's
  conditions are the ones that must hold; (e) the published fetch's
  by-id read showing up in `inject`'s latency once org-level published
  sets are large → the read is by primary key over a set the tree already
  bounded, so the fix is the cap in (b), not an index.

## Compliance notes

The PDP stays unbypassable and gains nothing to bypass. A climb takes two
Cedar decisions where a same-scope proposal takes one — `ProposalOpen` at
the target and `MemoryRead` at the source, each against that resource's
own chain suffix under that scope's effective pack — and every later act
(`ProposalRead`, `ProposalReview`, `ChannelPublish` + `MemoryRead` at
publication) decides at the target exactly as FLOW-3's do. No new action,
no new permit, no pack edit: the packs answered these questions before
this feature existed, and the privacy floor's `resource.kind != "user"`
clause is what makes "nobody climbs another principal's personal
material" true without a line of code about personal scopes.

No schema change. `vedaflow_proposals.source_scope_id` has been stored
and immutable since migration 0019, and both it and `target_scope_id` are
already in that migration's transition trigger, so a closed proposal
still cannot be re-pointed at another scope. Every read and write
continues to run inside `rls::begin_tenant_tx`; the ancestor check runs
against the tenant-keyed HIER-2 cache, whose miss query filters on
`tenant_id` in SQL rather than relying on the RLS backstop (ADR-0009).

The audit trail gains no action and two fields. Both scopes appear on
every proposal event and on `ChannelPublished`, so an auditor reading the
chain can see what climbed, from where, to where, under which pack, with
which requirement resolved and which roles counted — and a denial at any
level is either `ProposalRejected` with its mandatory reason or an
`authz.decision` deny, both already chained. A second action asserting "a
climb happened" would be a fact an auditor has to reconcile against the
first (ADR-0019 decision 4), and the source scope on the existing event
is the same fact with nothing to reconcile.

On the read path, `context.injected` records exactly what it recorded
before — every composed entry's record id, object address, and channel,
plus each planned scope's `MemoryRead` verdict and its published channel's
commit. Nothing about the event shape changes, and the address is what
ties a climbed entry back to the tree that admitted it.

What does change is which scope a composed entry belongs to.
`ComposedEntry::scope_id` now means the scope it *composed from* rather
than the scope it lives at, and the rendered block is sectioned by the
same value — so a climbed record appears under the publishing scope's
header and a reader is never shown a section for a scope they cannot see.
That is deliberate on both counts: the publishing scope is the one whose
`MemoryRead` decision admitted the record, so it is the honest answer to
"why was this in that block", and rendering the residence would leak a
source scope's path through the very mechanism that promoted its content.
The record's own scope remains one read away, on the record.
