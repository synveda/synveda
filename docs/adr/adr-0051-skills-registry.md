# ADR-0051: skills are the third authored asset and the first whose format belongs to somebody else — the bytes leave untouched so the receipt sits outside the bundle, the frontmatter is a strict subset rather than YAML, and the floor asked for a security reviewer without ever asking for a second signature

- **Status**: Accepted
- **Date**: 2026-08-03
- **Feature(s)**: SKIL-1
- **Deciders**: sujitn

## Context

SKIL-1's text is "SKILL.md + frontmatter + bundled files as a VedaFlow asset
type; validate against the open spec; import from anthropics/skills format",
and its acceptance criterion is one line: "a skill authored in Synveda
installs and runs unmodified in Claude Code and one other client (Cursor or
Codex)."

PRMT-1 landed the first authored asset and found FLOW-1 through FLOW-7 had
already built most of it. PRMT-2 landed the second and found that was true of
the governance half and emphatically not of the read half, because a pack's
content has to enter the corpus CTX-1 ranks. The third is different again,
and in a direction neither of them touched: **every asset so far had a format
Synveda chose and only Synveda reads.** A memory record, a prompt template
and a pack document are all canonical JSON envelopes whose only consumer is
this product. A skill's bytes are read by clients this product does not ship,
against a specification this product does not own, and the acceptance
criterion is a *third party's* loader accepting them. That inversion is the
whole of what is new here.

What is parked on this feature by name:

- **Seed §4.3** lists skills as the third of four managed asset types:
  "versioned skill definitions (SKILL.md-style) distributed to agents by
  scope". **§2.6** — "the harness is a guest" — decides where the
  materialisation lives before this ADR asks.
- **Tech plan §2.4** prices them: "Skill (executable!) → any `published`:
  steward + **security-reviewer role**; skills are treated like code because
  they are." It is the one row in that table written with an exclamation
  mark, and the one whose scope column says *any*.
- **ADR-0031 decision 1** fixed the channel vocabulary as
  `{asset-kind}/{channel}` with `skill` reserved, and `ChannelMember::name`
  as "a record id for memories, a path for the authored asset types".
- **ADR-0032's consequences** predict the curator glob "will need real path
  semantics when SKIL-1 and PRMT-1 bring path-named entries". PRMT-1 brought
  paths, PRMT-2 brought the first bundle to glob over; this brings the first
  bundle whose paths are *filenames*.
- **ADR-0035's consequences** predict "PRMT-1's prompts and SKIL-1's skill
  bundles will need a per-asset-kind renderer behind the same seam". The seam
  exists as of PRMT-1; skills are the third kind through it.
- **ADR-0036 decision 3** refuses the rewind and pin routes for asset kinds
  with no read action "by name rather than governed by memory's read action
  until PRMT-1 and SKIL-1 bring theirs". PRMT-1 shrank that refusal to
  `skill` and `context-pack`; PRMT-2 shrank it to `skill`. **This feature
  closes it**, and the `other =>` arm in `channels::decide_asset_read` and
  `proposals::decide_asset_read` stops being reachable by any asset kind that
  has a channel.
- **AUTHZ-3's status note** lists security-reviewer's skill approval as the
  last marker row in the role×action golden matrix awaiting a feature. It
  names SKIL-2; it closes here, because SKIL-2 adds a *report* to a review
  that has to be openable first, and nothing could open a `skill` proposal
  until now.
- **ADR-0041 decision 4** built the index tier's per-`AssetKind` rendering
  seam for "PRMT-1, PRMT-2 and SKIL-1". Two of the three have arrived through
  it. The third does **not**, and ADR-0049 option 10 already said why:
  advertisement of available assets is SKIL-4's shape
  ("inject index tier lists skills available to this identity"), not CTX-4's.
  This feature ships the registry SKIL-4 will advertise.
- **Research digest A3** is the commercial argument and it is unusually
  specific: agentskills.io became an open standard on 18 Dec 2025 and the
  ecosystem default across ~40 client platforms; the open catalogues have a
  quality and security crisis; "curated, reviewed, versioned skill registries
  with rollback are explicitly the emerging battleground"; and
  "**this is the sharpest wedge in the whole product**". A registry that is
  not portable is not that wedge.

Four forces bound the design, and only the last is inherited.

