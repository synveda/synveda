# ADR-0101: hardening preserves trust boundaries and makes failure bounds explicit

- **Status**: Proposed
- **Date**: 2026-08-26
- **Feature(s)**: CPR-44
- **Deciders**: Production-hardening review

## Context

The context-platform MVP has strong policy, tenancy and immutable-governance
boundaries, but a fresh audit reproduced defects in external pagination,
token time validation, audit verification, governed erasure, context database
use and operator key handling. It also found high coupling and repeated
transport shells in large gateway modules, dishonest frontend failure states,
and current documentation that contradicts executable evidence.

Several release blockers are real but cannot be closed by a refactor alone:
the Helm package does not yet have a complete published artifact set, no
backup/PITR restore drill exists, and legal ownership has not chosen a project
licence. The chart's missing reference to the already shipped local KMS
provider is code-local and can be fixed, but a reference is not evidence of
external custody or recovery. Treating a local test or prose update as closure
would make the readiness record less trustworthy.

## Decision

1. **The trust architecture stays fixed.** Cedar remains the decision point,
   forced RLS the database backstop, VedaFlow the governed mutation seam, and
   the tenant-complete hash chain the audit record. CPR-44 adds no bypass,
   alternate workflow, compatibility model or second public contract.
2. **Only reproduced defects receive semantic fixes.** External continuations
   must stay on their configured origin and share visible page, item, byte and
   cycle budgets. Service tokens must satisfy `iat <= exp` and a bounded future
   skew. Provider failures keep status and fixed classification but never raw
   response content.
3. **Verification and erasure are transactional invariants.** Audit verify
   freezes the head sequence and hash once and scans no row beyond that prefix.
   Knowledge erasure removes derived plaintext and nullable live Knowledge
   addresses from context, import and conflict evidence under one narrowly
   scoped transaction flag; hashes and content-free audit evidence remain.
   Immutable triggers admit only that exact scrub shape.
4. **A context request never waits for the pool while holding one of its
   connections.** Graph planning runs in a bounded transaction stage and the
   final persistence stage re-authorises and revalidates every selected
   revision before commit. Degradation remains explicit and preserves the
   authorised lexical/vector result.
5. **Preserved data keeps its key.** Default uninstall retains the deployment
   KEK whenever database volumes survive. Key destruction is coupled to the
   explicit purge path; warnings are not consent to irreversible loss.
6. **Restructuring follows responsibility, not size.** Local access, planner,
   trace, command and query seams may be extracted when they break an observed
   conceptual cycle or give pure logic a direct test. Repeated response and
   request-bound mechanics become narrow gateway helpers. No generic workflow
   framework or new cross-crate domain layer is introduced.
7. **Current evidence has one owner.** Per-feature files retain acceptance and
   evidence; generated or concise indexes may point to them. The completed
   prompt journal and copied phase narratives are deleted after current risks
   and decisions move to their owning ADR, open feature or readiness record.
8. **Readiness is a release property, not a test synonym.** Remaining artifact,
   KMS, backup, HA, abuse-control, signing, upgrade and external-verification
   gaps stay Not ready with explicit acceptance criteria. CPR-44 cannot label
   the product production-ready while any release-blocking gap remains.
9. **Helm accepts only configurations the gateway can start.** Extractor
   validation mirrors the gateway's closed implementation set and requires
   every setting that implementation needs. The chart requires an existing
   Kubernetes Secret containing the deployment KEK and stable key reference;
   it never generates, copies into values, prints or owns that material. The
   reference closes the missing configuration seam for the shipped local KMS,
   not external custody, rotation or restore evidence. Gateway and CNPG image
   names may change only with matching release build/publish automation; local
   image preloading is not publication evidence.

## Options considered

1. **Bounded fixes and local cohesion seams (chosen).** Reduces demonstrated
   risk while keeping the product contract and security layering reviewable.
2. **Split every large file and build a generic service framework.** Rejected:
   line count alone does not establish coupling, and generic orchestration can
   hide authorisation and mutation order.
3. **Restrict the work to prose and formatting.** Rejected: the reproduced
   credential, erasure, verification, pool and key-loss paths are behavioural.
4. **Close operational gaps by weakening their evidence standard.** Rejected:
   local images are not released artifacts, replication is not a backup, and
   deterministic replay is not a live client.

## Consequences

- Positive: semantic changes have explicit failure paths and regression tests;
  structural changes reduce actual coupling; support claims follow executable
  evidence.
- Negative: the final verdict remains Not ready until separately owned release
  and recovery work lands; erasure scrubbing and staged context transactions
  add targeted complexity where durability requires it.
- Reversal trigger: a second implementation genuinely needs shared workflow or
  planner polymorphism, or measurement proves a local seam cannot meet its
  resource bound; introduce the smallest abstraction backed by both callers.

## Compliance notes

Every ordinary read and mutation continues through the current PDP, tenant
transaction and audit seam. Erasure is the sole append-only exception and is
restricted by transaction-local database guards. No request or response gains
a tenant or acting-principal field. Generated API files and SQLx metadata are
regenerated from source if an accepted fix changes their inputs.
