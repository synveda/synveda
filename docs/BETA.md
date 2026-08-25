# Synveda beta — current product tour and honest limits

Synveda is governed Knowledge and context for AI agents. A session appends
immutable events, capture turns potentially durable material into reviewable
candidates, and accepting a candidate passes through VedaFlow before it can
become an immutable Knowledge revision. Every governed read and mutation is
PDP-decided, tenant tables are forced-RLS, and important transitions join a
hash-chained audit log.

This guide describes the current context-platform branch. It deliberately does
not mention the deleted ACME release seeder or `init --demo`: those invoked old
hierarchy, policy-assignment, global observe and global recall surfaces and
were removed by CPR-36 rather than kept as broken support claims.

## Install the one runtime

You need Docker. Released binaries currently target macOS arm64 and Linux
x86_64 and are unsigned/un-notarized.

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
synveda init --slug pulseboard --name PulseBoard --embedder tei
synveda login
```

Drop `--embedder tei` to avoid the approximately 2.3 GB BGE-M3 download. The
deterministic implementation is useful for reproducible functional checks but
has no semantic geometry; the product labels that path lexical-only.

`init` migrates the current schema, admits one tenant, provisions deployment
key material and a non-superuser/non-BYPASSRLS gateway login, configures the
bundled IdP and starts the gateway. It creates no scope, identity, grant,
Configuration or Knowledge row. Your first login creates the tenant root,
your principal scope and the `administrator` root grant under your real
subject. That is the only first-authority bootstrap.

The host binary, Compose service and Helm Deployment are the same runtime and
generated API. Personal, team and enterprise are complete governed
Configuration documents, not binaries or chart modes. Open **Advanced →
Configuration** to create a version from a template and bind it to a scope;
with no binding, the enterprise document is the conservative fail-safe.

## Walk the current product

Open <http://127.0.0.1:8120/console/> with the operator credentials printed by
`init`.

1. Create a workspace and project. Creation is idempotency-keyed, gives the
   creator an owner grant and is decided at the parent scope.
2. Connect Claude Code with `synveda plugin install`, or another MCP-capable
   client with `synveda mcp install --client <name>`. Both adapters use public
   session/context/Knowledge/Skill/Tool APIs; neither reads the store.
3. Start a session. Session events are ordered and idempotent. The Claude
   adapter first writes each frame to its atomic local spool; retry and a lost
   acknowledgement cannot duplicate it.
4. End the session or request capture. **New Learnings** shows a batch of
   candidates with source conversation evidence, proposed type/scope,
   duplicate/conflict matches and existing-Knowledge comparisons. A pending
   candidate is not active Knowledge.
5. Accept, edit-and-accept, merge, replace or dismiss. Publication invokes the
   Knowledge command layer and a typed VedaFlow change. Replace creates an
   explicit supersession and retains history.
6. Open **Knowledge** for current content, immutable revisions, provenance,
   relationships, verification and usage. Archive and forget are different:
   forget removes plaintext/embeddings/source payloads and retains only a
   content-free tombstone and audit hashes.
7. Start a clean session or invite a teammate. A context run selects only
   policy-visible current Knowledge within its budget. Another principal can
   reuse project Knowledge and cannot enumerate the first principal's private
   scope.
8. Open **Context Inspector** from the session timeline. It shows selected
   revisions, reason codes, score components, visible exclusions, token budget,
   retrieval/index versions, degradation mode and rendered-context hash. A
   denied candidate contributes no id/title/edge/count side channel.

From a checkout, the executable PulseBoard evidence for that loop is:

```sh
sh demos/cpr-22-mvp-acceptance.sh
sh demos/ops-1-smb-profile.sh
```

These scripts use the current public/runtime model and assert database, PDP,
VedaFlow, audit, capture, Knowledge and context state. The packaged
`synveda demo start --profile ...` experience is a later programme package;
until it lands, no release script pretends to be it.

## Advanced governed surfaces

- **Advanced → Reviews** is the one comprehensive VedaFlow queue across
  Knowledge, Skills, Tool servers/bindings, Configuration, policy relaxations,
  OKF publication, prompts and context packs. Exact typed artifact/version
  references, revision-aware verdicts, distinct-person rules, cancellation and
  separately authorised effect execution share one lifecycle. New Learnings
  remains the lightweight candidate decision surface.
- **Skills** stores immutable bundle versions, exact files/digests, scan and
  controlled-harness test evidence, project/principal bindings and usage marks
  that distinguish host observation from model self-report. Declared tools are
  metadata, never authority.
- **Tools** is the trusted MCP catalogue. Source/capability changes create a
  quarantined immutable version and never move an approved project binding.
  Credentials are stable secret references and normal APIs/configuration mask
  even opaque reference identifiers. The gateway does not execute imported
  stdio commands or act as a universal tool proxy.
- **Import / Export** implements pinned OKF v0.2. Validation, bounded archive
  handling and deterministic dry-run planning create capture candidates only;
  unknown extension metadata survives round-trip. There is no scheduled Git
  sync and no remote URL fetch.
- **People** uses the shared principal/Group/membership/grant model. SCIM push
  and scheduled directory pull project into those same rows; source-owned rows
  cannot be edited through ordinary direct routes.

## Verify trust evidence

```sh
synveda whoami --capabilities
synveda audit tail --limit 20
synveda audit events --artifact-family knowledge --artifact-id <uuid>
synveda audit events --context-run-id <uuid>
synveda audit knowledge <subject> --valid-at <rfc3339> --as-known-at <rfc3339>
synveda audit export --output audit-chain.json
synveda audit verify-export audit-chain.json
```

The authenticated audit API is tenant-wide and separately authorised. It
returns identifiers, hashes, counts, decisions and provenance, not message,
Knowledge, Skill, Tool or secret bodies. Export freezes a chain head before
auditing its own read and can be verified offline without a gateway/database.

## What is not claimed

- **Gateway HA:** Helm runs one gateway replica with `Recreate`. CloudNativePG
  failover is tested, but pending login state and cross-process cache
  invalidation have not passed OPS-7. A gateway upgrade has a brief outage.
- **Tenant deletion:** a populated tenant cannot yet be erased as one ordered
  operation. Export what you need and reset the deployment; TEN-5 remains open.
- **Live Entra/Okta:** fixtures are captured/transcribed from vendor contracts.
  No live tenant was available, so no live claim is made.
- **Second verified proprietary client:** Cursor configuration is documented
  but no authentic live Cursor lifecycle has been exercised. The later adapter
  conformance package must label it experimental unless a real client passes.
- **External key providers:** the shipped key provider is the local KEK. There
  is no cloud KMS, HSM or customer-managed-key implementation.
- **Release guarantees:** binaries are unsigned, Windows is not shipped,
  reinstalling is the current upgrade path, and there is no zero-downtime
  gateway upgrade or pre-1.0 old-schema translator.
- **Live-model certainty:** deterministic capture/evaluation is reproducible;
  a model-backed run needs its external credential and is reported separately.
  A replay is always labelled replay.

The context-platform redesign is a pre-1.0 hard cut. A database from before
the current schema epoch is refused with an explicit reset instruction:

```sh
synveda reset --database --force
```

That destroys every tenant, session, Knowledge item and audit event in that
database. It does not translate old rows, and it is not recoverable.
