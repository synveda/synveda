---
title: "TEN-2: Postgres row-level security as backstop"
labels:
  - epic:TEN
  - phase:1
size: M
---

# TEN-2: Postgres row-level security as backstop

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 1 · **Size:** M

## Description

RLS policies on every tenant-scoped table keyed to a session GUC set per connection. Defence-in-depth: app bug cannot cross tenants.

## Acceptance criteria

adversarial test suite — direct SQL with wrong tenant GUC returns zero rows on every table.
