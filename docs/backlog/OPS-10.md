---
title: "OPS-10: Uninstall & cleanup"
labels:
  - epic:OPS
  - phase:3
size: M
---

# OPS-10: Uninstall & cleanup

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** M

## Description

The mirror of OPS-8. `scripts/uninstall.sh` removes what the installer wrote
and stops what `init` started; `synveda mcp uninstall` and `synveda plugin
uninstall` remove what the operator later asked us to write into somebody
else's configuration. Data survives by default, and the flag that takes it
says so.

## Why this exists

Filed 2026-08-13. OPS-8 made the product installable by somebody else and
gave them no way to remove it. There is no `uninstall.sh`, no `mcp
uninstall`, no `plugin uninstall`; `grep -rn uninstall` over the CLI and the
scripts matches only a comment inside `plugin.rs` describing how *Claude
Code* replaces a plugin.

A beta asks people to run something on a machine they own. "How do I get rid
of this" is a question that has to be answered before it is asked, and
answering it in prose does not count — the footprint has three tiers with
three different owners, and only one of them is safe to delete without
thinking.

## The three tiers, and why they are not one command

**Ours, and exactly removable.** `install.sh` documents its own footprint in
its header and keeps to it: `$SYNVEDA_BIN/synveda` (default `/usr/local/bin`,
or `$SYNVEDA_HOME/bin/synveda` when the sudo path fell back), plus
`$SYNVEDA_HOME/{bin,console,profile,plugin}`. `init` adds `$SYNVEDA_HOME/data`
— the pidfile, the log, the rendered environment, and **`kms.key`**.

**The deployment, which holds the memory.** Containers and four named volumes
(`pg-data`, `rauthy-data`, `tei-cache`, `gateway-search`). Stopping is safe;
removing the volumes is not, and it is the **only** way to remove a tenant's
memory, because TEN-5 means the tenant row itself cannot be deleted. So the
default stops and keeps, `--purge` removes, and the message says that memory
is what it is removing.

**Somebody else's, and therefore surgical.** A `synveda` key inside Claude
Desktop's, Cursor's, Zed's, VS Code's, Windsurf's or Continue's own config,
and a plugin inside a cache Claude Code owns. `install.sh` deliberately
touches none of this and says so; uninstall must not either. Removal is the
exact mirror of `mcp install` — take out the one key we own, write every
other byte back as found, keep comments and layout — which is the same CST
splice and therefore belongs in the CLI beside the verb that wrote it, not in
a shell script that would have to reimplement it.

## What it deliberately does not do

It does not delete a tenant, because nothing can (TEN-5). It does not remove
Docker images, which are shared and cheap to re-pull. It does not touch a
config the operator wrote by hand from `mcp install --print`, because the
product never knew about it. And it makes no attempt to find a copy of the
CLI somebody moved somewhere of their own choosing — it removes what the
installer placed and reports anything of ours still on `PATH`.

## Acceptance criteria

- From a scratch HOME with an installed release: after `uninstall.sh`,
  nothing of ours is on `PATH`, nothing is left under `$SYNVEDA_HOME`, and no
  container of ours is running.
- **The data volumes survive by default** and are named in the output, with
  the command that would remove them.
- `--purge` removes them and says in the same breath that a tenant's memory
  is what it just removed, and that TEN-5 is why there was no smaller unit.
- `synveda mcp uninstall --client <c>` removes exactly the `synveda` entry:
  an adjacent MCP server survives, and a JSONC config's comments and layout
  are byte-identical afterwards.
- `synveda plugin uninstall` leaves `claude plugin list` without ours, read
  back from the vendor's own CLI rather than from the filesystem — the ADPT-1
  lesson (ADR-0065 amendment 2): installing and loading are different events,
  and so are removing and unloading.
- Everything is **idempotent**: a second run finds nothing, says so, and
  exits 0 rather than failing on what it already removed.
- `uninstall.sh --dry-run` lists every path and container it would touch and
  changes nothing.
