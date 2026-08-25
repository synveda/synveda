---
title: "CPR-30: Governed runtime configuration artifacts"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-30: Governed runtime configuration artifacts

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Replace mutable policy-pack assignment and ad-hoc runtime settings with stable
configuration aggregates, immutable versions and revisioned governed-scope
bindings. Personal, team and enterprise become canonical configuration
templates over one runtime rather than editions or code branches.

## Acceptance criteria

- Stable ConfigurationArtifacts own immutable, content-hashed ordered
  ConfigurationVersions and current-version pointers. ConfigurationBindings
  resolve nearest-scope-first, may follow current or pin an exact version, and
  change only under exact revision preconditions.
- Complete validated documents cover policy-profile selection, capture and
  extraction rules, context budget and channels, trace retention, type-aware
  freshness, Skill/Tool advertisement and allowed external-provider families.
  `personal`, `team` and `enterprise` are canonical templates copied into an
  ordinary version; no runtime edition/profile-name branch exists.
- Create, publish, bind, enable/disable, pin/unpin and rollback are typed
  VedaFlow `Configuration/apply` changes. Apply repeats ownership, PDP,
  proposal, payload-hash, expected-version and expected-revision checks; all
  semantic transitions are hash-chain audited without settings prose or
  secrets.
- Capture freezes the exact effective configuration, context planning records
  it and enforces its budget/channels/freshness/trace/provider settings, and
  Skill/Tool advertisement obeys the same current resolved document.
- The old default and scope policy-assignment tables and mutation routes are
  deleted without translation or dual writes. Policy packs remain Cedar and
  approval sources; the only assignment rows handed to the PDP are an
  in-memory projection of resolved configuration.
- Public generated APIs plus generated console and public-HTTP CLI surfaces
  support template inspection, list/show, version history/comparison, create,
  publish, bind, effective inspection and rollback. Collections are bounded,
  creation is idempotent, and mutable heads require preconditions.
- All new tenant tables use enabled and forced RLS. Focused domain/store/PDP/
  VedaFlow/audit/API/console/CLI tests, a runnable demo, `make ci` and full
  `make db-test` pass. ADR-0089.

## Status

Delivered 2026-08-25 from
`b33ba51c0101c171f1be43e209002c1cd21a127a` under accepted ADR-0089.
Migration `0055_governed_configuration` adds four forced-RLS tables, extends
context traces with an explicit current-Knowledge/unreviewed-candidate channel,
and deletes the two mutable policy-assignment tables. The generated contract
now has 162 operations and 255 schemas; the public-HTTP CLI and Advanced
Configuration console consume it without another application DTO.

Acceptance evidence: configuration domain 4/4, public database API 1/1,
capture 4/4, context planning 3/3, policy approvals 6/6, packs 7/7 and PDP
11/11, forced-RLS 83/83, OpenAPI 6/6 and complete console 210/210. Full
`make ci` and `make db-test` pass. The isolated
`demos/cpr-30-governed-configuration.sh` scenario passes with two stable
artifacts, three immutable versions, two revisioned bindings, six audited
applications and both replaced assignment tables absent.