1. **The format is not ours, so the bytes must leave untouched.** A client
   parses `SKILL.md` with its own loader. Anything Synveda adds — a
   provenance key in the frontmatter, a header comment, a sidecar file inside
   the directory — is at best noise in a namespace we do not own and at worst
   a validation failure in a client we cannot test. "Unmodified" is the
   criterion's own word.

2. **Which costs the watermark, and this is the first read surface that has
   to pay it.** Seed §4.4: "Every injected block is watermarked with record
   IDs for auditability." ADR-0031 decision 12 and every read since have
   answered with commits and addresses attached to what they serve. A
   materialised skill is a directory of files on somebody's laptop; there is
   nowhere inside it to put a commit hash that a foreign loader is guaranteed
   to ignore. The provenance has to ride somewhere else, and the somewhere
   else has to be provably not-the-bundle.

3. **A bundle becomes files on a person's machine, which nothing in this
   product has ever done.** `observe` takes text. `inject` returns text. A
   skill install writes a directory. Path traversal, case-folding
   filesystems, reserved device names and executable bits stop being
   formatting concerns and become product properties, decided here or
   discovered by whoever runs `synveda skill install` on Windows.

4. **YAML is a much larger language than the spec's frontmatter.** Anchors,
   aliases, merge keys, tags, block scalars, multiple documents, duplicate
   keys with implementation-defined precedence. The product's entire claim is
   that what two people reviewed is what runs; two parsers reading one
   document differently is precisely how that claim fails, and it fails
   silently.

