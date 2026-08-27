---
title: "OPS-9: Release-shaped beta acceptance"
labels:
  - epic:OPS
  - phase:3
size: L
---

# OPS-9: Release-shaped beta acceptance

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** L

## Problem and evidence

The packaged PulseBoard walkthrough now exists as
synveda demo start|status|reset with personal, team and governed profiles. It
uses only authenticated public APIs and is covered by
demos/cpr-41-one-command-demo.sh and
[ADR-0100](../adr/adr-0100-public-api-pulseboard-demo.md). What remains open is
release-shaped beta evidence from a clean machine and an independent operator;
a deterministic checkout run is not evidence that a published package,
authentication callback, console and supported client work together.
[Production readiness](../PRODUCTION_READINESS.md) therefore does not treat the
demo as a production claim.

## Scope

- Exercise one published release artifact from installation through login,
  PulseBoard creation/resume, console inspection and removal.
- Cover personal, team and governed outcomes without direct SQL, bootstrap
  authority or fabricated review completion.
- Record the exact binary, image, chart and verified client versions used, plus
  every unavailable external prerequisite.
- Produce a short beta tour whose support claims are derived from
  adapters/registry.json and docs/CLIENT_SUPPORT.md.

## Non-goals

- No new demo-only API, direct database seeder or policy bypass.
- No automatic activation of pending Knowledge, Skill, Tool or Configuration
  changes.
- No client-support, SaaS, directory-provider or production-readiness claim
  without its own evidence.
- No replacement for product acceptance, security evaluation or backup drills.

## Architecture seam

The CLI remains a resumable public-API client. It composes the existing
workspace, project, session, capture, Knowledge, context, Skill and Tool
operations and stores only a local receipt. Packaging and installation own the
artifact boundary; the gateway, PDP, RLS and VedaFlow paths remain unchanged.
[ADR-0066](../adr/adr-0066-beta-demo-profile.md) remains relevant only where it
requires operator authority after login.

## Acceptance criteria

- A clean supported host installs a published artifact, signs in to the
  console, starts each profile and resumes it idempotently.
- The personal profile proves principal-scope privacy; the team profile proves
  reuse by a real second principal or returns an explicit one-time invite; the
  governed profile leaves review-required effects pending.
- The console shows the created Sessions, Context, Knowledge, Skills and Tools
  through generated APIs, and the frozen audit prefix verifies offline.
- A failed dependency or missing credential produces an accurate limitation,
  not a successful support claim.
- The run records artifact digests, versions, elapsed time and cleanup result.

## Required tests

- Keep the deterministic CPR-41 and PulseBoard product-gate tests.
- Add installed-release acceptance on each claimed OS/architecture.
- Run one authenticated live lifecycle for every client named verified in the
  beta tour; captured or replay-only clients remain labelled as such.
- Mutation-test console sign-in, second-principal reuse and pending-review
  assertions so each fails when its claimed behaviour is removed.

## Rollout and rollback

Publish as a release candidate and retain the previous package manifest. A
failed beta run removes or archives only its own demo resources and downgrades
the support claim; it never rewrites product data or acceptance baselines.

## Dependencies

Release artifact parity, signing/licensing policy, supported platforms and a
real authenticated client are external prerequisites. The product owner must
define the beta audience, support channel, data-retention notice and evidence
required to promote beyond evaluation.
