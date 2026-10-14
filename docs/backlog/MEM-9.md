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
additive `0006` under accepted ADR-0146/0147. The next implementation starts with
a bounded descriptor/observation and inbound authority ADR, then durable local
public-API reconciliation before remote reads. Reuse typed artifact provenance,
Capture review and permanent aliases; do not label caller uploads as managed
Git identity. No external credential blocks that local work.
