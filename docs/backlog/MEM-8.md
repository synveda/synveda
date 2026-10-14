# MEM-8: Source lineage and lifecycle invalidation

## Problem and evidence

Reconciled on 2026-10-14 from clean `main` at
`4729aa5d9ef6929988bf01d1a9640c7a865ca8bc`, using GPT-6 Astra at max effort
for planning and GPT-6.1 Sol for implementation. v0.4.5 is published for
controlled evaluation. Existing Knowledge forget is a synchronous governed
transaction with a durable receipt; it removes revisions, embeddings, addressed
ContextRun copies and import/candidate payloads. It does not mean removal of an
originating Session event or every statement supported by that event.

At inspection, ContextRun replay checked SessionWrite but returned saved
content without the independent source checks used by diagnostic reads. That
disclosure defect is repaired under ADR-0141; the complete source-removal
contract below remains open.
ADR-0144 now retains private exact delivery proof in every diagnostic mode,
including authored packs and Skill advertisements. Older nonempty receipts
without producer proof fail closed. Checkpoints verify
their original event hashes and quarantine state, but item forget cannot be
treated as a request to erase the entire originating event. Capture freezes
event identities and reads live payloads; its completion fence does not yet
implement source-level suppression.

Baseline `make check-fast`, 187 types/8 retrieval unit tests and fresh exact-role
14 ContextRun/6 lifecycle tests passed before implementation. The archive replay,
numeric duplicate and detached retry-exception regressions were reproduced
before their repairs.

## Scope

First close the demonstrable saved-delivery disclosure path under CPR-20 and
CTX-6. Then define a separate governed source-removal operation, minimal scoped
suppression, complete content-free delivery dependencies independent of optional
diagnostics, final admission ordering and bounded cleanup. Inventory Session,
Capture, Knowledge, checkpoint, ContextRun, import and worker content separately.

The requested implementation matrix records the existing contracts and gaps:

| Capability | Existing implementation / status | Actual gap and feature | Acceptance example | Validation |
| --- | --- | --- | --- | --- |
| A: lineage/lifecycle | Session and actual ImportArtifact removal qualified at `0006`; exact roots, permanent suppression, durable cleanup and admission fences | MEM-8; narrow CPR-16/20 and CTX-6 repairs | Revoke/remove evidence during replay and queued derivation; suppress unchanged reingest; preserve independent evidence | `bash scripts/db-test.sh -p synveda-gateway --test knowledge_lifecycle --test context_runs --test sessions_api --test capture_api -- --test-threads=1` |
| B: health/diagnosis | CPR-21 Inspector and feedback delivered | CPR-21/40: authorized findings, actionable links, stage diagnosis and unknown outcomes | Distinguish uncaptured evidence, retrieval miss, qualifier loss and wrong downstream answer without denied counts | ContextRun tests, console tests, `make eval-product` |
| C: values/applicability | CPR-37 conflict review/freshness delivered; lexical classifier retains both propositions | CPR-37: bounded typed revision metadata, significant values/case, applicability overlap | Retry 3 vs 5 is reviewable contradiction; production vs development is separate applicability | Types tests, lifecycle/capture database tests, `demos/cpr-37-conflict-freshness.sh` |
| D: inbound reconciliation | Repository identity and outbound FLOW-8 exist; inbound absent | [MEM-9](MEM-9.md): separately governed local/private Git reads and path/hash observations | Relevant path change proposes review once; unrelated change does not stale everything; crash does not advance cursor | New source-reconciliation database suite, Git tests, dependency/contract gates |
| E: structured handoff | Grounded TaskState, v2 checkpoint and explicit handoff qualified at `0009`; live task quality open | [CTX-6](CTX-6.md): new verified method, structured evidence, explicit second-client receiving contract | Resume through Claude and the genuine Codex SessionStart/PreCompact interface with exact evidence and unknown operation outcomes | Session/context database tests, both adapters, registry checks; separate live qualification |
| F: task selection | CTX-8 off/conservative, exact local counter, source spans and shared budget implemented; quality open | [CTX-8](CTX-8.md): preserve cross-paragraph qualifiers and bounded task needs | Tight budget preserves value/unit/negation/exception or reports insufficiency | Retrieval/context/pack tests, `make eval-product`; CTX-7 real 1024-dimensional evidence only if ranking SQL changes |
| G: procedure learning | Skill versions/usage and feedback exist; outcome-to-proposal path absent | [SKIL-6](SKIL-6.md), [SKIL-5](SKIL-5.md): exact outcome evidence and deterministic review candidates after A | Verified outcome proposes one small edit; unsupported tests-passed claim stays unverified; removal invalidates proposal | New procedure-learning database suite, Skill/Capture/context tests and frozen held-out cases |
| H: domain example/demo | CPR-41 public-API fictional demo and CPR-40 evaluation delivered | CPR-41/40: glossary, metric definition, approved ambiguity and integrated source-change/handoff/outcome/remove replay | Applicable approved definition with provenance; changed definition opens targeted review | `make eval-product`, `make check-demos`; separate real-harness command |

