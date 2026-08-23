---
title: "CPR-13: The demo corpus re-point"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-13: The demo corpus re-point

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Filed 2026-08-23 by CPR-12, which went looking for the demos it had to
re-point and found that most of them had already stopped working.

**45 of the 67 shell scripts under `demos/` were dead**, and had been since
CPR-7 (2026-08-20). That prompt deleted `synveda role bind`, `synveda hierarchy`
and `/v1/hierarchy/*` whole — which was right, and is exactly what ADR-0074
decided — and it re-pointed the code, the tests, the CLI and the docs. It
did not re-point the demos.

Nothing said so. CPR-7, CPR-8, CPR-9, CPR-10 and CPR-11 all record clean
runs, because **no gate runs a demo**. `make ci` builds, lints, tests,
checks the backlog, the ADR statuses, the API types, the benchmarks, the
chart images, the corpus licences and the npm licences. It does not open a
single `.sh` file under `demos/`.

CLAUDE.md's own definition of done says a feature is done "ONLY when its
acceptance criteria pass, demonstrated by a test or a runnable demo script
under `demos/`". For most of Phases 1–3, the demo half of that sentence
currently refers to scripts that exit non-zero on their fourth line.

## Why this is not CPR-12's

CPR-12 deleted `/v1/observe`, `/v1/inject` and `/v1/recall`, which are
named in 32 demos. 28 of those 32 are inside the dead 45 — so re-pointing
their call sites onto the session plane produces scripts that are still
dead one command earlier, at `synveda role bind`.

Fixing the placement half is not a search-and-replace. `role bind
--role org-admin` and `hierarchy create --kind team` map onto a governed
scope model where a workspace mints its own scope and an `owner` grant in
one transaction, the tenant root is minted by the first thing that needs a
parent, and nobody declares an organisation. Each script's setup narrative
— which is most of what a demo *is*, because these scripts are written to
be read — has to be rewritten against that model. That is a prompt's worth
of work, and doing it badly inside another prompt would produce 45 scripts
nobody had run.

CPR-12 fixed the four demos that were actually live (`cpr-10-sessions`,
`eval-2-extraction`, `eval-4-qa` and `ops-2-helm-install`), re-pointed
`demos/fixtures/ops-2/client.sh` — which was in the dead 45, and is the one
place the new placement model and the new session plane meet in a script — and
deleted `ctx-5-recall`, whose subject no longer exists.

**43 of 65 remain**, which is the number this feature closes.

## What it adds

1. **Every remaining demo re-pointed** onto workspaces, projects, scopes
   and grants — and, for the 28 that also name them, onto the session
   plane.
2. **A gate**, which is the half that matters more. `make check-demos`
   fails when a script under `demos/` names a CLI subcommand
   `synveda --help` does not list, or a `/v1` path absent from
   `docs/api/openapi.json`.

The gate has four precedents in this repository and each exists for the
same reason: `check-backlog`, `check-adr-status`, `check-api-types` and
`check-benchmarks` were all written after a document drifted from the tree
once. This is the fifth, and the drift it catches is three prompts old.

## Acceptance criteria

1. No script under `demos/` names a CLI subcommand that `synveda --help`
   does not list, or a `/v1` path absent from `docs/api/openapi.json`.
2. The gate catches a deliberately reintroduced dead command — asserted by
   a test that reintroduces one, not by inspection.
3. The gate is part of `make ci`.
4. A representative demo from each of MEM, CTX, FLOW, AUTHZ and ADPT runs
   green against a live stack.

## Note on scope

A demo is documentation that executes. The reason to fix all 45 rather than
delete them is that they are the only artefact in this repository that
explains *why* a subsystem is shaped the way it is to somebody who has not
read its ADR — and the reason to gate them is that nobody noticed for three
prompts.
