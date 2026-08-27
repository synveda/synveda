# ADPT-3: Additional API transport decision

## Problem and evidence

The versioned REST `/v1` API, executable route catalogue, generated OpenAPI, and generated console client are already exact peers under ADR-0088. The repository has no Synveda protobuf contract, gRPC listener, or gRPC conformance suite. The remaining title therefore combines a delivered REST/OpenAPI boundary with an unproven second transport, and no accepted decision currently establishes that gRPC's operational and compatibility cost is required.

## Scope

- Write and accept an ADR that identifies a concrete gRPC consumer, the supported operation matrix, listener/TLS shape, protobuf evolution rules, deadlines, size limits, and REST-to-gRPC error mapping.
- If justified, add versioned protobuf services for the adapter runtime plane: session lifecycle and events, context runs, Knowledge query, and capture initiation/status.
- Reuse the same application operations, OIDC bearer/service identities, Cedar decisions, ordinary tenant transactions, forced RLS, VedaFlow effects, audit actions, and idempotency semantics as REST.
- Generate pinned client stubs and a descriptor set; publish an explicit matrix for REST-only administration operations instead of implying full parity.
- Add bounded transport metrics and tracing without identity, tenant, resource, or content labels.

## Non-goals

- Replacing REST/OpenAPI, duplicating domain logic in transport handlers, or exposing store access.
- Adding static API keys, transport-specific authorization, policy bypasses, or a second audit vocabulary.
- Claiming gRPC support for operations absent from the checked service matrix.
- Implementing gRPC-Web, streaming, reflection, or public internet exposure without a named consumer and separate threat review.

## Architecture seam

REST and gRPC adapters terminate at one gateway application boundary; protobuf DTOs convert to the same closed `synveda-types` commands and views used by REST. SQL remains in `synveda-store`. Authentication maps verified OIDC claims to the same `Identity`, and authorization remains per action/resource at execution time. OpenAPI continues to describe REST only; protobuf descriptors are a separately generated, checked contract.

## Acceptance criteria

- The accepted ADR and checked operation matrix name the consumer and every supported/unsupported capability.
- For every mapped operation, REST and gRPC produce equivalent authorized effects, idempotent replay, audit evidence, ordering, pagination, and non-secret error class.
- Deny, revoke, cross-tenant, oversized, expired-token, deadline, cancellation, and retry cases fail closed without partial governed effects.
- Generated descriptors and clients are reproducible and fail CI on source drift or an incompatible protobuf change.
- A real client completes the supported session-to-cross-session-Knowledge lifecycle over gRPC before the support matrix claims it.

## Required tests

- Golden REST/gRPC parity tests at the application boundary for each mapped operation.
- OIDC user/service identity, Cedar allow/deny/revoke, forced-RLS, audit-chain, and cross-tenant matrices.
- Duplicate request, cancellation, deadline, flow-control, message-size, malformed-frame, and connection-reuse tests.
- Descriptor compatibility and generated-client drift checks.
- Public demo using the named consumer and only published endpoints.

## Rollout and rollback

Keep gRPC disabled until the ADR, parity suite, and deployment TLS/readiness checks pass. Enable it on a separate internal listener for one named client, then canary. Rollback closes that listener and removes its service discovery entry; REST/OpenAPI remains authoritative and any accepted governed effects remain valid.

## Dependencies

The product owner must confirm a concrete gRPC consumer and required operation set. Security/operations owners must approve listener exposure, TLS termination, proxy/load-balancer support, limits, and compatibility horizon. Packaging depends on an approved protobuf toolchain and client-language support policy.
