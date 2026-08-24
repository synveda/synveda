# ADR-0086: MCP discovery is immutable evidence and project activation pins an approved version

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-25
- **Deciders**: sujitn

## Context

Synveda already serves MCP through `synveda mcp`. ADR-0057 deliberately makes
that process a thin public-API adapter, pins its server implementation to the
current protocol through `rmcp`, and separates host-observed writes from model-
chosen tools. That adapter does not answer a different product question: which
external MCP servers a project trusts, which exact capabilities were approved,
or whether a later schema/source/authentication change is still the thing a
project reviewed.

The official MCP specification changed materially after the earlier catalogue
planning. The stable revision as verified on 2026-08-24 is
[`2026-07-28`](https://modelcontextprotocol.io/specification/2026-07-28),
released from `modelcontextprotocol/modelcontextprotocol` commit
`5f5440bb26a62e2cf3440b92da5a667efa03b267`. It is stateless: each request
carries protocol and capability metadata; `server/discover` advertises server
identity, versions and capabilities; stdio and Streamable HTTP are the standard
transports. The former connection-scoped handshake/session era is compatibility
behaviour, and HTTP+SSE is retired. Tool descriptions and annotations are
explicitly untrusted. A trust catalogue that equates a server name with an
approval would therefore approve mutable prose and schemas by accident.

The context-platform hard cut requires stable aggregate identities, immutable
versions, VedaFlow mutations, PDP before every governed act, forced tenant RLS
and content-free audit. It also requires secret references rather than
credentials and forbids a gateway-side arbitrary-code execution seam. Local
stdio discovery consequently needs an application boundary: an authorised
local adapter may launch the command on the user's machine and report the
read-only protocol transcript; the gateway stores and governs that evidence but
never executes the imported command.

## Decision

1. **One product aggregate surrounds the external format.** A `ToolServer` is
   the stable catalogue identity. Each observed/imported shape is an immutable
   `ToolServerVersion`; one immutable `CapabilitySnapshot` retains both bounded
   raw MCP evidence and a deterministic normal form. MCP remains the boundary
   format rather than Synveda's domain vocabulary.

2. **Pin the stable `2026-07-28` revision.** Validation and fixtures bind to
   the official release commit above. Accepted transports are `stdio` and
   `streamable_http`. No `sse`, HTTP+SSE endpoint, `Mcp-Session-Id`,
   connection-scoped capability state or compatibility alias enters the new
   catalogue.

3. **A digest covers everything whose change changes trust.** Canonical JSON
   over source metadata, transport, endpoint/command metadata, authentication
   kind, secret-reference identity, requested permissions and the normalised
   tools/resources/prompts produces the version digest. Lists are sorted by
   stable external identity; unknown extension metadata is preserved. A changed
   digest mints a new quarantined version. An unchanged report returns the
   existing version and writes no duplicate snapshot or proposal.

4. **Quarantine is proposed state, not active state.** Import/discovery opens a
   typed `AssetKind::Tool` VedaFlow apply change and stages the immutable version
   as `quarantined` in that transaction. Auto-apply may approve it immediately;
   stricter matrices leave it inspectable and pending. Rejection records a
   terminal trust state without deleting evidence. Only an applied change may
   advance the aggregate's approved-version pointer.

5. **Project activation always pins.** A `ToolBinding` belongs to one project
   scope and names one exact approved version. Create, disable, re-enable,
   repin and remove are revision-preconditioned typed VedaFlow changes. There is
   no follow-current mode: approving a changed server cannot silently alter
   what an existing project advertises.

6. **Discovery and testing are read-only evidence seams.** Streamable HTTP
   metadata may be registered directly. An authorised trusted adapter reports
   actual `server/discover`, `tools/list`, `resources/list` and `prompts/list`
   results for remote or stdio servers. `ToolTestRun` admits only that closed
   method set, records exact version/harness/outcome/latency and never records
   tool results. The gateway never launches imported stdio commands and this
   feature exposes no `tools/call` proxy.

7. **Credentials are references.** Authentication metadata is a closed kind
   plus an optional bounded secret-reference identifier. Manifest/config import
   rejects embedded credential values. Generated client configuration contains
   source metadata and reference placeholders only; normal responses, audit,
   logs and traces never resolve or serialize secret material.

8. **Descriptions and requested permissions grant nothing.** Raw and
   normalised capability metadata is display/diff evidence. Neither tool names,
   annotations, descriptions, schemas nor a server's requested permissions are
   Cedar entities or roles. Project authority comes only from an approved,
   enabled exact-version binding and the caller's ordinary PDP decision.

9. **The generic Synveda MCP adapter stays separate.** Protocol parsing and
   authentic stateless fixtures may be reused, but `synveda mcp` remains a
   public-API client serving Synveda capabilities. It is not a registry entry,
   approval path, secret resolver or execution proxy.

## Options considered

1. **Immutable evidence plus exact approved bindings (chosen).** Makes source
   and schema drift reviewable and prevents a mutable server name from becoming
   execution authority.
2. **Bind a server and follow its newest discovery.** Smaller, but a changed
   description/schema/auth source would enter a project without review. Rejected.
3. **Store only normalised capabilities.** Easy to query but destroys the
   external evidence needed to debug a normaliser or verify forward-compatible
   metadata. Rejected.
4. **Have the gateway launch stdio servers.** Would make discovery convenient
   by turning imported commands into arbitrary gateway code. Rejected; the
   trusted local adapter boundary is explicit.
5. **Proxy tool execution.** Would combine catalogue trust with runtime
   authorisation and credential use before either has an execution design.
   Rejected as outside MVP scope.
6. **Support the retired HTTP+SSE transport for ecosystem breadth.** Rejected
   by the hard cut and the current specification target.

## Consequences

- Positive: every project can prove which exact MCP bytes and schemas it
  approved; drift is quarantined rather than activated; raw evidence survives
  normalisation; configuration is deterministic and secret-free; local command
  execution remains on the user's trusted adapter.
- Negative / accepted trade-offs: a local adapter must perform stdio discovery;
  bindings require an explicit repin after every approved version; the backend
  can test discovery/list connectivity but is deliberately not an execution
  proxy; servers that only implement retired protocol/transport shapes are
  refused rather than translated.
- Reversal triggers: a separately isolated execution service is designed and
  authorised → add execution as a different runtime plane referencing these
  exact versions; the official stable MCP revision advances → add a new
  versioned normaliser while retaining this pin and fixtures; supported client
  config shapes cannot express secret references → extend the adapter format,
  never embed the value.

## Threat model and abuse cases

- A catalogue entry is attacker-controlled evidence until its exact digest is
  approved. Names, descriptions, annotations, requested permissions and JSON
  schemas are rendered as data and never projected into Cedar authority.
- An imported stdio command is an arbitrary-code boundary. The gateway stores
  the literal metadata but never launches it; only a separately trusted local
  adapter may execute discovery on the user's machine and report the bounded
  transcript.
- A changed source, transport, authentication shape or capability can be a
  supply-chain substitution even when the server name is unchanged. Digest
  drift therefore creates a quarantined immutable version and cannot move an
  existing exact binding.
- Configuration files are a common credential-ingress path. Imports reject
  environment/header/token material, persistence accepts only an opaque secret
  reference, and generated configuration never resolves that reference.
- Discovery evidence can be oversized, recursive or crafted to exploit a
  normaliser. Closed collection and metadata bounds, deterministic sorting and
  preservation of the bounded raw report make validation replayable without
  trusting display text.
- Read-only connection tests can be disguised execution. The accepted method
  set is closed to `server/discover` and the three list calls; `tools/call`,
  results and embedded credential fields are rejected before persistence.
- Tenant or project identifiers can be used as existence probes. Ownership is
  checked before the PDP, each returned row is independently authorised, and
  forced RLS makes foreign and fictional identifiers indistinguishable.

## Compliance notes

- **PDP/VedaFlow:** reads use `ToolRead`; version approval and binding changes
  require `ToolWrite` plus `ProposalOpen`, repeated when an apply effect runs.
- **RLS:** every catalogue/version/snapshot/binding/change/test table is tenant-
  bound, forced-RLS and linked with composite tenant foreign keys.
- **Audit:** change, approval, binding and test events carry ids, digests,
  counts, method names, outcomes and decision context only — never capability
  prose beyond hashes, commands with environment values or credentials.
- **Secrets:** only opaque secret-reference identifiers persist. Import scans
  and rejects fields that appear to contain credential values; generated
  configuration never resolves them.
