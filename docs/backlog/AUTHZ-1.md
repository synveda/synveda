---
title: "AUTHZ-1: Cedar PDP embedded"
labels:
  - epic:AUTHZ
  - phase:1
size: M
---

# AUTHZ-1: Cedar PDP embedded

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 1 · **Size:** M

## Description

`authorize(subject, action, resource, ctx)` facade; entities materialised from hierarchy; policy store per tenant, hot-reload.

## Acceptance criteria

µs-level decision benchmark; decision + policy version logged for every call.
