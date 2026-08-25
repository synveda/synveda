---
title: "CPR-29: Public contract and client convergence"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-29: Public contract and client convergence

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Make the generated OpenAPI 3.1 document the complete authenticated application
contract rather than a newer-plane subset. Drive the executable `/v1` router
inventory and its parity test from one declaration, move every console product
call to generated operations and types, and remove the remaining ordinary CLI
and adapter seams that act as their own application service layer.

The unauthenticated login flow, operational health/metrics routes and the
standards-defined `/scim/v2` provisioning protocol remain separate surfaces;
the governed `/v1/scim/credentials` administration routes are part of this
contract.

## Acceptance criteria

- The executable router owns one method/path inventory and a no-database test
  proves exact equality with OpenAPI in both directions. Every documented
  operation is mounted and authenticated, and no mounted `/v1` method is
  absent from the contract.
- Every operation has a unique operation id, common API error envelope,
  generated schema, bearer security and accurate idempotency/revision
  precondition metadata. Existing bounded collections retain their cursor
  contracts rather than acquiring offset or unbounded variants.
- Generated console operations and response/request types cover the full
  product plane: proposals/reviews, capabilities, policies/lapses, audit,
  channels, prompts/packs, quarantine, service identities, directory and SCIM
  credentials as well as the already-generated foundation, Knowledge,
  capture, context, Skills, Tools and OKF planes. Handwritten governed calls
  and their duplicate DTOs are deleted.
- Ordinary `synveda service` and `synveda audit` commands use authenticated
  public application routes. Direct-store access remains only for explicitly
  documented local database/bootstrap, key/secret custody and break-glass
  policy-pack operations that no ordinary gateway client can safely perform.
- The generic MCP server remains a thin public client: session creation,
  current Knowledge query and event append use the generated application
  contract vocabulary, while available Skill and approved project Tool
  binding metadata are read through their public operations and grant no tool
  execution authority. The Claude adapter remains session-API-only.
- Focused contract, gateway, console, CLI and adapter tests, generated-artifact
  checks, a runnable public-client acceptance demo and `make ci` pass.
  `make db-test` runs if database-backed behaviour changes.

## Status

Delivered on 2026-08-25 from
`683a17d30a812d160781cccf16c8633e9251f425`; ADR-0088 Accepted. One route
catalogue now constructs all **156** authenticated `/v1` operations and the
OpenAPI suite proves exact method/path equality in both directions. The
generated document has **238** schemas, and the generated TypeScript client is
current.

The console transport contains no hand-written application operation or wire
DTO. `synveda service` and `synveda audit` now use the public gateway API, and
the generic MCP server uses public session, scoped Knowledge, available Skill
and approved project Tool configuration operations without importing execution
authority or secret material. A source guard proves the Claude adapter uses
only documented public session operations and has no store or retired global
runtime authority. Direct store access remains only for documented local
bootstrap/reset/migration, key/secret custody and break-glass policy-pack
operations.

No schema, migration, Cedar action or audit action changed. Focused evidence:
OpenAPI **6/6**, service identities **5/5**, audit query **13/13**, CLI service
**1/1**, CLI audit **2/2**, generic MCP **44/44**, complete CLI **156/156** plus
MCP corpus **5/5**, console **208/208**, Claude adapter **98/98**, generated API
and **78-script** demo drift gates. Complete `make ci`, full `make db-test` and
isolated `demos/cpr-29-public-contract.sh` pass.
