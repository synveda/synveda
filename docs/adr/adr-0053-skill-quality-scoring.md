# ADR-0053: a skill's quality score is two halves that are never averaged — a rubric recomputed from the bundle and a checklist bound to the bytes it was answered about — gated by a threshold a pack sets and released by the override the security gate deliberately refused

- **Status**: Accepted
- **Date**: 2026-08-03
- **Feature(s)**: SKIL-3
- **Deciders**: sujitn

## Context

SKIL-3's text is "SkillsBench-style rubric scoring (automated + reviewer
checklist) stored on the version", and its acceptance criterion is two
clauses: "score displayed at review and in the registry; low-score publish
requires override".

Two features have already been built on these bytes, and both of them left
this one something. ADR-0051 made a skill a draft row plus one
content-addressed object per file, published through an ordinary proposal.
ADR-0052 put a security scanner at the authoring seam and again at the
publish seam, and — unusually — wrote down what it thought this feature
would need. Its option 7 rejected storing the scan report on the version and
named the distinction to keep:

> a *score* is partly a human's checklist and cannot be recomputed, so it
> must be stored; a *scan* is a function of bytes the product already holds.
> If SKIL-3 brings that table anyway, a cached scan may join it — as a
> cache, keyed by ruleset version, never as the truth.

That is the right distinction and this ADR takes it, but it is only half of
the design, because it says *whether* to store without saying **what the row
is keyed by** — and that key is where this feature's first real decision
sits.

The other inheritance is a refusal. ADR-0052's sharpest recorded edge is
that the security gate has **no rule-level exception mechanism**: a
`critical` false positive is unpublishable until the rule is fixed in a
release, and the recorded shape for relief was a lapse that does not exist
on this plane. This feature is asked, in its own acceptance criterion, to
build an override. Whether that is the same escape hatch arriving one
feature late, or a different thing that happens to share a name, is a
question this ADR has to answer rather than sidestep.

Four forces bound the design.

1. **The two halves have opposite durability.** An automated check over a
   bundle is a pure function of (file bytes, rubric version) — the same
   shape as the scan, recomputable anywhere the bytes are, stale the moment
   the rubric moves. A reviewer's checklist is a person's judgement about
   those bytes on a particular afternoon; nothing can recompute it, and a
   product that lost it would be asking the next reviewer to do the work
   again. One belongs in no table at all and the other belongs in one, and
   the feature text's "stored on the version" is true of exactly one of
   them.

2. **A checklist bound to anything but the bytes is a checklist an edit
   launders.** ADR-0032 decision 6 is that approvals bind bytes: every
   member's address is re-verified at publication because content that
   moved after a review is content nobody reviewed. A checklist is a review
   artefact with exactly that exposure and no such protection — a reviewer
   answers "yes, somebody ran it", the author pushes a new `scripts/run.py`,
   and the answer is still sitting there attached to a proposal id or a
   skill name, describing a bundle that no longer exists.

3. **A quality threshold with no way past it is a threshold that gets routed
   around.** This is the force that separates this gate from SKIL-2's, and
   the separation is not a matter of degree. `critical` has no legitimate
   reading — that is the definition of the band — so a pack that could
   switch it off is a pack that can make the product's central claim false
   silently, and refusing an exception costs an author only the wait for a
   rule fix. A *low score* always has a legitimate reading: the rubric is a
   proxy, the skill may be excellent and unusual, and the organisation may
   simply need it today. A gate on a proxy with no escape hatch does not
   raise quality; it teaches people to publish somewhere else.

4. **The rubric will be wrong, and here that is affordable.** ADR-0052
   decision 10 kept the blocking band to what a lexical rule can decide with
   certainty, because a scanner that guesses at what it refuses is a scanner
   whose refusals get routed around. A rubric guesses by construction —
   "has examples" is a heuristic about a document, not a fact about it. The
   difference that makes this safe is the output: a rubric's answer is a
   number a human reads beside the file, and a wrong number costs a
   reviewer thirty seconds. It only stops being affordable at the point
   where the number *decides* something, which is precisely where force 3
   puts the override.

## Decision

**Two halves that are never summed into one number: an automated rubric
recomputed from the bundle wherever it renders, and a reviewer checklist
stored in one new table keyed by a digest of the bundle's own object
addresses. A pack sets the threshold and whether a checklist is required;
below either, publication is refused until somebody holding a new PDP
action records an override with a reason — a separate act, because the roles
that publish a skill and the roles that may excuse one are disjoint by
design.** Migration 0033 adds two tables and two cache columns; two new
audit actions; one new PDP action.

