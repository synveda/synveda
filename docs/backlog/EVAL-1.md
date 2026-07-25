---
title: "EVAL-1: Eval harness skeleton"
labels:
  - epic:EVAL
  - phase:1
size: M
---

# EVAL-1: Eval harness skeleton

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 1 · **Size:** M

## Description

Rust runner + fixtures; executes scenario suites against a live stack; CI-integrated with regression gates on the five axes: accuracy, latency, tokens, recall, abstention.

## Acceptance criteria

`make eval` runs the scenario suite against a live stack and reports all five axes
(accuracy, latency, tokens, recall, abstention) as machine-readable JSON plus a human
summary; a committed baseline gates the run; a real product change that degrades quality
(a bank-mode pack flip withholding derived memory) fails the gate naming the axis, the
baseline, the measurement, and the delta; nightly workflow; demo script.

_Written 2026-07-25 (EVAL-1, ADR-0028): the feature text specified a runner and gates but
no criteria. The gate is the load-bearing part — a harness that reports without failing is
a dashboard, and the five axes only mean something if a real regression trips them._
