# FLOW-9: Git bridge — GitLab and Bitbucket providers

## Problem and evidence

At `c00b447de5f3f0fdbde2ea19537c41eb4582e9c6`, both Git directions reach only an
operator-owned local bare repository or a pinned private GitHub.com repository:
[Git export](FLOW-8.md) under [ADR-0138](../adr/adr-0138-governed-git-export.md)
and [ADR-0139](../adr/adr-0139-private-github-git-export.md), and
[inbound Git source reconciliation](MEM-9.md) under
[ADR-0148](../adr/adr-0148-inbound-git-source-reconciliation.md) and
[ADR-0149](../adr/adr-0149-private-github-source-identity-and-custody.md).
Teams whose repositories live on GitLab or Bitbucket can use neither transport.

Repository identity already canonicalises `gitlab`, `bitbucket`, `azure_devops`
and `generic_git` remotes, but that label grants no authority and selects no
transport. GitHub is fixed in these places:

- `synveda-git` export constructs `api.github.com`/`github.com` URLs, runs a
  GitHub repository metadata preflight and authenticates Git as
  `x-access-token`. Inbound reads use only GitHub's REST Git-data endpoints
  (`/git/ref`, `/git/commits`, `/git/trees`, `/git/blobs`). Destination and
  source-reader selection dispatch only `Local | GitHub`.
- Governed Configuration's closed `ExternalProvider` vocabulary admits `github`
  as its only Git provider.
- Allowlist and sealed credential documents use GitHub-specific formats
  (`synveda-github-export-targets-v1`, `synveda-github-export-credential-v1` and
  their source equivalents) with a numeric `repository_id`.
- Additive migration `0008` keys the source identity projection on
  `github_repository_id bigint`, and `synveda_bind_github_source_identity`
  admits only `provider='github'` with a `https://github.com/` URI.

The receive-pack wire exchange and in-process pack builder are forge-neutral, and
the remote export push already needs no native Git. Inbound reads have no
reusable remote path.

## Scope

Deliver in this order, qualifying each step separately:

1. A provider ADR that settles the identity, authentication, preflight, egress
   and inbound-read decisions listed under Dependencies.
2. Export: push the unchanged deterministic projection to a pinned private
   repository on each selected host, with GitHub's old-head compare-and-swap,
   explicit unpack/ref report and exact-head rediscovery.
3. Inbound: read configured Markdown paths from a pinned private repository on
   each selected host with ADR-0148's bounds (32 paths, depth 16, 128 KiB per
   blob, 2 MiB total, 128 requests, 30 seconds), complete-object verification
   and existing Capture review.
4. Per-provider allowlists, sealed credential documents, `ExternalProvider`
   values, operator documentation and live canaries.

Selected hosts, cloud first: GitLab.com and Bitbucket Cloud, then self-managed
GitLab, Bitbucket Data Center and GitHub Enterprise Server.

## Non-goals

- Azure DevOps, generic Git hosts, caller-supplied URLs, SSH, and GitHub App or
  OAuth token minting.
- Webhooks, scheduled refresh or background sync. Export and refresh stay
  explicitly requested.
- Console export UI, published Compose/Helm wiring and the open live inbound
  GitHub check. These stay with Git export and inbound source reconciliation.
- Any change to the v1 projection, the offline verifier, or existing GitHub
  receipts, custody digests and document formats.
- Merge-request or pull-request review as a VedaFlow approval; Git as authority.

## Architecture seam

Extend the replaceable `synveda-git` leaf with one variant per selected provider.
Core crates gain no provider types. Work map:

- `destinations.rs` and `source_readers.rs`: new `Kind` variants, closed provider
  labels and new custody digest domains. Keep the existing `github` and
  `synveda.git.source.github.v1` digest inputs byte-stable so no existing
  receipt appears retargeted.
- `github.rs` and `wire.rs`: reuse the wire exchange and `Projection::pack`.
  Provider modules own their URLs, metadata preflight and authentication; move
  shared bounded-HTTP and error helpers out of `github.rs` only when a second
  caller needs them.
- A peer of `source_github.rs` per provider, or the shared reader the ADR selects.
- `ExternalProvider` in `synveda-types`, then OpenAPI and the generated console
  client, regenerated together as one source of truth.
- Gateway `git_bridge.rs` and `source_reconciliation.rs`, which compare stored
  secret providers against the `github` string and parse GitHub credential types.
- `runtime_config.rs`: per-provider allowlist settings with the same `_FILE`
  resolution; a target or repository mapped under two providers refuses startup.
  Cloud mappings keep fixed constructed origins. A self-hosted mapping carries
  one exact operator-configured HTTPS origin and, if needed, its private CA
  trust; requests never leave that origin.
