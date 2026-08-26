# Context-platform hard-cut inventory

Date: 2026-08-26
Feature: CPR-43
Baseline: schema epoch 3, `0001_context_platform.sql`

This is the classification record required by Prompt 33. It distinguishes
active compatibility implementation from words that remain for a current
semantic reason, an external specification, resilience, or historical proof.
The executable rule is `make check-context-hard-cut`; this document explains
the residue that a blind text search cannot classify correctly.

## Deleted and gated

The following are absent from active Rust/TypeScript code, deployment and
operator scripts, Cargo metadata, generated SQL metadata and OpenAPI:

- the tenant-global runtime routes for observation, injection and recall;
- `RecordKind`, the Record aggregate, its revisions/embeddings/search modules,
  and the evaluation tombstone DTOs that enumerated it;
- the fixed hierarchy, `Division`, `Department`, `RoleBinding`, hierarchy and
  role-binding tables, commands and routes;
- the old observation buffer/quarantine and policy-lapse tables;
- the Tantivy sidecar, its filesystem/configuration/volume paths, and PGMQ as a
  runtime or image dependency;
- serde or Clap compatibility aliases;
- old injection-specific timeout, token and tier telemetry names;
- the incremental migration chain and its pre-epoch schema.

The hard-cut checker scans 375 active files and `.sqlx`, rejects each of these
tokens, asserts the retired source paths do not exist, asserts the old routes
are absent from OpenAPI, and requires exactly one migration. Its fixture test
deliberately reintroduces a dead route, DTO, table and sidecar and proves the
gate fails.

## New-contract semantic defaults

These matches are current semantics, not compatibility:

- `PackOrigin::Fallback` means the explicitly configured default Cedar pack
  when no narrower assignment exists. It never reads an earlier schema or
  artifact representation.
- `idempotency_records` stores current request outcomes; “record” is the
  ordinary noun for an idempotency ledger entry, not the retired Knowledge
  aggregate.
- the console SPA fallback serves `index.html` only below `/console`; `/v1`
  and authentication routes retain their own 404s.
- `old_text`, `old_revision`, `old_start` and “backwards” in diff, rollback,
  cursor and ordering code compare two current immutable revisions or describe
  ordering. They do not invoke an earlier contract.
- scope aliases in evaluation environments are human-readable names resolved
  to current `ScopeId` values. No alias is accepted on an API or CLI payload.

## Verified external adapter requirements

These words are retained because Synveda is implementing or describing an
external contract rather than accepting its own retired contract:

- the generic MCP server pins protocol revisions `2026-07-28`, `2025-11-25`
  and `2025-06-18`. `initialize` support for the latter revisions is an MCP
  wire requirement; the retired HTTP+SSE transport is not implemented.
- Agent Skills “compatibility” metadata and controlled-client results are
  specification/host evidence. Declared compatibility never grants authority.
- OKF is deliberately pinned to v0.2. The adapter rejects v0.1, preserves
  extension metadata required by v0.2, and retains the official OKF status
  value `deprecated` as data rather than as a Synveda deprecation mechanism.
- Azure DevOps `*.visualstudio.com` and captured client “legacy JSON config”
  labels describe vendor inputs and honest support levels; they are not
  Synveda route, schema or DTO aliases.
- semantic-version examples such as `v0.2.0` are release identifiers.

## Resilience and degradation

Remaining uses of “fallback” are bounded current-runtime behaviour: an
unavailable embedding leg falls back to lexical retrieval and records a
degradation mode; XDG/install paths have OS-safe location fallbacks; unknown
display labels render an honest generic label; and the console has the scoped
SPA behaviour described above. None performs an old-data read, a dual read or
write, or selects a previous schema/API implementation.

## Historical documentation and negative proof

Git history and accepted ADRs retain superseded nouns so the decision and
deletion trail remains auditable. Current stack decisions are in
`SYNVEDA_TECH_PLAN.md`, ADR-0069 and this inventory; open implementation work
is linked from `docs/backlog/STATUS.md`.

Negative tests deliberately name retired material to prove:

- old routes return 404 and old payload/scope kinds fail validation;
- old schema epochs are refused with reset guidance and no data migrator;
- retired tables, extensions, commands and demo paths do not return;
- unknown or pre-cut spool formats are rejected rather than translated;
- deployment, demo and hard-cut drift gates catch deliberate resurrection.

Those tests are evidence of deletion, not executable compatibility readers.
The checkers themselves necessarily contain the forbidden tokens they detect;
their self-tests prove comments/fixtures cannot turn that inventory into a
supported product surface.
