# ADR-0117: Explicit client platform and private-state boundaries

- **Status**: Accepted; amended once
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

### Amendment 1 (2026-09-21): native Windows credential storage

Qualify the credential boundary before opening the other private-state paths.
The existing CLI remains the only credential authority. Windows uses safe,
Windows-only `windows-permissions` and `winapi-util` wrappers for process SID,
handle ACLs and file identity, and `winsafe` for local fixed-drive admission.
These MIT dependencies do not relax the product's unsafe-code prohibition.

Walk local drive paths with no-follow directory handles held against deletion.
Ancestors must have trusted owners and refuse unprivileged data/attribute,
delete or ACL mutation; public traversal and creating a new subdirectory are
allowed. This prevents inherited ACL changes from another account above the
private directory. Standard root ownership by TrustedInstaller is admitted.
Refuse reparse points, alternate streams, device names, ambiguous components,
non-disk objects and multiply linked credential files. Existing private
directories/files must belong to the process user and grant access only to that
user, LocalSystem and Administrators. Compare numeric SIDs from the native
descriptor; do not infer machine/domain identity from SDDL abbreviations.
Inheritable creator-owner entries are
allowed only when they cannot grant access on the parent. Unknown ACL forms
fail closed; no repair or silent adoption of existing permissions occurs.

Create missing directories only below an already private parent with private
inheritance. Seal new directories and files with a protected, explicit DACL
before writing private bytes. A stable empty lock holds the existing bounded
cross-process credential transaction; handle identity and ACLs are checked
before and after lock acquisition. Reads are bounded. Replacement writes and
flushes an exclusively created sibling, checks the current destination, closes
handles and renames in the same directory without a delete/truncate fallback.
Rename sharing/access refusals receive at most 20 retries with 25 ms delay, rechecking the
destination before each attempt, so brief readers do not prevent refresh commit.
Failure retains the original credential file. This is interruption resistance,
not a power-loss recovery or hostile same-account/administrator guarantee.

Login checks storage before its issuer round trip. Native filesystem and
mock-gateway refresh tests establish this local storage slice; real issuer,
installer, hook, receipt and spool qualification remain separate. Keep the
non-Unix refusal on those other private-state paths until their own consumers
and native behavior tests use the same protection. No Windows server port is
introduced.

### Original platform boundary

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

Until each Windows storage path has ACL, file identity and atomic replacement in a reviewed native
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
