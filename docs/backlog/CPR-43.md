---
title: "CPR-43: Final context-platform hard cut"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-43: Final context-platform hard cut

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Close the pre-1.0 redesign with one clean epoch-3 baseline, remove replaced
runtime, API, CLI and frontend vocabulary, and make every remaining occurrence
of the programme's compatibility-search terms either an intentional
new-contract refusal, a verified external-protocol requirement, explicit
resilience/degradation behaviour or historical documentation. There is no
old-to-new translator and no compatibility implementation.

## Acceptance criteria

- `crates/synveda-store/migrations/` contains one clean context-platform
  baseline and no obsolete migration history or top-level data movement;
  schema epoch 3 refuses epoch-2 and markerless databases with the destructive
  reset instruction, while a fresh database bootstraps and resets cleanly.
- The retired Record aggregate, embeddings, signatures, supersession,
  promotion/retention writers, legacy graph residue and any context-pack bridge
  to that model are absent from production schema and code. Current Knowledge,
  session, capture, context, Skill, Tool, OKF and governed-artifact paths remain
  the only product implementations.
- Forced-RLS completeness covers every tenant table. Checked SQL metadata,
  fresh fixtures, OpenAPI and generated TypeScript are regenerated from the
  baseline/current server contract.
- Old routes return 404, old payload fields and CLI nouns/flags fail, and
  OpenAPI, generated clients and the executable route/command inventories are
  exact. No hidden alias, dual read/write, compatibility view, fallback read,
  stale frontend route/storage key or duplicate API type remains.
- A repository hard-cut gate classifies every requested search term and fails
  on unexplained production residue. Historical ADR/backlog evidence and
  authentic external-protocol fixtures keep their honest labels without
  becoming live compatibility code.
- README, install, architecture, beta/demo, threat-model, support and reset
  documentation describe epoch 3 and the final platform truthfully.
- Focused hard-cut/schema/RLS/API/CLI/frontend tests, a runnable CPR-43 demo,
  clean bootstrap, `make ci`, `make db-test`, deterministic evaluations and
  personal/team/governed demo gates pass. External live-client criteria remain
  explicitly blocked unless genuinely rerun.

## Evidence

Delivered from `9bda16bc5bf471b6de2359a1396522cee42cc62d` on
2026-08-26. ADR-0069 was amended before implementation to record the final
baseline and epoch-3 refusal boundary.

- The 60-file development migration chain is replaced by the pure-DDL
  `0001_context_platform.sql`: 87 tables, two current-state views, 83
  tenant-bound tables with enabled and forced RLS, and only `vector` plus
  `btree_gin` as required extensions. Epoch 3 refuses markerless, epoch-1 and
  epoch-2 databases before sqlx compares checksums. There is no data mover.
- Record storage, bitemporal Record revisions, Record embeddings/signatures,
  promotion, retention, Tantivy, PGMQ and the old graph runtime are deleted
  from production code, schema, dependencies, deployment and SQL metadata.
  Knowledge revisions, sources, relations, capture, ContextRun and the
  governed artifact families are the only current paths. The checked sqlx
  cache contains 605 current query descriptions.
- The generated OpenAPI 3.1 document has 171 operations and 272 schemas and is
  exactly equal to the mounted application router; the generated console
  client is current. The retired global observe/inject/recall routes remain
  404s, retired payload fields and CLI shapes are rejected, and ordinary CLI,
  MCP and Claude clients use public application APIs.
- `make check-context-hard-cut` scans 375 active production files, the
  baseline, OpenAPI and sqlx metadata; its 3/3 mutation tests prove dead routes,
  DTOs, tables and sidecars fail the gate. The classification record is
  `docs/implementation/context-hard-cut-inventory.md`.

Focused acceptance passes: the CPR-43 demo; epoch 10/10; exact forced-RLS
coverage 1/1; OpenAPI 6/6; CLI hard-cut 4/4; console 216/216; Claude adapter
102/102; and the CPR-2 nine-step real-binary demo, which bootstraps epoch 3 at
`0001`, refuses a markerless populated database without writing, resets it
without carrying a row, repeats the reset idempotently, and reaches readiness.
The final `make ci` and full `make db-test` pass; the latter used disposable
`synveda_test_74163` and removed it.

The deterministic product gate passes all 18 scenarios and six zero-tolerance
trust counts. `make eval` passes six scenarios, all 50 extraction fixtures at
0.983 macro precision / 0.914 recall, all ten QA questions with every scope
axis at 1.0, and 1,276 security probes with 9/9 controls and zero tenant,
scope, sensitivity, attribution or watermark gaps. The BGE-M3 tier passes
10/10 at 0.800 bounded retrieval precision and 152.014 ms p95; the 10,000
variant security tier passes 10,876 probes and 9/9 controls with every leak
count zero. The authentic 264.5 MiB Stage-H slice was rerun: 4,927 turns, all
ten instances measured, zero empty or unattributed Knowledge blocks, 0.643
retrieval recall, 0.577 per-type score, 0.375 complete-instance rate and
580.503 ms p95. Its per-session diagnostics still report labelled evidence
that did not become rankable inside the fixed 1,800-second barrier; the gate
holds, but this is not represented as complete evidence-session coverage.

The exact deterministic Claude lifecycle test passes 1/1. Genuine Claude Code
2.1.241 live evidence from 2026-08-24 remains valid and separately labelled;
the closeout rerun was refused by the execution approval layer because it
could transmit repository/task data through an existing authenticated
proprietary client, so no rerun is claimed. No extraction-model credential or
local vLLM endpoint exists. CPR-39's real second-client criterion also remains
externally blocked: no Cursor or VS Code executable/authenticated lifecycle is
available, and the previously inspected VS Code Preview contract has no
SessionEnd. Personal/team/governed packaged walkthroughs likewise need a
reachable authenticated gateway and distinct credentials; their hermetic
public-domain acceptance and profile tests pass, but are not called live
walkthroughs.

The feature commit is recorded by the final bookkeeping commit, following the
programme's no-self-hash convention.
