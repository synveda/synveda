# ADPT-3: Additional API transport decision

## Problem and evidence

The versioned REST `/v1` API, executable route catalogue, generated OpenAPI
and generated console client are exact peers under ADR-0088. This brief
originally proposed gRPC as the second transport. No gRPC consumer has been
named, and the repository has no protobuf contract, gRPC listener or gRPC
conformance suite.

A concrete consumer now exists for a different transport. At `8a22b0e5`,
`synveda mcp` serves the `recall` and `remember` tools for MCP protocol
`2026-07-28`, over stdio only (`crates/synveda-cli/src/mcp.rs`). ADR-0057
decision 9 kept it local because an HTTP listener holding a live credential on
a laptop is a new exposure. It assigned the hosted path to this feature, and
its reversal triggers name this feature delivering service-identity access as
the point where an HTTP MCP transport becomes the hosted path. Agents
that cannot launch a local process, such as cloud coding agents and hosted SDK
agents, cannot reach Synveda over MCP today, and an enterprise IdP cannot
govern MCP access to it.

The MCP protocol changed in ways that make a remote endpoint simpler to
operate. The external facts below come from a survey checked on 2026-11-01:

- The `2026-07-28` specification is stateless: the `initialize` handshake and
  `Mcp-Session-Id` are gone, servers implement `server/discover`, HTTP
  requests carry `Mcp-Method` and `Mcp-Name`, OpenTelemetry trace context
  travels in `_meta`, and clients must validate the issuer (RFC 9207). Dynamic
  client registration and HTTP+SSE are deprecated; Client ID Metadata
  Documents replace registration
  ([changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)).
- The Enterprise-Managed Authorization extension, which lets an enterprise IdP
  issue an identity assertion grant (ID-JAG) for an MCP server, and the OAuth
  Client Credentials extension are both stable
  ([extensions](https://modelcontextprotocol.io/docs/extensions/overview)).
  The MCP blog names Claude, Claude Code, Cowork and VS Code as clients and
  Okta as the IdP
  ([blog](https://blog.modelcontextprotocol.io/posts/enterprise-managed-auth/));
  the community client matrix lists only Archestra and mcpc
  ([matrix](https://modelcontextprotocol.io/extensions/client-matrix.md)).
  Client support must be verified per client version.
- The Skills over MCP extension adds `skills/list` and `skills/get` with
  hashed `skill://` resources
  ([extension](https://modelcontextprotocol.io/extensions/skills/overview.md)).
  Client adoption was not verified.

This is not the gap in [production readiness](../PRODUCTION_READINESS.md)
about an MCP OAuth execution proxy. That gap concerns relaying calls to
third-party tools; this feature serves Synveda's own operations.

## Scope

1. **ADR, amending ADR-0057 decision 9.** Decide:
   - where the endpoint runs: a route set on the gateway or a separate service
     that calls the public API;
   - the authorization profile: Synveda as an OAuth protected resource for
     the tenant's configured issuer, Client ID Metadata Documents for client
     identity, the Enterprise-Managed Authorization extension for qualified
     enterprise IdPs, and the Client Credentials extension for service
     identities;
   - how session-write ownership (ADR-0057 decision 6) is chosen for a remote
     client;
   - the first named clients and IdPs to qualify.
2. Serve the existing `recall` and `remember` tools over streamable HTTP with
   the same application operations, Cedar decisions, forced RLS, audit and
   idempotency as REST.
3. Qualify each named client and IdP live, and record them in the generated
   client support matrix.
4. **Later, optional.** Serve approved, bound Skill versions through the
   Skills over MCP extension, once a supported client consumes it, so one
   endpoint can replace per-client `synveda skill sync` writes.

## Non-goals

- gRPC, or any other transport, without a named consumer. A named gRPC
  consumer reopens that option with its own operation matrix.
- Proxying or executing third-party MCP tools.
- Static API keys, dynamic client registration or the deprecated HTTP+SSE
  transport.
- A remote listener on a developer machine; local clients keep stdio.
- Transport-specific authorization or a second audit vocabulary.

## Architecture seam

The endpoint is an adapter over the same application operations REST uses.
MCP DTOs convert to the same closed `synveda-types` commands and views; SQL
stays in `synveda-store`. Authentication maps verified tokens to the same
`Identity`, and every tool call is decided per action and resource at
execution time. The route catalogue owns the endpoint so the public contract
remains one declaration.

## Acceptance criteria

- The amending ADR is accepted before implementation.
- A named remote client completes the session-to-cross-session-Knowledge
  journey over the endpoint, and the same calls over REST produce equivalent
  effects, audit and errors.
- Tokens for the wrong issuer, audience or tenant, expired and revoked
  tokens, and an unregistered client identity are refused without partial
  effects.
- Each qualified enterprise IdP path works live with its named client.
- Request limits, size bounds and timeouts match the REST path.

## Required tests

- Protocol conformance for the `2026-07-28` HTTP transport, including
  `server/discover`, required headers and version refusal.
- Authorization tests for issuer validation, audience, client identity, token
  expiry, revocation and cross-tenant tokens.
- REST and MCP parity tests for every served operation.
- Live runs per named client and IdP; replay alone does not qualify a client.

## Rollout and rollback

Ship disabled by default. Enable it for one named client on a separate path,
then promote per client and IdP. Rollback disables the path; REST and stdio
MCP keep working, and accepted governed effects remain valid.

## Dependencies

The owner names the first clients and IdPs. Security and operations owners
approve exposure, TLS termination and limits. [Delegated agent authority](AUTHZ-8.md)
must agree on how an agent is identified, because the enterprise-managed path
carries an IdP-issued agent assertion. [Session & token hygiene](AUTH-6.md)
supplies revocation and [request limits](OPS-14.md) supplies admission bounds.
