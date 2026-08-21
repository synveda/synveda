---
title: "HIER-1: Hierarchy store"
labels:
  - epic:HIER
  - phase:1
size: M
---

# HIER-1: Hierarchy store

**Epic:** HIER — Hierarchy & scopes · **Phase:** 1 · **Size:** M

> **Deleted whole by CPR-7 on 2026-08-20** (ADR-0074 decision 1).
> `hierarchy_nodes`, `hierarchy_closure`, `synveda_store::hierarchy`, its
> tests and its demo are gone; nothing was translated. The closure-table
> shape and its latency discipline survive in `scopes` + `scope_closure`
> (CPR-3, ADR-0070). This document is kept as the record of what the
> feature decided, not as a description of the product.

## Description

Closure table + materialised path; org→division→department→team→user with configurable depth; CRUD via admin API.

## Acceptance criteria

10k-node hierarchy; ancestor/descendant queries <1ms.
