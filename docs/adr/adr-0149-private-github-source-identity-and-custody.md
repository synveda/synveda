# ADR-0149: Preserve private GitHub source identity through read custody

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Inbound Git source reconciliation, Source lineage and lifecycle invalidation, OKF v0.2 knowledge exchange adapter
- **Deciders**: Owner-authorized implementation; GPT-6 Astra max planning review

## Context

ADR-0148's local source path is qualified at additive `0007`. A canonical URI
alone cannot preserve suppression when a GitHub repository is renamed, and
reusing an outbound write credential would conflate independent authority.
Historical artifact aliases and all qualified migration bytes must remain exact.

## Decision

Add immutable private provider-ID/root and actual-attachment/root projections at
additive `0008`. The first configured GitHub binding adopts its existing URI
root. A later attachment for the same pinned provider ID resolves to that root;
a conflicting ID at an already bound URI, or ambiguous existing roots, refuses
admission. Preserve all historical jobs, addresses, locators and suppression
keys. One SQL resolver governs new source heads, import identities, suppression
admission and duplicate lookup. Managed deduplication performs a deterministic
indexed lookup by permanent root and compared-head digest before insertion under
exclusive admission. Keep the old uniqueness constraint and every historical
row: valid `0007` case-only locator variants may already coexist. Public uploads
retain locator-based deduplication. An exact existing immutable binding returns
early; first-binding ambiguity checks use indexed retained root/attachment facts.

The deployment allowlist independently pins tenant, project scope, target,
canonical owner/name, immutable private GitHub repository ID and a `source_read`
secret reference. Its assertion is configured custody, not observed remote
identity. The fixed-origin reader must verify GitHub metadata identity, name,
private/nonfork/active status before source bytes can commit. Rename reconnection
requires an updated operator mapping and governed actual repository attachment.

Use a distinct `synveda-github-source-credential-v1` sealed document, bound to the
numeric repository ID and containing a fine-grained Contents-read token. Retain
separate outbound types, formats and behavior. Remote selection never falls back
to a local repository. Configuration's GitHub provider allowlist only narrows
current SourceRead/KnowledgeWrite authority.

Admission reads bounded sealed credential metadata and freezes its logical value
revision. Release every database transaction before opening keys or waiting on
HTTPS. Final exclusive admission rechecks scope, kind, provider, active state,
secret identity/revision, descriptor, provider root and operation lease without
opening the key again. Key opening can acquire another tenant transaction and
therefore must never happen beneath the exclusive gate.

Fetch GET-only metadata, exact branch ref, pinned commit, nonrecursive parent
trees and blobs from `https://api.github.com`. Disable redirects and proxies;
ignore response-supplied URLs. Retain the local path's time, byte, path and request
bounds. Verify returned commit/tree association and exact object IDs, reconstruct
and verify complete tree objects, and independently hash decoded blobs. Omit the
`recursive` parameter entirely. A truncated tree, object error, timeout or bounds
failure produces incomplete work and advances no cursor. No scripts, Git process,
write endpoint or arbitrary provider URL is admitted.

## Options considered

1. Replace old roots with provider-ID hashes: breaks retained aliases and fails
   permanent suppression.
2. Keep URI roots without an identity projection: rename can create a new alias
   namespace and bypass earlier withdrawal.
3. Bind the provider ID to the existing permanent root: retains exact evidence
   and makes rename/replacement an explicit custody decision.

## Consequences

The shared reader reuses the qualified durable observation/Capture path. Two
private content-free projections add a measured migration head. Controlled HTTPS
fixtures qualify implementation; a named authorized private repository and sealed
credential are needed for the separate live check. Revisit the provider leaf only
when an additional explicitly approved provider requires a different contract.

## Compliance notes

Embedded Cedar, forced tenant RLS, exclusive source admission, existing Capture
review and content-free audit remain mandatory. Provider content is inert data.
An operator pin or model assertion never grants policy authority or proves task
verification. See GitHub's [tree](https://docs.github.com/en/rest/git/trees#get-a-tree)
and [blob](https://docs.github.com/en/rest/git/blobs#get-a-blob) contracts.
