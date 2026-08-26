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

The implementation now includes scripts/uninstall.sh, surgical MCP client
removal and Claude plugin removal. Unit tests cover idempotency, symlink
refusal and the rule that a default uninstall keeps both persistent volumes
and data/kms.key. The remaining gap is end-to-end evidence against an actual
installed release and the vendor-owned client/plugin state. The governing
boundary is [ADR-0067](../adr/adr-0067-uninstall-and-cleanup.md).

## Scope

- Remove only files placed by the installer and stop only the selected Synveda
  deployment.
- Preserve Postgres volumes and the matching local KMS key by default.
- Make purge an explicit coupled destruction of deployment data and key, with
  a dry-run that names every target.
- Remove only Synveda-owned entries from supported client configuration and
  confirm plugin unload through the vendor CLI.

## Non-goals

- No tenant or data-subject erasure claim; that is TEN-5.
- No deletion of shared images, hand-written client configuration or binaries
  copied outside the installer footprint.
- No automatic client-config or plugin mutation by the shell uninstaller.
- No claim that destroying a key alone is erasure.

## Architecture seam

The shell script mirrors the release installer and Compose profile. Client
configuration removal stays in the CLI parser that wrote the entry. Plugin
removal stays behind the Claude CLI. Persistent database data and KMS material
are one recovery unit; default cleanup must not separate them.

## Acceptance criteria

- On every claimed installed platform, default uninstall stops the deployment,
  removes installer-owned program files, retains the named volumes and KMS key,
  and reports remaining client entries.
- Reinstallation against retained data and key can sign in and open previously
  sealed tenant data.
- Purge removes volumes and key only after explicit confirmation; if volume
  removal fails, the key survives and the command exits non-zero.
- MCP removal preserves adjacent servers and JSONC layout byte-for-byte.
- Plugin removal is confirmed by the vendor CLI.
- Default, purge and client/plugin removal are idempotent; every dry-run writes
  nothing and lists exact targets.

## Required tests

- Keep scripts/uninstall.test.mjs and CLI MCP/plugin unit tests.
- Add an installed-release test using a scratch home, isolated Compose project
  and retained-data reinstall.
- Add fault injection for failed Compose teardown and unwritable/linked paths.
- Run the plugin assertion only when an authenticated supported Claude CLI is
  available; otherwise record the prerequisite, not a pass.

## Rollout and rollback

Ship removal alongside the installer version whose footprint it understands.
Retain compatibility with one supported installed footprint. A failed default
uninstall is rerunnable and preserves data/key material; purge is irreversible
and has no rollback beyond independently tested backups.

## Dependencies

TEN-5 owns per-tenant erasure. OPS-5 owns recoverability before purge can be
recommended operationally. Release owners must define supported installer
versions, confirmation UX and whether system package-manager integration is
needed.
