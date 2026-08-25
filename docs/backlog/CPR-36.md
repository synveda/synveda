---
title: "CPR-36: One-runtime deployment convergence"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-36: One-runtime deployment convergence

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** L

## Description

Converge local install, source/release Compose and Helm on one context-platform
gateway, schema epoch, generated public API, PDP, VedaFlow, audit and forced-RLS
path. Deployment shapes choose infrastructure; personal, team and enterprise
behaviour comes only from governed Configuration versions and bindings.

## Acceptance criteria

- The host gateway, both Compose manifests and Helm use the same gateway
  artefact and current schema/public contract. No deployment switch selects a
  product edition or duplicates governed runtime settings.
- Every deployed gateway connects with a LOGIN role that is non-superuser,
  non-BYPASSRLS and a member of `synveda_app`; migration and tenant admission
  retain a separate admin identity. A database test proves tenantless access is
  refused under the Compose runtime role.
- Installation creates only the schema, tenant, deployment key boundary and
  runtime database identity. First login creates identity/scope/grant; all
  workspace, project, session, Knowledge and Configuration work uses the public
  API under PDP/VedaFlow/audit.
- `init --demo`, bundled demo identities, the dead release seeder/shape and all
  packaging/documentation references are deleted. No removed hierarchy,
  policy-assignment, Record, global observe/inject/recall or old configuration
  route is present in an executable deployment artefact.
- A deterministic deployment check renders source/release Compose and Helm,
  validates the least-privilege DSNs and current route inventory, packages the
  release in an upgrade-shaped repeat, and fails on reintroduced retired
  surfaces or seeder assets. It is part of `make ci`.
- Install, deployment, Helm and beta documentation describe Configuration
  templates as governed data, retain the single-gateway/unsigned/no-Windows/no
  zero-downtime limits, and make no unsupported HA or live-provider claim.
- Focused CLI/database tests, deployment check, chart lint, acceptance demo,
  `make ci` and `make db-test` pass.

## Evidence

Delivered from `2e70aaf5a10a74dea6f224e1fefff4e81d798db3` under
ADR-0095. CLI init tests pass **19/19**, including a real least-privilege
runtime-login/RLS transaction; deployment checker fixtures pass **4/4**;
epoch tests pass **10/10**; OpenAPI/router tests pass **6/6**; strict Helm
lint/render, both Compose renders and repeat-package replacement pass. The
isolated `demos/cpr-36-deployment-convergence.sh` and the **84-script** demo
drift gate pass. Complete `make ci` passes, and full `make db-test` passes on
fresh scratch database `synveda_test_77612`, removed by the harness. The
resulting commit hash is recorded by CPR-37.
