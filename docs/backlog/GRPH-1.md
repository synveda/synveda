---
title: "GRPH-1: Multi-graph schema"
labels:
  - epic:GRPH
  - phase:2
size: M
---

# GRPH-1: Multi-graph schema

**Epic:** GRPH — Knowledge graph & relationships · **Phase:** 2 · **Size:** M

## Description

Named graphs — entity, episode, provenance (MAGMA-informed) — carried as a mandatory discriminator over a bitemporal edge pair in Postgres. Edges carry bitemporal validity.

## Acceptance criteria

An edge written through the store API reads back through the traversal API with its kind, endpoints and validity intact; a supersession closes the prior edge's window with both versions readable as-of; the shipped statements' plans contain no sequential scan over the edge table.

_Amended 2026-07-27 (ADR-0043): the title was "Multi-graph AGE schema" and the criterion named Cypher round-trip tests. GRPH-4/ADR-0029 measured relational adjacency 3–8× faster than AGE at 2.5× less storage and handed the schema call to this feature's design ADR; the substance of the criterion survives, the Cypher mechanism does not._
