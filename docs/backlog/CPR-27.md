---
title: "CPR-27: OKF v0.2 knowledge exchange adapter"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-27: OKF v0.2 knowledge exchange adapter

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Add a versioned Open Knowledge Format boundary around the existing Knowledge
and capture domains. Import validates and plans portable artifacts into
reviewable candidates only; export serialises selected current Knowledge
deterministically. Neither direction becomes a second domain or publication
path.

## Acceptance criteria

- The adapter is pinned to the canonical OKF v0.2 specification at an exact
  upstream revision. It accepts unknown v0.2 concept types and extension
  fields but does not implement the specification's v0.1 fallback.
- Directory, zip, tar and explicitly identified checked-out Git sources are
  bounded, path-safe inputs. Every concept is UTF-8 Markdown with YAML
  frontmatter and a non-empty `type`; reserved index/log files are inspected
  under their v0.2 rules rather than materialised as candidates.
- Immutable import artifacts retain source kind/revision, logical path,
  canonical content hash and preserved extension metadata. Dry-run mappings
  classify additions, updates, duplicates and conflicts before any candidate
  exists; unchanged reimport is idempotent.
- Materialisation creates capture candidates and proposed relations only.
  It never creates active Knowledge or bypasses VedaFlow; later candidate
  acceptance remains the existing governed publication path.
- Provenance, generation, verification, status and staleness map explicitly.
  Internal Markdown links become proposed relations and unknown metadata
  remains available for a deterministic round trip.
- Export considers only current Knowledge that passed row-level PDP, uses
  stable paths and ordering, and preserves source, verification, staleness,
  relationship and extension evidence without leaking denied artifacts.
- Traversal, symlink escape, excessive archive expansion, unsupported binary
  content, execution attempts, SSRF and private-address redirects are refused.
- Import jobs, artifacts and mappings are tenant-bound, forced-RLS and
  content-safe in audit. The generated public contract, focused tests, demo,
  `make ci` and `make db-test` pass.

## Status

Delivered 2026-08-25 from
`98f5bcdac7d3313c99cd4bd27ecd6243189a6be3` under accepted ADR-0087.

The leaf `synveda-okf` adapter pins canonical OKF v0.2 at upstream commit
`ad30107c31c06aec8a7d5636e0d1058118604e6f`. It validates bounded directory,
zip, tar, tar-gzip and explicitly identified checked-out-tree inputs without
network access or execution, rejects v0.1 fallback, traversal, links, special
files, binary content, credential-shaped remote sources and expansion bombs,
and preserves unknown v0.2 types and extension frontmatter. Immutable import
jobs, artifacts and mappings classify a dry-run before the separate,
idempotent materialisation step creates reviewable capture candidates only.
Candidate acceptance remains the existing VedaFlow Knowledge command path;
accepted OKF evidence becomes normalised artifact and declared-source
provenance. Deterministic export considers only current Knowledge independently
authorised through the PDP and preserves stable paths, sources, relations,
verification, staleness and extensions.

Migration `0054_okf_imports.sql` adds four forced-RLS tenant tables and extends
the capture source invariant without a data translator. OpenAPI grows from 101
to 106 operations and 139 to 150 schemas; the generated console client and New
Learnings source display follow it. The epoch remains 2 with 52 migration files
and 669 checked SQLx query descriptions. Focused evidence: adapter 6/6, shared
types 1/1, store imports 1/1, public OKF lifecycle 1/1, capture regressions 4/4,
OpenAPI 5/5, forced-RLS completeness 1/1 and console 197/197. The isolated
`demos/cpr-27-okf-v02.sh` passes with one job, three artifacts, two mappings,
two candidates, one governed Knowledge item and two normalised sources.
`make ci` and full `make db-test` against `synveda_test_1177` pass. This is
deterministic local evidence, not a claim of fetching or verifying a remote
Git host.
