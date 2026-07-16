---
title: "TEN-4: Per-tenant encryption keys"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-4: Per-tenant encryption keys

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Description

Envelope encryption; key ref per tenant; KMS trait (local dev impl + AWS/GCP/Vault impls later).

## Acceptance criteria

tenant export is unreadable without that tenant's key.
