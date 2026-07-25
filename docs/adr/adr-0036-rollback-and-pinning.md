# ADR-0036: Rollback & pinning — a rewind may only install a state the channel has already held, and a pin freezes what a channel serves without moving where it points

- **Status**: Accepted
- **Date**: 2026-07-25
- **Feature(s)**: FLOW-7
- **Deciders**: sujitn

## Context

FLOW-7's text is three clauses — "ref rollback; agents heal next session;
assets pinnable to a commit per scope" — and its acceptance criterion is
"bad-prompt rollback demo <60s to fleet-wide effect". Tech plan §2.5 states
the promise it is the mechanism for: "**Rollback**: bad prompt shipped?
`refs` move back one commit; every consuming agent heals on next session
start." Seed §4.3 makes it a property of all four managed asset types:
"All four: versioned, scope-attached, policy-gated, auditable, **with
rollback**."

Most of the mechanism is already built and has been since FLOW-1.
`force_update_ref` exists, is a separate function with its own name so that
"no rollback is ever a typo" (ADR-0030 decision 11), and is still a
compare-and-swap. Composition reads the published channel per request with
no cache (ADR-0031 decision 3), so "agents heal next session" is not a
feature to build but a property to demonstrate: the very next `POST
/v1/inject` composes whatever the ref points at when it runs. And ADR-0031
decision 1 kept `vedaflow_refs` deliberately generic — "FLOW-3's proposal
refs and FLOW-7's pins need names this table should not have opinions
about".

So this feature is small in code and almost entirely about **what a rewind
is allowed to do**, which is the part nothing has decided yet. Four forces:

- **A rewind is the one governed act that removes trust without a
  review.** Everything else that changes what an agent is told crosses the
  approval matrix: FLOW-2's publish resolves it with the actor as the only
  approver, FLOW-3's proposals resolve it with a quorum, FLOW-5's climbs
  resolve it at the target. A rewind resolves nothing. That is either a
  hole or a consequence of a property nobody has stated.

- **"Move back one commit" is not implementable as written.** A channel's
  commits form a DAG, not a line. Since FLOW-3 a publication through a
  proposal is a *merge* commit whose second parent is the proposal commit
  (ADR-0032 decision 10), so the set of commits reachable from a channel
  head includes commits the channel never pointed at — and a proposal
  commit's tree is the proposed member set at the moment of *opening*,
  which may have been rejected. FLOW-1's `is_ancestor`, the fast-forward
  test, admits every one of them.

- **The blast radius is the whole point and the whole risk.** A rewind at
  a scope changes what every agent in that subtree is told, on their next
  session, with no further human act. That is exactly what the 60-second
  acceptance criterion asks for, and exactly why the question "which
  commits may it install" cannot be answered with "any of them".

- **ADR-0034 left one question explicitly to this ADR.** Reversal trigger
  (c): "FLOW-7 landing → a rewind at the source does not un-publish at the
  target, so a climbed record survives its source's rollback; whether that
  is right is FLOW-7's decision and this ADR does not presume it."

The pinning half is a different shape. "Assets pinnable to a commit per
scope" is the *stability* counterpart to rollback: rollback is the
publisher fixing history for everyone, a pin is a scope refusing to move.
PRMT-1's acceptance criterion names the same thing from the consumer's
side ("consumer pins channel or commit"), and tech plan §2.5's
reproducibility claim — "`inject` responses cite commit hashes" — is what
makes a pin expressible at all: the commit is already in the watermark.

## Decision

**A rewind may only install a state the channel has actually held.** The
target must be a strict first-parent ancestor of the current head:
`write_channel` puts the head first in every commit's parent list, so
first-parent ancestry is exactly the sequence of states the ref has been
in, and every one of those states was approved under the approval matrix
at the time it was installed. Rewinding therefore needs no approvals of
its own — it takes `ChannelRollback` and the asset kind's read action, and
that is all.

**A pin is a ref named `pin/{asset}/{channel}` pointing at the commit that
channel serves.** It freezes what the channel *serves* without moving
where it points: publications keep landing, the head keeps advancing,
readers stay at the pinned commit until it is released. The read path
coalesces — pinned commit if a pin exists, head otherwise — in the query
it already runs, and the block's watermark says which it got.

### 1. The rewind target is the channel's own first-parent history

Not "any reachable commit", which is what FLOW-1's `is_ancestor` answers
and what a naive rollback would use. `first_parent_history` walks
`vedaflow_commit_parents` at `ordinal = 0` only, and
`is_first_parent_ancestor` is the same walk as a predicate.

