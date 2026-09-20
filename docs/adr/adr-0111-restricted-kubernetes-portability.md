# ADR-0111: Restricted Kubernetes portability

- **Status**: Accepted
- **Date**: 2026-09-19
- **Feature(s)**: OPS-11
- **Deciders**: Owner's OpenShift/cloud portability request; deployment audit

## Context

The external and starter installations already pass Kind acceptance. Their
images and authority boundaries are reusable, but the chart lacks Routes,
NetworkPolicies and explicit application seccomp. OpenShift 4.19 and 4.20 are
distinct qualification targets: restricted-v2 versus restricted-v3 with Linux
user namespaces. No OpenShift or managed-cloud target is configured locally.

## Decision

Extend the existing chart with narrowly scoped edge Routes, opt-in explicit
NetworkPolicy allowances, assigned-ID security contexts, and existing native
HTTP-client trust/proxy inputs. Preserve external PostgreSQL/OIDC, the locked
Keycloak dependency and optional preinstalled CNPG. Do not install controllers.

Route and Ingress selection are mutually exclusive. Edge termination encrypts
browser-to-router traffic only; backends remain HTTP. Expose only application
paths and the packaged identity's public realm/resources. Certificates use the
existing router default or a same-namespace TLS Secret, with narrowly scoped
router read access. Re-encrypt/passthrough require a separately qualified TLS
backend and are not offered as misleading settings.

Use RuntimeDefault seccomp, non-root, dropped capabilities, no escalation and
read-only application filesystems. An OpenShift preset leaves UID/GID assignment
to admission. For restricted-v3, a small kubectl/kustomize Helm post-renderer
adds hostUsers=false to every workload, including the unchanged upstream
Keycloak chart, which has no value for that PodSpec field. Required-SCC pod
annotations prevent silent fallback. CNPG's operator-generated database pods
and storage/user-namespace support require separate platform qualification.

NetworkPolicy is opt-in and refuses missing DNS, ingress, database and identity
allowances. Rules use native selectors/CIDRs and ports, never hostname promises.
Provider, telemetry, directory and model-cache egress are explicit inputs.
Core operation needs no remote account or model endpoint. Preserve OIDC's
deliberate no-proxy client; native OpenSSL HTTP clients can consume a mounted
organisation CA bundle. OTLP retains its private local-Collector boundary.

## Options considered

1. **One chart with overlays and a bounded v3 post-renderer** — selected; keeps
   dependency archives unchanged and states platform prerequisites explicitly.
2. **Fork charts, broaden SCCs or add a proxy/operator** — adds maintenance or
   authority without solving the requested small-team portability boundary.
3. **Call arbitrary-UID simulation OpenShift support** — rejected; it cannot
   exercise SCC admission, SELinux, router, CNI or CSI semantics.

## Consequences

- Existing Compose and Kubernetes paths retain their product contracts.
- v3 installs/upgrades must retain the post-renderer; admission fails if the
  required user-namespace setting is absent. Remove it only after all selected
  dependencies expose and test the native PodSpec field.
- Render/schema checks and local execution are recorded separately from real
  OpenShift, managed-cloud and NetworkPolicy enforcement evidence.
- CNPG/OpenShift qualification cannot be inferred from external-service tests.

## Compliance notes

No changes to Cedar, forced RLS, VedaFlow, tenant transactions, audit or the job
engine. Runtime service accounts receive no Kubernetes API credentials or RBAC.
Certificate read RBAC belongs only to the existing router service account.
