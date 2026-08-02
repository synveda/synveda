# ADR-0049: prompts are the first authored asset — the draft is a row because `staged` has no writer, the name is the id, and a consumer's pin is a parameter a rewind refuses rather than outlives

- **Status**: Accepted
- **Date**: 2026-08-02
- **Feature(s)**: PRMT-1
- **Deciders**: sujitn

## Context

PRMT-1's text is "versioned, variable-schema'd templates; draft→review→
publish; consumed via API/SDK by id + channel", and its acceptance
criterion is one line: "prompt change behind review; consumer pins channel
or commit."

Everything upstream of this feature was built for material the *pipeline*
produces. `observe` writes at the caller's home scope, extraction lands
records there, and every governance surface since FLOW-1 has taken record
ids as its members. Seed §4.3 has listed four managed asset types since
day one and tech plan §2.3's diagram has an arrow labelled "manual
authoring (prompt, skill, context pack, pinned memory, policy)" pointing
straight at review — but nothing has ever travelled it. **Prompts are the
first asset a human writes rather than a pipeline derives**, and that is
the whole of what is new here.

Six accepted ADRs park obligations on this feature by name:

- **ADR-0031 decision 1** fixed the channel vocabulary as
  `{asset-kind}/{channel}` "and the same three for `prompt`, `skill`,
  `context-pack`, and `policy` as those asset types arrive", and its
  `ChannelMember::name` is documented as "a record id for memories, **a
  path for the authored asset types**". No path has ever been written.
- **ADR-0032 decision 2** settled that `staged` stays unwritten: "a set
  channel cannot express withdrawal — retraction is FLOW-7's
  `force_update_ref` by name". That decision was taken with only
  pipeline-derived material in the product. A draft is the first thing
  that would plausibly want it, so it is re-asked here and answered the
  same way, for a reason that is now specific rather than general.
- **ADR-0032's consequences** note the curator file's glob language "will
  need real path semantics when SKIL-1 and PRMT-1 bring path-named entries
  — the parser accepts the shape now so that growth is not a format
  change."
- **ADR-0035's consequences** predict that "PRMT-1's prompts and SKIL-1's
  skill bundles will need a per-asset-kind renderer behind the same seam,
  which is a new ADR rather than a widened `match`".
- **ADR-0036 decision 3** refuses prompts and skills at the rewind and pin
  routes "by name rather than governed by memory's read action until
  PRMT-1 and SKIL-1 bring theirs", and its decision 12 refused reader-side
  pinning — "a scope holding an *ancestor's* channel for its own members,
  which is what PRMT-1's 'consumer pins' phrasing suggests" — for two
  reasons this ADR has to either inherit or overturn.
- **FLOW-4's status note** records that multi-member promotion rules
  "cannot fire on anything the write path produces" and that they "need
  material at a shared scope, which arrives with the first authoring path
  (PRMT-1, context packs) or with FLOW-5's climb".

Two forces bound the design. The approval matrix has priced prompts since
FLOW-3 — `regulated-strict` asks for **a steward and a curator, two
distinct people**, straight off tech plan §2.4's "1 × dept steward + 1 ×
any curator (peer review)" — and not one cell of it has ever resolved,
because nothing could open a prompt proposal. And FLOW-7's headline
property is that a bad instruction stops reaching a fleet in under sixty
seconds, with "every consuming agent heals on next session start" as the
mechanism. A consumer pin is the obvious way to make that false.

## Decision

**A prompt is a draft row plus content-addressed objects, published
through the proposal path that already exists, and resolved by name up the
caller's own placement chain.** The registry adds no channel shape, no
proposal effect, no approval rule and no publication event. What it adds
is two PDP actions, one table, one pair of read/author surfaces, and one
new answer: **a consumer may pin a commit, and a rewind that takes that
commit off the channel refuses the pinned read by name rather than serving
withdrawn content or silently upgrading the consumer.**

Decisions, specifically:

1. **The draft is a row; its history is the channel's.** `prompts` is
   keyed `(tenant_id, scope_id, name)` and holds exactly one current
   authored version: template, variable schema, description, sensitivity,
   owner. It is not bitemporal and it is not a second history. Every write
   also puts the content-addressed object, so the bytes a proposal binds
   and a reviewer read are immutable and already stored; the versions a
   channel has *served* are its first-parent line, which is the same
   history FLOW-7 rewinds and `synveda channel history` renders. One
   history, in the place the product already keeps history.