The rule is short to state and load-bearing: **the first parent of a
channel commit is the state that commit replaced.** `write_channel` has
built its parent list that way since FLOW-2 — head first, then merge
parents "so the channel's own line is unbroken" — and FLOW-3 relied on the
same ordering for the fast-forward check. Following ordinal 0 from the head
therefore enumerates precisely the states the ref has held, newest first,
back to the channel's first commit.

The commits this excludes are the ones that make the distinction matter.
A proposal commit is the second parent of every publication that came from
a review; it is reachable, its tree is a real tree, and installing it would
set the channel's membership to a proposed set — possibly one that was
rejected and then re-proposed differently, possibly one whose approvals
bound different bytes. Nothing about it was ever a published state.

### 2. A rewind rewinds; it never advances

The target must be a *strict* first-parent ancestor. Moving the ref forward
— including undoing a rewind — is not this route's job.

This is a restriction rather than an oversight, for a reason that is not
obvious: a first-parent *descendant* of the current head need not be a
state the channel ever held either. `write_channel` retries a lost
compare-and-swap up to three times, and each attempt mints its commit
before attempting the swap, so a channel under contention leaves orphan
commits parented on a head they never replaced. Admitting descendants would
make it possible to install a member set that no publication ever
installed, which is the same failure decision 1 exists to prevent, arriving
from the other side.

Recovery from a mistaken rewind is therefore **publishing**: the records
are still there, the ordinary route re-admits them, and the approval matrix
resolves again. That is the right price. Re-admitting content across the
trust boundary is the thing the matrix guards, and a rewind that could be
undone by another rewind would be a way to reinstate a set without ever
resolving it.

### 3. `ChannelRollback` is its own action, and it does not resolve the approval matrix

Two reasons, either sufficient.

**Separability.** Reusing `ChannelPublish` would make "may publish" and
"may rewind fleet-wide" one grant that no pack could ever separate. The
packs grant them identically today — curator, steward, org-admin,
pack-uniform like the rest of the channel plane — but a pack that wants
publication broad and rewinds narrow can now say so, and a Cedar action is
the only place that sentence can live.

**The 60 seconds.** Resolving the matrix would put `regulated-strict`'s
department-and-org requirement — a curator *and* a steward, two distinct
people (ADR-0032, as FLOW-5 exercised it) — in front of an incident
response. A product whose answer to "a bad instruction is reaching every
agent right now" is "convene two people" has not shipped rollback.

The matrix is safely absent because of decisions 1 and 2, and only because
of them: every state a rewind can install cleared the matrix when it was
installed. A rewind moves the channel *back* through states that were each
approved; it cannot conjure one that was not.

The second decision — the asset kind's read action, `MemoryRead` for
memories — is ADR-0031 decision 12's rule unchanged: nobody governs
material they cannot read. It is what keeps a team's curator out of a
teammate's personal published channel, through the privacy floor, with no
clause about personal scopes anywhere in this feature's code. Asset kinds
whose read action does not exist yet (prompts, skills) are refused rather
than governed by the wrong one; PRMT-1 and SKIL-1 bring theirs.

### 4. A rewind moves one ref, and only one

This discharges ADR-0034 reversal trigger (c), and it answers it the way
that trigger anticipated: **a climbed record survives its source's
rollback.**

A record that climbed from `acme/eng/platform` to `acme/eng` is named by
the department's published tree at its own address, admitted by the
department's approvers under the department's matrix. A rewind at the team
does not touch it, and a platform engineer still receives it — as the
*department's* published material, sectioned and labelled that way by
ADR-0034 decision 6's nearest-scope rule, which is true rather than a
leak.

The alternative is a cascade, and a cascade is the veto ADR-0034 decision 5
refused, running downhill instead of uphill: a team curator would be able
to un-publish at the org, undoing a decision the org's own stewards made,
by rewinding their own channel. The remedy at the department is a rewind at
the department, by principals the department's pack permits. This is the
same shape as everything else in FLOW: authority is per scope, and the
channel a scope owns is the only thing its principals move.

### 5. A pin is a ref, so pinning needs no table

