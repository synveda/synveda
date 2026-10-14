---
type: Fact
title: Iris acknowledged delivery rate
generated:
  by: human:fictional-iris-author
  at: "2026-10-14T00:00:00Z"
stale_after: "2027-01-01T00:00:00Z"
description: Fictional metric definition awaiting normal publication review.
---

acknowledged_delivery_rate is a percentage: 100 multiplied by acknowledged unique eligible ProviderEvents divided by all unique eligible ProviderEvents. Its grain is project and UTC event day, using provider event time rather than ingestion time. Exclude sandbox events and events with an unknown commit outcome from both numerator and denominator. A zero denominator yields an unknown rate, never zero percent. Acknowledgment uses the approved Iris unknown commit decision.
