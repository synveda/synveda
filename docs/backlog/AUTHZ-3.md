---
title: "AUTHZ-3: Roles & role bindings"
labels:
  - epic:AUTHZ
  - phase:1
size: M
---

# AUTHZ-3: Roles & role bindings

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 1 · **Size:** M

> **Deleted whole by CPR-7 on 2026-08-20** (ADR-0074 decisions 1 and 6).
> `role_bindings`, `synveda_types::Role`, `/v1/roles/*`, `synveda role
> bind`, the tests and the demo are gone. Authority is a **grant** of one
> of six role keys at a governed scope, inherited by its subtree (CPR-5,
> ADR-0072) and decided by the anchor model (CPR-6, ADR-0073). Decision 3
> (roles are decision context, never a second decision point) and decision
> 6 (the `synveda-admins` convention) survive on the new noun. This
> document is kept as the record of what the feature decided, not as a
> description of the product.

## Description

viewer/contributor/curator/steward/org-admin/auditor/security-reviewer/compliance; bound per node, inherited downward.

## Acceptance criteria

full role×action matrix golden-tested.
