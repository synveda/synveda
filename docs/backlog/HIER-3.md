---
title: "HIER-3: Cedar entity sync"
labels:
  - epic:HIER
  - phase:1
size: M
---

# HIER-3: Cedar entity sync

**Epic:** HIER — Hierarchy & scopes · **Phase:** 1 · **Size:** M

> **Re-cut onto governed scopes by CPR-7 on 2026-08-20** (ADR-0074). The
> capability survives — a scope move governs the very next decision — but
> read "org unit" for "department" and "scope" for "hierarchy node"
> throughout. Its demo is now `demos/cpr-7-scopes.sh`.

## Description

Hierarchy changes stream into Cedar entity store transactionally.

## Acceptance criteria

move a team between departments → authz decisions reflect it in the same transaction boundary.