No phase is qualified by planning or by the earlier release. New persistent
learning waits for A's lineage and source-disclosure safeguards.

### First implemented increment (2026-10-14)

On `codex/context-memory-improvements`, the first source increment implements:

- CPR-20/CTX-6: current independent Knowledge, disclosed provenance, graph and
  checkpoint admission on saved replay, with bounded proof and uniform 409.
  Exact historical diagnostic reads remain independently authorized. At this
  first increment, nonempty hashes-only/disabled and authored/Skill receipts
  lacked sufficient proof and refused replay; safe empty receipts replayed.
- CPR-21: six health dimensions and five stage observations in the existing
  public detail/console, derived after authorization. Production recall,
  synchronization, cleanup, host acknowledgment and model outcome stay unknown.
- CPR-37: closed optional v1 claims in immutable revision metadata, checked
  value/unit/version/range/applicability comparison and conservative unparsed
  review. The real 3/5 procedure case retains both exact propositions, normal
  conflict review, respective provenance and historical transaction semantics.
- CTX-8: applicability sections and explicit qualifiers remain exact atomic
  source units, including nested Markdown fences and group-bound exhaustion.
  A tight-budget public API case preserves the payment retry exception;
  requiring the entire oversized body still returns explicit insufficiency.

Actual validation: 193 types and 11 retrieval unit tests; strict Clippy for
types/retrieval/gateway; 19 ContextRun, 10 context-pack, 7 lifecycle, 28 Session
and 8 Capture public-API tests on ordinary-role PostgreSQL. Capture's separate
serial migrator-lock acceptance also passes under its explicit ignored-case
invocation. The focused `bash demos/mem-8-saved-delivery.sh` passes six public-API
cases on its own fresh fixture and removes its owned volume. The generated
OpenAPI has 287 schemas/177
operations; console generation and the SDK's 15-operation/53-schema digest are
current. Console compilation, 264 tests and its production build pass with
pinned pnpm 11.13.1; both SDK languages have 10 passing tests. Repository static,
dependency, demo and strict workspace Rustdoc gates pass.

`make eval-product` passes all 26 scenarios and unchanged six zero-tolerance
trust gates on a fresh PostgreSQL 17.11/pgvector 0.8.6 fixture. The report binds
inspection HEAD `4729aa5d9ef6929988bf01d1a9640c7a865ca8bc` and dirty patch SHA-256
`28553539a2b7bc7180f6c8d0e2c3f4ea89ca686f67f9e7841b109d3f6388d0ac`.
That run measures 675/446 off/conservative rendered-text tokens in the existing
optional fixture, four unchanged exact required-body pairs, 12/12 checkpoint
facts and four-sample local assisted p95 30.037625 ms. It does not measure model
quality, provider usage or cost. Reports remain under ignored
`target/product-evaluation/`; this brief records their actual boundary.

Full A remains incomplete: final admission/removal ordering, scoped source
withdrawal/suppression and crash-safe cleanup are not supplied by replay proof.
D and G are absent; E's structured
method and second-client receiving contract and H's integrated demonstration
remain to implement. B's offline causal oracle cases and F's further task/host
allowance contracts remain open. No external credential blocks these
deterministic implementation steps.

### Private delivery lineage increment (2026-10-14)

ADR-0144 adds immutable private per-run manifests independent of optional trace
retention. New saved deliveries replay their exact receipts in all four modes;
Knowledge/graph/checkpoint evidence, current authored channels/pins and immutable
Skill bindings/versions are rechecked. Older nonempty unproven receipts still
refuse uniformly. No manifest fields enter public diagnostics. Governed item
erasure clears addressed rendered copies in every mode; content-free dependency
proof remains. This is item cleanup, not source-event erasure or suppression.

Additive migration `0004` leaves published `0001`–`0003` unchanged. Exact baseline,
forward and login prefixes retain their previous catalogue checks and additive
upgrade path. The raw-migration authority reporter independently measured:

