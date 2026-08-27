# ADR-0067: uninstall removes what we wrote and stops what we started, keeps the data unless told otherwise, and treats another application's config as somebody else's property

- **Status**: Proposed
- **Date**: 2026-08-13
- **Feature(s)**: OPS-10
- **Deciders**: sujitn

## Context

OPS-8 made this product installable by a stranger. Nothing removes it.
`grep -rn uninstall` over `crates/synveda-cli/src` and `scripts/` matches one
comment inside `plugin.rs` describing how *Claude Code* replaces a plugin.

A beta asks somebody to run software on a machine they own, which makes
removal part of the product rather than an afterthought — and the reason it
needs a decision record rather than a script is that the footprint has three
tiers with three different owners, and treating them alike would be wrong in
a different way for each.

The forces:

- **Seed §2**: this product sells trustworthiness. Software that is hard to
  remove is not trustworthy, and software that removes more than it was asked
  to is worse.
- **TEN-5 is not done.** A tenant row cannot be deleted — 32 foreign keys,
  every one `ON DELETE NO ACTION` — so there is no per-tenant erasure to
  offer. The smallest unit of "remove this memory" available today is the
  Postgres volume.
- **`install.sh` documents its own footprint** in its header and keeps to it,
  including the promise that it touches nothing belonging to an editor or an
  AI client. Uninstall inherits that promise: it may only remove what we can
  show we put there.

## Decision

Three surfaces, split by who owns the bytes:

1. **`scripts/uninstall.sh`** — the mirror of `install.sh`. Removes what the
   installer wrote, stops what `init` started, and **keeps the data**.
2. **`synveda mcp uninstall --client <c>`** — removes exactly the `synveda`
   entry from a client's own config, the precise mirror of `mcp install`.
3. **`synveda plugin uninstall`** — drives `claude plugin uninstall` and
   `marketplace remove`, the mirror of `plugin install`.

## Options considered

1. **One `synveda uninstall` verb doing everything.** Rejected on two counts.
   A binary that deletes itself is a trick rather than a design — on macOS,
   unlinking a running Mach-O is legal and confusing, and the failure mode
   (a half-removed install with no CLI to finish the job) is exactly what
   ADR-0065 amendment 6 spent a release fixing in the other direction.
   Removal also has to work when the CLI is *already* broken, which is one of
   the reasons somebody reaches for it.
2. **One `uninstall.sh` doing everything, including client configs.**
   Rejected: editing Claude Desktop's or Zed's config from shell means
   reimplementing the JSONC splice that `mcp install` already does correctly,
   in `sed`, against files whose comments and layout we promised to preserve.
   The mirror of a CST edit is a CST edit.
3. **Documentation only — a list of paths to `rm`.** Rejected. It is what
   exists today in effect, it cannot be idempotent, it cannot know whether
   the sudo fallback moved the CLI, and it puts `docker compose down -v` in
   front of somebody as a copy-paste line with no sentence attached about
   what it destroys.
4. **The three surfaces above, as chosen.** More pieces, and the split lands
   on a real boundary rather than a convenient one: ours, the deployment's,
   and somebody else's.

## Decisions in detail

### 1. Data survives by default, and `--purge` says what it takes

Stopping containers is reversible. Removing the volumes is not, and because
**TEN-5 means a tenant cannot be deleted**, those volumes are the only unit
of erasure this product currently has. A default that took them would make
`uninstall.sh` the most destructive command we ship, run by people whose
mental model is "undo the install".

So the default stops the deployment and leaves `pg-data`, `rauthy-data` and
`tei-cache` in place, **naming them** and printing the command that would
remove them. `--purge` removes them and says in the same breath that a
tenant's memory is what it just removed and that TEN-5 is why there was no
smaller unit. (`gateway-search` was removed by the context-platform hard cut.)

This is the one place the missing feature is visible to a user as a missing
feature, and the message says so rather than hiding it.

### 2. Preserved data keeps `kms.key` (amended 2026-08-26 by CPR-44)

`$SYNVEDA_HOME/data/kms.key` survives a default uninstall with the database
volumes. The script removes the other runtime entries under `data/`, leaves
the key at its canonical path and names it in the result. Reinstalling and
running `synveda init` therefore reuses the same volumes and the same key.

