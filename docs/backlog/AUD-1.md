---
title: "AUD-1: Hash-chained audit log"
labels:
  - epic:AUD
  - phase:1
size: M
---

# AUD-1: Hash-chained audit log

**Epic:** AUD — Audit (functional requirement) · **Phase:** 1 · **Size:** M

## Description

Append-only, BLAKE3-chained per tenant; every authz decision, inject (with commit-hash watermarks), recall, observe, proposal transition, policy change, lapse, admin action.

## Acceptance criteria

tamper test — mutating any historic row breaks chain verification.
