---
title: "CTX-2: Composition engine"
labels:
  - epic:CTX
  - phase:1
size: L
---

# CTX-2: Composition engine

**Epic:** CTX — Context engine (read path) · **Phase:** 1 · **Size:** L

## Description

Scope-gradient assembly (user>team>dept>org), pinned-first, conflict rules, token budget (default 1.5k, per-scope configurable); channel rules (published + policy-permitted derived).

## Acceptance criteria

deterministic given same inputs; every block watermarked with commit hashes + record IDs; tokens_per_inject metric emitted.
