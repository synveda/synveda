---
title: "MEM-5: Always-on dedup & conflict detection"
labels:
  - epic:MEM
  - phase:2
size: L
---

# MEM-5: Always-on dedup & conflict detection

**Epic:** MEM — Memory core (write path) · **Phase:** 2 · **Size:** L

## Description

Near-dup merge (embedding + minhash); contradiction detection creates explicit supersession edges with validity windows (Graphiti pattern) — never ADD-only.

## Acceptance criteria

LongMemEval knowledge-update category score ≥ baseline; superseded facts retrievable via as-of but excluded from current inject.
