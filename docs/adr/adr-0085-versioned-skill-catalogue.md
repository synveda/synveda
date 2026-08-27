# ADR-0085: skill distribution resolves immutable versions through governed bindings, not mutable drafts or channel heads

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-23
- **Deciders**: sujitn

## Context

SKIL-1 through SKIL-4 established the parts of the skill plane that remain
correct: Agent Skills bundles leave Synveda byte-for-byte, every file is
content-addressed, authoring and publication are scanned, quality evidence is
bound to exact bytes, reads take `SkillRead`, and the client rather than the
gateway owns materialisation. They did not establish the product identity the
context-platform programme now needs. A skill is currently one mutable draft
row plus mutable file pointers at a scope; its only durable "version" is an
anonymous VedaFlow channel commit; and changing or rewinding the channel is
also how distribution changes. There is no stable skill id, named immutable
version, project binding, usage evidence, or test-run record.

That shape cannot answer several ordinary questions. A project cannot pin a
reviewed version without pinning every authored asset at the scope. Disabling
one skill rewrites a shared channel. A rollback changes publication history
rather than the binding that chose a version. Advertised, discovered and
executed are collapsed into one client-side inference. A manifest's declared
tools are visible text but have no place in the domain that can state, and
test, that declarations grant no authority.

The applicable external format is the unversioned Agent Skills specification
at [agentskills.io/specification](https://agentskills.io/specification), pinned
for this implementation to `agentskills/agentskills` commit
`69ef37e9424c0a7ea9dd2293b559e43ec8176379` as observed on 2026-08-24. It
requires `SKILL.md`, `name` and `description`; defines optional `license`,
`compatibility`, `metadata` and experimental `allowed-tools`; permits arbitrary
bundle resources; and requires progressive disclosure from metadata to
instructions to resources. It publishes no numbered protocol release, so
inventing a version string would be a false compatibility claim.

The redesign is pre-1.0. There is no old-data migrator, dual read, legacy route
or mutable compatibility projection to preserve. Every effective install,
update, binding change and rollback must be a VedaFlow `apply` change; the PDP
is repeated when the effect runs; tenant tables require enabled and forced
RLS; and audit payloads may name ids, digests, counts and outcomes but never
bundle content.

## Decision

Replace the draft/channel registry with one stable tenant catalogue, immutable
versions and revisioned bindings. Store exact bundle files in the existing
content-addressed VedaFlow object store, store version provenance and scan
evidence beside the immutable version, and make only a successfully applied
VedaFlow change alter the catalogue's current version or a binding. Resolve
distribution from bindings, never from `skill/published` refs.

Specifically:

1. **One stable `Skill`, many immutable `SkillVersion`s.** `skills.id` is a
   UUIDv7 and the tenant-wide `(name)` is unique. The aggregate records its
   governing scope and current approved version. A version has its own UUIDv7,
   monotonically increasing ordinal, bundle digest, manifest projection,
   sensitivity, source provenance, scan ruleset/report, quality rubric/score,
   creator and transaction time. A content change always mints another row;
   no update or delete privilege exists on versions or version files.

2. **The bundle remains an external format.** `skill_version_files` points at
   existing `vedaflow_objects` whose `SkillAsset` canonical bytes retain scope,
   name, tier, path and exact content. The served file content is recovered
   from those objects and written unmodified by clients. Unknown keys inside
   the specification's `metadata` extension map are preserved. Experimental
   `allowed-tools` and observed client extension fields are retained and shown
   as declarations only; no Cedar entity, role, grant or execution permission
   is derived from them.

3. **The specification pin is executable evidence.** The parser gains the
   official `compatibility` field, enforces the published no-consecutive-hyphen
   name grammar, and documents the upstream commit/date rather than claiming a
   fabricated Agent Skills version. Fixture tests pin required/optional fields,
   extension metadata and unchanged bundle bytes.

4. **A binding is the only distribution switch.** `skill_bindings` is a stable
   aggregate at a `project`- or `principal`-shaped scope. It names a skill, is
   enabled or disabled, carries a revision counter, and optionally pins an
   exact version. An unpinned binding follows the aggregate's current approved
   version; a pinned binding resolves only its named version. Disable, enable,
   pin, unpin and rollback update the binding under an exact revision
   precondition. Rollback therefore changes which immutable version a binding
   selects and never rewrites history.

5. **Every effective mutation is a typed VedaFlow change.** Install, update,
   create-binding and change-binding requests validate and scan first, then
   open an `AssetKind::Skill` proposal with `ProposalEffect::Apply` and an
   immutable command manifest. The erasable typed projection binds the full
   command payload to that manifest hash. The live approval matrix decides
   whether the command applies immediately or remains under review; execution
   repeats ownership, `SkillWrite`, `ProposalOpen`, precondition and payload-
   binding checks. A pending version or binding has no active domain row.

6. **Scans and quality scores are immutable version evidence.** Current
   scanner and automated rubric rules run before a change opens and again
   before an approved version effect applies. The immutable version retains
   the admitted scan report, ruleset version, rubric score/version and bundle
   provenance. A newly blocking scan or below-current-minimum score rejects the
   change rather than applying evidence computed under an obsolete gate. The
   old draft-bound checklist and quality-override mutation path is deleted;
   human approval is the VedaFlow review and fixture evidence is an immutable
   `SkillTestRun`.

7. **Usage is an append-only, evidence-labelled event stream.** A
   `SkillUsageEvent` names the exact binding and version plus one of
   `advertised`, `discovered`, `activated`, `instructions_loaded`,
   `resource_loaded`, `script_requested`, `executed`, or `outcome_reported`.
   It labels evidence as `host_observed` or `model_reported`; the two are never
   merged. A client event id makes replay idempotent. Recording an event first
   proves the binding/version is visible through `SkillRead`; it cannot make a
   version visible.

8. **Test runs never execute gateway-side bundle scripts.** A
   `SkillTestRun` is immutable and names the exact version, harness identity,
   result, scan/rubric versions and content-free evidence. The built-in
   `validation_sandbox` only parses and scans the stored bundle under the
   gateway's existing CPU/memory bounds; it never spawns a process or imports
   code from the bundle. External controlled-client results may be reported as
   such, with their observer and fixture provenance, but are never relabelled
   host execution.

9. **Public APIs expose stable ids and keyset pages.** The generated contract
   covers catalogues, versions, files, bindings, resolution, usage and test
   runs. Creation takes `Idempotency-Key`; updates take an exact current version
   or binding revision. Collections advance a cursor over the last candidate
   considered. The existing CLI resolves and materialises through these APIs;
   there is no draft/channel or direct-store skill client.

10. **Distribution and context advertisement use the same resolved set.** The
    binding query walks the already-authorised context scope order, filters
    each exact version by that scope's `SkillRead` tiers, then applies name
    shadowing. The ContextRun skill citation records version id and bundle
    digest instead of a channel commit. A denied or disabled binding contributes
    no name, id, count or declaration.

11. **The old skill runtime is deleted whole.** Migration 0052 drops the
    mutable `skills`/`skill_files` registry and the draft-bound
    `skill_reviews`/`skill_quality_overrides`, then creates the new epoch
    tables; there is no row translation. `SkillChannel`, `ChannelRef::skill`,
    `skill/published`, draft query parameters, special checklist/override
    mutations, skill publish effects, channel-wide pin/rollback and their
    DTOs/tests/docs leave production. The object store and generic
    proposal/approval engine remain because they are the immutable storage and
    review machinery of the new model, not compatibility surfaces.

## Options considered

1. **Stable aggregate, immutable versions and binding-only distribution
   (chosen)** — gives every API, audit event and client receipt an exact domain
   address while reusing the object store, scanners and VedaFlow engine.
2. **Add version rows beside mutable drafts and keep channels active** — smaller
   initially, but creates two current-version answers and makes rollback mean
   two different operations. Rejected by the hard cut and by seed §2.1.
3. **Treat VedaFlow commits as public version ids** — avoids one table but a
   commit is a tree implementation address, can hold several skills, and has
   no stable aggregate, provenance or usage identity. Rejected.
4. **Copy bundle content into version rows** — simpler reads but duplicates the
   content-addressed truth and risks returning bytes different from the ones
   reviewed. Rejected.
5. **Execute fixture scripts in a gateway subprocess** — superficially proves
   more behaviour, but turns untrusted reviewed content into gateway code and
   confuses declared tools with authority. Rejected; controlled clients or a
   separately isolated runner are the extension point.

## Consequences

- Positive: exact versions can be inspected, pinned, rolled back and cited;
  binding changes no longer rewrite publication history; scan/provenance/test
  evidence is retained; usage says who observed what; and one governed model
  serves personal and project scopes.
- Negative / accepted trade-offs: existing skill channel history is discarded
  at the epoch boundary; an unpinned binding deliberately follows a newly
  approved current version; pending changes have VedaFlow identities but no
  active catalogue rows; and the built-in test harness validates safely rather
  than claiming arbitrary scripts ran.
- Reversal triggers: a tenant needs two catalogued skills with the same name →
  add a source-qualified lookup key without changing stable ids; validation
  sandbox evidence proves insufficient → add a separately deployed sandbox
  runner over the stored test-run job boundary; binding resolution binds the
  context SLO at measured scale → add a generation-keyed projection, never a
  second authority.

## Compliance notes

- **PDP**: all mutations take `SkillWrite` and `ProposalOpen` at the governing
  or binding scope; reads and usage admission take tiered `SkillRead` on the
  exact binding scope. Apply repeats the decisions. Declared tools never enter
  Cedar input.
- **Tenancy/RLS**: every new table is tenant-bound, has composite tenant foreign
  keys where applicable, enabled and forced RLS, least-privilege grants and an
  entry in the completeness/adversarial suite. Cross-tenant ids resolve to the
  same 404 as missing ids before a PDP decision.
- **Audit**: semantic events record change, skill/version/binding ids, digests,
  stages, evidence kind, harness and outcomes. Bundle content, frontmatter
  prose, tool declarations and test output never enter the chain.
- **Secrets**: MEM-2 redaction and SKIL-2 scanning run before any VedaFlow
  command projection is stored. API, trace, audit and test evidence carry no
  matched text or secret-bearing file content except the separately authorised
  exact file read.