Decisions, specifically:

1. **The score is a pair, and the pair is never averaged.** A bundle
   carries an automated `score` (0–100 over the rubric) and, separately, a
   `checklist` that is either absent or a set of answers. Collapsing them —
   the obvious design, and the one the phrase "a score" invites — buys one
   number to sort a registry by and pays for it by making each half able to
   hide the other: a bundle with a perfect rubric and an unanswered
   checklist scores the same as one a reviewer worked through, and a
   reviewer's "these instructions are wrong" becomes fifteen points rather
   than the thing it is. A human's judgement summed into an average is a
   human's judgement laundered into arithmetic. The gate below reads both
   and names which one refused.

2. **The automated half is recomputed at every seam that renders it, and
   stored nowhere a decision is made.** ADR-0052 decision 6 inherited
   whole and for its reasons: it is a pure function of (file bytes, rubric
   version), both already present wherever it is needed. `RUBRIC_VERSION`
   rides every report and every payload, because a rubric that did not say
   which table produced it could not be compared with one taken at review
   time — ADR-0052 force 4, restated one plane over.

3. **The one exception is a cache, in the registry listing and nowhere
   else** — which is ADR-0052 reversal trigger (e) arriving exactly as it
   was written. `skills` gains `quality_score` and `rubric_version`, written
   at authoring, because a registry listing at a scope with forty skills
   would otherwise read every object of every bundle to draw a column. The
   discipline that keeps it a cache rather than a truth is two lines long:
   a cached score whose `rubric_version` is not the compiled-in one is
   rendered as **stale** rather than as current, and **no gate ever reads
   these columns** — the publish seam recomputes, always, from the bytes it
   is about to publish. A cache that a decision reads is not a cache.

4. **The checklist is stored, and its key is a digest of the bundle's
   object addresses.** Force 2. `bundle_digest` is BLAKE3 over the
   domain-separated, path-sorted `(member name, object address)` pairs — a
   tree hash by another name, and deliberately over *addresses* rather than
   raw file bytes, because ADR-0051 decision 2 put the governed context
   (scope, skill, sensitivity, path) inside each object's address. So
   reclassifying a bundle from `internal` to `confidential` re-keys its
   checklist, which is correct: a reviewer who signed off on an internal
   skill did not sign off on a confidential one.

   What this buys is that the checklist needs **no invalidation logic at
   all**. An edited bundle has a different digest, so the answers are simply
   not found — the same thing content-addressing buys everywhere else in
   this product, applied to the one review artefact that was going to need
   an `updated_at` comparison and a staleness bug. The answers are not
   deleted; they remain attached to the bytes they were true of, which is
   what makes the audit trail readable backwards.

