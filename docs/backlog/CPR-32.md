---
title: "CPR-32: Unified approvals across governed artifacts"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-32: Unified approvals across governed artifacts

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Extend the one VedaFlow proposal/change model across every context-platform
artifact family. Make the exact aggregate/version/operation under review,
separation-of-duty rules and lifecycle evidence first-class, then make
Advanced Reviews capable of completing the same common workflow. Do not add a
Knowledge, Skill, Tool, Configuration, relaxation or OKF-specific review
queue.

The session-event secret quarantine remains a distinct admission control: it
decides whether a redacted event may enter extraction, not whether an artifact
version may publish. New Learnings remains a candidate-decision surface whose
accepted actions call the common Knowledge command layer.

## Acceptance criteria

- The common VedaFlow proposal persists immutable typed artifact references
  for Knowledge, Skill, Tool server/binding, Configuration, policy relaxation
  and OKF-sourced publication. References bind stable ids, operations, exact
  proposed versions/digests and any stale-head precondition.
- The approval matrix continues to resolve live from the effective inherited
  profile and nearest curator requirements. It additionally expresses an
  author self-review prohibition and optional separation of the effect actor
  from both author and recorded reviewers. Required roles and distinct-person
  counts remain independent of PDP authority.
- Approval and rejection requests name the exact proposal commit. A stale
  verdict is rejected; rejection still requires a reason; when configured, an
  author cannot cast either kind of reviewer verdict and must cancel instead.
- Cancellation is the existing terminal proposer withdrawal semantics, now
  exposed in the comprehensive review experience rather than duplicated by a
  second lifecycle. Applying or publishing an approved proposal repeats PDP,
  approval, separation and artifact revision checks.
- An empty matrix still auto-applies only after the proposal, typed projection,
  object/commit, PDP decisions and audit evidence exist. Knowledge, Skills,
  Tools, Configuration and relaxations are covered by one acceptance map.
- The generated public proposal contract and Advanced Reviews expose typed
  references, inherited requirements, approvals and opened/reviewed/closed
  timeline evidence, artifact-family filtering, verdicts, cancellation and
  effect execution. New Learnings stays separate and lightweight.
- No duplicate artifact proposal/review screen or handler remains. The
  session-event quarantine is retained and documented as a different security
  control, not counted as an artifact workflow.
- The proposal schema remains tenant-bound and forced-RLS; audit payloads stay
  content- and secret-free. Focused domain/policy/gateway/console/CLI tests, an
  isolated acceptance demo, `make ci` and `make db-test` pass.

## Evidence

Delivered 2026-08-25 from
`92819516ee35abf3f5a0fe6cd8c0658f666269af` under accepted ADR-0091.
Migration `0057_unified_artifact_approvals` makes a validated, bounded and
immutable typed-reference array mandatory on the existing forced-RLS proposal
row and indexes family filtering; it creates no second review table and
translates no old proposal. All Knowledge (including OKF provenance), Skill,
Tool server/binding, Configuration, policy-relaxation, Prompt, Context Pack and
pre-cut authored-Memory proposal callers now bind stable ids, exact operations,
versions/digests and stale-head preconditions. Approval rules merge monotonic
self-review and distinct-effect-actor restrictions with the existing live
role/subject/person matrix. Both verdicts require the exact inspected commit;
effect execution repeats Cedar, matrix, separation and artifact-head checks.

The 164-operation generated contract exposes typed references, family
filtering and a content-free lifecycle timeline without adding an alias route.
Advanced Reviews uses that contract to filter, inspect, approve/reject, cancel
through the existing withdrawal act and execute approved effects. New
Learnings and session-event quarantine remain the distinct candidate and
admission boundaries they actually are. The isolated
`demos/cpr-32-unified-approvals.sh` proves 81 typed proposals across seven
families, 23 exact-commit review acts, regulated author/reviewer/effect-actor
separation and zero audited artifact content. Focused results: types 212/212
plus serde 50/50, policy 77/77, VedaFlow 73/73 plus object-store 10/10,
configuration 1/1, Knowledge 4/4, OKF 1/1, relaxations 3/3, Skills 1/1,
Tools 1/1, Context Packs 10/10, Prompts 6/6, OpenAPI 6/6, console 210/210,
policy-pack store 5/5 and forced RLS 83/83. Checked SQLx metadata compiles
offline; `make ci`, the 80-script demo drift gate and full `make db-test`
against fresh scratch database `synveda_test_43866` pass.
