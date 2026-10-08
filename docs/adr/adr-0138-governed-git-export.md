# ADR-0138: export authored channel evidence through an append-only Git projection

- **Status**: Accepted
- **Date**: 2026-10-04
- **Feature(s)**: FLOW-8
- **Deciders**: implementation under the owner's FLOW-8 request

**Extended 2026-10-05 by [ADR-0139](adr-0139-private-github-git-export.md):**
the pinned private GitHub HTTPS transport and stable tenant-secret custody are
source-tested. Identity-bound v2 operational receipts extend the recovery
decision below; the deterministic source projection remains v1. Credentialed
provider and deployment/recovery qualification remain open.

## Context

Prompt and ContextPack channels have immutable objects and commit DAGs in
PostgreSQL. Git reviewers need inspectable history and independently verifiable
evidence. Rewinding a source ref or setting a pin must not silently force-push
the exported history. Disclosure needs current authority even on replay.

## Decision

1. Ship the brief's **local bare-repository rollout first**. A deployment-owned
   private root enables the transport; an immutable governed Configuration's
   sorted `git_export_targets` enables destination identifiers per scope. Both
   defaults are disabled. Repository identity is tenant/scope/target, never an
   asset name or caller-supplied filesystem path. ADR-0139 separately extends
   this boundary to pinned private GitHub repositories, existing tenant secret
   custody and fixed verified-HTTPS egress. Local transport grants no remote
   authority; live-provider qualification remains separate.
2. Add `ChannelExport` at the channel scope, alongside `ChannelRead` and exact
   sensitivity-aware PromptRead/ContextPackRead at **each object's original
   governing scope**, including merge ancestry. Built-in packs require a scoped
   administrator for export. Configuration only narrows this authority.
3. A replaceable `synveda-git` leaf renders the v1 projection and owns Git
   transport. Domain crates neither execute Git nor gain transport dependencies.
   Use installed Git plumbing with isolated configuration, disabled hooks,
   bounded subprocess I/O/time and no network protocols. Git's GPL executable
   is an external operator prerequisite, not linked into the permissively
   licensed core. This supersedes ADR-0003's prospective gitoxide selection for
   this local increment. SHA-1 names Git objects only; BLAKE3 remains the
   evidentiary integrity rule.
4. Each VedaFlow commit maps deterministically to one unsigned Git commit,
   preserving ordered parents, identity-ID authorship and source timestamps.
   Raw canonical object bytes and source tree/commit metadata are retained;
   asset paths use injective UTF-8 hex encoding. Ed25519 signatures and key IDs
   remain VedaFlow evidence, verified with independently supplied public keys.
   No VedaFlow signature is labelled a Git-native signature.
   Offline verification follows the complete retained export-state chain,
   including signed history superseded by source rollback. A target's cursor
   caps that chain at 1,024 states and 64 MiB of cumulative manifests so every
   accepted export remains inside the verifier's work bound.
5. One branch per asset/channel points to a deterministic **export-state
   commit**. Its first parent is the previous exported state, followed by the
   source head and distinct pin. Its manifest binds exact source ref state and
   all object/tree/commit mappings. Rollback and pin transitions therefore
   fast-forward this branch while source commits retain their original DAG.
6. Persist prepared/completed content-free cursors as immutable VedaFlow
   Configuration-kind objects/trees/commits under non-channel
   `git-export/<target>/<asset>/<channel>` refs in the existing forced-RLS
   tables. These operational receipts are not Configuration distribution
   channels or an alternative settings authority. Cursor updates are ordinary
   compare-and-swap ref advances. No baseline or schema migration is needed.
7. Commit a prepared cursor and disclosure-intent audit before writing to the
   destination, then gather fresh Configuration and Cedar decisions before the
   external effect. Resume rehydrates the prepared exact source heads and
   reauthorizes every object. The transport accepts only the recorded previous
   head or the exact prepared result, using atomic Git compare-and-swap. A
   crash after Git advancement is recovered by completing that same cursor.
   A divergence, deletion or unexpected head refuses; there is no destructive
   reconciliation route. An approved new target ID starts a separate private
   repository and retains the old destination and receipt.
8. Audit records actor, scope, target ID, asset/channel, source head/pin,
   outcome and mapping digest, never paths, messages, exported content or
   credentials. Closed outcome counters and a bounded operation duration
   provide telemetry. The public API and CLI share this authority seam;
   offline verification requires neither bearer nor database.

## Options considered

1. **Local Git plumbing and deterministic evidence (chosen):** runnable without
   a forge or credential, with real Git interoperability and existing RLS
   persistence. The local root needs operator custody and retention.
2. **Mirror source refs directly:** natural branch tips, but a source rollback
   requires a force-push and a pin cannot be represented honestly. Rejected.
3. **Add a forge-specific queue and credential now:** requires unselected
   custody/egress/provider semantics. Defer until the local acceptance passes.
4. **Let the CLI read PostgreSQL and push:** bypasses the public API/PDP/audit
   boundary. Rejected.

## Consequences

- Positive: deterministic, independently verifiable Git history; recoverable
  disclosure intent; rollback/pin evidence without destructive Git updates.
- Negative / accepted: bounded exports refuse oversized histories; local
  files and PostgreSQL cannot commit atomically, so prepared receipts explicitly
   represent that recovery window. Revocation stops future disclosure but does
   not erase already exported Git content. Credentialed remote rollout remains
   open under ADR-0139.
- Reversal trigger: a measured legitimate history exceeds the bounds, or a
  selected forge requires resumable network transport: accept a pagination/
  provider ADR preserving these integrity and authority rules first.

## Compliance notes

Cedar, current governed Configuration, ordinary tenant transactions and forced
RLS remain mandatory. Existing immutable VedaFlow writes retain operational
receipts; enable/disable/new-target mutations use the existing typed
Configuration proposal lifecycle. Offline hash/signature verification grants
no runtime access. No database baseline, compatibility path or Git import is
introduced.