| Inventory | Rows | BLAKE3 |
| --- | ---: | --- |
| Application ACL | 363 | `4ad61b01acec1e2bdf2e42ed369c76d2108daf9a3d27b9b18e8a1fc380604799` |
| Routine | 71 | `20ab135f1741edb64af6e6df1d8dddb06b113db0cbf083238c3beb9db0b4c259` |
| Trigger | 115 | `c3cd252d613d8a7f2be43f6a01e144b8a0dbb11b395855c791cd7407997fd60c` |
| Forced RLS | 95 | `aa6f293275476565a187599c124e27d1db9363ee8ce0a023f55dc654545138f7` |

The migration SHA-256 is
`6339e5518f08807ce7cf509e9c2e1644fb9536ebe17d2c0b4677062f41247631`.
The documented fresh `SYNVEDA_DB_TEST_TASK=sqlx-prepare bash scripts/db-test.sh`
passes generation and metadata check; three new query hashes and the replaced
ledger-bound hash were reviewed. Store epoch/runtime-authority acceptance and
genuine pgvector 0.8.2, 0.8.3, 0.8.4 and 0.8.5 profiles pass; current 0.8.6 is used
by the fresh API/evaluation fixture. Full current-head API acceptance passes
20 ContextRun, 10 preexisting context-pack, 7 lifecycle and 28 Session cases;
the new authored and Skill replay regressions also pass. Adversarial RLS passes
65 ordinary cases (five deliberately separate administrator cases ignored),
plus the explicit serial administrator trace/manifest tamper case. New private
proof is included in the tenant-table completeness guard.

The expanded fresh `make eval-product` passes all 30 scenarios and unchanged
trust/quality gates. It binds HEAD
`f8e8fdb9a5d0987c9357189fb45e09791f694eb7` and dirty patch SHA-256
`6e74650cf46ac5e15d5ca76dffab5e5882d0007e187eeca905e3d2ff454301e2`.
It measures 701/472 off/conservative rendered-text tokens (229 fewer), four
exact required-body pairs, 12/12 declared checkpoint facts and four-sample
local assisted p95 31.501542 ms. Absolute token counts vary with generated
wrapper IDs. Provider/model correctness, host usage and monetary cost remain
unmeasured. The expanded `bash demos/mem-8-saved-delivery.sh` passes nine focused
public-API cases and removes its own volume. An initial focused invocation
mistakenly used the demo task selector, then omitted a required tenant-fixture
credential file; corrected ordinary-role runs pass. No product failure was
inferred from those command setup failures.

### Exact policy replay increment (2026-10-14)

ADR-0145 adds a frozen request-local evaluator for the exact stored source and
typed configuration. It reuses a compile only on exact source/configuration
equality under the same embedded schema/base, with at most 64 packs/1 MiB of
source text and two concurrent local builders. Malformed or unavailable current
policy refuses the original receipt; the process cache is not changed.

The isolated `d59ad7d12753dd00ab8971c6cec63cd0096d6d67` baseline, with the new
regression copied into a managed comparison checkout, returned 200 with Alice's
saved plaintext after an Alice-only policy denial committed. The fixed focused
test returned 409 for Alice and the exact original receipt for Bob while the
process still held version 1. Test policy fixtures now persist their source in
ordinary tenant transactions. The core policy suite has 79 passing tests, and
ordinary RLS/inventory acceptance has 66 passes with five separate administrator
cases ignored. The exact-policy SQLx query was generated and checked on a fresh
current database and reviewed at hash
`cbdfe5778887e21e7d2ebc828a80171a498dbbe4e0551f33c95a88b165d3c6d8`.

The first comparison reused the primary Cargo target directory. Subsequent
binaries reused baseline artifacts, and those failed repeat/evaluation runs do
not qualify the fixed source. All affected workspace packages were rebuilt from
the primary source; future comparisons must use a separate target directory.
The seven saved Knowledge/checkpoint/authored/Skill cases then passed, strict
Clippy passed for policy/store/gateway, the fresh 31-scenario product suite passed
with unchanged trust/quality floors, and the ten-case demo passed and removed
its owned volume. The primary implementation files remained intact. The managed
comparison checkout is archived with its copied regression.

