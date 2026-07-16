---
title: "TEN-6: Cross-tenant isolation test harness"
labels:
  - epic:TEN
  - phase:3
size: M
marker: "continuous"
---

# TEN-6: Cross-tenant isolation test harness

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M · **Marker:** continuous

## Description

Fuzzing suite that attempts cross-tenant reads via API, recall, inject composition, and graph traversal.

## Acceptance criteria

runs in CI nightly; any leak fails the build. (This is also an evaluation deliverable — see EVAL-5.)
