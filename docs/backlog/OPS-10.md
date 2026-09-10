---
title: "OPS-10: Uninstall & cleanup"
labels:
  - epic:OPS
  - phase:3
size: M
---

# OPS-10: Uninstall & cleanup

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** M

## Problem and evidence

The canonical launcher can stop an exact Compose project while preserving its
volumes, or reset that project after an exact confirmation. The release
installer separates immutable releases from mutable state and backups. It does
not yet persist a strict ownership receipt that proves the installed launcher,
project selection, artifact paths and CLI destination. Without that evidence,
automatic artifact deletion would be guesswork.

`scripts/uninstall.sh` therefore fails closed: normal invocation exits without
mutation, `--dry-run` is also a no-op, and destructive purge is unsupported.
It does not call Docker, signal a PID, traverse state, delete credentials or
guess old volume names. This honest refusal replaces the retired
profile/host-gateway uninstaller; it is not feature completion.

## Scope

- Define a strict, non-executable installer receipt for the canonical reference
  deployment and exact CLI destination.
- Stop only through the receipt-bound installed `synveda-compose` launcher.
- Remove only receipt-owned immutable artifacts after a successful stop.
- Preserve Compose volumes, state, KMS/OIDC material and backups by default.
- Keep client configuration and plugin removal as separate explicit CLI acts.
- Make every destructive reset use the launcher's exact project confirmation.

## Non-goals

- No tenant or data-subject erasure claim; that is TEN-5.
- No deletion of shared images, external databases, external object stores,
  hand-written client configuration or unrecorded binaries.
- No recursive privilege escalation or guessed legacy compatibility path.
- No automatic deletion of recovery backups.

## Architecture seam

The receipt must bind one immutable release, its source/environment identity,
the exact installed launcher, fixed Compose project/provider selection, owned
artifact roots and CLI destination. It is parsed as an allowlisted data format,
never sourced or evaluated. Mutable state and backups stay outside the
immutable release tree.

Default uninstall delegates `down` to that launcher and removes artifacts only
after success. Destructive deployment reset delegates the existing
confirmation-gated `reset`; backup deletion remains a separate custody choice.
The shell boundary never invokes Docker directly.

## Acceptance criteria

- A malformed, missing, linked, foreign or ambiguous receipt causes zero
  filesystem, process and Docker mutation.
- Default uninstall stops exactly the recorded project, removes only recorded
  immutable artifacts and CLI, and retains state, volumes and backups.
- A failed stop preserves the launcher, receipt and all recovery material for
  retry.
- Any destructive reset requires the exact launcher confirmation and never
  targets an external dependency.
- `--dry-run` lists exact actions and paths while making no lifecycle call or
  write.
- Reinstallation against retained state signs in and opens the existing
  product evidence.
- Explicit MCP/plugin removal preserves adjacent client configuration and is
  idempotent.

## Required tests

- Keep the current fail-closed `scripts/uninstall.test.mjs` boundary until the
  receipt implementation lands.
- Add receipt parsing, hostile path/symlink and foreign-project tests.
- Add installed-reference stop/reinstall evidence using an isolated Compose
  project.
- Inject stop/reset/filesystem failures and prove recovery material remains.
- Run vendor plugin assertions only when an authenticated supported client is
  available; otherwise report the prerequisite as unavailable.

## Rollout and rollback

Ship receipt-aware removal only alongside the installer version that writes the
receipt. Until then retain the refusal. A failed default uninstall is rerunnable
and preserves data; destructive reset has no rollback beyond independently
verified backups.

## Dependencies

OPS-5 owns recoverability before destructive reset can be recommended. TEN-5
owns per-tenant erasure. Release owners must define supported installer
versions, confirmation UX and whether a system package manager becomes the
artifact owner.