This supersedes the original OPS-10 decision to delete the file after only a
warning. Under ADR-0064 the local KEK wraps the deployment key and every
tenant key. Destroying it while retaining the database irreversibly loses
console sessions and tenant secrets and makes previously sealed tenant
exports unusable. A warning is not consent to that loss, and searchable
Knowledge being substrate-encrypted rather than application-sealed does not
make the partial destruction sound.

`--purge` is the explicit coupled destruction path: Compose removes the
persistent volumes and uninstall removes `data/kms.key`. `--purge --dry-run`
names both operations and performs neither. The retained key remains a secret
that must stay `0600` and be backed up. If Compose cannot confirm volume
removal, purge exits non-zero and preserves the key; a default uninstall no
longer claims to leave `$SYNVEDA_HOME` empty.

### 3. Another application's config is somebody else's property

`mcp uninstall` removes the `synveda` key and nothing else — an adjacent MCP
server survives, and a JSONC file's comments and layout come back
byte-identical, because that is the promise `mcp install` already makes in
the other direction and half a promise is not one.

It is a CLI verb rather than part of the shell script for the reason option 2
gives, and it is *not* run automatically by `uninstall.sh`: the installer
never wrote those files, so the uninstaller may not remove them without being
asked. `uninstall.sh` lists what it found and names the verb.

### 4. `plugin uninstall` asks the vendor, not the filesystem

ADR-0065 amendment 2 found that a plugin can be present on disk and never
loaded, so `plugin install` asserts `claude plugin list` rather than file
presence. Removal inherits it exactly: removing and *unloading* are different
events, and the second is the one that matters. Claude Code copies a plugin
into a versioned cache it owns, so removing our marketplace is not removing
the plugin — that is `claude plugin uninstall`, then `marketplace remove`.

### 5. Idempotent, and a `--dry-run`

A second run finds nothing, says so, and exits 0. An uninstaller that fails
on what it already removed is one nobody runs twice, and the moment somebody
runs it twice is the moment something went wrong the first time.

`--dry-run` lists every path and container it would touch and changes
nothing — the same courtesy `init` and `seed.sh` extend, and more warranted
here, because this is the destructive direction.

## Consequences

- **Positive.** The product can be removed as cleanly as it is installed, by
  the person who installed it, without a support conversation. The three-way
  split means each tier's removal is exactly as careful as its ownership
  demands. Retained encrypted state remains recoverable because its KEK is
  retained with it.
- **Negative / accepted.** Three surfaces rather than one, so "remove
  everything" is more than one command — mitigated by `uninstall.sh` printing
  the other two when it finds their traces. The default leaves data behind,
  which will surprise somebody who expected uninstall to mean erase; the
  output names the volumes and the flag. And a CLI copied somewhere by hand
  is not found, because the installer did not put it there.
- **Reversal trigger.** If TEN-5 lands and a tenant can be erased properly,
  `--purge`'s "the volume is the only unit" reasoning expires and uninstall
  should offer per-tenant removal before offering volume destruction. If
  telemetry or beta feedback shows people reaching for `--purge` routinely to
  get a clean slate, the missing verb is "reset this deployment", not a
  bigger hammer on uninstall.

## Compliance notes

- **Audit**: uninstall emits no audit event and deliberately cannot. It runs
  against a stopped deployment, and an "I was removed" entry in a chain that
  is about to be deleted with its volume would be theatre. What *is* audited
  is everything that happened before it, and `--purge` is what ends the
  chain's existence — stated at the prompt.
- **Erasure**: this is **not** GDPR erasure and the documentation must not
  imply it is. Destroying a volume removes a whole deployment, not a data
  subject; per-tenant, ordered, certificate-producing erasure is TEN-5's, and
  ADR-0064 decision 7 already records that destroying keys is not erasure
  either, since records are not sealed.
- **Secrets**: a default uninstall preserves `kms.key`; only the explicit
  `--purge` path removes it with the volumes. The operator must continue to
  protect and back up the retained `0600` file. Losing it still makes console
  sessions, tenant secrets and sealed exports unrecoverable; uninstall no
  longer causes that loss implicitly.
