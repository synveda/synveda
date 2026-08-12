---
title: "OPS-8: Release & distribution"
labels:
  - epic:OPS
  - phase:3
size: M
---

# OPS-8: Release & distribution

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** M

## Description

A tagged release somebody else can install. Prebuilt `synveda` and
`synveda-gateway` binaries, public GHCR images, the console bundle and a
self-contained profile bundle, installed by one `curl | sh` — so the only
prerequisite a tester needs is Docker.

## Why this exists

Filed 2026-08-11. OPS-1 built an installer that works and cannot leave this
laptop, and said so three times in its own source:

- `init.rs:95` resolves the compose file relative to the working directory and
  errors with "run `synveda init` from a Synveda checkout".
- `init.rs:701` looks for the gateway in `target/release` and `target/debug`,
  under a comment reading "a release ships this binary".
- `repo_root()` carries the other half: "a released binary would carry its own
  profile — see ADR-0055 decision 6's trigger."

Nothing is published. `synveda/gateway:dev` builds from source at install time
(`--build`), `synveda/dev-postgres:17` builds from `deploy/compose/postgres`,
`.github/workflows/` holds `ci.yml` and `eval.yml` with no release job, and the
repository has no tags. INSTALL.md states the prerequisite plainly — "a Synveda
checkout and a Rust toolchain, until there is a release to download" — and this
is that release.

OPS-2 has the same gap one layer up: the chart names
`synveda/gateway:<appVersion>` and `synveda/enterprise-postgres:17`, and nobody
outside this laptop can pull either.

## Why it is not just Docker

ADR-0055 decision 8, which was found by building the container first and
watching it fail. The bundled Rauthy's issuer is `http://localhost:8100/auth/v1/`,
RFC 6761 makes every resolver answer `localhost` with the caller's own loopback,
and ADR-0010 compares the issuer byte-for-byte against the discovery document
and the `iss` claim — so inside a container that URL is the container, and the
gateway correctly returns `502 {"service":"oidc-jwks"}`. `extra_hosts` with
`host-gateway`, a network alias and `network_mode: host` were all measured and
none of them survives on the two platforms this targets.

So the default install — the one with no IdP to configure — runs the gateway as
a host process, and a release that shipped only images could not serve it. The
release ships both, built from one tree at one tag.

## What ships

- **Binaries** — `synveda` and `synveda-gateway` for `darwin-arm64` and
  `linux-x86_64`, built `SQLX_OFFLINE=true` against the committed query cache.
- **Images** — `ghcr.io/synveda/gateway` and `ghcr.io/synveda/dev-postgres`,
  public, multi-arch, tagged with the release.
- **The console bundle**, so `/console/` works on the host-gateway path. It has
  not until now: the image sets `SYNVEDA_CONSOLE_DIR` and the host process never
  did, so it falls back to `console/dist` relative to the working directory
  (`console.rs:43`) and 404s for anyone without a checkout and a `pnpm` build.
- **A profile bundle** — `deploy/release/`: a compose file naming published
  tags with no `build:` stanza, the Rauthy config and the Postgres initdb SQL.
  Separate from `deploy/compose/docker-compose.yml`, which stays what its header
  calls it — the contributor's loop.
- **The Claude Code plugin**, as a *marketplace* — the unit Claude Code
  installs. Without it the release drops the prerequisite list to Docker and
  still cannot put governed memory in the harness ADPT-1 was built for: a
  tester's only route was to clone the repository and run `npx tsc`. Installed
  by `synveda plugin install`, which drives `claude plugin` rather than
  writing the three JSON files Claude Code keeps.
- **`install.sh`** — arch detection that refuses an unsupported platform by
  name rather than guessing, checksum verification, and one line saying the
  binaries are unsigned.

## What `init` learns

Discovery precedence `SYNVEDA_COMPOSE_FILE` > checkout in the working directory
> installed bundle at `$SYNVEDA_HOME/profile`, so a contributor who has also
installed a release still gets the tree they are editing. The same order for the
gateway binary. `SYNVEDA_CONSOLE_DIR` set on the host spawn. No `--build` from a
bundle. And a refusal when the bundle's version is not the CLI's — a stale
`~/.synveda/profile` under a newer CLI presents as a service that will not start
or a variable the gateway does not read, both of which look like product bugs.

## What it deliberately does not do

No Windows (WSL2 undocumented and untested), no upgrade path, no package
manager, no code signing or notarization, and no hosted instance. Each has a
reversal trigger in ADR-0065 rather than a plan. It also proves nothing new
about Entra or Okta: it makes the bundled-issuer path installable and leaves the
other one exactly where AUTH-4 left it.

## Acceptance criteria

- On a scratch HOME with **no checkout and no Rust toolchain**, `install.sh` →
  `synveda init --demo` → `synveda login` → a governed recall, within OPS-1's
  ten-minute budget with image pull inside the clock.
- The demo installs from the **packaged** bundle rather than from
  `deploy/release/` in place, so a bundle that has drifted from the product
  fails it.
- OPS-1's invariant re-asserted from the installed path: 0 scopes, 0 identities,
  0 role bindings and 0 records the moment the installer finishes, and exactly
  one break-glass event in a verifying chain.
- `/console/` serves on the default host-gateway path.
- `synveda plugin install` puts the plugin into Claude Code and **Claude Code
  loads it** — `✔ enabled`, four hooks, one MCP server, read back from the
  vendor's own CLI. Installing and loading are different events, so the status
  is the assertion and the install is not.
- The installer creates nothing under `~/.claude`, `~/.cursor`, `~/.config/zed`
  or Claude Desktop's config directory.
- The installer refuses an unsupported platform with the platform it detected
  and the path that still works.
- The release profile's images are in the licence inventory and the checker
  covers them.
