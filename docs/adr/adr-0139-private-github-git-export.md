# ADR-0139: push the governed Git projection to a pinned private GitHub repository

- **Status**: Accepted
- **Date**: 2026-10-05
- **Feature(s)**: FLOW-8
- **Deciders**: implementation under the owner's request to continue FLOW-8

## Context

ADR-0138 supplies a deterministic export, fresh disclosure authorization and
prepared/completed VedaFlow receipts. Its local acceptance passes. The next
increment needs one remote provider, credential custody and an exact destination
identity without putting credentials into native Git processes or making Git
the authority. This repository already uses GitHub as its forge.

## Decision

1. Select **GitHub.com over verified HTTPS**. An optional bounded deployment
   allowlist maps an exact tenant/scope/target to owner, repository, immutable
   GitHub repository ID and a stable `synveda-secret://` reference. A repository
   ID belongs to only one mapping; a configured remote target never falls back
   to local export. The public request still supplies only a destination ID.
   Effective governed Configuration must independently allow that target and
   the closed `github` provider family. Both remain disabled by default.
2. Resolve an active, same-tenant, exact-scope `import_export` secret with provider
   `github` inside an ordinary forced-RLS transaction after Cedar has authorized
   the channel and every historical asset. Its sealed v1 credential document
   binds a repository-restricted fine-grained PAT to the immutable repository ID.
   Use the existing tenant DEK,
   AAD and stable secret UUID. Reopen current custody for each effect and replay;
   rotation preserves identity, while revocation or metadata mismatch refuses.
   No deployment credential fallback exists.
3. Keep credentials in memory and use the existing permissively licensed
   `reqwest` HTTPS stack. The Git leaf sends the deterministic pack through the
   bounded Git smart-HTTP receive-pack protocol, negotiating `report-status` and
   exactly one branch update. It executes no network-capable Git subprocess.
   Allow only constructed `api.github.com` and `github.com` URLs, public-PKI TLS,
   no redirects or environment proxies, bounded response bodies and request
   deadlines. Never echo provider bodies or credential parse details.
4. Before every remote effect/no-op, GitHub repository metadata must match the
   configured immutable ID and canonical name, and be private, active, unarchived
   and not a fork. Ref discovery must report either the recorded previous head
   or the exact prepared result. A push includes that exact old head; the server
   atomically checks it. Accept only an explicit successful unpack/ref report,
   then rediscover the exact resulting head. Missing, foreign or force-pushed
   refs refuse. Branch protections remain authoritative; no force, deletion,
   repository creation or destructive reset is implemented.
5. Bind each prepared/completed receipt to a content-free destination digest,
   including transport, immutable remote identity and stable credential ID.
   Retargeting a receipt refuses, including changing local-root custody. A new
   governed target starts separate retained history. Source projection and
   offline verification stay at ADR-0138's v1 format. Lost responses after a
   remote write resume the frozen prepared projection without extra commits.
6. Content-free audit adds provider, destination digest and the stable secret
   ID/current value revision when used. Existing closed outcomes and duration
   metrics cover both transports. Operators retain remote history and matching
   database/key receipts; disabling Configuration or the deployment allowlist
   stops future effects and never deletes disclosed data.

## Options considered

1. **GitHub smart HTTP with in-memory credentials (chosen):** interoperates with
   real Git receive-pack, uses the existing TLS/dependency contract and permits
   strict URL, body, response and compare-and-swap validation.
2. **Native Git with an askpass/helper or transient credential file:** adds a
   secret-bearing subprocess or persistent credential surface. Rejected.
3. **GitHub Git-data API to reconstruct every object:** does not accept exact
   Git commit bytes/authorship and pack topology as one ordinary Git transfer.
   Rejected for the deterministic ADR-0138 projection.
4. **Arbitrary forge URLs/SSH/private CAs:** expands egress, custody and protocol
   decisions before evidence exists. Defer to a separately accepted increment.

## Consequences

- Positive: remote disclosure uses the same public API, Cedar, forced RLS,
  deterministic history, durable intent and recovery boundary as local export.
- Negative / accepted: provider metadata and Git push cannot atomically lock
  repository visibility. Repository owners must restrict visibility changes,
  access and retention independently; checks establish observed private state,
  not an external confidentiality guarantee. Network errors may represent a
  completed push, so receipts remain prepared until verified recovery.
- Reversal trigger: measured provider incompatibility or an owned requirement
  for enterprise hosts/App token minting/proxy custody earns a provider ADR.
  Controlled TLS/native-Git acceptance is separate from a credentialed GitHub
  canary and published deployment/recovery qualification.

## Compliance notes

Every runtime disclosure, replay and resumed effect repeats current channel and
exact-artifact Cedar decisions and Configuration narrowing. Secret lookup and
cursor state remain forced-RLS tenant operations; static SQL stays in the store.
The existing documented operator secret commands own provision/rotate/revoke.
Neither ordinary APIs nor Git objects, manifests, subprocess arguments,
environment, logs or audit contain credentials. No baseline migration, policy
relaxation, Git import or provider approval path is introduced.
