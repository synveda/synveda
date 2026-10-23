# ADR-0147: Apply source removal to exact imported artifacts

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Source lineage and lifecycle invalidation, Inbound Git source reconciliation, OKF v0.2 knowledge exchange adapter
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

ADR-0146 qualifies Session source removal. ADR-0087 imports real artifacts into
the same Capture review workflow without synthetic Sessions. Those artifacts,
their mappings and accepted revisions are persistent source copies and must
share the removal boundary before inbound Git or procedure learning is added.
An arbitrary metadata artifact id does not establish exact provenance.

## Decision

Add typed `import_artifact_id` provenance and a private, flat artifact-root
projection beside the event-root projection. Keep the qualified Session
addresses and cleanup primitive. The shared eligibility function checks both
root families; strict publication never uses cleanup exceptions. Each mixed
revision becomes unavailable if either supporting root is removed. Independent
later revisions and unrelated artifacts remain eligible.

Retain immutable artifact addresses with actual tenant/job/project/scope,
workspace, path digest, source identity digest and original content hash.
Managed imports retain their repository attachment id and a server-derived
digest of tenant/project and lower-case canonical URI. That digest survives
detach/reattach; a caller's Git label cannot establish it. Other OKF imports bind
their admitted source kind and locator. Permanent aliases bind that source
identity, scope, path and original bytes. They survive job recreation. They are not a
global content blacklist. Changed bytes may create a new reviewable candidate;
an unchanged suppressed alias requires separately governed new admission and
cannot clear the old suppression or restore its receipts.

Add a typed `ControlArtifact` Knowledge/VedaFlow command with an exact inspected
artifact hash, actual scope, mode and bounded reason. Current project-scoped
KnowledgeRead inspects the source, KnowledgeWrite and ProposalOpen govern the
intent, and KnowledgeForget additionally governs physical cleanup. Pending
review removes nothing. Resolve all authority after the existing exclusive
tenant admission wait. Suppression and the applied operation commit together.

Reuse `SourceCleanup` operations, attempts, outbox and the same worker with a
closed Session/artifact target branch. Artifact cleanup retains owner-only
authorization bound to the exact applied command, live attempt, lease and
dependent root. Scan all revision holds before clearing any content. Page
delivery, candidate/decision, pending command, revision, embedding, relation,
descriptor, mapping and artifact copies at no more than 100 targets per page.
Retain ids, hashes and tombstones, and clear artifact body/frontmatter, replace
its path with the unique inert `erased/<artifact-id>` marker, clear
producer mapping text and affected job notices. Mask notices at logical removal
so asynchronous cleanup cannot expose them. Item Forget keeps its separate
checked capability and can erase artifact-suppressed revisions.

Derive existing artifact links only from complete matching job/artifact,
candidate and normalized descriptor tuples, including scope, locator, revision
and content hash. Do not trust `metadata.artifact_id` alone. Track ambiguous
legacy lineage privately and fail closed where proof is missing; no migration
may fabricate source authority. New descriptors and candidate links validate
their exact source before publication. Check aliases before duplicate import
replay, artifact creation, materialization and candidate decisions.
Frozen-plan replay requires complete visible artifact and mapping coverage.
An explicitly erased mapping remains an inert, non-materializable state under
the existing item Forget contract; a hidden source row cannot become a silently
filtered materialization.

Use additive head `0006`; preserve all earlier migration bytes and fingerprints,
measure its catalogue independently, and regenerate SQLx and public contracts.
No transport work, synthetic Session or separate review workflow is introduced.

## Options considered

1. **Parallel flat projections (chosen).** Extends the common eligibility
   boundary while keeping the qualified event-keyed projection intact.
2. **Replace event keys with a generic root graph.** Rewrites every qualified
   Session trigger and cleanup proof without a product need for a graph.
3. **Trust metadata or hide only artifact GET.** Leaves accepted Knowledge,
   saved deliveries and repeated imports able to recover removed content.

## Consequences

The existing coarse tenant fence also serializes artifact controls. Cleanup
retains inaccessible held content and may reach a terminal blocked or exhausted
state. Exact source identity permits independent evidence with the same bytes;
managed Git identities cannot be retargeted to bypass an alias. Revisit the
parallel projection if another supported root family creates demonstrated
duplication that requires a separately reviewed migration.

## Compliance notes

Cedar, forced RLS, typed VedaFlow, static SQLx checked store SQL and content-free
audit remain mandatory. External Git history, exports, backups and already
delivered harness bytes retain their separate retention contracts.

## Validation

Nine ordinary-role artifact controls pass, including aliases and saved context,
independent evidence, both item/source Forget orders, pending review, typed
provenance, denied/absent equality with audit, holds, reader admission and
102-revision cleanup with fifth-attempt continuation, stale-owner rejection and
NULL page-bound refusal. Fourteen Session controls continue to pass on head
`0006`. All 32 actual serial migration lifecycle cases pass, including the real
`0005` complete/ambiguous/generic provenance backfill. Fresh SQLx prepare/check,
independent catalogue preflight, all supported genuine pgvector profiles,
31 product scenarios, strict changed-crate Clippy and generated contracts pass.
