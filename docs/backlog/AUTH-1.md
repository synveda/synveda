---
title: "AUTH-1: OIDC login (code+PKCE)"
labels:
  - epic:AUTH
  - phase:1
size: M
---

# AUTH-1: OIDC login (code+PKCE)

**Epic:** AUTH — Authentication & identity (functional requirement) · **Phase:** 1 · **Size:** M

## Description

Any compliant IdP; Rauthy bundled for dev/SMB. JWKS cache, rotation handling.

## Acceptance criteria

login via Rauthy and via a mock Entra config both yield a Synveda session.
