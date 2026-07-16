---
title: "TEN-5: Tenant lifecycle"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-5: Tenant lifecycle

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Description

Create/suspend/export/delete workflows (Temporal); delete produces signed destruction certificate; export = portable archive (records+assets+audit).

## Acceptance criteria

GDPR-style erasure E2E test; export re-imports into a fresh instance.
