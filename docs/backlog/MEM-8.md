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
| A: lineage/lifecycle | Immutable revisions/sources, inline governed forget, Capture fences, checkpoint hashes; source removal incomplete | MEM-8; narrow CPR-16/20 and CTX-6 repairs | Revoke/remove evidence during replay and queued derivation; suppress unchanged reingest; preserve independent evidence | `bash scripts/db-test.sh -p synveda-gateway --test knowledge_lifecycle --test context_runs --test sessions_api --test capture_api -- --test-threads=1` |
| B: health/diagnosis | CPR-21 Inspector and feedback delivered | CPR-21/40: authorized findings, actionable links, stage diagnosis and unknown outcomes | Distinguish uncaptured evidence, retrieval miss, qualifier loss and wrong downstream answer without denied counts | ContextRun tests, console tests, `make eval-product` |
| C: values/applicability | CPR-37 conflict review/freshness delivered; lexical classifier retains both propositions | CPR-37: bounded typed revision metadata, significant values/case, applicability overlap | Retry 3 vs 5 is reviewable contradiction; production vs development is separate applicability | Types tests, lifecycle/capture database tests, `demos/cpr-37-conflict-freshness.sh` |
| D: inbound reconciliation | Repository identity and outbound FLOW-8 exist; inbound absent | [MEM-9](MEM-9.md): separately governed local/private Git reads and path/hash observations | Relevant path change proposes review once; unrelated change does not stale everything; crash does not advance cursor | New source-reconciliation database suite, Git tests, dependency/contract gates |
| E: structured handoff | CTX-6 deterministic excerpt checkpoint implemented, live task quality open | [CTX-6](CTX-6.md): new verified method, structured evidence, explicit second-client receiving contract | Resume through Claude and manual Codex public API with exact evidence and unknown operation outcomes | Session/context database tests, both adapters, registry checks; separate live qualification |
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

No external credential is needed for deterministic work. New source authority,
suppression and delivery-admission ordering require accepted decisions before
schema implementation. Real harness/model quality and inbound private-provider
qualification require a named authorized environment and explicit spend if any.
Next executable increment: accept a Session-event source-withdrawal contract,
using exact source-event row locks and permanent scoped suppression. Implement
the typed governed operation, final admission checks, per-revision/all-source
eligibility and Capture completion/acceptance fences before enabling bounded
cleanup or persistent outcome learning. Preserve the distinction between an
undisclosed private descriptor's access revocation and explicit withdrawal of
the source content supporting a conclusion. No external credential blocks this
local source-family implementation.
