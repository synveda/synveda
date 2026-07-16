---
title: "AUTH-2: JIT user provisioning from claims"
labels:
  - epic:AUTH
  - phase:1
size: M
---

# AUTH-2: JIT user provisioning from claims

**Epic:** AUTH — Authentication & identity (functional requirement) · **Phase:** 1 · **Size:** M

## Description

First login: map groups/claims → hierarchy nodes via mapping rules (convention defaults `synveda-{dept}-{team}`, override table).

## Acceptance criteria

new user lands in correct team scope with zero admin action; unmapped users land in quarantine scope with no read rights.