5. **Eight automated checks, weighted by confidence × consequence, summing
   to 100.** The weights are not importance — they are how certain the
   check is, multiplied by what its failure costs, which is force 4 made
   mechanical:

   | Check | Weight | What it decides |
   |---|---|---|
   | `description-states-when` | 20 | the description says *when* to reach for the skill, not only what it is |
   | `no-placeholders` | 20 | no `TODO`, `FIXME`, `XXX`, `TBD`, `lorem ipsum`, `<placeholder>` anywhere in the bundle |
   | `manifest-concise` | 15 | `SKILL.md` is within the progressive-disclosure budget |
   | `has-examples` | 15 | at least one fenced code block |
   | `has-structure` | 10 | at least one `##` section |
   | `references-resolve` | 10 | every path `SKILL.md` names *into a directory the bundle owns* exists |
   | `description-length` | 5 | informative, and under the spec's own cap |
   | `files-referenced` | 5 | every bundled file is named somewhere in `SKILL.md` |

   `description-states-when` is joint-heaviest because it decides whether
   the skill is ever loaded at all: a client reads descriptions at ~80
   tokens to choose among them, and SKIL-4 will advertise this same line.
   `no-placeholders` joins it because it is the most nearly *decidable*
   check here — a marker is present or it is not — over the bundle about to
   reach a fleet of laptops. `files-referenced` carries least because it is
   most likely to be wrong: a helper imported by another script rather than
   named in the manifest is a legitimate bundle it marks down, and five
   points is what that mistake is allowed to cost.

   **This table was corrected by measurement before it shipped, and the
   correction is the most useful thing in this ADR.** `references-resolve`
   was drafted at 20 — the heaviest — on the reasoning that a path either
   is in a bundle or is not. Run over the 37 installed bundles SKIL-1 left
   as a standing corpus, it fired on **29 of them**, and almost none were
   broken. What real manifests name are files in the *user's* project the
   skill will read (`CLAUDE.md`, `package.json`, `.mcp.json`), illustrative
   paths inside examples (`src/api/users.ts`, `path/to/file.rs`), and files
   the skill instructs the agent to *create* — none of which are claims
   about the bundle. It also read `Node.js` and `Next.js` as filenames,
   because `.js` is an extension.

   A manifest mentioning a path is not a claim that the bundle contains it,
   and separating the readings is not lexically decidable — ADR-0052
   decision 10's line arriving one plane over. So the claim is narrowed to
   where it *is* decidable: a path counts as a reference only if it is
   multi-segment and its first segment is a directory the bundle actually
   ships. `scripts/check.py` in a bundle with a `scripts/` directory is a
   reference; `package.json` is not. The check now fires on 2 of 37, and it
   carries 10 rather than 20 — because a check that turns out less certain
   than it looked should get cheaper in the same commit, rather than keep a
   weight its accuracy does not earn.

   `MANIFEST_BUDGET_CHARS` was set the same way and was wrong the same way.
   Drafted at 8,000, it failed 21 of 37 bundles — the corpus's median
   manifest is 8.4K, so the check was firing on the *typical* skill. It is
   now 16,000, near the corpus's 75th percentile: a quality signal that
   fires on the median measures the number somebody picked rather than the
   bundle. Across the corpus the rubric now runs 50–100 with a mean of 85
   and 10 bundles at full marks, which is the distribution a score has to
   have to be worth rendering — it separates bundles without judging the
   ecosystem.

6. **Five checklist items, and each is something no machine can answer.**
   `instructions-correct` (the procedure is right for this organisation),
   `scope-appropriate` (it belongs at this scope rather than nearer or
   further up), `not-duplicate` (it does not repeat one already published on
   this chain), `dependencies-available` (the tools and APIs it assumes
   exist for the people who will be served it), `tested` (somebody ran it).
   Each answered `yes`, `no` or `n/a`. The list is deliberately short and
   deliberately not configurable in this feature: a checklist a tenant can
   extend is a checklist whose stored answers stop being comparable across
   scopes, and nothing yet needs that.

8. **Three reasons a publication needs an override, and the refusal names
   which.** Publication is refused unless the pack's threshold is met on
   every one of: the recomputed score is at or above `min_score`; a
   checklist exists for exactly these bytes, if the pack requires one; and
   that checklist carries no `no`. The third is what makes the second worth
   having — a checklist whose answers nobody has to act on is a form, and
   an answered "no, these instructions are not correct" followed by an
   unremarked publication is the exact failure the feature exists to
   prevent.

   **Amended 2026-08-04 by ADR-0056 decision 6, on the arrival of a second
   renderer.** A `QualityShortfall` serialised its *data* and not its
   sentence, and the CLI composed the prose, so that a reader was never
   shown a `kind` slug to look up. That was right against the forces this
   ADR had — one client, and a gateway that would have been serialising
   layout. With CNSL-1's console it becomes a drift source: the same
   shortfall explained in two languages by two authors, with nothing able
   to fail when they diverge. The report now carries `detail` beside the
   data — [`QualityShortfall::describe`], the sentence the refusal at
   publication and the audit payload already used — and both surfaces
   display the served one. The data stays on the wire, so a client that
   wants to lay out the arithmetic itself still can; what stops being
   duplicated is the *wording*.