The rebuilt product report binds HEAD
`d59ad7d12753dd00ab8971c6cec63cd0096d6d67` and dirty patch SHA-256
`70dfe58fa6c304709185693aec556a648ea1e33fa6fbd7769c71af9a98ba834b`.
It measures 716/487 off/conservative rendered-text tokens (229 fewer), four
exact required-body pairs, 12/12 checkpoint facts and four-sample assisted
preview p95 32.929792 ms. It does not measure snapshot compile latency, model
correctness, provider usage or cost.

The shared/exclusive authority gate is still unimplemented. This snapshot closes
the already-committed policy replay window; it does not serialize a policy change
committing after its read, and fresh creation/other reads retain their current
refresh contract. Full source withdrawal, suppression and cleanup remain open.

## Non-goals

No second memory store, graph engine, context planner, scheduler or mandatory
model runtime. No changes to FLOW-8 export receipts or remote history. No release,
push, production deployment, HA programme or incidental paid model calls.

## Architecture seam

Reuse Cedar, forced RLS, typed VedaFlow effects, immutable revision-source links,
Session event identities and durable operations. An additive migration must keep
all published migration bytes and prior-head/extension fingerprints and measure
the new catalogue independently. Item forget and source forget are distinct;
one event may support unrelated Knowledge. Current authority is rechecked at
disclosure; historical queries rewind data, never permissions.

### Remaining source-control implementation seam

GPT-6 Astra's final read-only review identifies this next complete vertical
slice; these choices still need a source-control ADR before implementation:

- Reuse typed Knowledge/VedaFlow Apply for separate Session-source withdrawal
  and forgetting, deriving scope from the actual Session, independently
  authorizing source control and resolving current approval requirements.
  Pending review has no removal effect. Existing item `Forget` keeps its meaning.
- Choose shared/exclusive tenant authority mode before authorization or domain
  locks. Cover Configuration, stored policies, grants/groups/membership, identity
  departure, scope status/moves and relaxation changes, including directory
  wrappers. After lock waits, gather current anchors and use one exact frozen
  policy evaluator for all decisions and thresholds; read fresh database time.
- Lock ordered Session/source identities before descendant/operation rows.
  Ordinary `FOR SHARE` on immutable event rows requires unavailable UPDATE
  privileges. Use a narrowly checked bounded definer fence or a short coarse
  Session-row fence; never broaden event UPDATE rights. Append already locks
  Session first, so an event-then-Session order would create a cycle.
- Check suppression in a second statement after acquiring the fence, avoiding
  a pre-wait READ COMMITTED snapshot. Retain a non-cascading alias over tenant,
  original Session, client-event-ID digest and original raw source-payload hash,
  plus the public payload hash and approved effect identities. Check it before
  duplicate payload return and new append, even after scoped event retention.
  Explicit governed reimport creates a new materialization; it does not clear
  the old decision or resurrect old receipts/approvals.
- Check every exact revision's normalized source links, including undisclosed
  descriptors. Access revocation of an undisclosed descriptor differs from
  explicit withdrawal of its content. Flat multiple-source provenance does not
  prove independent support: preserve unaffected revisions/candidates and require
  review for ambiguous mixed support.
- Apply per-revision/per-event eligibility to raw Session diagnostics, append,
  summaries, timeline, quarantine list/release/reject, checkpoint verification
  and chains, Capture input/results/acceptance, Knowledge history/source/usage,
  conflict expansion, OKF export, Context previews/delivery/replay/detail/graph,
  and proposal command diffs. Item authorization alone cannot filter historical
  revisions independently. Quarantine release cannot lift suppression.
- Admit extractor input before releasing the connection for provider work;
  reacquire authority/source fences and check exact frozen input before candidate
  commit. Repeat the check inside publication and delayed apply, including
  inherited Verify/Merge evidence and embedding completion. A late extractor
  cannot change the old batch's input hash to pretend it processed new evidence.
- Add a domain-specific `source_cleanup` path over existing operations, attempts
  and outbox. Current dispatch/claim/complete methods hardcode Skill joins; do
  not pass a new kind through them. Run it under the existing worker generation,
  shutdown and authority supervisors. Each bounded chunk checks a live lease/
  attempt, applies idempotent work and commits cursor/progress/audit together.
  Suppression remains authoritative while cleanup is blocked or retrying.
- Discover and clear affected delivery copies before disposing checkpoint
  excerpts or lineage. Narrow one-way erasure primitives must verify the approved
  effect and exact attempt, not a caller-set GUC. Source/history retained under
  hold is inaccessible, not physically erased. Preserve export receipts and
  remote history. Manually copied authored/Skill prose without exact lineage
  cannot be inferred as an automatic source derivative.

