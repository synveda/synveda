---
title: "OPS-1: Single-node deployment form"
labels:
  - epic:OPS
  - phase:3
size: M
---

# OPS-1: Single-node deployment form

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** M

## Description

One gateway binary + Postgres + Rauthy + optional TEI Compose. `synveda init`
bootstraps schema, tenant, key material and the RLS-enforced runtime login; it
creates no governed product data (re-cut by CPR-36/ADR-0095).

## Acceptance criteria

laptop → login → working governed context in <10 minutes, documented; no
workspace, session, Knowledge or Configuration exists before the governed
post-login path creates it.