9. **The override is a PDP action, `SkillQualityOverride`, spent through a
   separate governed act with a mandatory reason** — and this is where
   force 3 is spent. `POST /v1/proposals/{id}/quality-override` records the
   decision; the publish seam then *looks it up* rather than being told
   about it. Without one, a publication below the bar is a `Conflict`
   naming which bar and what would clear it.

   The action is separate from the `ChannelPublish` the publication takes,
   so a publisher who may ship a good skill cannot necessarily ship a bad
   one. That is ADR-0051 decision 18's argument in its own idiom: the
   content of separating two authorities is that they can be two people.

   **The first design put the override on the publish request, and it was
   wrong — not stylistically, but unusable.** Under every product pack
   `curator` holds the `SkillRead` and `ChannelPublish` that publishing a
   skill takes, and `steward` holds this action and *no content read at
   all*. Requiring one principal to hold both meant nobody could publish a
   below-bar bundle under any pack: a wall rather than a gate. The
   acceptance test found it, which is the argument for writing the AC test
   against the roles the packs actually grant rather than against an
   omnipotent fixture.

   Splitting the act is not a workaround for that; it is ADR-0032
   decision 9's own shape, the one that already separates "the approval
   that decides" from "the act that runs the effect" for exactly this
   reason. The authority records the override, the publisher spends it, and
   the override binds bytes like everything else here — an override granted
   over one bundle does not follow the author's next edit, because nobody
   agreed to ship whatever it became.

   A role check written inline in the handler would have been three lines
   and would have been a policy decision made outside the PDP, which seed
   §2.2 forbids and CLAUDE.md restates. The packs decide who may override,
   which is where "relaxable by design" belongs.

10. **A pack-carried `SkillQualityConfig` with two fields, and its fail-safe
   is no gate at all.** `min_score: u8` and `require_checklist: bool`,
   riding `PackConfig` beside `scan` exactly as ADR-0052 decision 9's
   config rides beside `redaction`. `regulated-strict` ships
   `{min_score: 70, require_checklist: true}`; `standard` ships
   `{min_score: 50, require_checklist: false}`; `open-collaboration` and an
   unconfigured stored pack get `{min_score: 0, require_checklist: false}`,
   which is no gate — `score < 0` is never true.

   Two fields rather than ADR-0052's one, because they gate two different
   things and one number cannot express both: `min_score` is a bar on a
   machine's measurement, `require_checklist` is whether a human's
   judgement had to be recorded at all, and a pack that wants the second
   without the first (an SMB that trusts its people but wants the review to
   have happened) is a coherent position the product should be able to hold.

   The fail-safe is the **opposite** of ADR-0052's and the difference is the
   whole distinction between the two gates. There the unconfigured reading
   was the invariant floor, because a pack that says nothing must still not
   ship a credential stealer. Here there is **no floor**, because quality is
   not an invariant: a pack that says nothing about quality has not asked
   for a quality gate, and a product that started refusing publications on a
   rubric nobody opted into would be a product that broke every tenant on an
   upgrade.

11. **Two new audit actions.** `skill.checklist.recorded` chains a
    reviewer's answers with the digest they are bound to — the durable
    record of the human half, and the reason the table's mutability costs
    nothing (a row is last-writer-wins, a chained event is every writer).
    `skill.quality.overridden` chains the override with the reason, the
    score, which of the three bars was missed, the pack that set it, and the
    identity that held the action. That second event is the feature's most
    valuable output for an auditor: "what did we ship that we knew was below
    the bar, and who said so" is a question no other event in the product
    answers.

12. **The score renders in three places, and it is a statement about a
    bundle rather than about its author.** `ProposalDetail` gains a
    `quality` field beside SKIL-2's `scan`, recomputed over the same member
    bytes, with the checklist looked up by the digest of exactly those
    members; the CLI's review block renders both, quality after the scan,
    because a reviewer decides *whether it is safe* before *whether it is
    good*. `SkillView` carries it on every author, so an author sees what
    they scored before a reviewer does. The listing carries the cached pair
    from decision 3. Nowhere does a report name the author, aggregate by
    author, or persist a per-identity number: this is a rubric over a
    document, and the first time it becomes a metric about a person is the
    last time anybody writes an honest checklist.

## Options considered

1. **Two halves never averaged, rubric recomputed, checklist keyed by a
   bundle digest, threshold from the pack, override as its own PDP action
   (chosen)** — every mechanism is one the product already has: the
   recompute is ADR-0052 decision 6, the digest key is ADR-0032 decision 6's
   "approvals bind bytes" applied to a review artefact, the pack config is
   ADR-0021 decision 3's shape, and the second decision at one seam is
   ADR-0032 decision 9's. Con: it is two numbers where the feature text says
   one, and every surface that renders it has to render both.
2. **One number, automated and checklist averaged** — what "score" most
   naturally means, sortable, one column in a listing. Rejected on decision
   1: it makes each half able to hide the other, and it converts a
   reviewer's judgement into arithmetic that can be outvoted by a
   well-formatted document. The registry column this would have bought is
   the cached automated score, which decision 3 provides without the
   conflation.
