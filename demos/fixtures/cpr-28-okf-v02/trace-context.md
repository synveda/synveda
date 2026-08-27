---
type: decision
title: Propagate traceparent
summary: Public requests use W3C trace context.
status: stable
verification:
  method: repository-review
  verified_at: 2026-08-25T00:00:00Z
x-migration-ticket: PB-418
---

Public requests use `traceparent`; `X-Request-Id` is superseded.
