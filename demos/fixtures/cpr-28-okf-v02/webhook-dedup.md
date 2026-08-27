---
type: pulseboard-practice
title: Deduplicate webhook delivery
summary: Provider event IDs are the webhook idempotency key.
tags:
  - webhooks
  - reliability
x-owner: platform
x-retention-class: operational
---

Webhook deliveries are deduplicated by provider event ID.

The request propagation convention is [trace context](trace-context.md).
