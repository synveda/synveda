# ADR-0088: one executable application-route inventory and public clients only

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-29
- **Deciders**: autonomous context-platform continuation

## Context

The context-platform cut already derives 106 OpenAPI operations from gateway
handlers and generates the console client from the committed document. That
contract is internally consistent but deliberately partial: the gateway still
mounts older governance, policy, audit, channel, prompt, context-pack, lapse,
service-identity, directory and SCIM-credential routes outside it.

The partial boundary has three observable costs. First, a documented route is
proved mounted, but a mounted undocumented sibling cannot be discovered from
the router, so “authoritative” means only one direction. Second,
`console/src/api.mts` is both the shared transport and a handwritten list of
the omitted routes, with UI DTOs separately copied into components. Third,
ordinary service-identity and audit CLI commands still implement store-level
behaviour despite public PDP/audit routes existing. The Rust MCP server uses
public session operations, but privately mirrors response DTOs and does not
read the governed Skill and Tool bindings a session may advertise.

The unauthenticated OAuth endpoints, health/metrics endpoints and the
standards-defined `/scim/v2` protocol have different authentication and
contract authorities. Folding them into the bearer-authenticated application
document would blur rather than close the boundary.

## Decision

1. **The generated OpenAPI document covers every authenticated production
   `/v1` method.** It does not cover `/auth`, health/readiness/metrics, static
   console assets or `/scim/v2`. The governed `/v1/scim/credentials` routes do
   belong to it.

2. **One declarative route table builds the Axum `/v1` router and exposes its
   method/path inventory.** A route entry states each HTTP method and handler
   once; the macro constructs the `MethodRouter` and the inventory from those
   same tokens. Wildcard router syntax has an explicit canonical OpenAPI path.
   The contract test compares this executable inventory with the generated
   document in both directions and then probes every operation through tenant
   middleware. A manually maintained expected-path list and hard-coded
   operation count are deleted.

3. **Rust handler annotations and DTOs are the only edited contract source.**
   `docs/api/openapi.json`, generated operation metadata and generated console
   types remain derived artefacts. Every 4xx response uses the shared
   `ApiErrorBody`; creation idempotency and mutable-revision requirements are
   represented on the operation that enforces them, not inferred by a client.

4. **`console/src/api.mts` becomes transport and outcome classification only.**
   Every governed request is described by a generated operation and invoked by
   `client.mts`; component-local wire DTOs that duplicate generated schemas are
   removed. Authentication remains the opaque same-origin cookie, and the
   gateway remains the sole decision point.

5. **Ordinary CLI product actions are gateway clients.** Service-identity
   register/list/remove and audit event/verification reads move to public
   routes under the stored bearer. Database migrate/reset, tenant admission,
   key rotation/export, secret custody, dev-token minting and explicitly
   labelled break-glass policy-pack bootstrap remain local operator actions:
   they either create the authority needed to call the gateway or handle
   secrets/raw database recovery that an ordinary application route must not.

6. **Adapters share the public application vocabulary and gain no authority.**
   The generic MCP server opens a session, queries current Knowledge, appends
   model assertions and reads the session's permitted Skills and exact approved
   Tool bindings through public APIs. Those reads are advertisement metadata,
   never execution permission; no imported command is launched and no core
   store/policy/audit service is called. The Claude Code adapter continues to
   use public session/event/context/end operations only.

7. **This package changes no persisted domain or authorisation semantics.**
   It may expose already-supported operations in the contract and remove
   bypassing clients, but a schema, Cedar action or audit action would require
   a separately recorded decision rather than being smuggled into convergence.

## Consequences

- Route and contract drift fails at the declaration that mounts the route,
  before a console click or generated client encounters a 404.
- The OpenAPI diff grows substantially once, and future additions pay the
  ordinary handler-annotation cost rather than creating a handwritten client
  exception.
- Older handler response structs become public-to-the-crate schema types. That
  is contract visibility, not domain ownership; store types remain behind the
  gateway.
- The CLI keeps operator dependencies because this binary also owns bootstrap
  and key custody. The boundary is command-specific and tested: MCP and
  ordinary product modules may import the HTTP client, never store services.
- `/auth` and `/scim/v2` retain their own appropriate contracts and are not
  falsely described as bearer-authenticated application operations.

## Compliance notes

- **PDP and VedaFlow:** no new effect path. Re-cut clients reach the same
  handlers and exact per-object decisions as the console; generated metadata
  never authorises an act.
- **RLS and tenancy:** unchanged. Tenant identity still comes from the verified
  bearer and every database-backed handler opens a tenant transaction.
- **Audit:** unchanged vocabulary. Moving a CLI act to the gateway strengthens
  attribution from break-glass database custody to the authenticated subject.
- **Secrets:** generated SCIM/service/tool schemas expose secret references or
  one-time issuance responses only; ordinary listings, logs and audit metadata
  retain no plaintext credential.
