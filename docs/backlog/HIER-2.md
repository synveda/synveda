---
title: "HIER-2: Scope chain resolver"
labels:
  - epic:HIER
  - phase:1
size: S
---

# HIER-2: Scope chain resolver

**Epic:** HIER — Hierarchy & scopes · **Phase:** 1 · **Size:** S

> **Deleted whole by CPR-7 on 2026-08-20** (ADR-0074 decision 2). The
> `ScopeChainCache`, its invalidation seam, its metrics, its tests and its
> demo went with the tree they cached. Chains resolve per request through
> `scope_closure`. This document is kept as the record of what the feature
> decided, not as a description of the product.

## Description

Given identity → ordered scope chain for composition (user→…→org), cached with invalidation on hierarchy change.

## Acceptance criteria

cache invalidation test; p99 <0.5ms warm.
