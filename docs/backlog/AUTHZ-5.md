---
title: "AUTHZ-5: ABAC conditions"
labels:
  - epic:AUTHZ
  - phase:2
size: M
---

# AUTHZ-5: ABAC conditions

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 2 · **Size:** M

## Description

Sensitivity, residency, channel (published/derived), time-of-day, purpose-of-use as Cedar context.

## Acceptance criteria

`restricted` records never injected without compliance-granted permission, proven by leak-test suite.