Acceptance needs deterministic barriers for both admission/removal orders,
stale extractor/indexer completion, unchanged append after retention, crash/
reclaim between cleanup chunks, exact current policy after waits, every trace
mode and independent remaining evidence. Additive head `0005` must preserve
head-0004 authority checks, independently measure its catalogue and regenerate
metadata/contracts before enabling the complete source-control path.

## Acceptance criteria

- Saved delivery never bypasses current source authority or removal checks.
- Logical source-removal commit prevents future service admission and unchanged
  replay/ingest resurrection, including previously assembled derivatives.
- Cleanup is bounded, idempotent, crash-resumable and truthful about retained
  versus physically erased content; independently supported content survives.
- Already delivered bytes, independent exports and backups retain their actual
  separate retention contracts.

## Required tests

Use real ordinary-role PostgreSQL/RLS and public APIs for revocation, erasure,
checkpoint retention, stale-worker completion and in-flight ordering. Reproduce
the unsafe replay before fixing it. Exercise all four trace retention modes,
unrelated authorized users and hidden-but-not-disclosed source descriptors.

## Rollout and rollback

Saved deliveries without sufficient dependency proof fail closed and retain
their receipt; a new key requests new composition. A source-removal implementation
must ship complete admission/suppression checks before enabling cleanup. Do not
claim that the first replay repair provides complete source removal or retracts
already delivered external content.

## Dependencies

### Qualified Session source-control increment (2026-10-14)

Sol 6.1 implementation follows read-only Astra 6 max planning and review.
Accepted ADR-0146 and additive head `0005` implement Session withdrawal/Forget
as typed VedaFlow effects, a shared/exclusive tenant admission gate, exact
current stored Cedar decisions, permanent non-cascading event/alias hashes,
private normalized dependencies, immutable revision/change headers and bounded
durable cleanup. ImportArtifact roots remain the next Phase A prerequisite for
inbound reconciliation and learning; MEM-8 remains open.

Fourteen ordinary-role `source_withdrawals` cases pass: raw/history/saved
withdrawal, independent clean replacement, source cleanup followed by item
Forget, exact withdrawal/Forget operation receipts, denial masking with retained
audit, both admission orderings, all four trace modes, paused late embedding and
Capture providers, more than 100 targets with fifth-attempt continuation,
stale-owner/valid-GUC/NULL-bound checks, a second-page hold, retention disposal
and incident suppressed relations. The 21 ContextRun cases, 7 Knowledge lifecycle
cases and 8 Capture cases pass; Capture retains its existing ignored case.
The fresh RLS suite has 66 passing and 5 existing ignored cases. Thirty actual
serial lifecycle cases pass, plus the exact `0004` source backfill upgrade case.
The 31-scenario product evaluation, 264 console cases, console build, changed-
crate strict Clippy, fast/dependency gates and generated OpenAPI/console/SDK
checks pass. Fresh SQLx prepare/check and independently measured authority
preflight pass. Genuine pgvector 0.8.2–0.8.5 functional profiles pass alongside
current 0.8.6. Earlier migration bytes and catalogue checks remain unchanged.

Current-head catalogue measurement:
ACL 383 `dabe07a6c9cb563fd6c8e32eb878a3e9dcf1357df43eeffff2f49964c0f00a8b`;
routines 83 `a9293d4ed4135a39929b0c24423c9c60b0d6df52502b22c15e9e84ac375eb47b`;
triggers 138 `626789db3036c5c5c1f563cd599bd109885b8acfa764cebe72267c2e40b06f49`;
RLS 113 `99d158d3906a9e574c7a5238f70ded0ddfccf139f29df3d2c28ddc6d87053a6b`.

Operator commands are source-build CLI commands in `docs/CONSUMER_CLI.md`.
All ten retained disposable fixtures were removed after checking their exact
Docker labels and immutable network ownership receipts, including private
credential state. Demo command checks and source CLI help pass.
Blocked or exhausted cleanup remains inspectable and terminal; suppression
remains effective. Holds retain inaccessible content. Already delivered bytes,
exports, remote history and backups retain their separate retention contracts.
No new provider/model quality, host-token usage or cost measurement is claimed.