2. **`staged` still stays unwritten, and now for a specific reason.**
   ADR-0032 decision 2 argued generally that a set channel cannot express
   withdrawal. A draft makes it concrete: an author who replaces a draft
   would leave the old one in a set that has no DELETE, and "what is
   drafted here" would drift from the truth the first time anybody changed
   their mind. `Channel::Staged` keeps its place in the vocabulary for a
   future set-shaped view; the authoring state is a row that can be
   overwritten, which is what authoring is.

3. **The name is the id.** A prompt is addressed by a path-shaped name —
   `support/triage-reply`, lower-case, `/`-separated, up to four segments
   — and that name is the channel tree's entry name, discharging
   ADR-0031's reserved "a path for the authored asset types" and giving
   ADR-0032's curator glob its first real paths. A consumer writes the
   name in source code; a uuid there would be unreadable, and worse, it
   would be *unique*, which destroys decision 8: the same name at two
   scopes is precisely how a team overrides the org's version.

4. **Two actions, `PromptRead` and `PromptWrite`.** `PromptRead` carries
   `context.sensitivity` — the AUTHZ-5 shape, the same closed four-value
   vocabulary — so a pack can price a `confidential` prompt exactly as it
   prices confidential memory. `PromptWrite` is the authoring seam and
   mirrors `MemoryWrite`: the home-scope floor role-free, bound content
   roles beyond it. Both are scope actions, never tenant-level, so
   AUTH-3's confinement forbid covers agents for free.

   This is what discharges ADR-0036 decision 3: `prompt/published` now has
   a read action, so rewinding and pinning a prompt channel is decidable,
   and the route's refusal-by-name shrinks to the two asset kinds that
   still have no feature.

   `PromptRead` deliberately carries **no** `lapsed` attribute. The lapse
   vocabulary is closed over `memory.read` (ADR-0037 decision 2) and
   widening it is a lapse feature's decision made in two reviewed places,
   not a side effect of adding an asset type.

5. **`restricted` prompts are unrepresentable.** The only mechanism in the
   product that mints that tier is a classification proposal over
   *records*, priced by the invariant floor at compliance plus two
   distinct approvers (ADR-0038 decision 8), and PRMT-1 ships no classify
   effect for prompts. Authoring one is refused by name at the surface,
   the way an extractor proposing the top tier is refused (ADR-0038). The
   base layer's `restricted` forbid is therefore left naming `MemoryRead`
   alone: extending it to an action no content can reach would be a rule
   about nothing, and the day a prompt can carry the tier is the day that
   forbid grows a name, in the ADR that adds the minting path.

6. **A prompt publication is an ordinary FLOW-3 proposal whose asset is
   `prompt`.** Same route, same lifecycle, same review log, same approval
   arithmetic, same `vedaflow.channel.published` event with `asset` reading
   `prompt`. No new effect: `ProposalEffect::Published` publishes members
   onto the target's published channel, and a prompt member is a member.
   What the publish path gains is a branch on the proposal's own asset
   kind — the per-asset-kind seam ADR-0035 predicted — deciding how
   members are named, where their current version is read, and how a
   reviewer's diff renders.

7. **The direct publish route takes prompts too.** ADR-0032 decision 8's
   invariant is that *every* path across the trust boundary resolves the
   same matrix, with the direct route as the degenerate case where one
   approval is enough. Refusing prompts there would have created a second
   rule — "prompts are special" — where the product already has a better
   one: under `regulated-strict` a direct prompt publish refuses on its
   own arithmetic, naming the steward and curator it is short of, and
   under `standard` a single curator may publish, which is that pack
   saying what that pack exists to say.

