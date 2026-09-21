---
title: "OPS-12: Consumer installation and harness setup"
labels:
  - epic:OPS
  - phase:4
size: XL
---

# OPS-12: Consumer installation and harness setup

## Problem and evidence

Start: `f9620cd2d51d18873523e9ab1abec1b3464c13ec`, clean `main`, workspace
version `0.4.0`. Committing and pushing this OPS-12 checkpoint to `main` is
authorized. Branch changes, version bumps, tags and release publication still
require separate authorization; never replace v0.4.0. The published release uses
`e59284619567d6a13b70ce3f3b3e81121b7621e6`; it is not this working tree.

Deliver authenticated local evaluation through the existing Compose graph and
native client-only installation. Keep external PostgreSQL, generic OIDC,
Keycloak, Cedar, forced RLS, VedaFlow, audit, session ownership, governed Skills
and durable spooling. Ordinary shutdown retains both databases and their keys.

Published commands: `./synveda-compose up|down|status|logs|credential|sample`,
`synveda login --gateway URL --profile NAME`, `synveda whoami --profile NAME
--json`, `synveda plugin install --scope user|project|local`, and
`synveda mcp install|uninstall --client CLIENT`. `synveda init` refuses.
`up/down/status/logs/doctor/setup/adapter` are not yet native CLI commands.
The published reference launcher prepares host files before Compose. This
working tree adds a plain-Compose candidate, explicitly excluded from the
publishing workflow until its named-volume recovery path is qualified.

Current release builds six images once per native Linux architecture: product,
postgres, cnpg-postgres, keycloak, proxy and browser-acceptance. GHCR is the only
published registry. Native binary archives contain CLI, gateway and worker for
macOS arm64 and Linux x86_64. The plugin archive contains compiled Claude,
Codex and Copilot hooks but requires system Node. The installer downloads server
artifacts for every client. Credentials still assume HOME/XDG. This working tree
fixes Claude removal/replacement scope and serializes credential refresh, login
persistence and logout across updated CLI processes; published v0.4.0 lacks both
fixes.

## Scope

1. Extend the existing release workflow with Docker Hub, immutable version
   refusal, registry-specific manifests, anonymous native pulls and existing
   full deployment gates. Authenticate release artifacts and document owner setup.
2. Make the same Compose graph start fresh through named-volume initialization
   and completed-successfully dependencies, without host secret preparation.
   Preserve missing-key refusal, identity, first-credential retrieval and paired
   recovery. Qualify plain Compose, then route native lifecycle commands to it.
3. Implement public-API setup and unified adapter routes through existing
   installers, exact scope, consent, private receipts, locks, atomic edits and
   conflict-safe removal. Use supported native commands and reviewed vendor
   versions. Distinguish configured, trusted, loaded, replayed and live evidence.
4. Package private pinned hook runtimes and client-only artifacts for native
   macOS/Linux/Windows x86_64 and arm64 where dependencies support them. Add
   Unix/PowerShell installation, OS-aware private paths/ACLs and concurrent refresh
   protection. Never equate cross-build, emulation or WSL with native execution.
5. Run artifact-based login, recall, lifecycle, denial, outage, recovery and
   uninstall acceptance. Record cold/warm timings and download size. Keep the
   published README/UI/help/version contract consistent. Homebrew/WinGet follow
   qualified direct artifacts and separately authorized publication.

## Non-goals

No second installer framework, deployment supervisor, authority bypass, native
Windows server/database, mobile/BSD distribution or implicit data migration.
No publication, account changes or package-manager submission without approval.

## Architecture seam

Extend the current release workflow and Compose/runtime graph; route new client
commands to existing public APIs, plugin/MCP installers and credential authority.
ADR-0115 records the registry increment, ADR-0102 the candidate volume layout,
and ADR-0065 amendment 11 exact native plugin scope. ADR-0027 amendment 4 records
the shared bounded credential mutation/refresh lock. Registration scope and
project-level observation consent remain separate decisions.

