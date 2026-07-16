---
title: "FLOW-1: Object store"
labels:
  - epic:FLOW
  - phase:2
size: M
---

# FLOW-1: Object store

**Epic:** FLOW — VedaFlow (git-style governance) · **Phase:** 2 · **Size:** M

## Description

BLAKE3 content-addressed objects/trees/commits/refs in Postgres; commits record author identity, signature, and policy-pack snapshot hash.

## Acceptance criteria

property tests — identical content dedups; history immutable under concurrent writers.
