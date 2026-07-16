---
title: "EVAL-5: Security evals"
labels:
  - epic:EVAL
  - phase:2
size: M
---

# EVAL-5: Security evals

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 2 · **Size:** M

## Description

Policy-leak suite (restricted content never crosses sensitivity/scope under 10k generated query variants); cross-tenant fuzz (TEN-6); prompt-injection-via-memory suite (a memory containing instructions must not alter agent behaviour when injected — content is data, wrapped and labelled).

## Acceptance criteria

nightly; zero-tolerance gate.