3. **Keying the checklist by proposal id** — the obvious key, since the
   checklist is answered during a review. Rejected on force 2: a proposal's
   members can be re-authored beneath it, and the answers would survive the
   edit. This is the same laundering ADR-0032 decision 6 exists to prevent,
   and it would arrive in the one artefact that has no address check.
4. **Keying it by the published commit** — "stored on the version", read
   most literally. Rejected because the checklist must exist *before*
   publication in order to gate it, and a commit does not exist until after.
   The digest is what the commit's tree would have hashed anyway; using it
   directly is what lets one key serve the draft, the proposal and the
   published version without a migration between them.
5. **No override: a low score simply blocks** — the strict reading, and
   consistent with SKIL-2. Rejected on force 3, and the asymmetry is the
   point: `critical` is a band defined by having no legitimate reading, so
   refusing an exception costs an author only a wait; a low score always has
   one, so refusing an exception costs the product its registry. The
   acceptance criterion asks for an override, and it is right to.
6. **The override as a field on the publish request** — one call, no new
   route, and the shape this ADR was first written with. Rejected by
   measurement rather than by argument (decision 8): the roles that publish
   skills and the roles that may override are disjoint under every product
   pack, so a single request needing both is a request nobody can make.
   Recorded here because the reasoning that produced it was sound and the
   result was still unusable — the packs, not the prose, decide whether a
   design is reachable.
7. **The override as an extra required approver** rather than a second
   action — raise `distinct_approvers` when the score is low, so the price
   is paid in signatures. Genuinely attractive, and rejected on legibility:
   the requirement is resolved and displayed before anyone reads the score,
   an extra approver appears in the matrix without a stated cause, and
   nobody in the trail ever says "I am shipping this below the bar". An
   override that nobody performs is one an auditor cannot find. Recorded as
   the reversal shape if the override turns out to be too easy to reach.
8. **A general escape hatch for both gates — a lapse over the skill plane**,
   discharging ADR-0052's recorded deferral at the same time. Rejected as
   out of scope and, on reflection, as wrong to want: ADR-0052's missing
   exception is for a rule the product got *wrong*, which is a defect with a
   release as its remedy, and this one is for a bundle the organisation
   judges good anyway, which is a decision with a person as its remedy.
   Sharing a mechanism would have made the security gate's floor negotiable
   by whoever holds the quality override. They stay separate.
9. **An LLM-judged rubric** — a model scoring the bundle against a written
   standard, which is what "SkillsBench-style" most fully means and would
   answer questions no lexical check can. Deferred with a trigger: it puts a
   model call on the authoring path (latency, cost, and an air-gapped
   deployment with no model), it is non-deterministic where every other
   number in this product is reproducible from bytes, and a score that
   changes when the judge model is upgraded is a score no reviewer can
   compare across two months. The shape is the `Extractor` seam — a
   `SkillJudge` trait with the lexical rubric as the default — and the
   trigger is EVAL-3's harness giving a way to measure whether a judge
   actually predicts anything the lexical rubric does not.
10. **Storing the automated half on the version too**, so a review is
   reproducible verbatim. Rejected on decision 2 and ADR-0052 decision 6's
   argument, with decision 3's cache as the concession: the durable record
   of what a reviewer saw is the chained event, and the recompute is what
   keeps a rubric change from having to migrate history.
11. **A configurable rubric — weights and checks as pack data.** Rejected
    for this feature: a rubric a tenant can reweight is one where a score of
    72 means nothing across two scopes, and the first thing anybody would do
    with the knob is tune it until their existing skills pass. If it arrives
    it should arrive as *named alternative rubrics* with versions, not as
    free weights.
12. **Refusing the bundle at authoring on a low score**, symmetrically with
    SKIL-2's gate. Rejected: a draft is where a skill is *supposed* to be
    unfinished, and a registry that refuses to hold work in progress is one
    where the work happens in a text editor instead. The score is
    information at authoring and a gate only at the seam where bytes go
    fleet-wide.

## Consequences

