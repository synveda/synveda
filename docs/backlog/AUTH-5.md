---
title: "AUTH-5: Directory sync fallback"
labels:
  - epic:AUTH
  - phase:3
size: M
---

# AUTH-5: Directory sync fallback

**Epic:** AUTH — Authentication & identity (functional requirement) · **Phase:** 3 · **Size:** M

## Description

Scheduled pull sync (Temporal) for IdPs without SCIM push.

## Acceptance criteria

drift converges ≤ sync interval; deletions handled as leavers.