`pin/memory/published` at a scope points at the commit that scope's
`memory/published` serves. `vedaflow_refs` has been generic since FLOW-1
for exactly this (ADR-0031 decision 1 names FLOW-7's pins), the commit
foreign key means a pin can only ever name a commit that exists, and the
primary key `(tenant_id, scope_id, name)` means a scope has at most one pin
per channel without a constraint being written for it.

The name cannot collide with a channel: `ChannelRef::from_str` splits on
the first `/` and parses the halves, so `pin/memory/published` fails on
`pin` not being an asset kind — the same refusal that already keeps
FLOW-3's proposal refs out of the channel listing.

Pins apply to **set** channels only. A log channel's tree is one commit's
additions rather than its membership (ADR-0031 decision 3), and composition
sweeps derived material out of `records` rather than reading that ref at
all, so pinning it would freeze nothing.

### 6. A pin freezes what a channel serves, not where it points

Publishing to a pinned channel succeeds, the ref advances, history is
unbroken. Readers stay at the pinned commit. The publish response reports
the standing pin, because a curator who publishes and sees no effect must
be told why rather than left to discover it.

The rejected alternative is reader-side pinning: a scope pinning an
*ancestor's* channel for its own members ("the platform team holds the
org's prompts at v3"). It is the more powerful feature and it is what
PRMT-1's "consumer pins" phrasing suggests, but it fails on authority and
on the read path at once. Authority: the pin would be a decision by people
who neither own the content nor answer for it, and there is no action in
the vocabulary for "govern what someone else's channel serves me". Read
path: a scope's published channel would resolve differently for different
callers, so "what did this scope publish on date D" stops having one
answer, and the watermark stops being a fact about the channel. A scope
that wants its ancestors held has a real remedy already — ask them, or run
`published-only` and pin its own.

### 7. Exactly one thing decides what readers see

The pin, when there is one. Therefore **a rewind of a pinned channel is
refused** with `Conflict`, naming the pin's commit and who set it.

The asymmetry with publish is deliberate and comes from what each act
promises. A publication's contract is "this channel now holds these
records", which stays true under a pin — the tree is the tree — so it
succeeds and reports. A rewind's contract is the FLOW-7 sentence itself:
every consuming agent heals on next session start. Under a pin that is
false, and a well-formed request against a world where the caller's
assumption does not hold is `Conflict` — the code FLOW-5 settled on for
exactly this shape. Releasing the pin is one call, and the refusal names
it.

### 8. Unpinning deletes the pin ref, and migration 0021 grants exactly that and nothing more

`vedaflow_refs` has held no DELETE grant since FLOW-1: "a ref is a standing
channel pointer, created once per scope per asset type, and disposal is
TEN-5's". A pin is the first ref that is a standing *decision* rather than
a pointer into history, and a decision that cannot be reversed is not one
this product should write.

The grant is narrowed twice over, because widening a deletion power on the
table that holds every channel pointer is the kind of thing that gets
noticed years later: a *restrictive* RLS delete policy that admits only
names beginning `pin/`, and a `before delete` trigger that raises on
everything else. The policy is what the product runs under — the
application role can never delete a channel ref, and the statement matches
nothing rather than being refused. The trigger is what an attacker has to
disable first: anyone who bypasses RLS gets an exception naming the rule.
That is migration 0018's own split between the path the product uses and
the path tampering needs, and a truncate trigger closes the statement that
would otherwise take every pointer at once.

### 9. The pin's reason lives in the audit chain, not on the ref

Three audit actions join the vocabulary:
`vedaflow.channel.rolled_back`, `vedaflow.channel.pinned`,
`vedaflow.channel.unpinned`. The rewind's payload carries the commit
abandoned, the commit installed, how the membership changed, and the
operator's mandatory message; the pin's carries the commit held and the
reason.

The ref itself records who and when (`updated_by`, `updated_at`) and
nothing else. The rejected alternative was a pin *log channel* inside
VedaFlow — commits whose messages are the reasons, giving pins a native
history for free. It costs three joins and a decode on the p99-bounded
inject path to answer "is this pinned", and it makes VedaFlow a second
source of truth for a question the audit chain answers for every other
governed act in the product. AUD-2's query surface is where "why is this
pinned, and by whom" is asked, alongside "who approved this publication".

### 10. The block discloses the pin

`ChannelWatermark` gains `pinned`, and it rides the inject response and the
`context.injected` event.

A watermark that cites a frozen commit without saying it is frozen is worse
than no watermark: the whole reproducibility claim (tech plan §2.5) is that
a block's citations answer "what did the agent know", and a reader who
cannot tell a pinned channel from a current one will read "the latest
reviewed material" into a block that deliberately is not. This is the same
discipline as CTX-3's degradation header — a response that quietly differs
from the one the caller expects has to say so.

### 11. History is a route, because you cannot rewind to a commit you cannot see

`GET /v1/channels/{scope}/history` under `ChannelRead`: the first-parent
walk from the head, newest first, each entry carrying its commit, its
author, its message, when it was committed, how many members it held, and
the proposal it was the effect of when it had one. Bounded by a `limit`
with a hard ceiling, because it is a walk.

It renders exactly the set decision 1 admits, so the surface an operator
reads and the set the route accepts cannot drift apart: if it is on the
listing it can be rolled back to, and if it is not it cannot.

### 12. `synveda channel` is HTTP-only, on FLOW-6's precedent

`history`, `rollback`, `pin`, `unpin`, and `list` are gateway calls under
the bearer `synveda login` stored. No `--database-url`, no SQL, no
break-glass row. ADR-0035's argument transfers without amendment: a rewind
is an act whose authority is `ChannelRollback` at a scope, whose actor is
an identity with roles, and whose event is chained by the gateway. A CLI
that moved the ref directly would have to invent an identity for
`updated_by` and would leave no decision in the trail — for the act with
the largest blast radius in the product.

## Options considered

1. **`is_ancestor` as the rewind rule** (FLOW-1's fast-forward test, run
   backwards) — already written, already tested, one line. Rejected: it
   admits proposal commits, whose trees are proposed member sets that may
   never have been approved. The distinction between "reachable" and "was
   a state" only exists because FLOW-3 made publications merge commits,
   which is recent enough that reusing the older predicate would look
   correct.

2. **Rollback resolves the approval matrix like a publication** — one
   matrix in front of everything that changes the channel, which is
   ADR-0032 decision 8's stated ideal. Rejected on the acceptance
   criterion: `regulated-strict` asks for two distinct people at a
   department, and an incident response that needs a quorum is not a
   rollback. Safe to omit only because decisions 1 and 2 confine a rewind
   to states the matrix already cleared — the ADR's most load-bearing
   dependency between decisions.

3. **Rollback reuses `ChannelPublish`** — no new action, no pack edits, no
   new column in the role×action matrix. Rejected: it makes "may add
   reviewed content" and "may retract it fleet-wide" the same grant
   forever. A product that sells governance should be able to express the
   difference even if every shipped pack chooses not to.

4. **A rewind cascades to scopes that climbed the retracted records** —
   the intuitive reading of "un-publish", and it makes a rewind a complete
   remedy. Rejected: it hands a team curator a veto over the org's own
   approvals, which is precisely what ADR-0034 decision 5 refused when it
   declined to enforce a ladder. Decision 4's rule is the same principle
   pointed downhill.

5. **A pin as a column on `hierarchy_nodes`, or a `channel_pins` table** —
   a pin is configuration, and configuration lives in tables. Rejected:
   ADR-0031 decision 1 already reserved the ref namespace for it, a table
   would need its own foreign key to commits to be as safe as the ref
   already is, and the read path would gain a join where the ref costs a
   coalesce in a query that is already running.

6. **A pin as a log channel inside VedaFlow** — pins get commit messages,
   authorship, and their own history in the substrate built for exactly
   that, with no DELETE grant anywhere. Rejected on the read path: ref →
   commit → tree → object → parse a hash out of the bytes, on every
   inject, to answer a boolean. The audit chain already records the reason
   (decision 9).

7. **Reader-side pins** (a scope pins the channels it consumes, including
   its ancestors') — the more powerful feature, and PRMT-1's phrasing
   points at it. Rejected in decision 6: no authority in the vocabulary
   expresses it, and it destroys the property that a scope's channel has
   one answer for every reader.

8. **A pin blocks publication** ("frozen means frozen") — arguably what a
   release manager means. Rejected: it stops a scope from doing reviewed
   work while it holds its readers steady, which is the case pinning
   exists to serve. The head advancing under a pin is the feature, not a
   leak in it.

9. **Rollback silently succeeds on a pinned channel** — symmetric with
   publish, one fewer refusal. Rejected in decision 7: it returns 200 to a
   request whose entire meaning is a fleet-wide effect that did not
   happen.

10. **Do nothing — retraction by publishing a superseding record.** It is
    what MEM-5's supersession will make ordinary, and it needs no feature
    at all. Rejected because it cannot retract: the bad record stays on
    the published tree, keeps composing as reviewed material, and the
    remedy is bounded by the token budget rather than by the act. Tech
    plan §2.5's promise is a ref move, and a ref move is what heals a
    fleet in one act.

## Consequences

- **Positive**: the promise tech plan §2.5 makes is now a governed route
  with an audit action, and "agents heal next session" is measured rather
  than asserted. A rewind cannot install a state that was never approved —
  a property that holds by construction of the parent list rather than by
  a check somebody has to remember. Pinning gives a scope a way to hold its
  fleet steady that does not require anyone to stop working, and the
  watermark stops being able to overstate a block's freshness. No new
  table, and the only new grant is a delete narrowed to names that begin
  `pin/`. FLOW-1's `force_update_ref` finally has the caller it was written
  for, and ADR-0034 reversal trigger (c) is discharged with the answer
  recorded rather than assumed.

- **Negative / accepted trade-offs**: a mistaken rewind is undone by
  publishing, not by another rewind, which under `regulated-strict` at a
  department means re-convening the approvers — deliberate (decision 2),
  and the operator's fastest safe path is a pin at the good commit while
  they arrange it. Commits abandoned by a rewind stay in the store,
  unreachable from any ref: `verify` still recomputes them (it walks rows,
  not reachability), FLOW-8's export will have to decide whether they
  belong in a git mirror, and packing/GC remains the open question ADR-0030
  left open. A pinned channel is invisible to anyone who does not read the
  channel listing or a watermark, and the pin's reason needs the audit log
  until AUD-2. A rewind at a scope whose records climbed elsewhere is a
  partial remedy by design, and an operator who wants the record gone
  everywhere must rewind at each scope that admitted it — the history route
  shows which those are only one scope at a time. And the rewind's blast
  radius genuinely is the subtree: one call by one authorized principal
  changes what every agent under that scope is told, which is the feature
  and remains the risk.

- **Reversal triggers**: (a) an operator needing to reinstate a rewound
  state without re-review → a reflog (one row per ref move) is the shape
  that makes "states the ref has held" a stored fact rather than a walk,
  and it is a new table plus a write on the publish path, so it needs its
  own ADR; (b) reader-side pinning arriving as a real requirement (PRMT-1's
  consumers, most likely) → decision 6's two objections are what must be
  answered, and the authority half needs an action in the vocabulary
  before the mechanics matter; (c) the first-parent walk showing up in
  `GET /history` latency at a scope with a long channel → the walk is
  bounded by `limit` and indexed by the parents primary key, so the fix is
  a cursor rather than a table; (d) a tenant wanting a rewind to cascade
  to scopes that climbed the records → that is a cross-scope retraction
  with its own approvals at each target, which is FLOW-5's machinery
  pointed the other way and a new ADR, never a flag on this route;
  (e) pins being used as an ambient deployment mechanism (every scope
  pinned, permanently) → the channel becomes decorative and the honest fix
  is a policy-pack setting rather than a per-scope ref.

## Compliance notes

- **Audit.** Three new actions chain in the operation's own tenant
  transaction, atomic with the ref move (ADR-0019 decision 1). The rewind's
  payload carries both commit hashes, the counts of members dropped and
  restored, the record ids affected, the operator's message, and the
  `ChannelRollback` decision context; the pin's carries the commit and the
  reason. No payload carries record content, unchanged from FLOW-2. The
  chain is the pin's only history, which is a deliberate placement
  (decision 9) rather than an omission.

- **Isolation.** No new table. The pin ref lives in `vedaflow_refs` under
  the tenant policy already in force, and joins the adversarial RLS suite
  through the new DELETE path: a forged cross-tenant unpin, and a delete
  aimed at a channel ref, are both new cases. The completeness guard in
  `crates/synveda-store/tests/rls.rs` covers migration 0021 because the
  table it touches is already enrolled.

- **Policy.** Two new Cedar actions bring the embedded packs to `@8`.
  Both are scope actions — never tenant-level, like the rest of the
  channel plane — and both are pack-uniform for ADR-0031 decision 12's
  reason: how content crosses the trust boundary, in either direction, does
  not loosen per pack. `ChannelRollback` additionally requires the asset
  kind's read action at the same scope, so the privacy floor governs
  rewinds of personal channels with no code that mentions personal scopes.
  There is no path that moves a ref without a PDP decision: the CLI is an
  HTTP client (decision 12), and the store-level break-glass half of the
  CLI gains no channel verbs.

- **Read path.** Composition gains no query. The published-members read
  left-joins the pin ref and coalesces, so the inject path's round-trip
  count is unchanged, and `MemoryRead` is still decided per planned scope
  before any of it runs. A pin cannot widen what composes — it can only
  hold the membership at an older approved set — so no pin is a
  disclosure.
