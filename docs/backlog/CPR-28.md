---
title: "CPR-28: OKF import and export product workflows"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-28: OKF import and export product workflows

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Make CPR-27's pinned OKF v0.2 boundary usable from the terminal and project
console. Local clients read or write local paths; all governed import/export
acts still use the generated public application API and one capture/Knowledge
model.

## Acceptance criteria

- CLI validate and inspect apply the exact local v0.2 adapter to a directory,
  zip, tar, tar-gzip or explicitly revisioned checked-out Git tree without
  contacting the gateway or running content.
- CLI dry-run submits inert bytes through the public project import operation
  and renders additions, updates, duplicates and conflicts without creating a
  candidate. Non-dry-run separately materialises reviewable candidates and
  prints their destination in New Learnings.
- CLI export uses the public deterministic export operation and atomically
  writes exact stable output beneath a new output directory, refusing path
  escape and accidental overwrite.
- A generated-contract project console shows source and validation evidence,
  source revision, immutable artifacts/mappings, classification counts, import
  history/progress, candidate results, export selection/status and bundle
  summary. Capability forecasts hide unavailable controls while the gateway
  remains authoritative.
- Unknown OKF types and extension metadata are visible and survive a tested
  import/accept/export round trip. There is no scheduled Git synchronisation,
  server filesystem authority, direct publication or competing bundle format.
- Focused CLI and console component acceptance, production build, runnable
  public-API demo and `make ci` pass. No database suite is required unless the
  package changes persistence, RLS, policy or database-backed behaviour.

## Status

Delivered 2026-08-25 from
`0dbf163d67dc1aba78de5f79089a47e5c989de48`; ADR-0087 already fixes the
format, threat and public-API boundary, so no new architectural decision was
introduced.

`synveda okf validate|inspect` enumerates a selected directory or bounded
archive through the exact pinned leaf adapter without starting Git, fetching a
URL or contacting a gateway. An explicit `--source-revision` labels already
checked-out bytes; `.git` administration data never enters the request.
`import` derives a stable idempotency key from the canonical inert request and
calls the public project plan operation. `--dry-run` stops there; the ordinary
form invokes the separate idempotent materialisation operation and points to
New Learnings. `export` calls the public projection, revalidates its v0.2/spec
pin, bytewise paths, file hashes and aggregate digest, then atomically renames a
private staging directory to a new output path. It refuses traversal,
duplicate paths and overwrite.

The primary **Import / Export** project page uses CPR-27's five generated
operations and generated types. It accepts an explicit folder or one archive,
shows validation/source/revision/digest, immutable history, all four planning
classifications, exact frontmatter and unknown producer types, candidate
materialisation/New Learnings links, current Knowledge selection and completed
deterministic file summaries. Project capability forecasts hide controls the
caller cannot use while the gateway performs the authoritative decision.

Focused evidence: pinned adapter **6/6**, CLI OKF **3/3** and full CLI
**150/150**, console helpers/components plus generated request shapes **10/10**
and complete console **207/207**. The production bundle builds at 68 modules,
402.53 kB JavaScript (113.56 kB gzip) and 18.84 kB CSS (4.29 kB gzip).
`demos/cpr-28-okf-workflows.sh` passes against an isolated epoch-2 database:
it validates/inspects the real PulseBoard fixture, runs the public import →
materialise → governed accept → deterministic export lifecycle **1/1**, and
renders the full generated-contract console acceptance. `make check-demos`
passes for 77 scripts and complete `make ci` passes. `make db-test` is N/A:
this package adds clients and pure response validation only; schema,
persistence, RLS, PDP, VedaFlow, audit vocabulary, OpenAPI (**106** operations)
and epoch (**2**, 52 migration files) are unchanged. No remote-host, scheduled
Git-sync or live third-party verification is claimed.