And one thing nothing parked here, found by reading the matrix this feature
makes resolvable — the same way PRMT-2 found its cell, one layer further
down. **The invariant floor's skill rule requires the `security-reviewer`
role at `distinct_approvers: 1`** (`approval.rs`'s `FLOOR`). Under
`regulated-strict` the pack's own rule raises it to 2, so two people sign.
Under `standard` and `open-collaboration` the pack asks for a steward at 1,
the max is 1, and **one person holding both roles publishes executable code
alone**. The floor exists to say a skill's security review is not a pack's to
opt out of; it guarantees the *role* is present and never that it is a second
*person*, which is the entire content of separating those two roles. Tech
plan §2.4's skill row is unconditional where its memory row is not, and the
SMB line beneath it says "**most** of the above collapses" rather than all of
it. Nothing has ever exercised the cell, because nothing could open a `skill`
proposal.

## Decision

**A skill is a draft row plus one content-addressed object per bundled file,
published through the proposal path that already exists, resolved by name up
the caller's own placement chain, and materialised by the client-side CLI
into whichever client's skills root the caller names — byte for byte, with
the receipt written outside the bundle.** The registry adds no channel shape,
no proposal effect, no publication event and no composition behaviour. What
it adds is two PDP actions, two tables, a strict frontmatter subset, a
filesystem-safety grammar, and one correction to the invariant floor.

Decisions, specifically:

1. **The draft is a row, its files are objects, its publication is an
   ordinary FLOW-3 proposal whose asset is `skill`.** ADR-0049 decisions 1,
   2, 6, 7 and 15 and ADR-0050 decision 1 apply unchanged and for their
   reasons: the draft is a row because a set channel cannot express
   withdrawal, the name is the id because it is what a scope's override is
   expressed in, the direct publish route takes skills and refuses on the
   matrix's own arithmetic, and a draft read names its scope.
   `AssetKind::Skill` has been in the vocabulary since FLOW-1 and in the
   approval matrix since FLOW-3; this is the first feature that can resolve
   one of its cells.

2. **One object per file, in ADR-0050's envelope, with the file's bytes as
   the `content` field.** `SkillAsset` is canonical JSON —
   `{content, file, scope, sensitivity, skill}` — because the governed
   context has to be *inside the address*: ADR-0030 decision 4's rule is that
   identical bytes governed differently are different objects, and a tier
   that lived only on a mutable row could be raised or lowered after review
   without moving anything a reviewer signed.

   The envelope is not in tension with force 1, and saying why is worth a
   line: **"unmodified" is a property of materialisation, not of storage.**
   What a client reads is the `content` field written verbatim to
   `<root>/<skill>/<path>`. The tree entry is `<skill>/<path>`, ADR-0031's
   reserved "a path for the authored asset types", and the first bundle whose
   paths are real filenames rather than a naming convention.

3. **`SKILL.md` is required, at the bundle root, spelled exactly.** A bundle
   without one is not a skill under the open spec, so the surface that would
   accept one is a surface that ships an artefact no client will load. It is
   refused at authoring, at import, and in the store's own CHECK.

4. **The frontmatter is parsed by a strict subset, and the subset refuses
   rather than guesses.** Plain and quoted scalars, block and flow sequences
   of scalars, one level of nested mapping — the shapes real skills in
   `anthropics/skills` actually use. Refused by name: anchors, aliases, tags,
   block scalars, merge keys, `%YAML` directives, duplicate keys, tabs in
   indentation, unquoted values carrying `:` or ` #`, unknown backslash
   escapes, nesting past one level, and any key outside the spec's
   vocabulary. Force 4 is the reason, and the
   refusal is safe in a way a permissive parser is not: because the bytes
   ship verbatim, **a construct this parser refuses is a construct nobody can
   author**, so there is no document in the product whose meaning two parsers
   could disagree about.

5. **The spec's rules are enforced at authoring, where they can still be
   fixed.** `name` present and equal to the skill's own name (the spec's
   "must match the directory", which is also this product's row key);
   `description` present and non-empty (it is what a client loads at ~80
   tokens and what SKIL-4 will advertise). A validation this product skips is
   a refusal a third-party client delivers to a user who has already
   published.

6. **The name grammar is the spec's, which is stricter than the product's.**
   One segment, `[a-z0-9]` then `[a-z0-9-]`, at most 64 characters — no `_`
   and no `.`, both of which `pack::validate_name` allows. Reusing the
   product's grammar would admit at the first step exactly what the last step
   refuses, and the AC would fail in somebody else's loader for a name this
   product said yes to.

   The gradient comes with it, and gains a physical form: because the name is
   also the installed **directory** name, a team's `code-review` overriding
   the org's is not a policy resolution that happens to work out — the
   client's namespace is flat, so only one of them *can* exist on disk.
   ADR-0049 decision 8's nearest-first walk is what decides which.

7. **Bundled paths are validated against filesystems, not against taste.**
   Relative, `/`-separated, bounded in segments and length; no `.` or `..`
   segment, no absolute form, no backslash, no colon, no control character,
   no trailing dot or space, no reserved device name (`CON`, `NUL`, `COM1`…),
   and **no two paths equal under ASCII case folding**. The last is the one
   that is not obvious and is the most dangerous: macOS and Windows fold
   case, so `Scripts/run.py` and `scripts/run.py` are two governed objects
   that become one file, and the loser is whichever the installer wrote
   first. Refusing the pair at authoring is the only place that can be
   caught, because the store, the tree and the address all consider them
   distinct — correctly.

8. **Every materialised file is non-executable.** A skill invokes its scripts
   through an interpreter (`python scripts/check.py`), which is what the
   bundles in `anthropics/skills` do and what the spec's own examples show.
   Mode is not in the open spec, so there is nothing to be compliant *with*,
   and inventing a mode field would put a privilege on the wire that a
   reviewer reads as a line of YAML. A governed bundle cannot arrive carrying
   an execute bit nobody reviewed.

9. **A skill's content never becomes a record, and never composes.**
   ADR-0049 option 4's third reason for refusing "prompts as memory records"
   — *"a prompt is fetched by name where a record is ranked by relevance"* —
   was inverted by PRMT-2 for packs and is **restored here**, for a reason
   packs did not have: the client's own progressive disclosure is already the
   loader. Ranking a SKILL.md body into an inject block would spend the token
   budget doing the job the client does for free, and would do it worse,
   because the client can read the bundled files on demand and a block
   cannot. The three authored assets now sit at three distinct points, which
   is the shape worth writing down: a **prompt** is fetched by name and
   composes into nothing; a **context pack** is ranked into the block; a
   **skill** is fetched by name and materialised into the harness.

10. **`SkillRead` and `SkillWrite`**, mirroring ADR-0049 decision 4 and
    ADR-0050 decision 7: both scope actions, `SkillRead` carrying
    `context.sensitivity` and no `lapsed` attribute, `SkillWrite` the
    authoring seam with the home-scope floor role-free. This is what
    discharges ADR-0036 decision 3 for the last of the three kinds it refused
    by name — after this, every asset kind that has a channel has a read
    action, and the refusal arm survives only for `policy`, which has no
    channel at all (ADR-0037 decision 16).

11. **Sensitivity is per skill, not per file.** ADR-0050 decision 12 put it
    on the *document* because a pack is read piecemeal by a ranker and a
    glossary of public terms and an internal runbook are plausibly the same
    bundle. A skill is loaded **whole** by a client — a bundle whose
    `SKILL.md` is `internal` and whose script is `confidential` is a bundle
    that cannot be half-loaded — so the tier is the bundle's, carried on
    every file's envelope so that reclassifying re-addresses all of them.
    `restricted` is unrepresentable, exactly as it is for prompts and packs
    and for their reason (ADR-0049 decision 5).

12. **The materialisation is the CLI's, and the receipt lives in the CLI's
    own state, never in the client's skills root.** Seed §2.6: the harness is
    a guest, and a gateway that owned an archive format and a per-client
    directory layout would be a gateway that has to ship a release when a
    client renames a folder. `synveda skill install --client claude-code`
    resolves through the ordinary read route and writes the bundle; the
    receipt — scope, commit, per-file addresses, the pack in force — goes
    beside the credentials in the CLI's config directory. Force 2's answer:
    the bundle on disk is *exactly* the reviewed files, which is a claim that
    can be checked by hashing the directory, and the provenance is a hash
    comparison away rather than a string inside a file.

13. **A consumer may pin a commit, and a rewind refuses the pinned read.**
    ADR-0049 decisions 9, 10 and 11 inherited whole, including the part that
    was learned rather than designed: the ancestry is measured against what
    the scope **serves** rather than its head, so a standing FLOW-7 pin is a
    ceiling a consumer pin may reach at or below and never over. This is what
    makes a receipt reproducible — `synveda skill install --commit <hash>`
    reinstalls what it recorded — and what keeps FLOW-7's "<60s to
    fleet-wide effect" true of an asset that lives on laptops.

14. **MEM-2's scanner runs at authoring, on ADR-0050 decision 11's ladder and
    with its departure.** A skill bundle is bulk external text like a pack,
    and more: it is the first content the product governs that is *code*.
    `deny` and `quarantine` both refuse to the author (a synchronous request
    has somebody to tell), `quarantine` additionally chains
    `skill.quarantined`, `redact` scrubs and continues. The guarantee is the
    one that matters and it is stronger here than for packs, because there is
    no vector space to keep clean — there is a laptop: **no secret reaches a
    client's disk.**

15. **Import is a client-side read of an `anthropics/skills` directory, and
    it refuses rather than partially imports.** A symlink is not content and
    is not followed; a missing `SKILL.md`, a file over the bound, a path
    decision 7 refuses, or a bundle over the file count are each a refusal
    naming the offender. Importing three files of four and calling it a skill
    is the failure mode a registry exists to prevent.

16. **Two new audit actions, and no third.** `skill.authored` for a draft
    write and `skill.resolved` for a served read, carrying names, scopes,
    channels, commits, addresses and tiers — never SKILL.md text and never
    file content. Publication is `vedaflow.channel.published` as it always
    is. There is deliberately no `skill.installed`: an install is a
    client-side act on bytes an audited resolve already served, and an event
    the server cannot verify is a fact an auditor has to reconcile
    (ADR-0019 decision 4).

17. **A skill is authored whole, and the request is the bundle.** ADR-0050
    decision 1 inherited PRMT-1's authoring semantics and PRMT-2 added one of
    its own: documents not named in a pack request are left alone, because
    "a bundle is edited a file at a time and a request that dropped the rest
    would make every save a full re-upload". A skill is the one authored
    asset where that is wrong, for decision 11's root reason — a client loads
    the bundle **whole**. A file the author deleted that stayed in the draft
    would be published back onto a laptop by the next proposal, so an
    authoring request replaces the file set and `skill_files` carries the
    only DELETE grant in the three registries. It cannot reach a published
    version: a tree names object addresses, objects are append-only, and what
    a channel serves does not depend on which draft rows exist. Removing the
    *skill* is still FLOW-7's rewind.

18. **The invariant floor's skill rule gains a second distinct approver.**
    `distinct_approvers: 1` → `2`, on the floor rather than in a pack,
    because that is where "not a pack's to opt out of" lives. It is
    satisfiable in every pack (a security reviewer and a steward are two
    people), it changes nothing under `regulated-strict`, and it lands as a
    pack version bump with the role×action golden re-recorded. The argument
    is decision 8's from the other side: a skill is executable, review by the
    person shipping it is not review, and the floor already asserts that this
    particular review is not negotiable — it simply forgot to say by whom.
    Taken now because no tenant has ever published a skill, and taken as its
    own decision, with its own option below, so it can be refused on its own.

## Options considered

1. **Per-file objects in a canonical envelope, tree entry `<skill>/<path>`,
   materialised by the CLI (chosen)** — inherits the channel, the proposal
   path, the approval matrix, the curator glob, the review CLI, the rewind,
   the pin and the audit chain, and dedups per file so editing one line of
   `SKILL.md` re-stores one object. Con: the reviewed unit is a tree rather
   than a single address, so "which version am I running" is a commit rather
   than an object hash — which is what a receipt records anyway.
2. **The whole bundle as one object** — one address per version, and the
   receipt is that address. Rejected: no per-file dedup, and FLOW-6's diff
   would have to unpack a JSON map to say which file changed, which is the
   renderer working around the storage instead of with it. The commit already
   names the whole tree, so the property this buys is one the product has.
3. **Raw file bytes as the object payload, governed fields only in the tree
   entry name** — the most literal reading of force 1. Rejected: the tier
   would then live only on a mutable row, so an author could reclassify
   published material without review. Decision 2's envelope is what keeps
   ADR-0030 decision 4 true, and it costs nothing a client can observe.
4. **A general YAML parser** (`serde_yaml`, `saphyr`) — accepts everything a
   client accepts, and is somebody else's problem to maintain. Rejected on
   force 4 and on the licence-and-dependency rule behind it: the failure mode
   is not a crash, it is two parsers reading `description:` differently and
   nobody finding out. Recorded as the reversal trigger — a real skill in the
   wild that the subset refuses and a client accepts is a bug in the subset,
   and the fix is to widen it deliberately.
5. **Skills as context-pack documents with a reserved pack name** — no table,
   no action, no branch. Rejected three ways: the floor prices `skill`
   differently and would stop being able to, the asset kind is inside the
   address by ADR-0030 decision 4, and a pack's documents become ranked
   records where a skill's must not (decision 9).
6. **Materialising from the gateway** — a `GET /v1/skills/{name}/bundle.tar`
   or a per-client layout endpoint. Rejected on seed §2.6: the gateway would
   own an archive format and a directory convention for every client in a
   40-client ecosystem, and would need a release when one of them moved a
   folder. The CLI is the guest-facing half and can be wrong cheaply.
7. **Writing the receipt inside the skill directory** (`.synveda/receipt.json`
   beside `SKILL.md`) — provenance travels with the bundle, which is what
   every other read surface does. Rejected: it is precisely the modification
   the criterion forbids, it puts a file no reviewer approved into a
   directory a client walks, and it would make the byte-identity check
   compare a tree against itself plus something.
8. **Allowing an executable bit** — some skill will eventually want
   `./scripts/run.sh`. Deferred with a trigger rather than rejected: it is a
   mode field on the envelope and a line in the diff renderer, and it should
   be added when a real bundle needs it and a reviewer can see it, not
   speculatively.
9. **Allowing binary bundled files** (base64 in the envelope) — a skill with
   a reference image or a compiled helper. Deferred with a trigger: FLOW-6
   renders diffs of what reviewers approve, and a base64 blob is a diff
   nobody reads. UTF-8 text is refused-loudly-or-accepted, which is the
   property review depends on.
10. **Leaving the floor's skill rule at one distinct approver** (decision 17)
    — the floor's job is to guarantee the role, and how many people sign is a
    pack's business; `standard` exists to be cheaper. Rejected on the narrow
    ground that this is the feature which makes the cell reachable at all:
    shipping the first skill publication under a rule where the author can be
    their own security reviewer would set that precedent rather than inherit
    it, and the SMB pack still asks for exactly two signatures where
    `regulated-strict` asks for two plus the tier floor.
11. **Skills in `inject`'s index tier** — an agent would see the skills
    available to it without asking, and ADR-0041 decision 4 named this
    feature. Deferred, unchanged from ADR-0049 option 10: advertisement is
    SKIL-4's own acceptance criterion, and shipping half of it here would
    make that feature's AC untestable against a baseline that already
    contains it.
12. **Per-file sensitivity, as packs have** — the same flexibility PRMT-2
    chose. Rejected on decision 11: a client loads a bundle whole, so a
    per-file tier is a promise the loader cannot keep.
13. **Validating against a vendored copy of the agentskills.io schema** —
    machine-checkable, and it moves when the spec moves. Deferred: the spec's
    required surface is two keys and a filename, the vendored artefact would
    need its own review-and-update path, and the subset parser is where the
    rules actually have to hold. Revisit if the spec grows a real schema
    document with versioning.

## Consequences

- **Positive**: the approval matrix's `skill` cells resolve for the first
  time since FLOW-3 wrote them, security-reviewer's last marker row in the
  role×action golden matrix closes, and ADR-0036 decision 3's
  refusal-by-name reaches zero asset kinds with channels. Rewind, pin, climb,
  the review CLI, the curator globs, the audit chain and the uniform 404 all
  extend to skills with no new concepts. The registry the research digest
  calls "the sharpest wedge in the whole product" exists, and its portability
  is a hash comparison rather than a marketing claim.
- **Negative / accepted trade-offs**: this is the first governed asset whose
  served form carries no watermark, and the compensation — a receipt in the
  CLI's state — is only as good as the CLI, so a bundle copied by hand from
  one machine to another loses its provenance entirely. A skill cannot carry
  a binary file, cannot ship an executable bit, and cannot be `restricted`.
  The frontmatter subset will refuse some document a client would have
  accepted, and the first person to hit that will experience it as Synveda
  being wrong about YAML — which is the cost of option 4 and is recorded as
  its trigger. SMB tenants on `standard` now need two people to publish a
  skill where a moment ago they needed one, which is a real cost paid
  deliberately (decision 17). And **every served resolve appends to the
  tenant's audit chain**, inherited from ADR-0049's consequences with the
  same recorded upgrade (AUD-1 option 2's buffered appender) — a skill is
  installed rather than fetched per turn, so the rate is lower than a
  prompt's.
- **Reversal triggers**: (a) a real `anthropics/skills` bundle the subset
  parser refuses and every client accepts → widen the subset by name, in a
  commit that says which construct and which bundle; (b) a skill needing an
  executable script or a binary asset → options 8 and 9, both of which are a
  field on the envelope and a line in the renderer; (c) the client ecosystem
  diverging on where skills live badly enough that the CLI's client table is
  wrong more often than right → the table moves to configuration, not to the
  gateway; (d) anyone needing a `restricted` skill → the same classify effect
  for authored assets that ADR-0049 and ADR-0050 both name; (e) hand-copied
  bundles losing provenance often enough to matter → a `synveda skill verify
  <dir>` that rehashes a directory against a commit, which is a read-only
  addition and not a change to any decision here.

## Compliance notes

- **PDP**: no path reaches a skill's bytes without `SkillRead` at the scope
  that holds it, decided at the tier that version carries, at request time,
  under the live pack (decisions 10, 11, 13). Authoring takes `SkillWrite`;
  publication takes `ChannelPublish` plus the approval matrix plus
  `SkillRead`, the three-part rule memory publication has obeyed since
  ADR-0031 decision 12. The materialisation performs no authorization of its
  own — it writes bytes an authorized resolve returned.
- **Tenancy**: the skill table and its file table arrive with forced RLS,
  tenant-isolation policies and least-privilege grants in their own migration
  (0031), and join the adversarial suite and its completeness guard
  (ADR-0009). Objects and refs are already per tenant, and content-addressed
  dedup has never crossed one (ADR-0030 decision 3).
- **Audit**: authoring and every served resolve chain in the caller's own
  transaction; publication chains the event it always did. No payload carries
  SKILL.md text or file content — the discipline every plane has followed
  since AUD-1, and the reason a leak sweep over the chain is a test rather
  than an assertion.
- **Redaction**: decision 14 puts a skill bundle through the same scanner and
  the same per-pack modes as the observe path and the pack authoring path, so
  the surface that would otherwise have been the easiest way to move a
  credential onto a fleet of laptops is the one MEM-2 already governs.
