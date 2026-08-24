---
title: "CPR-16: Governed Knowledge mutation lifecycle"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-16: Governed Knowledge mutation lifecycle

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Put create, edit, verify, supersede, merge, archive, restore and forget behind
the PDP, the existing VedaFlow proposal/approval engine and the hash-chained
audit log. A permissive policy may auto-apply a proposal; no policy bypasses
the proposal, immutable revision or audit evidence.

Extend VedaFlow with a typed, content-free Knowledge change manifest and an
`apply` effect rather than adding a Knowledge-specific review system. Add a
reusable forced-RLS durable operation ledger. Forget evaluates a retention/
legal-hold hook, removes Knowledge and pending-command plaintext plus index
state when allowed, and retains only a content-free tombstone.

Stop the old extraction, auto-promotion and retention loops: sessions must no
longer manufacture, promote or destroy records behind the Knowledge lifecycle.
The controlled read interval retains two explicitly governed old-plane seams
until their consumers are re-cut: VedaFlow record classification (needed by
the restricted-tier adversarial proof) and context-pack chunk materialisation
(needed by the still-old context composer). They are neither Knowledge writes
nor a dual write, and their deletion/re-anchor is pinned below rather than
hidden as compatibility.

## Acceptance criteria

1. Every command returns one VedaFlow change id and `applied`,
   `pending_review` or `rejected`, plus resulting item/revision ids where
   applicable.
2. Auto-apply still creates a content-addressed VedaFlow proposal, closes it
   as applied, appends immutable Knowledge state and chains content-free audit
   evidence atomically.
3. A stricter effective policy leaves the exact change open for review; an
   approved change revalidates its revision precondition before applying.
4. Edit and verify append immutable revisions. Stale preconditions apply
   nothing and produce a rejected change rather than overwriting a head.
5. Supersession creates an explicit relation, marks old current state
   superseded and preserves all revision history. Merge retains every input
   source and records its input relations.
6. Archive and restore are governed lifecycle transitions and preserve
   content/revision identity.
7. Forget creates and runs a durable operation, honours an erasure hold,
   removes authorised plaintext and index state, retains only ids/hashes in a
   tombstone and is retry-safe.
8. All new tenant tables are forced-RLS and cross-tenant invisible. Every
   important state transition is audited without body, title, summary,
   locator, source payload or secret.
9. The gateway starts no old extraction, promotion or retention writer; no
   public Knowledge mutation touches records or bypasses VedaFlow. The two
   controlled old-plane seams are enumerated and scheduled for deletion.
10. Focused PDP, VedaFlow, store, gateway, RLS and audit tests, a runnable demo,
    `make ci` and `make db-test` pass.

## Decision

[ADR-0081](../adr/adr-0081-governed-knowledge-lifecycle.md) — reuse the one
VedaFlow proposal engine with a typed Knowledge effect, bind content through a
content-free manifest and payload hash, and perform erasure through the shared
durable operation ledger.

## Controlled-cutover deletion checklist

CPR-17, the Knowledge API and browser cutover, must delete:

- the raw-record browser, hand-written record DTOs, fixtures and ordinary
  record terminology;
- the record classification route `POST /v1/proposals/{id}/classify`,
  `synveda proposal classify`, its eval client method and record-only proposal
  rendering once no supported read requires a mutable record tier;
- public or console record listing/detail/search mutations and their tests;
- any generic proposal input that can name `record_ids` once Knowledge APIs
  replace the user-facing publication flow.

CPR-18, which re-cuts session context and recall onto Knowledge revisions,
must delete or re-anchor:

- old record search/index composition and the disabled extraction/promotion/
  retention runtime implementations;
- context-pack chunks currently materialised as pinned records, preserving the
  VedaFlow context-pack aggregate while moving its retrieval projection off
  `records`;
- the remaining record-only security/evaluation fixtures, replacing their
  assertions against the Knowledge query and context-run surfaces.

## Completion evidence

Delivered 2026-08-24 in migration `0048_knowledge_lifecycle`. The public
application command service implements all eight commands and returns the
VedaFlow change id plus `applied`, `pending_review` or `rejected` and the
resulting stable addresses. Personal/open policy auto-apply and regulated
policy review both use the same proposal, approval matrix and typed effect.
The public proposal detail and the execution boundary both verify the command
payload against its immutable content-free VedaFlow manifest; the latter also
re-runs ownership, PDP, lifecycle and revision checks before committing
reviewed work.

Three database-backed gateway acceptance tests prove immutable edit/verify,
stale rejection, private-scope denial, archive/restore, explicit
supersession, source-preserving merge, strict-policy review and live policy
drift, allowed erasure, held erasure, terminal rejection of other pending
changes, content-free tombstones/audit and cross-tenant RLS for every new
table. Policy approval/pack/PDP suites pass, the dynamic RLS inventory covers
`knowledge_changes`, `durable_operations`, `knowledge_erasure_tombstones`
and `knowledge_index_invalidations`, and the leak regression retains the
restricted-tier proof during the controlled cutover.

`demos/cpr-16-knowledge-lifecycle.sh` runs this evidence in an isolated fresh
database and reports 19 governed changes with zero old records. `make ci` and
`make db-test` pass. The database gate found and corrected two checked-contract
drifts: the capability fixture now exposes `knowledge.write` and
`knowledge.forget`, and the row-effect rollback test pins the precise “no
VedaFlow channel” refusal. Neither correction weakened a product assertion.