## Acceptance criteria

Each scope increment must have artifact-based behavior and refusal tests. A
fresh consumer needs Docker/download tools only for local deployment, and no
Docker/server download for a client connected to an existing gateway. Harness
configuration preserves unrelated entries, exact scope, trust and credentials.
Every claimed native platform and live client requires its own execution report.

## Required tests

Available host: macOS arm64, Node 24.18.0, Rust 1.96.0, OrbStack, Compose 5.1.2,
Helm 4.2.3. PowerShell and native Windows/Linux/macOS Intel execution environments
are unavailable here. Retained deployments must not be reset or reassigned.

Required local checks: `make check-fast`, focused Node packaging/release tests,
`make check-release-parity`, `make check-deploy chart-lint`, and `git diff --check`.
Rust increments also require formatting, strict CLI Clippy and focused tests.
Report services, credentials and native platforms separately from test failures.

Current results: `make check-fast check-release-parity` passes (45 Node tests
plus deterministic packaging/Helm parity). `cargo fmt --all --check`,
`SQLX_OFFLINE=true cargo clippy -p synveda-cli --all-targets -- -D warnings` and
`SQLX_OFFLINE=true cargo test -p synveda-cli -- --test-threads=1` pass; the latter
has 185 unit, 3 database-boundary, 5 MCP replay, 5 native-installer fixture
and 5 cross-process credential tests (203 total). Eight simultaneous native CLI
processes share one rotating-token refresh; separate profiles retain both
updates, logout waits for an in-flight refresh, a killed process releases its
lock without rewriting credentials, and failed refresh preserves private bytes
and the existing pre-emptive fallback. These use a loopback mock gateway, not a
live issuer or harness. Lock timeout, symlink/hardlink refusal, private modes,
exclusive temporary creation and diagnostic redaction also pass.
`make check-deploy chart-lint` passes (379 Node tests, zero skipped,
plus deployment/Helm/static portability gates); `git diff --check` passes.
The first full CLI/deployment runs could not bind sandboxed loopback listeners;
rerunning with local socket access resolved that environment restriction.

Claude Code 2.1.241 was exercised through the built native CLI in a private
`CLAUDE_CONFIG_DIR`, using an inert test marketplace outside the user's config.
Project install, repeated install, forced replacement and project uninstall
passed while a user registration survived. Fixture registrations were then
removed. This proves native registration/scope, not hook or MCP loading.

The local container candidate uses cached Linux ARM64 images on OrbStack
(macOS ARM64 host), not anonymously published Docker Hub images. Real browser
issuer/PKCE login, an API operation, logout/401, two opt-in public-workflow sample
runs, deliberate password retrieval and absence of that password from logs
passed. The reusable `--consumer-candidate` gate also passed fresh ordinary
startup, repeated initialization, missing-key refusal, lost-installation-volume
refusal beside retained PostgreSQL, and down/up preserving all secret hashes,
identity and the sample receipt. All nine reported checks passed. Its report
identifies a dirty source tree and cached local images; this is not release
verification, anonymous registry evidence or new container-build evidence.
An initial mount-layout failure and stale metadata from edited bind-mounted
files were corrected; the final drill used immutable extraction.
macOS Intel, native Linux clients, native Windows (both architectures), Docker
Desktop and new live Claude/Codex/Copilot lifecycle runs remain unverified.

Measured with Docker engine 29.4.0 / Compose 5.1.2 on OrbStack: fresh **state**
startup 278.393s, retained-state startup 223.027s, browser login/API/logout 1.690s.
Every required consumer image was already cached. Cold-download bytes/time and
first authenticated harness-call time are unmeasured; no release startup budget
is established from this one host. The host had development tools installed;
a separately restricted-PATH consumer environment is still required.

