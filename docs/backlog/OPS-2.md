---
title: "OPS-2: Helm deployment form"
labels:
  - epic:OPS
  - phase:3
size: L
---

# OPS-2: Helm deployment form

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** L

## Description

The same gateway/schema/generated API as every deployment, with CloudNativePG,
customer IdP wiring and optional model infrastructure. It is not a product
edition (re-cut by CPR-36/ADR-0095).

## Acceptance criteria

Kind-cluster install, least-privilege runtime role, governed session/context
round trip and CloudNativePG data-plane failover test. The gateway remains one
replica until OPS-7.
