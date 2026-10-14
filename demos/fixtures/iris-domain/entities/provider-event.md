---
type: Entity
title: Iris ProviderEvent
generated:
  by: human:fictional-iris-author
  at: "2026-10-14T00:00:00Z"
stale_after: "2027-01-01T00:00:00Z"
description: Canonical event identity for the fictional Iris project.
---

An Iris ProviderEvent is uniquely identified by provider_event_id within a project. Multiple Deliveries may refer to one ProviderEvent. Deduplicate eligible events by provider_event_id before computing a rate.