Next executable increment: record the ImportArtifact-root extension decision,
add typed normalized artifact provenance and additive `0006`, and qualify
withdrawal, permanent alias suppression, precise dependent cleanup and import/
Capture concurrency using actual OKF artifacts. Then continue MEM-9 local and
pinned-private Git reconciliation, structured CTX-6 handoff, SKIL-6 outcome
candidates and the integrated CPR-41 domain demo. No external credential blocks
that local work. Real private GitHub and native-model qualification require
their named authorized environment; no A–H completion is claimed.

### Qualified ImportArtifact source-control increment (2026-10-14)

Accepted ADR-0147 and byte-pinned additive `0006` extend source control to actual
OKF artifacts. Typed provenance and parallel private flat roots preserve the
qualified Session contract. Exact scoped source/path/hash aliases survive job
recreation and managed repository detach/reattach. Suppression masks job notices
and fences plan replay/materialization; changed bytes remain reviewable. All
source controls use current Cedar, typed VedaFlow and the tenant admission gate.
The same SourceCleanup worker erases affected service copies in pages of 100,
checks holds before erasure and retains exact operation receipts. Item Forget
retains its inert mapping replay contract in either cleanup order.

Nine artifact cases, fourteen Session controls, twenty-one ContextRuns, seven
Knowledge lifecycle cases, eight Capture cases and sixty-six RLS cases pass.
Existing Capture/RLS ignored cases remain unchanged. All 32 actual serial epoch
cases pass, including genuine `0005` complete provenance normalization,
ambiguous lineage refusal and generic Document metadata preservation. Fresh
SQLx prepare/check generated 22 reviewed replacement/new query hashes; product
migration preflight and idempotent apply pass. Strict changed-crate Clippy,
193 types cases, 264 console cases/build, generated OpenAPI/console/SDK checks,
fast/dependency gates, all 31 product scenarios and genuine pgvector 0.8.2–0.8.5
profiles pass alongside default 0.8.6. Earlier migration bytes and measured
catalogues remain unchanged.

Current-head catalogue: ACL 400
`33ec00dcd21f3acb45db8c7e189d19d909dffdf54987acb382e7e4606dfa659f`;
routines 93
`ae644352f93d9e324c14a7f373f25863f3728eff05ada9ce33e1313cf20dba98`;
triggers 154
`4df9483d71839a52dbd982b177327e444767a98d244d6030e3be53f32c439042`;
RLS 120
`f772bf89cda01243e13a78f1a0ec2fc04693f646070dad8831521438dd6c1727`.
Migration SHA-256:
`28a9f6f3a510c50137f9c99a2b352b2065593690bd4d7fa03440db3693fef7f2`.
All seven retained draft fixtures and private credentials were removed after
exact Docker-label and immutable network-receipt ownership checks. Fresh
qualification fixtures removed themselves through their normal lifecycle.

Reproduce artifact acceptance with
`bash scripts/db-test.sh -p synveda-gateway --test artifact_withdrawals --test source_withdrawals -- --test-threads=1`;
use the workspace fixture's dedicated lifecycle role files for the serial epoch
suite, and the documented `sqlx-prepare`, `product-evaluation` and genuine
`pgvector-0.8.2` through `pgvector-0.8.5` tasks for deeper qualification.
Operator interfaces are in `docs/CONSUMER_CLI.md` and the OKF console.

Phase A's existing producer families are qualified. MEM-8 remains open to carry
this contract into new reconciliation, structured handoff and procedure-outcome
producers. Next: accept the MEM-9 descriptor/observation/inbound authority ADR,
then implement durable local Git reconciliation and targeted path freshness,
followed by the pinned private GitHub path and remaining CTX-6/SKIL-6/CPR-41
increments. No external credential blocks local implementation. Live private
GitHub and native/model qualification remain separate environment-dependent
checks. No full A–H completion is claimed.


### Qualified grounded handoff increment — 2026-10-14

ADR-0150 extends the existing Session ledger with exact-evidence TaskState,
complete replacement checkpoints and an independently authorized source
snapshot. Ten handoff cases plus the existing ContextRun, Session, source and
lifecycle suites pass on a fresh `0009` fixture. Current classification ceilings,
both root families and indexed Knowledge copies govern disclosure and cleanup.
Artifact cleanup includes its final copied-Session phase; retained headers
prevent old-goal fallback and retain concurrency evidence after erasure.
Same-window replacement, batched compaction markers and scoped-retention
recovery have explicit regressions. `CTX-6.md` and the operator handoff guide
own the contract and controlled evidence. Live vendor/model quality remains
open. Next implement authenticated outcomes and qualified small Procedure
candidates under SKIL-6, then the health/budget/domain demonstration slices.