- A forward migration that generalises the source identity projection and binding
  function beyond `github_repository_id`, retaining every `0008` row, permanent
  root and suppression key; regenerated SQLx metadata.
- `RepositoryProvider` stays unchanged; identity already covers these hosts.

## Acceptance criteria

- An accepted ADR records the owner's hosting choice and, per provider, the
  immutable repository identity type, least-privilege token kinds and scopes,
  Git HTTP authentication form, private/active/non-fork preflight fields and
  inbound read mechanism, citing current provider documentation.
- Export to each provider yields the same Git head and mapping digest as a local
  export of the same channel. Repeating or resuming adds no commits; foreign,
  force-pushed or deleted refs refuse; branch protection is respected.
- Inbound refresh from each provider yields the same observation and candidates
  as a local fixture of the same commit. Truncated, over-bound, missing-object or
  timed-out reads advance no cursor and never imply deletion.
- A renamed repository re-pinned to the same immutable identity keeps its
  permanent root; a different identity at an existing URI refuses.
- Credentials bind to purpose and provider: a GitHub, outbound or other-provider
  secret never satisfies a mapping. Rotation preserves identity; revocation
  refuses with the existing non-oracular error.
- A self-hosted mapping reaches only its exact configured origin with its
  configured trust. Redirects, environment proxies and response-supplied URLs
  stay refused, and a certificate outside that trust refuses.
- Existing local and GitHub receipts, digests and audit chains verify unchanged.
- Responses, audit and metrics stay content-free and name the closed provider.

## Required tests

- Controlled TLS and native-Git receive-pack fixtures per provider, following
  `github_tests.rs`, covering advertisement and report framing (GitHub needed an
  extra HTTP flush), stalls, malformed reports and old-head races.
- Deterministic HTTP fixtures per provider for every preflight refusal (public,
  archived, fork, identity or name mismatch) and every inbound bound.
- Self-hosted fixtures served under a private CA: accepted with the configured
  trust, refused with public-PKI-only trust, and refused on redirect or origin
  mismatch.
- Gateway database tests for Configuration provider narrowing, Cedar/RLS
  isolation, cross-provider credential refusal, cursor crash/retry and digest
  stability of pre-existing GitHub receipts.
- A migration test proving `0008` rows, roots and suppression survive.
- Generated OpenAPI, console client and SQLx contracts through the documented
  commands, plus the dependency, licence and backlog gates.

## Rollout and rollback

Each provider ships disabled: no allowlist mapping and no `ExternalProvider`
value in effective Configuration. Export qualifies before inbound, and cloud
hosts before self-hosted ones. Integration
docs, readiness and support claims name a provider only after a credentialed
canary against an owner-authorized synthetic private repository records
evidence, as the GitHub export canary did. Rollback removes the mapping or the
Configuration value. Retained receipts, observations and already disclosed
history are preserved, never deleted or rewritten.

## Dependencies

- On 2026-10-31 the owner selected cloud and self-hosted hosting: GitLab.com,
  Bitbucket Cloud, self-managed GitLab, Bitbucket Data Center and GitHub
  Enterprise Server. Self-hosted hosts reopen ADR-0139's deferred egress
  decision. The ADR must settle per-mapping origins, private CA trust, whether
  any proxy is admitted, and the network-policy guidance operators apply.
- Bitbucket Cloud and Data Center, and GitHub.com and Enterprise Server, may
  differ in API shape, versions and token kinds. Treat each as its own contract
  until the ADR shows from provider documentation that they share one.
- Repository identity refuses a non-default port. An instance served on one
  needs an additive URI grammar change, or stays out of scope until then.
- The provider ADR must establish from current provider documentation, not
  assume: whether each immutable repository ID is numeric (current types and
  `0008` assume a positive 64-bit integer); the Git HTTP username and token form;
  least-privilege token kinds for metadata, write and read; and whether the REST
  API exposes the exact tree and blob object IDs that ADR-0149's complete-object
  verification needs.
- If a provider's REST API cannot support that verification, the ADR chooses
  between per-provider REST readers and one forge-neutral smart-HTTP
  `upload-pack` reader. The latter needs pack and delta parsing, for example
  gitoxide, which ADR-0138 set aside for the local increment; it must pass the
  licence and `check-deps` gates.
- Owner-authorized synthetic private repositories and sealed credentials per
  provider for the live canaries.
- Builds on Git export and inbound source reconciliation. Their open live and
  deployment qualification does not block source work; published deployment of
  new providers follows theirs.
