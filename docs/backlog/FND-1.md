---
title: "FND-1: Workspace scaffold"
labels:
  - epic:FND
  - phase:0
size: S
---

# FND-1: Workspace scaffold

**Epic:** FND — Foundation · **Phase:** 0 · **Size:** S

## Description

Rust workspace per tech plan §8 + pnpm workspace; empty crates compile; CI: fmt, clippy -D warnings, test, deny (licence check).

## Acceptance criteria

`cargo build --workspace` green in CI.