- **Positive**: the acceptance criterion's two clauses land on machinery
  that already existed — the score renders beside SKIL-2's scan in the same
  review block and beside the description in the same registry listing —
  and the third thing, the override, gives the product its first *legible*
  exception: a published skill that was known to be below the bar carries
  who said so and why, on the chain, permanently. The checklist keyed by
  bytes closes a laundering path that a proposal-keyed design would have
  opened, and closes it structurally rather than with a check. And ADR-0052
  option 7's prediction is discharged exactly as written: the table arrived,
  and the scan did not move into it.
- **Negative / accepted trade-offs**: the rubric is a proxy and will
  sometimes be wrong about a good skill — `files-referenced` is wrong about
  any bundle whose helper is called from a script rather than named in the
  manifest, and that is a deliberate five points rather than a fixed bug.
  Two numbers instead of one is more to render everywhere and more to
  explain once. The cache in decision 3 can be stale, and a stale score in a
  listing beside a fresh one at review is a discrepancy somebody will report
  as a bug before reading the label. The override is only as strong as the
  packs' role bindings: a tenant that grants `SkillQualityOverride` widely
  has a gate in name only — visible in the audit trail as a stream of
  overrides, which is the honest failure mode but is still a failure mode.
  And a rubric change moves every score in the product at once, so a skill
  that scored 75 last week can need an override this week, for the same
  reason ADR-0052 accepted for its rule table and with the same surprise.
- **Reversal triggers**: (a) overrides frequent enough to be routine → the
  threshold is wrong or option 6's extra approver is the better price, and
  the audit stream is the evidence either way; (b) EVAL-3 showing the
  lexical rubric predicts nothing a judge model does → option 8's
  `SkillJudge` trait; (c) a check firing on more than a third of a real
  corpus → it is measuring the ecosystem rather than the bundle, and the
  remedy is decision 5's: narrow the claim and move the weight, in one
  commit, with the corpus run in the message.
  `crates/synveda-ingest/tests/skill_corpus_rubric.rs` is the standing
  instrument and asserts the weak form of this continuously; (d) a
  checklist item answered `n/a` on nearly every bundle → the item is wrong
  and the list should shrink; (e) a tenant needing a different standard →
  option 10's named rubrics, versioned, never free weights; (f) the
  registry cache diverging often enough to confuse → drop the columns and
  pay the reads.

## Compliance notes

- **PDP**: one new action, `SkillQualityOverride`, a scope action taking no
  `context.sensitivity` — it is a statement about a process, not about
  content — resolved at the target scope in the same decision input the
  publication already gathered. It never replaces a decision: a publication
  with an override still takes `ChannelPublish`, still takes `SkillRead`,
  and still satisfies the approval matrix in full. The threshold that
  decides whether the override is *needed* comes from `EffectivePack`, the
  same resolved value SKIL-2's `scan` config rides on, so a pack change or a
  lapse governs the very next publication and the refusal names the pack and
  version that decided.
- **Tenancy**: `skill_reviews` is a new tenant-scoped table and takes the
  full treatment every table since AUTHZ-2 has — RLS policy keyed to the
  session GUC, `force row level security`, and grants that stop at
  `select, insert, update`. **No DELETE**, deliberately, and the contrast
  with ADR-0051 decision 17 is the argument: `skill_files` has one because a
  bundle is authored whole and a dropped file must not be published back
  onto a laptop; a checklist is a record that a person judged something on a
  day, and a product that can erase one is a product whose review trail can
  be edited. The two cache columns on `skills` add no surface — a column
  inherits that table's RLS, its forced flag and its grants, which is why
  every config since ADR-0025 has arrived the same way.
- **Audit**: `skill.checklist.recorded` chains inside the transaction that
  writes the row, so an answer and its record land together or not at all.
  `skill.quality.overridden` chains inside the publish transaction, atomic
  with the publication it permitted — an override whose event was lost would
  be a publication with no explanation, which is the one outcome this
  feature must not produce. Payloads carry item ids, verdicts, the score,
  the failing bars, the digest and the reason as the reviewer wrote it;
  never file content, which is the discipline every plane has followed since
  AUD-1. The leak sweep in `tests/skills.rs` extends to cover both.
- **Redaction**: the reviewer's free-text `reason` and checklist `note` are
  the first author-supplied prose this plane stores that is not a bundled
  file, so both go through MEM-2's scanner before they are written, on the
  authoring seam's own ladder. A reason carrying a credential is refused
  rather than scrubbed: unlike a bundled file there is nothing a placeholder
  would preserve, and the person who wrote it is on the other end of the
  request.
