# ADR-0148: Reconcile bounded inbound Git sources through Capture

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Inbound Git source reconciliation, Source lineage and lifecycle invalidation, OKF v0.2 knowledge exchange adapter, Conflict, supersession and freshness engine
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

Git export is an outbound authored-channel disclosure contract. Its permissions,
credentials and cursors do not authorize inbound reads. ADR-0147 establishes
actual ImportArtifact provenance and permanent suppression. Inbound observation
must reuse those artifacts and Capture review without adding a raw-source store.

## Decision

Add optional, omitted-when-empty `git_sources` to governed Configuration. At most
16 sorted stable source ids bind an actual project repository, deployment target,
exact `refs/heads/*` ref and at most 32 sorted exact Markdown paths. An enabled
descriptor applies only at its actual project scope. A source id cannot retarget
its repository identity. Disconnect disables future reads and preserves approved
Knowledge and retained observations.

Introduce scope-only Cedar `SourceRead` (`source.read`). Refresh additionally
requires existing project KnowledgeWrite. Configuration grants neither action.
Status and expansion independently decide their applicable source, Knowledge and
Capture reads. Public refresh queues `SourceReconcile` runtime work; it does not
publish Knowledge. Only existing typed Capture/Knowledge VedaFlow commands can
publish an imported candidate.

Use the current durable operation, attempt and outbox ledgers and supervised
worker, with one immutable source target and one bounded execution lease. Persist
the requesting User identity. Before transport, resolve exact current stored
Cedar, effective Configuration, repository and credential custody. Release the
database connection while reading Git. Before committing, reacquire exclusive
tenant admission and repeat authority, descriptor, credential revision, lease
and suppression checks. Commit complete observation, import/candidate work,
targeted freshness and successful cursor atomically. A failed or partial fetch
records unknown/unreachable status and advances no successful cursor.

The `synveda-git` leaf reads immutable commit/tree/blob objects only. Deployment
allowlists narrow exact tenant/scope targets. Local reads use operator-owned bare
repositories, without checkout, filters, hooks, submodules or lazy network fetch.
Resolve the configured branch once. Limit paths to 32, depth to 16, each blob to
128 KiB, total bytes to 2 MiB, object requests to 128 and total execution to
30 seconds. Refuse unsupported entries and incomplete/missing objects. Successful
tree lookup may establish an exact absent path; an error cannot establish deletion.

The remote leaf supports fixed `api.github.com` HTTPS Git-data reads with no
redirects, proxy or arbitrary URL. Deployment pins canonical owner/name and an
independently obtained immutable private repository id. Custody uses a distinct
`source_read` tenant-secret kind and `synveda-github-source-credential-v1`
document bound to that id, with Contents read permission. Outbound credentials
fail both checks. Credential rotation/revocation takes exclusive tenant admission.
Current custody must match again after transport. Never put tokens in native Git
arguments, audit or transport errors. Preserve the existing outbound transport.

Content-free observations retain commit/tree, descriptor digest and each allowed
path's content hash/artifact identity or proven absence. Successful cursors refer
to committed observations. Fetched plaintext lives only in existing OKF artifacts
and Capture candidates. Unchanged allowed bytes reuse their prior source identity
and produce no proposal, even if an unrelated commit changed. Deletion-only and
unchanged scans create an observation without a fabricated empty import job.
Compare managed updates only within the exact configured source and path lineage.
Managed job deduplication also binds the exact compared current revisions. Known
source bytes returning after a different approved revision produce a new review;
repeated work against the same head reuses that review. Original artifact aliases
remain permanent across all of these jobs. Semantic defaults cannot turn the same
admitted bytes into a change merely because observation time advanced.
Freeze import time from the durable database request for deterministic retries.

Freshness compares an exact revision's artifact roots to the latest complete
observation for that configured source. Relevant changed/deleted paths require
review for all applicable Knowledge types, including Procedure. Unrelated paths
do not change freshness. Do not modify coarse repository `content_revision`.
Force-push and rename are complete snapshot differences; unavailable history or
partial reads remain unknown. Reconnection and retries preserve ADR-0147 aliases.
Private repository ids strengthen source identity across remote rename.

## Consequences

Explicit refresh is the first execution interface; no new scheduler is added.
Local real-Git and deterministic HTTP fixtures qualify implementation separately
from a live private repository. Native/model outcomes and enterprise readiness
are separate claims. This accepted design is under implementation; feature
inventory and readiness remain open until runnable acceptance passes.
