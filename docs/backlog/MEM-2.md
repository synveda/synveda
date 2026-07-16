---
title: "MEM-2: Redaction & secret scanning"
labels:
  - epic:MEM
  - phase:1
size: M
---

# MEM-2: Redaction & secret scanning

**Epic:** MEM — Memory core (write path) · **Phase:** 1 · **Size:** M

## Description

PII patterns + gitleaks-derived secret rules; modes deny/redact/quarantine per policy pack.

## Acceptance criteria

seeded secrets never reach storage in any mode; quarantine review queue works.
