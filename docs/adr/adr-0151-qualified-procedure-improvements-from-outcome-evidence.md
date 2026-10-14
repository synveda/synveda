# ADR-0151: Qualify small Procedure improvements against exact outcome evidence

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: SKIL-6, SKIL-5, MEM-8, CPR-40, CPR-41
- **Deciders**: Owner-authorized implementation; GPT-6 Astra max planning

## Context

Skill advertisement, caller-labelled usage and the non-executing validation
sandbox cannot prove task success. Procedure learning needs authenticated
evidence, independent qualification and the existing publication boundary.
ADR-0150 exact-current Knowledge copies cannot represent historical derivation:
advancing the approved head would invalidate an improvement's own old revision.

## Decision

Use the existing Session evidence plane for reserved typed task outcomes and
small human-supplied Procedure improvement requests. Bind exact Procedure,
optional Skill and ContextRun revisions, reported repository/environment,
supporting event hashes and closed outcome states. The initial optional delivery
reference admits bounded Knowledge-only snapshots without restart, authored
channel or Skill-advertisement ancestry; explicit Skill involvement has its own
exact SkillRead check. Ordinary reports stay
unverified. A registered service must pass scoped Cedar `OutcomeAttest` and an
explicit governed reporter allowlist before creating an authenticated execution
receipt. Authentication establishes reporter identity, not universal truth or
causal credit to every delivered procedure.

The first producer appends at most 1024 UTF-8 bytes to one current Procedure,
at the evidence Session's same scope/project and without lowering sensitivity.
At most eight outcomes support one request. The normal Capture worker has a
typed deterministic extraction branch; it freezes and preserves the exact
proposal without model calls. Exact outcome/target/patch identity deduplicates
processing across retries and batches. Contradictory outcomes remain separate.

Freeze the final candidate content hash, evaluator/configuration identity and
owned held-out case digests before accepting runner results. Governed Configuration pins approved evaluator/suite/case digests. All required cases
must pass without a baseline regression. A changed candidate requires a new
attempt. Missing or failed qualification cannot be overridden by review.
Learning requires the current approval matrix to require a reviewer distinct
from the author. Submission edits the same exact Procedure through a dedicated
Capture decision; ordinary Capture Replace retains its Supersede contract.
Publication uses Capture decisions and ordinary typed Knowledge/VedaFlow
effects, with the quality and source checks repeated at apply. No automatic
Skill activation, generated arbitrary-code execution or reflection worker.

Add bounded private historical derivation addresses alongside existing root
projections. Flatten exact historical Knowledge dependencies; mere head advance
does not erase evidence, while current Cedar, item forgetting and both Session
and artifact source controls still govern disclosure. Extend copied candidates,
commands, revisions, Session evidence and deliveries through invalidation and
physical cleanup before enabling their producers. Public disclosure never
inherits authority from a private dependency index.

Keep source/network/model waits outside database transactions. The gateway
records evidence and frozen results; a separately authorized controlled runner
executes owned fixtures. Development cases and held-out variants remain distinct.

## Options considered

1. Trust `host_observed` or sandbox success: neither proves authenticated task
   execution.
2. Directly publish a successful reflection: bypasses review and qualification.
3. Bound authenticated evidence, exact candidates and independent quality gates
   through the existing Session/Capture/VedaFlow path: chosen.

## Consequences

An additive migration preserves all prior bytes/catalogues. The initial scope
improves Procedure Knowledge; exact Skill involvement grants no execution or
activation claim. Controlled deterministic acceptance is separate from live
model quality and proprietary-client qualification. Reviewers can inspect
old/new content, applicability, evidence, uncertainty and frozen evaluation.

## Compliance notes

Embedded Cedar, forced tenant RLS, current source authorization, content-free
audit and VedaFlow remain mandatory. Governed reporter configuration only
narrows permission. All work, source fan-in and retained copies are bounded.

The gateway and worker scan all free-form proposed text and arbitrary revision
metadata. Structurally validated UUID/hash/timestamp proof and server-owned
freshness metadata are excluded from prose scanning, preventing numeric UUID
groups from being mistaken for payment cards. Reserved producer validation and
SQL provenance checks still apply. Frozen specification/results are closed JSON
objects at both DTO and ordinary-role SQL boundaries; they cannot retain extra
plaintext fields beyond their content-free identity/status contract.
