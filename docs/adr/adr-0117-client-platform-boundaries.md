# ADR-0117: Explicit client platform and private-state boundaries

- **Status**: Accepted
- **Date**: 2026-09-21
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The Unix client candidate carries a native CLI and private Node. The CLI also
contains deployment bootstrap code with Unix descriptor/ownership checks.
Several non-Unix storage fallbacks omit those checks, and the CLI and hooks can
disagree on state paths or fall back to a repository-relative spool. Compiling
those fallbacks would not establish a private Windows client.

## Decision

Keep one CLI. Isolate the Unix peer-witness implementation and explicitly refuse
that deployment witness on other platforms. Preserve its no-follow, ownership,
bounded-read and change-detection checks on Unix. Do not introduce a Windows
server/bootstrap implementation or weaken the existing unsafe-code prohibition.

Share an executable path contract between the CLI and hooks. Unix retains
absolute XDG overrides and HOME defaults. Windows resolves config and state
under LOCALAPPDATA/synveda/config and LOCALAPPDATA/synveda/state; explicit XDG
overrides must be fully qualified local drive paths. Relative XDG overrides
are ignored, while missing/relative default roots fail rather than using the
working directory. No automatic copy or migration of credentials is performed.

Until Windows ACL, file identity and atomic replacement have a reviewed native
implementation, private credentials, receipts, logs and spools explicitly
refuse access on non-Unix platforms. Refuse login before the issuer round trip
and refuse local-state mutations before creating files. Hooks must not report
durable recording when their private storage is unavailable. Path resolution
and compilation are separate from permission enforcement and support claims.

## Options considered

1. **Explicit platform boundaries** — chosen; preserves the existing client and
   exposes the remaining privacy work without silently granting access.
2. **Remove permission checks to cross-build** — a compiling executable could
   disclose credentials or transcript data through inherited ACLs.
3. **Split out another Windows client** — duplicates credential and public-API
   behavior before there is native qualification evidence.

## Consequences

The path contract is testable on any development host. Native Windows build and
refusal checks can run without deployment services; they do not qualify login,
refresh, spooling, vendor loading or an installer. PowerShell installation and
Windows artifact publication remain gated on private storage and native
acceptance for each claimed architecture. Replace the explicit refusal only
with those checks and evidence, never with inherited-permission assumptions.

## Compliance notes

Public APIs retain Cedar, forced RLS, VedaFlow and audit authority. Local paths
and receipts grant no product authority. Existing Unix credential serialization,
observation consent, spool formats and retained deployment data are unchanged.
