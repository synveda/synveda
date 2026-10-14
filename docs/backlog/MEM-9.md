# MEM-9: Inbound Git source reconciliation

## Problem and evidence

At `4729aa5d9ef6929988bf01d1a9640c7a865ca8bc`, FLOW-8 exports authored
Prompt/ContextPack channels. It does not authorize inbound source reads. Current
repository freshness compares repository revisions broadly, without path/hash
dependency observations. The [implementation matrix](MEM-8.md#scope) records
this as absent, separately from the delivered export path.

## Scope

One bounded local Git fixture and one narrowly allowed read-only private remote
path; governed descriptors, immutable observations, durable reconciliation and
reviewable source-backed candidates. Provide setup/status/refresh through public
API/CLI and applicable console seams. Relevant path hashes drive targeted review.

## Non-goals

No outbound export refactor, arbitrary URL fetching, script/hook/submodule
execution, universal connector, code-indexing platform or second scheduler.

## Architecture seam

Reuse canonical repository identity, governed Configuration/secret custody,
synveda-git transport leaf, durable operations and typed Knowledge review. Define
inbound authority and credential purpose separately in an accepted ADR before
implementation. Commit observations and candidate work before a successful cursor.
Release database connections before transport waits. MEM-8 suppression applies
before fetching and again before materialization.

## Acceptance criteria

Initial scan creates reviewable candidates; relevant changes target dependencies;
unrelated changes do not invalidate them. Duplicate/restarted work cannot advance
past uncommitted proposals. Deletion, rename, force-push, missing objects,
credential rotation/revocation, disconnection and network failure have distinct
recoverable outcomes. Unknown fetch state never implies deletion.

## Required tests

Local real Git fixtures and ordinary-role source-reconciliation database tests;
transport bounds/failure tests, Cedar/RLS isolation, exact observation hashes,
cursor crash/retry and prior suppression/re-import tests. Generate all public and
SQLx contracts with documented commands.

## Rollout and rollback

Disabled by default under governed Configuration. Disconnect stops future
synchronization and preserves approved Knowledge. Retain observations/cursors
for safe recovery. Remote qualification is distinct from deterministic replay.

## Dependencies

MEM-8 Session and actual ImportArtifact lineage/suppression are qualified at
additive `0006` under accepted ADR-0146/0147. The local descriptor/observation and inbound authority contract is qualified
below. The next implementation adds narrowly allowed private GitHub reads. Reuse typed artifact provenance,
Capture review and permanent aliases; do not label caller uploads as managed
Git identity. No external credential blocks that local work.

### Qualified local source increment (2026-10-14)

Accepted ADR-0148 implements disabled-by-default governed descriptors,
scope-only SourceRead, runtime SourceReconcile requests and once-pinned local
Git reads at additive `0007`. Existing ImportArtifacts/Capture carry all
plaintext and review; private content-free observations advance atomically with
candidate work. Exact project/source/path/hash lineage drives freshness. A
managed review digest binds the compared current heads, so a reviewed 3→5→3
reversion creates one new review and repeated work reuses it. Permanent artifact
aliases continue to suppress withdrawn bytes across refs, jobs and reconnection.

Public source list/status/refresh, operation receipt/cancellation, CLI `sources`
and the console Configuration/Imports path are implemented. Current stored Cedar
is decided under admission before fetch and before commit. Credential writes now
use exclusive admission in preparation for the separate remote increment.
[Operator setup and commands](../integrations/git-sources.md) define the actual
native Git/private-root prerequisites and bounds.

Actual qualification: six real-Git public-API cases pass, including independent
refs, targeted updates, reviewed reversion, concurrent idempotent requests,
persisted-pin crash recovery, an actual partial-observation rollback, failed
object reads, disconnect/cancellation, private denial and caller-upload identity
separation. Two source-reader unit cases and the complete 12-case Git leaf suite
pass (controlled loopback TLS tests require socket permission). Nine artifact,
14 Session-source, 21 ContextRun, seven lifecycle and eight Capture cases pass;
Capture's pre-existing migrator-lock case remains explicitly ignored in this
ordinary-role invocation. All 33 actual serial epoch cases pass, including the
exact `0006` upgrade. The 66 ordinary RLS cases include four new forced tables;
five pre-existing administrator cases remain explicitly ignored. Every genuine
supported pgvector 0.8.2–0.8.6 profile and the 31-scenario product suite pass.

Fresh SQLx generation/check and all 33 new query hashes were reviewed. OpenAPI
has 298 schemas/186 operations; generated console and SDK contract checks pass.
194 types, 11 policy, 264 console and both ten-case SDK suites pass, alongside
console build, changed-crate strict Clippy and fast/dependency/demo gates.
The product report records `acd9df3ab101f2321c6afd7471fbe30a41558ad5` plus patch
SHA-256 `a323ddd5b3291308384fe3dd32a547228af86aee9233e14950dcea1fd05b8df6`;
subsequent audit/cancellation changes passed focused source acceptance. These
are local/protocol results, with no model calls or live private-remote claim.

Measured `0007` authority: application ACL 410 rows / `396c4f1e05e48ae342a346f25575171bcb3795c5e07f5175f83db51b00eb0117`,
routines 96 / `256a43620cb06900f0755ea6df9953afa22e53744fa64b69bef3d0e40e2defa8`,
triggers 161 / `4f2a6552aae921255ccf0152e9b222123131a5960982c8b4b3b032ff78d3c1c0`,
forced RLS 124 / `8d9e7407c9affc3f475d9a974ec68fd3adfa118b3068bb5ec492e1ee32940bfa`.
All earlier migration bytes and independently measured fingerprints remain pinned.

The feature stays open for private GitHub reads, persistent provider identity
across rename, credential rotation/revocation during fetch, controlled HTTPS
acceptance and separate live qualification. Next add the immutable provider-ID
binding and distinct source-read allowlist/credential, reuse the durable local
commit path, then continue CTX-6 structured handoff and SKIL-6 verification.
No credential blocks that implementation or controlled transport tests.