Both task-owned projects (`synveda-local-acceptance-ops12` and
`synveda-local-acceptance-132bc91b`) were stopped with their installation,
PostgreSQL and browser volumes retained. The temporary empty volume used to
prove lost-installation refusal was removed after checking its test label.
No unrelated retained deployment was reset or reassigned. The local detailed
report is `/tmp/synveda-ops12-consumer-qualification.json`; these recorded facts
and the runnable gate, not temporary control files, are project truth.

## Rollout and rollback

Keep existing v0.4.0 instructions until a new authorized release qualifies.
Retain immutable bytes and data/keys on failure; never overwrite release tags,
reset an unrelated deployment or promise an untested downgrade.

## Dependencies

Implemented locally:

- `.github/workflows/release.yml`, `release-registries.mjs`, packaging, image
  verification, chart inventory and tests share six native builds across Docker
  Hub and GHCR, refuse reused tags, preserve attestation descriptors and retain
  destination-specific digests. GitHub attestation authenticates the final
  checksum inventory in the configured publishing job. Dispatch stays private
  and nonpublishing. `docs/RELEASING.md` records exact owner setup and limitations.
- `plugin.rs`, CLI options, native-process fixtures and adapter documentation
  preserve Claude scope, persistent data, shared marketplaces and other native
  registrations. Failed/ambiguous inventories and conflicting marketplace
  sources refuse; output distinguishes configured/enabled from loaded.
- `credentials.rs`, `login.rs`, logout routing and native process tests share
  one stable, empty OS lock file through profile reread, gateway refresh and
  atomic replacement. Login/logout use the same lock. Acquisition is bounded
  to 20 seconds; cancellation releases the OS lock without deleting its file.
  Temporary files are uniquely/exclusively created, and parse diagnostics omit
  credential values. Mixed historical CLI versions do not participate in this
  lock. Native Windows ACL/path/replacement qualification remains open.
- The existing packager accepts `SYNVEDA_PACKAGE_CONSUMER_CANDIDATE=1` for local
  qualification. `package-consumer-compose.mjs` renders canonical fragments;
  `initialize-consumer.mjs` prepares named state without network/socket access,
  seals retained identity/secrets and projects only each service's existing
  authority. `CONSUMER.md` accompanies the candidate. Tests cover fresh render
  under spaces/Unicode, private projections, changed identity and unsafe input.
- The existing `qualify-release.mjs` now accepts `--consumer-candidate` and
  delegates its bounded startup/recreation subset to `qualify-consumer.mjs`.
  This is not the complete release gate and never substitutes for paired recovery.

No branch change, tag, owner account change, release publication or signing is
part of this checkpoint. Publication is blocked on an owner-owned
Docker Hub namespace/public repositories, expiring push credential, GitHub
variables/secret and protected environment, plus authorization of a new version
and release trigger. These do not block local implementation and tests.

Next task: extend the existing logical paired-recovery implementation to the
candidate's `synveda-local` named volumes and qualify restoration before
enabling candidate packaging in release CI. Reuse `synveda-logical-backup`,
`synveda-logical-restore` and `recovery-set.mjs`; do not introduce another recovery
format or relax project confirmation. The current host-state launcher must not
operate candidate volumes. Existing backup/recovery project allowlists need the
new layout explicitly; its restore must quiesce writers, refuse retained target
databases and verify the database pair with the original sealed private state.
Then implement native lifecycle/setup/adapter routes,
receipts/consent, private Node runtimes, native
client-only artifacts/installers and their platform/live acceptance. Installer
attestation enforcement and Homebrew/WinGet remain unimplemented; configuration
of GitHub attestations alone does not authenticate the current shell installer.

Continue from this OPS-12 checkpoint on `main` and retain version `0.4.0` until an owner
authorizes a coordinated new version. For another local candidate, use the
existing `package-release.sh` arguments with the explicit candidate flag, extract
the archive, then run `node scripts/qualify-release.mjs --consumer-candidate
EXTRACTED_BUNDLE REPORT.json`. It creates an absent random acceptance project,
leaves its database/installation volumes retained and reports unfinished recovery
separately. Do not reuse an unrelated deployment or replace published artifacts.
