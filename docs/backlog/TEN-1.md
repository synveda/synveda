---
title: "TEN-1: Tenant model & resolution"
labels:
  - epic:TEN
  - phase:1
size: M
---

# TEN-1: Tenant model & resolution

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 1 · **Size:** M

## Description

Tenant table, per-request resolution from token claims; tenant context propagated via tower middleware + task-local.

## Acceptance criteria

request without resolvable tenant → 401; traces carry tenant_id.