8. **Resolution walks the caller's placement chain, nearest first, and
   skips what the PDP refuses.** This is seed §4.4's specificity gradient
   applied to a fetch rather than a composition, and ADR-0024 decision 1's
   universe unchanged: the caller's own chain, not recall's wider one,
   because a prompt is consumed by name rather than searched for. A scope
   whose copy the caller may not read is skipped rather than fatal, so a
   nearer copy nobody may read does not shadow the org's — and a name
   nothing publishes gets the uniform `NotFound`, so the surface is never
   an existence oracle (ADR-0012 decision 7; CTX-4's handle refusal).

9. **The consumer's pin is a request parameter, and it is not the pin
   ADR-0036 decision 12 refused.** That one was a *stored* decision by one
   scope about what an ancestor's channel serves its members, refused
   because no action expresses "govern what someone else's channel serves
   me" and because it would make a scope's channel resolve differently per
   caller. This one is a query parameter on one read, stored nowhere,
   governing nobody else, and expiring with the request: it is a
   lockfile's `resolved` field, not a policy. "What did this scope publish
   on date D" still has exactly one answer.

10. **A pinned commit must still be a state the channel has held, and a
    rewind refuses the pinned read.** The pin is checked with
    `is_first_parent_ancestor` — FLOW-7's own rule, the states the ref has
    actually been in — against the head at request time. A publication
    leaves earlier pins resolving, which is the pin working. A rewind that
    takes the pinned commit off the line makes the read a `Conflict`
    naming both commits and the channel, because the two alternatives are
    both false statements: serving it anyway makes FLOW-7's sixty seconds
    a lie, and silently serving the head makes the pin a lie. A refusal is
    the only answer that keeps a rewind meaning what FLOW-7 says it means,
    and it reaches the consumer at their next call rather than at their
    next session.

11. **A pin freezes bytes, never authority.** The PDP decision is taken at
    request time against the live pack, at the tier the *pinned* version
    carries. CTX-4's handle rule restated: a commit hash is a name, not a
    capability, and a consumer who pins one gains nothing when the policy
    behind it changes.

12. **The variable schema is enforced where it can fail.** A template's
    placeholders and its declared variables must agree exactly: an
    undeclared placeholder is refused at authoring (a consumer cannot
    supply what the schema does not name) and a declared variable no
    placeholder uses is refused too (dead configuration every consumer
    would fill in for nothing). Rendering is a function in `synveda-types`
    — one implementation of the substitution rule, used by the CLI and
    available to the SDKs — that refuses a missing required value and an
    undeclared one rather than substituting an empty string. A schema
    returned beside a template and checked by nobody is a document.

13. **Every `{{` opens a placeholder.** It must close with `}}` on a
    declared name (surrounding whitespace trimmed); anything else is
    refused at authoring with the offending span named. The strict reading
    is deliberate: the alternative treats `{{ user name }}` as literal text
    and ships a typo to a fleet. The cost is that a template cannot carry a
    literal doubled brace, which is recorded as a limitation with a trigger
    rather than solved by an escape syntax nobody has needed yet.

14. **Two new audit actions, and no third.** `prompt.authored` for a draft
    write and `prompt.resolved` for a served read, carrying names, scopes,
    channels, commits, object addresses and tiers — never template text.
    Publication is `vedaflow.channel.published` exactly as a memory
    publication is, because it is the same governed act with the same
    consequence and a second action asserting it would be a fact an
    auditor has to reconcile (ADR-0019 decision 4).

15. **A draft read names its scope.** `channel=published` walks the chain;
    `channel=draft` requires an explicit scope and takes `PromptRead`
    there. Unreviewed content never reaches a consumer who did not ask for
    that scope's unreviewed content by name — the same instinct bank mode
    encodes for memory, expressed here as a parameter because a draft is
    not on a channel to exclude.

16. **A prompt climbs exactly as a memory does.** FLOW-5's two senses of
    "the source holds it" become: the draft row lives there, or the
    source's published tree names it at its current address. No new rule,
    no new column — which is the same evidence ADR-0034 decision 8 offered
    about `source_scope_id`, arriving from the asset side.

## Options considered

1. **Draft as a row, published through the existing proposal path
   (chosen)** — one history, one review surface, one matrix, and FLOW-6's
   CLI reviews prompts on the day it lands without a line changing. Con: a
   wire vocabulary where `channel=draft` names a row rather than a ref,
   which this ADR has to explain (decision 2).
2. **Draft on a `prompt/derived` log channel** — every version a commit,
   exportable by FLOW-8, and the word "channel" stays honest. Rejected on
   the read: "the current draft of X" becomes a parent-chain walk of
   unbounded length, so the row comes back anyway as a cache, and a log
   channel cannot express withdrawal any better than a set can.
3. **Draft on `staged`** — the channel the vocabulary already reserves and
   the one tech plan §2.3's diagram points authoring at. Rejected by
   ADR-0032 decision 2's own argument, now with a concrete instance: a
   replaced draft would linger in a set with no DELETE.
4. **Prompts as memory records with a reserved class** — no table, no
   action, no branch anywhere. Rejected three times over: the approval
   matrix already prices `prompt` differently from `memory` and would stop
   being able to, the asset kind is *inside* the object address by
   ADR-0030 decision 4 precisely so identical bytes governed differently
   are different objects, and a prompt is fetched by name where a record
   is ranked by relevance.
5. **A uuid id with a display name** — consistent with every other entity
   in the product. Rejected: the identifier goes in a consumer's source
   code, and a uuid there cannot express decision 8's gradient, where the
   *same* name at a nearer scope is the override.
6. **A server-side render route** — the rendered text would be auditable,
   and an SDK would have less to do. Deferred with a trigger: it puts
   prompt text through a second surface and turns a fetch into a
   computation, for a benefit nobody has asked for yet.
7. **Reader-side stored pins** (a scope pinning an ancestor's prompt
   channel for its members) — refused again on ADR-0036 decision 12's
   unchanged reasons; decision 9 records why the parameter is a different
   thing rather than a smaller version of the same thing.
8. **Serving a pinned commit after a rewind** — the consumer gets what it
   asked for, always. Rejected: it makes a rollback partial in exactly the
   case rollback exists for.
9. **Silently upgrading a pinned consumer to the head after a rewind** —
   nothing breaks, everyone heals. Rejected: then the pin means "the
   version I was built against, unless something happened", which is not a
   pin, and the consumer learns nothing.
10. **Prompts in `inject`'s index tier** — an agent would see the prompts
    available to it without asking. Deferred: CTX-4's tier names records
    that did not fit a budget, and asset advertisement is SKIL-4's shape
    (`inject index tier lists skills available to this identity`).

## Consequences

- **Positive**: the approval matrix's prompt cells resolve for the first
  time since FLOW-3 wrote them, and the peer-review requirement tech plan
  §2.4 specified becomes behaviour rather than a table row. FLOW-6's
  review CLI, FLOW-7's rewind and pin, FLOW-5's climb, AUD-2's
  disclosures and the audit chain all extend to prompts with no new
  concepts. `MAX_CHANNEL_MEMBERS`, `MAX_PROPOSAL_MEMBERS` and the uniform
  404 are inherited unchanged. FLOW-4's blocked multi-member rules now
  have a source of shared-scope material, as its status note predicted.
- **Negative / accepted trade-offs**: the wire vocabulary gains a
  `channel=draft` value that is not a `Channel`. A draft write leaves an
  object nothing references until a proposal names it — the same GC
  question ADR-0030 left open, not worsened in kind. A template cannot
  contain a literal `{{`. A prompt cannot be `restricted`, so the most
  sensitive prompt a bank can author is `confidential`. And a pinned
  consumer discovers a rewind as an error, which is a real operational
  cost paid deliberately for decision 10.
- **Reversal triggers**: (a) a tenant pinning commits widely enough that
  rewinds routinely break consumers → the pin needs a "nearest surviving
  ancestor" mode, which is a new parameter and a new ADR, not a change of
  default; (b) anyone needing a `restricted` prompt → a classify effect for
  authored assets, which brings the base-layer forbid a second action
  name; (c) a template legitimately needing literal `{{` → an escape
  syntax, which is a format change and re-addresses every object; (d) the
  draft row becoming something callers want history of → the objects are
  already immutable, so the change is a table of addresses, not a
  rewrite; (e) prompt resolution appearing on the inject latency budget →
  the walk is one indexed read per chain scope and can be folded into the
  composition query, which is why resolution reuses the cached chain.

## Compliance notes

- **PDP**: no path reaches a template without `PromptRead` at the scope
  that holds it, decided at the tier that version carries, at request
  time, under the live pack (decisions 4, 8, 11). Authoring takes
  `PromptWrite`; publication takes `ChannelPublish` plus the approval
  matrix plus `PromptRead`, which is the same three-part rule memory
  publication has obeyed since ADR-0031 decision 12.
- **Tenancy**: `prompts` is tenant-scoped, so it arrives with forced RLS,
  a tenant-isolation policy and least-privilege grants in its own
  migration, and joins the adversarial suite and its completeness guard
  (ADR-0009). Objects and channel refs are already per tenant, and
  content-addressed dedup has never crossed one (ADR-0030 decision 3).
- **Audit**: authoring and every served read chain events in the caller's
  own transaction; publication chains the event it always did. No payload
  carries template text — the discipline every plane has followed since
  AUD-1, and the reason a leak sweep over the chain is a test rather than
  an assertion.
