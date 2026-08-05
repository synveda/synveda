---
title: "ADPT-6: LlamaIndex memory adapter"
labels:
  - epic:ADPT
  - phase:4
size: M
marker: "Phase 4"
---

# ADPT-6: LlamaIndex memory adapter

**Epic:** ADPT — Adapters & SDKs · **Phase:** 4 · **Size:** M · **Marker:** Phase 4

## Description

Synveda behind LlamaIndex's memory and retriever interfaces; governed recall as a retriever, writes host-owned (ADR-0057 decision 6) so the same turn is not also stored by ADPT-2's tool.

## Acceptance criteria

example app persists and recalls across sessions, and a run with the adapter and the MCP server both configured writes each turn once.
