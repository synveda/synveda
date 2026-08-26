# FLOW-8: Git bridge — export

## Problem and evidence

VedaFlow already provides immutable content-addressed objects, commits, signatures when present, and governed draft/published channels for Prompt and ContextPack assets. Reviewers who work in Git cannot inspect that history in a real repository, but naïvely copying mutable heads or translating a VedaFlow signature into a Git signature would lose the evidence Synveda is meant to preserve. No Git bridge exists in the current code.

## Scope

- Export authorized Prompt and ContextPack channel history to a configured private Git repository with deterministic paths, byte-stable files, commit ordering, authorship labels, and timestamps.
- Persist a manifest mapping each Git object/commit to the exact VedaFlow asset, object hash, commit hash, parent set, channel/ref state, policy snapshot hash, and signature/key identifier when present.
- Preserve VedaFlow signatures as verifiable evidence files; create a Git signature only when an independently configured Git signing identity actually signs the Git commit.
- Make repeated and resumed exports idempotent, detect remote divergence/force-push, and refuse destructive reconciliation without an explicit governed reset.
- Reauthorize every exported asset/ref and record the external disclosure destination and result in content-free audit evidence.

## Non-goals

- Git-to-Synveda import, bidirectional round-trip, Git as authority, or pull-request approval as a VedaFlow approval.
- Exporting Knowledge, Skills, Tools, Policy, or Configuration as if they used VedaFlow channels; Knowledge exchange remains OKF.
- Embedding remote credentials in commits, manifests, configuration documents, logs, or audit metadata.
- Claiming a VedaFlow Ed25519 signature is a native Git commit/tag signature.

## Architecture seam

Add an export application service above `synveda-vedaflow` and a replaceable Git transport adapter outside core domain crates. The service reads exact authorized channel/commit/object history in ordinary tenant transactions, renders a canonical projection, then performs the external write with a secret-plane credential. Mapping and export cursors are tenant-bound governed state; SQL remains in `synveda-store`.

## Acceptance criteria

- A published Prompt and ContextPack history exports to a real Git repository with deterministic trees, parent topology, refs, and a complete hash mapping.
- A verifier can independently validate every retained VedaFlow object/commit hash and signature from the export without mistaking it for Git-native signing.
- Repeating or resuming the same export produces no extra commits; changed history advances only the intended ref.
- Revoked authorization, tenant mismatch, missing secret, non-fast-forward remote, size bound, and network failure stop safely without credential/content leakage or corrupted refs.
- The audit chain identifies actor, tenant-bound export target identifier, asset/channel, source head, outcome, and mapping digest, but no exported content or secret.

## Required tests

- Canonical projection and hash/signature verification fixtures for Prompt and ContextPack histories, merges, pins, and rollback commits.
- Local real-Git end-to-end tests for first export, no-op replay, resume, divergence, force-push refusal, and credential failure.
- Cedar allow/deny/revoke and forced-RLS cross-tenant tests at asset and channel boundaries.
- Size/path/encoding, malicious name, secret-redaction, cancellation, timeout, and bounded-work tests.
- Runnable demo that clones the result and verifies the mapping against source VedaFlow history.

## Rollout and rollback

Ship local bare-repository export first, then one private remote provider behind an allowlist and disabled-by-default configuration. Canary with non-sensitive assets. Rollback disables outbound writes and freezes the last mapping cursor; it does not delete or rewrite the remote repository, and VedaFlow remains authoritative.

## Dependencies

An accepted ADR must fix canonical projection, merge/ref mapping, disclosure authorization, signature semantics, and divergence recovery. The owner must choose Git implementation/licence, remote provider, tenant-safe repository/branch naming, credential custody, egress policy, retention, and who may enable or reset an export.
