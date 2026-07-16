---
title: "HIER-2: Scope chain resolver"
labels:
  - epic:HIER
  - phase:1
size: S
---

# HIER-2: Scope chain resolver

**Epic:** HIER — Hierarchy & scopes · **Phase:** 1 · **Size:** S

## Description

Given identity → ordered scope chain for composition (user→…→org), cached with invalidation on hierarchy change.

## Acceptance criteria

cache invalidation test; p99 <0.5ms warm.
