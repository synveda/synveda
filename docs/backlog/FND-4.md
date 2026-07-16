---
title: "FND-4: Migrations & bitemporal base tables"
labels:
  - epic:FND
  - phase:0
size: M
---

# FND-4: Migrations & bitemporal base tables

**Epic:** FND — Foundation · **Phase:** 0 · **Size:** M

## Description

sqlx migrations; records with (tx_from, tx_to, valid_from, valid_to); triggers for tx-time maintenance.

## Acceptance criteria

as-of query returns historical row states; property tests.
