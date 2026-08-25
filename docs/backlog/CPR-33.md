---
title: "CPR-33: Context-platform audit query and deterministic export"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-33: Context-platform audit query and deterministic export

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Extend the existing hash-chained, tenant-complete AuditRead surface over the
current context-platform nouns. Answer from recorded decisions and immutable
references, never by replaying a historical PDP decision or resolving content
under auditor authority. Produce deterministic frozen-head evidence that an
authorised caller can verify offline without database access.

## Acceptance criteria

- The cursor-keyset event query accepts exact typed artifact family/id/version,
  session and context-run filters. Unknown families, malformed dependent
  filters and invalid cursors are refused rather than returning misleadingly
  empty answers.
- Knowledge disclosure remains what the chain records a subject being served,
  not hypothetical authority. The answer accepts distinct valid-time and
  as-known transaction-time instants, resolves only content-free immutable
  revision timing/hash evidence, and labels erased or hashes-only addresses as
  unresolved rather than inventing a bitemporal match.
- Proposal, approval, rejection, publication/application, supersession,
  archive, restore, forget, Skill, Tool, Configuration and relaxation audit
  evidence carries consistent typed artifact references through the terminal
  act, not only when a change opens.
- A context-composition trail identifies exact selected Knowledge revisions,
  advertised Skill binding/version evidence, the effective Configuration
  aggregate/binding/version/digest/policy source and each active relaxation
  version actually gathered for that run. Denied artifacts and content are not
  added to the chain.
- `GET /v1/audit/export` freezes one chain head before its own audit event,
  walks that snapshot with a sequence cursor, includes every canonical hash
  input plus genesis and frozen-head hashes, and never includes a later query
  read. A public-HTTP CLI assembles deterministic complete JSON and verifies it
  offline; a mutated, incomplete, reordered or cross-tenant bundle fails.
- All routes remain tenant-wide `AuditRead` or refuse; RLS bounds every query;
  ordinary rows carry ids, hashes, counts, decisions and provenance, never
  Knowledge/Skill/Tool/configuration content or plaintext secrets. There is no
  SIEM stream, WORM sink, historical authorisation replay or duplicate audit
  store.
- OpenAPI, generated TypeScript, console and CLI use the public contract. Old
  record-noun audit branches, aliases and storage-coupled ordinary-client
  claims are deleted. Focused tests, an isolated acceptance demo, `make ci`
  and `make db-test` pass.

## Evidence

Delivered 2026-08-25 from
`cf52f34b4d408ef147310041f9367b1e445b4162` under ADR-0092. Focused evidence:
audit crate 23/23 plus tamper 7/7; gateway audit 16/16; terminal typed-reference
regressions for Knowledge, Skills, Tools, Configuration and relaxations 5/5;
CLI audit 4/4 and complete CLI 157/157; OpenAPI 6/6; console 212/212; forced-RLS
completeness PASS. The isolated `demos/cpr-33-audit-export.sh` reports seven
self-audited frozen export reads, 49 typed artifact events and one
tenant-leading payload index. Generated OpenAPI/TypeScript and 700 SQLx query
descriptions are current; `make ci` and full `make db-test`
(`synveda_test_51591`) pass.
