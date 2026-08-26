---
title: "CPR-42: Context-platform security and product-integrity audit"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-42: Context-platform security and product-integrity audit

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Adversarially audit the completed context platform across tenant isolation,
policy ordering, source/provenance integrity, trace disclosure, governed
erasure, executable artifact handling, external-format boundaries, adapter
state, public clients, configuration/governance extensions, directory and key
planes, deployment profiles and the exact generated contract. Fix each
confirmed issue in scope and make the audit repeatable.

## Acceptance criteria

- Every tenant table remains enabled and forced RLS, and adversarial tests
  cover cross-tenant ids, grants, secrets, sources and governed artifacts.
- PDP decisions precede retrieval, graph expansion and disclosure; denied
  Knowledge, source and path evidence cannot leak through ids, content,
  fingerprints, counts, reasons, schemas or errors.
- Invitation replay, session-event spoofing, capture-source forgery, stale
  preconditions, forget/tombstone content retention and audit payload
  sensitivity are exercised against the public/runtime paths.
- Skill and OKF archive paths, expansion limits and inert-script boundaries;
  Skill-declared Tool confusion; MCP descriptor/schema quarantine, secret
  references, local stdio safety and read-only tests; and adapter-spool
  tampering are pinned by adversarial evidence.
- UI capability forecasts grant no server authority, every ordinary client is
  public-API-only, route/OpenAPI/generated-client inventories remain exact,
  and configuration, relaxations, directory, key and deployment extensions
  share the same PDP/VedaFlow/RLS/audit path.
- A repository security gate searches for plaintext credentials, unsafe
  logging, unbounded externally supplied data, direct product-store client
  mutations, post-load authorisation and permissive denied-resource errors
  without treating historical documentation or explicit test attacks as
  production findings.
- Confirmed defects are fixed with regressions. Focused adversarial tests, the
  CPR-42 demo, deterministic security/product evaluations, `make ci` and
  `make db-test` pass; unavailable external live systems remain labelled.

## Evidence

Delivered 2026-08-26 from `8a4b944`; the resulting commit is recorded by
CPR-43. ADR-0078 is amended because its durable-spool decision needed the
missing-versus-refused distinction and deployment binding; no new domain ADR,
schema, public route, Cedar action or audit action was required.

The audit found and fixed four defects at the Claude adapter boundary:
automatic retries now verify every payload hash; corrupt, unreadable and
future-version spool files are held rather than treated as missing and
overwritten; a spool is pinned to one gateway origin; and adapter diagnostics
use fixed error classes with recursive secret/payload redaction instead of raw
exception messages. The hash remains honestly documented as corruption
detection rather than authentication against an attacker controlling the same
local account.

`make check-context-security` adds five mutation tests and a CI inventory of 27
cross-layer adversarial boundaries, 16 production adapter files, 58 ordinary
client files, the exact four read-only MCP discovery methods and the spool
runtime guards. Focused results: adapter 103/103, CLI 2/2, OKF 5/5, Skill
path/bounds 12/12, generated adapter/demo/backlog/ADR gates PASS, and
`demos/cpr-42-security-integrity.sh` PASS. Full `make ci` and fresh-scratch
`make db-test` (`synveda_test_49069`, removed) pass. `make eval-product` passes
18/18 with all six hard trust counts zero; `make eval-security` passes 10,000
variants and 10,876 probes with 9/9 controls and every tenant, scope,
sensitivity, unattributed-line and watermark-gap count zero. Exact findings,
evidence and residual limits are in `docs/SECURITY.md` and the implementation
record. No unavailable live client/provider was represented by replay.
