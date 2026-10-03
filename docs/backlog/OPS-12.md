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
version `0.4.0`. The owner subsequently authorized merging the CI fixes and
publishing a successor from latest main. v0.4.1 and v0.4.2 tagged publication
stopped before their final signed asset sets. The next immutable version,
v0.4.3, was published from `2acc66f02625727b2ccdfe223358468bf10eef85`
after exact-artifact draft recovery. Never replace the existing tags, images or
charts. The earlier v0.4.0 release remains at
`e59284619567d6a13b70ce3f3b3e81121b7621e6`.

Deliver authenticated local evaluation through the existing Compose graph and
native client-only installation. Keep external PostgreSQL, generic OIDC,
Keycloak, Cedar, forced RLS, VedaFlow, audit, session ownership, governed Skills
and durable spooling. Ordinary shutdown retains both databases and their keys.

Published commands: `./synveda-compose up|down|status|logs|credential|sample`,
`synveda login --gateway URL --profile NAME`, `synveda whoami --profile NAME
--json`, `synveda plugin install --scope user|project|local`, and
`synveda mcp install|uninstall --client CLIENT`. `synveda init` refuses.
The v0.4.3 client implements `up/down/status/logs/doctor/setup/adapter` under
[ADR-0116](../adr/adr-0116-native-consumer-commands.md); these were not in the
v0.4.0 binary. The [consumer command guide](../CONSUMER_CLI.md) owns their
exact scope, observation and receipt contracts.
The published reference launcher prepares host files before Compose. This
release adds a plain-Compose bundle with native named-volume recovery reports.
Release CI packages it and requires its separate lifecycle and recovery reports
before publication.

The v0.4.0 release used GHCR only. Current release builds six images once per
native Linux architecture: product, postgres, cnpg-postgres, keycloak, proxy and
browser-acceptance, then copies the same OCI candidates to Docker Hub and GHCR.
Native binary archives contain CLI, gateway and worker for
macOS arm64 and Linux x86_64. The plugin archive contains compiled Claude,
Codex and Copilot hooks but requires system Node. The installer downloads server
artifacts for every client. Credentials still assume HOME/XDG. The v0.4.3 client
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
the shared bounded credential mutation/refresh lock. ADR-0116 records the native
routes, shared local-operation lock, ownership receipts and managed observation
checks. Registration scope and project-level observation consent remain separate
decisions; Codex/Copilot hook registration and vendor trust remain manual.

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

Concurrent-conversation verification on 2026-09-23 used captured Claude Code
2.1.241 frames, the built hook, isolated PostgreSQL, and local Git worktrees.
Ordinary exit now retains the native binding and `clear` closes it; duplicate
event IDs with different content conflict. A new live two-conversation,
resume, fork and subagent run remains open: this host has Claude Code 2.1.241
but no native login, isolated provider credential or Synveda bearer. The next
action is an explicitly authorized provider run in a disposable configuration,
capture of the native child/fork fields, and comparison against persisted
session/event provenance. The same-gateway cross-principal switch of one native
ID also needs a stable authenticated namespace: Stop and PreCompact are local
only, so the current spool cannot prove a principal change at those hooks.
Until that is resolved, start a new native conversation when changing
principals. A fixture replay alone does not qualify the live claim.
An offline native run that never opened a Synveda Session still needs a
SessionStart for that same native ID; a later different conversation cannot
guess its placement and promote its undelivered events.
The same shared-runtime audit found and fixed a Codex/Copilot regression:
native runtime exit must flush without ending the task. Existing captured-frame
tests failed on an unexpected `/end` call before the fix; Codex's eight and
Copilot's 31 tests pass after it. An origin-mismatched Copilot start also now
holds the saved spool byte-for-byte. Registry fixture/digest checks, shared MCP
interleaving and the fresh captured-Claude/Python/TypeScript gateway workflow
pass. These replays do not replace an authenticated native multi-conversation
run for the three harnesses.
Two additional failing-then-passing regressions pin automatic spool lookup and
background retry to the persisted client installation ID. A copied spool from
another installation is held; explicit operator recovery remains separate.
An unwritable installation-ID store also now refuses automatic binding rather
than minting a different namespace at each hook invocation.

Earlier OPS-12 results: `make check-fast check-release-parity` passes (48 Node tests
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
`make check-deploy chart-lint` passes (382 Node tests, zero skipped,
plus deployment/Helm/static portability gates); `git diff --check` passes.
The first full CLI/deployment runs could not bind sandboxed loopback listeners;
rerunning with local socket access resolved that environment restriction.

Candidate packaging and all 48 release-parity tests also pass with Compose
2.38.2, the hosted runner's version. The generated graph preserves explicit
`create_host_path: false` in the archive; a live missing-source check with both
2.38.2 and 5.1.2 refuses without creating a host directory. OPS-2 failover
fixtures cover transport errors, bounded retry exhaustion and authorization
refusal while retaining the pre-existing session and one idempotency key.
`make claude-acceptance` passes on fresh isolated PostgreSQL fixtures locally.
The [hosted replay rerun](https://github.com/synveda/synveda/actions/runs/35587876306/job/106311668496)
also passes without a database change; the original bootstrap refusal has not
reproduced in either run. `make check-demos` passes all seven fixture tests and
validates 91 executable demos against the generated public contracts.

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

### Named-volume recovery evidence (2026-09-21)

The named-volume recovery increment passed the complete extracted candidate gate
on macOS arm64 / OrbStack, Docker Engine 29.4.0 and Compose 5.1.2. All 20 reported
checks passed: ordinary startup/recreation, missing-key and lost-installation
refusals, recovery-in-progress refusal, real browser issuer/PKCE/API/logout,
repeated sample use, quiesced logical backup, damaged archive and duplicate-ID
refusal, exact restore confirmation, retained-target refusal, tenant/audit/key
verification and wrong-key refusal, original sealed secret hashes, restored
browser identity and the original sample receipt. The restored proxy had no
public port and logs contained no private values. Both projects were stopped
with their installation, PostgreSQL, browser and recovery volumes retained;
no unrelated deployment was reset or reassigned.

This was a dirty working-tree candidate from `601aa6d46cb46eb52a73236c93fcc4a3b68695ec`,
using cached local Linux ARM64 images and a newly built PostgreSQL image with
the changed backup project allowlist. It is not anonymous registry or native
Linux-host release evidence. Fresh-state startup took 278.283s, retained-state
startup 217.903s, paired backup 13.156s and private restore 220.034s; required
images were cached, so cold-download size/time remain unmeasured. The first
restore exposed omitted empty credential directories; the shared recovery
copy now preserves them, and a fresh immutable extraction passed. Its failed
target remains stopped for diagnosis. The detailed local passing report is
`/tmp/synveda-ops12-paired-verified.json`; these facts and the runnable gate are
the durable evidence, not that temporary path.

`make check-fast check-release-parity` passes with 55 Node tests plus packaging
and Helm parity. `make check-deploy chart-lint` passes with 389 Node tests,
zero skips, and the deployment/Helm/portability contracts. The existing
evaluation socket fixture needed local socket access; its sandbox refusal was
an environment restriction. Focused native logical-recovery tests also prove
that retained tables or active database connections prevent either restore.
Consumer and private-restore renders pass with Compose 2.38.2 and 5.1.2.
No Rust, schema, generated API or SQLx contract changed in this increment.

### Native command increment (2026-09-21)

The source CLI now routes lifecycle through the exact extracted consumer graph,
with closed Docker child settings, local-engine/version checks and a private
engine/project/bundle receipt. First startup refuses retained unowned resources;
partial startup can retry with its original receipt. Normal shutdown retains
the database, installation, browser and recovery volumes. Read-only status/logs
and doctor distinguish configured prerequisites from product authentication.

Public-API `setup` lists visible projects or verifies one active project and
workspace before updating the existing root config. Workspace/project creation
remains in console onboarding. Observation defaults off; enabling it binds a
private receipt to the project, workspace and selected credential profile.
Updated hooks use the Git root even when invoked below it, and require the
matching local receipt for managed observation. Missing, copied or changed
managed config cannot silently grant recording. The packaged Claude runtime has
a checked contract marker so the new route refuses historical plugin bytes.

`adapter` wraps the existing Claude and registry-backed MCP installers. Exact
scope, private intent receipts, atomic file replacement, shared OS locking and
conflict-safe removal preserve other entries and native registrations. Claude's
managed route is project/local only. Native trust/loading and the manual
Codex/Copilot hook steps are not inferred from configuration. Updated older
plugin/MCP mutation routes share the same process lock.

The built macOS arm64 CLI passed a fresh native `up`, `status`, bounded `logs`,
`doctor` and `down` against the earlier extracted named-volume artifact on OrbStack.
The unchanged browser fixture then passed real issuer/PKCE/console login, an
authenticated API operation, logout and post-logout 401. This uses cached local
Linux ARM64 images and the dirty source CLI, not anonymous release bytes or a
new native harness session. The task-owned project is
`synveda-local-acceptance-native-01`; its local CLI receipt is under an isolated
temporary XDG directory. After native shutdown, no project containers remained;
the installation, PostgreSQL and browser volumes were retained. No unrelated
deployment was changed. Native client-only artifacts, private Node, Windows
ACL/path handling and hosted/native client qualification remain open.

Focused tests cover native subprocess lifecycle/refusals, API denial, setup
consent, interrupted Claude registration, MCP drift/unowned refusal, unrelated
JSONC preservation and cross-process lock waiting. The existing refresh fixture
now explicitly makes accepted sockets blocking: macOS inherited the listener's
nonblocking mode and intermittently failed before receiving HTTP bytes. No
production credential logic or assertion was weakened. Adapter tests also cover
nested repositories and receipt/profile/selection/missing-file consent refusal.
Archive replay checks the new hook contract and uses extracted bytes for consent
and the existing Codex/Copilot lifecycle suites.

Validation passes: formatting, strict CLI Clippy, all 211 CLI tests, 107 shared
adapter tests, 35 extracted plugin tests, `make check-fast`, and
`make check-deploy chart-lint` (389 Node tests, zero skips, release parity and
deployment/Helm/portability contracts). `git diff --check` passes. Local sockets
and Docker required execution outside the restricted sandbox. The pnpm shim
could not retrieve its package-manager signature in that environment; adapter
compilation and tests used the already installed locked TypeScript compiler and
Node directly, without changing dependencies or disabling signature checks.

### Private runtime and Unix client increment (2026-09-21)

ADR-0065 amendment 12 extends the existing release pipeline and shell installer
with a client mode. Four native Unix build targets package the existing CLI,
three adapters and private Node 24.21.0, pinned to upstream archive SHA-256s.
The Node executable and complete upstream licence are retained; npm, Corepack,
server binaries, console and Compose artifacts are absent from the client
archive. Claude's copied plugin carries the private runtime and names it in
both hook and MCP manifests. Codex/Copilot keep their manual registration and
trust steps, using the printed private Node and hook paths.

Installation requires only shell/download/archive/checksum tools. It verifies
the archive, rejects links/traversal before extraction, checks a bounded content
inventory and executes the native CLI/runtime identity probes before mutation.
Private ownership receipts and an exclusive installer lock protect versioned
releases, a stable user-local launcher and an atomic current link. Reinstall
preserves earlier releases, deployment data, credentials, spools and vendor
configuration. Unowned or modified files fail closed. The CLI discovers the
client marketplace first and refuses a damaged client link instead of selecting
a historical plugin. An interrupted installer lock requires inspection; automatic
client artifact removal remains open. ADR-0132 adds publisher-attestation
enforcement to the current source bootstrap, with qualification recorded below.

The extracted candidate passed on macOS arm64 using a source debug CLI and the
official pinned Node archive. It used a private home containing spaces,
apostrophes and Unicode and a PATH without Node, Docker, compilers or package
managers. Installation, CLI/runtime execution, all three hook entry points,
repeat installation with retained deployment bytes and Codex/Copilot lifecycle
replays passed. These are mock-gateway replay and native executable checks, not
real issuer or native harness qualification. The local report records source
`b861fa708e8bd83cd9777bdb5a14e969a8c68ded` with dirty-tree evidence. Cold network
download size/time and first authenticated harness call remain unmeasured.

The final debug-CLI archive is 54,258,566 bytes, SHA-256
`84ca08cb2a46cfeb828c435fb42f087292ae1525a4f0c8d44729f23c111f521b`.
Local file-backed installation took 2.061s and repeat installation 2.128s;
all six artifact checks and 31 extracted Codex/Copilot replay tests passed.
Claude Code 2.1.241 also accepted the extracted plugin manifest in an isolated
configuration directory; that proves manifest acceptance, not loading.
The source CLI's seven plugin unit tests and five native scope fixtures pass,
as do formatting and strict CLI Clippy. The artifact replays and the existing
deployment evaluation fixture need local socket access; sandbox socket refusals
are environment restrictions, not passing behavior checks.
`make check-fast`, `make check-release-parity` (69 Node tests) and
`make check-deploy chart-lint` pass; the latter includes 403 Node tests with
zero skips plus deployment, Helm and portability contracts. `git diff --check`
passes. No schema, generated API, SQLx metadata or dependency lockfile changed.

Release assembly now requires six native client reports matching the archive
hash/size, source, platform and Node pin. Tagged publication additionally requires
clean source and matching CLI version; the reports enter the attested checksum
inventory. The Windows storage and installation increments below record their
separate native evidence. The four Unix release jobs remain unqualified;
a matrix row alone does not establish a working or private client.

### Client platform boundary prerequisite (2026-09-21)

[ADR-0117](../adr/adr-0117-client-platform-boundaries.md) isolates the Unix
deployment peer-witness reader while preserving its original ownership,
no-follow and bounded-read checks. Other platforms explicitly refuse that
witness. Unix-only direct dependencies and imports are conditional. One shared
14-case fixture defines CLI/hook config and state roots, including Windows
LOCALAPPDATA, drive-qualified XDG overrides and UNC/device/relative-path refusal.
On Unix, missing or relative HOME no longer selects a repository-local spool.

The initial non-Unix boundary refused login, credential access and local
mutations. The Windows storage amendments below selectively replace those
refusals; unported operations still refuse before mutation.
The added Windows x64 CI job requires native strict CLI Clippy, path-contract,
peer-witness and built-executable refusal tests, plus the hook contract checks.
Its passing hosted execution and subsequent credential evidence are recorded
below. Local cross-checking stopped in native
dependencies because this Mac lacks the MSVC assembler and Windows SDK; even a
temporary BLAKE3 pure-backend check stopped at stacker's missing `windows.h`.
No product dependency, feature or lockfile was changed to bypass that limit.

Local formatting, strict CLI Clippy, all 215 CLI tests, all 109 shared adapter
tests and 35 extracted-plugin tests pass. Initial loopback fixture refusals were
resolved by granting local socket access. `make check-fast` and release parity
(69 Node tests plus packaging/Helm checks) pass. All deployment/Helm stages pass
(403 Node tests, zero skips); the final stages were rerun with socket access
after the evaluation fixture's sandbox refusal, retaining the already-passing
prerequisite results. `git diff --check` passes. The native credential evidence follows below.
The subsequent Windows storage and installation results follow below;
real issuer/harness qualification remains outstanding.

### Windows credential increment (2026-09-21)

ADR-0117 amendment 1 adds native credential storage: process-user ACL checks,
fixed local-drive admission, no-follow ancestor handles held against rename,
file identity/hard-link refusal, bounded reads, protected new ACLs, the stable
process lock and flushed sibling replacement. Native numeric SIDs preserve
account identity when Windows abbreviates SDDL. Rename retries are bounded and
revalidate the destination; failure keeps the original file. Existing unsafe
ACLs are refused without repair. Windows-only MIT wrappers preserve the product
unsafe-code prohibition.

The [native Windows x64 job](https://github.com/synveda/synveda/actions/runs/35634324225/job/106448482489)
passed at `c627375`. Strict CLI Clippy, all eight storage/admission tests, all
five rotating-refresh process tests, three executable platform tests, the path
and peer-witness checks, and the selected hook contract tests pass. The 18 Rust
tests cover independent .NET ACL verification, spaces/Unicode, private creation
and replacement, brief and held readers, directory rename refusal, broad ACLs,
hard links, junctions, oversized files and ambiguous paths. Eight concurrent CLI
processes spend one refresh token; separate profiles retain updates, logout
waits, termination releases the lock, and issuer refusal preserves credentials.
These use a loopback mock gateway, not a real issuer or harness.

Local strict CLI Clippy and the 217-test CLI suite passed, followed by all three
focused ACL tests (including the added ancestor case) and the five strengthened
refresh tests. Formatting, dependency direction, licence/source/bans/advisory
checks, `make check-fast`, release parity, deployment convergence, Helm and
`git diff --check` pass. Local socket fixtures required sandbox permission; no
test was skipped to avoid that restriction. The isolated storage module and its
tests compile for both Windows MSVC architectures from macOS; this is not full
CLI cross-compilation or native arm64 execution.

The following candidate extends that boundary; the earlier credential evidence
does not qualify its new consumers.

### Windows receipt/spool candidate (2026-09-21)

ADR-0117 amendment 2 reuses native ACL/identity/storage checks for private Rust
receipts and spools. Windows Node hooks use the hidden version-1 `private-state`
pipe protocol for spool access, setup-receipt reads, stable installation identity
and disclosure markers. An absolute `SYNVEDA_CLI` executable is required; no shell,
credentials or gateway requests participate. Unsafe state remains held without
repair. A stable spool lock and snapshot digest prevent stale replacement or
retirement. Windows files/scans have 16-MiB/4096-entry limits. Repository/vendor
edits, deployment witnesses and Windows diagnostic-log writers still refuse.

Rust rewrites now retain transcript/model fields and omit absent optional event
timestamps so Node can read them. Purge retains an empty spool that owes a close;
hooks retire only after successfully persisting their completed state.
The local CLI suite (219 tests) and shared adapter suite (109 tests) passed;
local loopback fixtures required socket access. Native receipt, spool, protocol
and Rust/Node interoperability tests pass on x64/arm64 CI as recorded below.
The local host has no native Windows; real issuer/harness
acceptance stays separate.

### Windows installation candidate (2026-09-21)

ADR-0065 amendment 13 extends the existing packager and JavaScript installer
with native Windows x64/arm64 ZIPs and a PowerShell 5.1+ bootstrap. The pinned
Node 24.21.0 Windows archives retain upstream SHA-256 and complete licences.
Bootstrap validates private local ancestry, checksum uniqueness, bounded ZIP
entries and native PE architecture before executing extracted code. It never
requests elevation or changes execution policy.

The installer uses the Rust ACL/identity backend through a separate hidden
filesystem protocol. A private ownership receipt, retained immutable releases,
digest-checked PowerShell launcher and atomic `current.json` selection preserve
the existing upgrade contract without symlink privileges. The fixed-root hook
storage protocol remains separate. Setup/vendor writers, deployment and
diagnostic logs remain refused on Windows.

CI and release jobs now build each Windows architecture natively and require
restricted-PATH install/reinstall, private Node/CLI/hook startup, Rust/Node
storage interoperability and checksum/ZIP/drift/interruption refusals. Release
assembly requires all six platform reports tied to their archive bytes and
source; adding a matrix row does not establish qualification. The source
PowerShell script passes the PowerShell 7.6.6 parser on macOS; the native
execution results below separately establish Windows PowerShell behavior.

Local validation: formatting and strict CLI Clippy passed, as did all 219 CLI
tests and `make check-fast check-release-parity` (70 Node tests). An isolated
MSVC compile/Clippy probe of the actual credential, receipt, spool and installer
modules/tests passed for x86_64 and aarch64 after consolidating a duplicate test
helper import. That probe uses pure BLAKE3 to avoid the absent MSVC assembler;
it is not a full native build or execution report. A fresh macOS arm64 archive
passed restricted-PATH installation/reinstallation and all 31 extracted
Codex/Copilot replay tests. Its source identity is `c63eb95` plus the recorded
dirty worktree; this local candidate is not publication evidence.
The required `make check-deploy chart-lint` gate also passed, including its
405 Node tests, workflow checks and all chart contract renders.

On clean source `418669f20fcfbadf693ebc52de384b71667fa0af`, the native
[Windows x64 job](https://github.com/synveda/synveda/actions/runs/35651936985/job/106506431543)
and [Windows arm64 job](https://github.com/synveda/synveda/actions/runs/35651936985/job/106506431555)
passed strict Clippy, Rust path/credential/receipt/spool/protocol checks,
mock-gateway process refresh and all five Rust/Node private-state checks.
Those checks prove shared installation identity/disclosure, format-preserving
spool rewrites, stale mutation/retirement refusal, owed-close retention,
hardlink/ACL/size/junction refusal and confined receipt access. They do not
establish real issuer or native harness execution.
The [full CI run](https://github.com/synveda/synveda/actions/runs/35651936985)
also passed for this clean source.

Both jobs also passed all seven exact-archive checks with private Node 24.21.0.
These cover native identity, client-only inventory, PowerShell install/reinstall under a
restricted PATH, all three hook entry points, retained state, Rust/Node storage
interoperability, duplicate checksums, changed launchers, interrupted locks,
malformed ZIPs and overlapping roots. Paths contain spaces, apostrophes and
Unicode. The downloaded x64 ZIP's 67 content hashes, source identity, runtime pin
and CLI/Node PE architecture match its report.

| Target | Archive bytes | Local install | Reinstall |
|---|---:|---:|---:|
| Windows x86_64 | 45,597,184 | 9.220s | 6.564s |

The x64 ZIP SHA-256 is
`244ed4dc0a2d94023518207e7d4d48acdb4c5a59580f3525a8f9ff927aa184e7`;
its report SHA-256 is
`84340e9a759f9958a60109ba0368fbca5ce2ab338b3baefbe89117228279a6ac`.
The arm64 job uploaded `synveda-client-report-windows-arm64.json`, its exact
`synveda-client-0.4.0-windows-arm64.zip` and native storage report in
[windows-state-arm64](https://github.com/synveda/synveda/actions/runs/35651936985/artifacts/10663822207).
GitHub records evidence-bundle SHA-256
`ada67a92d627c494e14d10ae642d3cc1bedda3e34b1376db7340f9986c118e5c`.
Its native assertions passed; the arm64 bundle was not downloaded for a second
local inspection. This bundle digest is distinct from its contained client ZIP
and report digests.

Timings use local candidate files; network download, real issuer login and
native vendor harness execution remain unqualified. Next: obtain the four Unix
hosted archive reports and artifact-based real issuer/harness evidence.
No new release or version is authorized.

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
  lock. Native Windows x64/arm64 credential, storage and installation evidence
  is recorded above; real issuer/harness acceptance remains open.
- The existing packager accepts `SYNVEDA_PACKAGE_CONSUMER_CANDIDATE=1` for local
  qualification. `package-consumer-compose.mjs` renders canonical fragments;
  `initialize-consumer.mjs` prepares named state without network/socket access,
  seals retained identity/secrets and projects only each service's existing
  authority. `CONSUMER.md` accompanies the candidate. Tests cover fresh render
  under spaces/Unicode, private projections, changed identity and unsafe input.
- The existing `qualify-release.mjs` now accepts `--consumer-candidate` and
  delegates lifecycle and paired-recovery checks to `qualify-consumer.mjs`.
  It verifies the original secret hashes and sample receipt after restoring
  into a different private project, plus archive corruption, immutable backup
  IDs, exact confirmation, recovery-in-progress and retained-target refusals.
- `synveda-recovery` addresses only consumer projects and the existing logical
  backup/restore commands. A separate named recovery volume retains the existing
  database/key formats and checksummed private configuration. All source
  profiles stop before dumping; a restore refuses retained target assets,
  rebinds only the explicitly confirmed project, reconverges database authority
  and checks the audit chain, tenant-key unwrap and wrong-key refusal before
  normal tenant convergence. It needs no source database/installation volume
  once the complete recovery set exists. Interrupted operations retain a marker
  that blocks ordinary initialization. The host-state launcher remains separate.
  `CONSUMER.md` owns the runnable ceremony and recovery limits.

No branch change, tag, owner account change, release publication or signing is
part of this checkpoint. Publication is blocked on an owner-owned
Docker Hub namespace/public repositories, expiring push credential, GitHub
variables/secret and protected environment, plus authorization of a new version
and release trigger. These do not block local implementation and tests.

Next task: execute the four Unix hosted candidate jobs and artifact-based
real issuer/harness acceptance, retaining their exact source/archive/report
identities. Windows x86_64/arm64 receipt/spool and PowerShell archive checks
passed in native CI as recorded above. Unported private-state operations still
refuse on non-Unix hosts. Native Windows/MSVC remains unavailable locally; the
macOS PowerShell runtime provides syntax checks only.
Keep Codex/Copilot registration and hook trust manual until their native installer
contracts have their own reviewed implementation and execution evidence. Installer
attestation enforcement now follows ADR-0132 in the source bootstrap; its
next-release platform qualification and Homebrew/WinGet remain open.

The FND-1 / OPS-8 pipeline refactor now shares all six native archive jobs between
CI and Release and executes the credential-refresh/platform tests against each
installed binary. Its local macOS ARM64 debug archive passed restricted-PATH
install/reinstall, Codex/Copilot replay and seven auth/platform process tests;
the dirty source report is not publication evidence. Exact OCI candidates must
pass native Compose and four-mode Helm qualification before copying to public
registries. The first hosted PR CI and nonpublishing Release runs on `34355d7`
passed macOS/Windows client packaging and the ARM64 Docker/Helm candidate.
Linux x64/ARM64 packaging refused Cargo's hard-linked executable; an isolated
copy now retains the packager's single-link rule. AMD64 Docker failed when the
bundled Keycloak realm convergence service became unhealthy; the fixture now
captures bounded, redacted diagnostics before cleanup. CI Result rejected the
run. Next action: rerun PR CI and nonpublishing Release for the follow-up
commit, inspect the AMD64 diagnostic if it recurs, and retain all six client
and both candidate reports. Local retained evaluation volumes remain untouched.

The second hosted attempt on `3a4da43` passed Linux archive packaging but
found that the restricted installer test PATH omitted GNU tar's `gzip` helper.
That helper is now allowed; both Linux targets passed the third PR run on
`4c91fcc`. AMD64 Compose again reached an unhealthy realm gate after about ten
minutes, though generation capture and the Keycloak management network probe
both passed at failure. The evaluation healthcheck had exhausted its retries
around the old 600-second Compose wait. Evaluation, plain Compose and recovery
now use a 900-second wait with a matching health start period, without relaxing
the generation gate. Failure diagnostics fall back to content-free process
names when Docker rejects `top` formatting. The final hosted [PR CI run](https://github.com/synveda/synveda/actions/runs/35844879556)
on `47fb126` passed CI Result, all six native packages and both Docker/Helm
candidates. The [nonpublishing Release run](https://github.com/synveda/synveda/actions/runs/35845344195)
on that exact clean source commit passed the same required native qualification.
Both candidate reports record seven launcher Compose checks, 20 direct-consumer
and recovery checks, and four independent Helm modes per architecture. The
dry-run asset bundle has 21 checksum-verified payloads plus `SHA256SUMS`; its
registry inventory marks the outputs unpublished. No tag or public release
was created. The existing v0.4.0 release cannot gain these native client-only
archives through this dry run.

The owner configured both Docker Hub variables and the protected release
environment secret. `CI Result` is now required on main; a `v*` ruleset blocks
updates and deletions. A separate creation-only `v*` ruleset with administrator
release-operator bypass was verified on 2026-09-24 under
[CI](../CI.md#manual-owner-settings). The owner authorized
the v0.4.1 follow-up release.
Local 0.4.1 version parity, chart lint, generated contracts, SDKs, TypeScript,
website, formatting, strict Clippy and focused native CLI tests passed. The
temporary worktree's root group and sandbox loopback permission initially
blocked two fixtures; after correcting those environment inputs, the complete
`make check-deploy chart-lint` gate passed, including all 171 Compose contract
tests. The exact merged source `d74e75b8991f22d8f4dd07034b3cd91db1ffb867`
then passed [full main CI](https://github.com/synveda/synveda/actions/runs/35884349132)
and the [nonpublishing Release drill](https://github.com/synveda/synveda/actions/runs/35884456435),
including all six native CLI packages and both Docker/Helm candidates. The
immutable `v0.4.1` tag triggered a [Release run](https://github.com/synveda/synveda/actions/runs/35900269117).
Both native published-image jobs failed in the anonymous pull step; the linked
ARM64 job reported `product: local image lost its release digest`. The final
publish job was skipped.
The verifier compared the bundle's `docker.io/<namespace>/product@sha256:...`
with Docker Engine's familiar `<namespace>/product@sha256:...` RepoDigest.
The two-registry fixture now reproduces that exact failure and accepts only
the same repository/digest in either Hub spelling. A local native ARM64 pull of
the public v0.4.1 proxy index on Docker 29.4.0 independently returned
`synveda/proxy@sha256:...` in `RepoDigests` for a `docker.io/synveda/proxy@sha256:...`
request. This source fix cannot
change the immutable tag's workflow code. The image candidates and assembled
assets remain in that run, but no signed inventory or public GitHub Release
was produced. Do not rerun the tag into write-once image coordinates.

The v0.4.2 source `d7441eb4a403b7cee2122c249c6bd931edb1e672` passed
[full main CI](https://github.com/synveda/synveda/actions/runs/35976416487)
and the [nonpublishing Release drill](https://github.com/synveda/synveda/actions/runs/35976499619).
Its [tagged run](https://github.com/synveda/synveda/actions/runs/35987467297)
published both registries' exact candidate copies and the OCI chart. Both
anonymous native pulls and executable checks passed, resolving the v0.4.1
RepoDigest failure. The public verification jobs then repeated both full
Compose drills and four Helm modes serially. ARM64 completed all those checks
but was cancelled before uploading its report at the 90-minute limit. AMD64
completed the reference Compose drill and was in the plain-Compose recovery
drill when its limit cancelled it. Final attestation and GitHub Release were
skipped. Preserve the v0.4.2 tag, image coordinates and chart; no new signed
public asset set was produced.

The v0.4.3 source includes the Docker Hub RepoDigest verifier fix, keeps full
native candidate Compose/Helm qualification and retains those source-bound
reports in the signed asset set. The final jobs checked both registries
anonymously and compared the OCI chart without repeating the slow deployment
drills. Source `2acc66f02625727b2ccdfe223358468bf10eef85` passed
[main CI](https://github.com/synveda/synveda/actions/runs/36048573688) and the
[nonpublishing Release drill](https://github.com/synveda/synveda/actions/runs/36048616477).
The [tagged run](https://github.com/synveda/synveda/actions/runs/36062182772)
passed six native client jobs, both native Docker/Helm candidates, both registry
copies, assembly and both anonymous public verifiers. Its final job attested
and uploaded all 35 assets, then failed because `/releases/tags/v0.4.3`
returned 404 for its draft. The exact draft `396162642` was verified by signer,
source, names and all server-computed digests and promoted by ID. The
[public v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
and checksum inventory were downloaded anonymously. The workflow remains
failed at that post-upload lookup; [PR #63](https://github.com/synveda/synveda/pull/63)
repairs future draft publication. Preserve both earlier failed tags and their
existing image/chart coordinates.
For another local candidate, use the
existing `package-release.sh` arguments with the explicit candidate flag, extract
the archive, then run `node scripts/qualify-release.mjs --consumer-candidate
EXTRACTED_BUNDLE REPORT.json`. It creates an absent random acceptance project,
leaves source/restore databases, installation state and the paired recovery set
retained. Release CI runs the same gate on both native architectures and requires
its checksummed reports. The first hosted dry run stopped before consumer
qualification because AMD64 Docker failed; the final hosted run completed it
on both native architectures. Next OPS-12 work is artifact-based real
issuer/harness acceptance, installer attestation qualification, OS
signing/notarization and a supported platform/version policy. Future releases
still require exact-source main CI, owner settings and version coordination.
Do not reuse an unrelated deployment or replace published artifacts.

### Publisher verification source increment (2026-09-30)

[ADR-0132](../adr/adr-0132-verify-publisher-before-installing-release-code.md)
requires the Unix/Windows source installers to authenticate `SHA256SUMS` before
fetching or executing release code. Remote assets require a trusted GitHub CLI
and an expected source commit; repository, release workflow, canonical tag,
OIDC issuer, SLSA predicate and GitHub-hosted runner policy are fixed. HTTPS
mirrors retain that identity policy. Missing proof, an unavailable verifier or
failed verification stops the install. Inventory/bundle size and verification
time are bounded. Local unsigned candidates are explicitly labelled and have
no publisher claim. Future release notes render the exact commit and fetch the
inspected bootstrap script by commit. Published v0.4.3 scripts are unchanged.

Forty focused installer/release tests pass, including strict verifier argv,
proof-before-code ordering, missing/oversized proof, wrong input policy,
deadline termination and preservation of the existing client on rejection.
These invocation stubs do not prove cryptography. A separate real GitHub CLI
2.100.0 run on macOS arm64 verified the anonymously downloaded v0.4.3 inventory
for source `2acc66f02625727b2ccdfe223358468bf10eef85`, rejected a wrong tag,
wrong source and a corrupted inventory, then the source bootstrap installed
the published macOS arm64 client through both a signed local directory and
HTTPS. The client archive SHA-256 was
`d7af41276ba43cea299e318ade0a01a0f058606fde5b8498e2b7708274a513e0`;
the installed CLI reported `synveda 0.4.3`. A second remote attempt with the
wrong source failed and retained the installed manifest. All proof directories
were invocation-owned and removed; no deployment, account, tag or release
was created or changed.

The Windows candidate gate now tests strict verifier arguments and rejection
before code execution through a fixture executable, and release assembly
requires its `publisher-policy-and-pre-execution-refusal` report check.
Native Windows/MSVC is unavailable locally; the hosted source qualification
below now records that fixture passing on native x64/ARM64 and all six archive
jobs. This does not establish real Windows issuer or signed-install acceptance.
Bootstrap inspection,
complete archive/chart SBOMs, vulnerability remediation/exception policy,
publisher incident/revocation procedure, OS signing and published real
issuer/harness acceptance remain open. GitHub CLI may refresh its trusted roots;
no air-gap qualification is claimed.

Validation also passed `make check-fast check-ci`, `make chart-lint`, shell
syntax, Rust formatting and diff checks. Every `make check-deploy` component
passed: its initial aggregate stopped when the sandbox denied the evaluation
fixture's localhost bind; all 44 deployment-convergence tests passed when
rerun with socket access, and the remaining uninstall/convergence/chart checks
completed. No live Docker deployment was part of this validation.

### Licence and notice carriage source increment (2026-09-30)

PR-13's existing Apache-2.0 distribution contract now has explicit carriage
checks. Native server and console archives, the staged Helm chart and every
independently copied plugin/private shared package retain the exact root
`LICENSE` and `NOTICE`; private package metadata retains Apache-2.0. The
reference bundle already carried these files. All six Synveda OCI targets copy
them readably to `/usr/share/licenses/synveda/`. Existing Node and upstream
notices remain in place; this is first-party carriage, not a complete review
of third-party obligations or a complete SBOM.

Native client checks compare every packaged notice with source bytes and emit
`licence-and-notice-carriage`, required from all six archive reports. Assembly
checks the six other TAR/chart archives for one regular, exact-byte file per
notice, refusing omissions, changes, links and duplicates. Candidate and public
image smoke checks compare both notice hashes inside isolated containers;
final qualification requires `notice_sha256` evidence for every first-party
image and registry. Upstream Collector/Prometheus images retain their separate
contracts. Chart parity checks the actual package and repeated-package bytes.
The client installer retains its existing inventory contract so valid earlier
installed clients can still be preserved and replaced.

Forty-three focused packaging/release tests passed with no skips. Extracted
plugin notices and metadata passed, along with four configuration, eight Codex
and 23 Copilot replay tests. A dirty-source macOS arm64 client candidate from
parent `2616401bf855fd74175fa3ca41c34805830ee942` used pinned Node 24.21.0,
reported CLI `synveda 0.4.3` and passed the native notice check, install/reinstall,
seven installed-CLI process/authentication tests and both adapter replays. Its
archive had SHA-256
`64f5e365ad57c3e8be0d8ac50937d2ec359e8895f09ac24c2e4310990e935116`
and 54,355,007 bytes; local install/reinstall took 2,082/2,118 ms. This was a
local candidate with no real issuer or native vendor loading, not a released
artifact or a network-install measurement. Its owned temporary files were
removed.

`make check-fast check-ci check-release-parity chart-lint`, actionlint 1.7.7
(`-shellcheck=`), formatting, shell syntax and diff checks passed. Chart checks
packaged the real source chart and compared notices byte for byte. The cached
Keycloak image has the existing SHA-256 utility used by the new verifier; no
new image build or public-image notice acceptance is inferred from that probe.
All `make check-deploy` components also passed. The aggregate first stopped at
the reviewed Keycloak Dockerfile hash after its one-line notice-copy addition;
that exact fingerprint was reviewed and renewed. The static Compose matrix,
review-lock refusal test and remaining 44 convergence/five uninstall tests then
passed. The 173 lifecycle/entrypoint tests had already passed. This validation
did not run a live application deployment or rebuild the six OCI candidates.

The hosted checkpoint below qualifies notice carriage on all six native clients
and both Linux image architectures from one clean source and release run.
Complete third-party notice/SBOM review, OS signing, publisher incident policy,
real issuer/harness acceptance and supported platform/version lifecycle remain
open. Earlier published artifacts and retained deployment state are unchanged.

### Hosted source qualification (2026-09-30)

Clean source `f433eb1719bc58b5f36b0516bedf71e886d23185` on
`codex/synveda-production-roadmap` passed all 23 jobs in
[full CI](https://github.com/synveda/synveda/actions/runs/36765529532), including
workspace Rust, all six native client targets, both native four-mode image/Helm
matrices, both dedicated operations cases and CNPG install/failover.
The same-source [nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36765540733)
completed successfully with fifteen jobs passed and final publication skipped.
Both native Linux architectures passed full launcher and plain-Compose consumer
recovery, all four ownership modes, both day-two operations cases and local
browser acceptance. Both workflows identify the exact source at attempt 1.
[Retained evidence](../../demos/evidence/ops12-source-qualification.json)
records independent verification of the original assembled checksum inventory
and all 31 payloads, six native reports and both complete deployment report sets.
The five ARM producer-report hashes also match their assembled payloads.
Earlier failed or cancelled runs do not supply reports for this candidate.

Required native evidence binds the archive, source, target and private Node pin,
installed authentication/process behavior and exact first-party notices.
Windows exercises the actual cleanup helper with native sharing code 32 and a
synchronously acquired exclusive data-file handle released by a .NET thread
after two seconds. Persistent sharing-lock exhaustion, immediate non-sharing
refusal and installation-state retention remain required. Production cleanup
accepts only sharing/lock codes 32 and 33, with twenty attempts and nineteen
250-ms pauses; installation locks are never automatically cleared.

OCI evidence requires searchable notice directories, exact source notice hashes
and isolated native runtime probes for all six images. Helm operations require
successful migrations 1, 2 and 3 and complete ledger equality before/after both
lock-contending reruns. During database outage, gateway-loopback readiness must
be 503 and liveness 200; the public flow must recover afterward. Retained
reinstall requires direct gateway readiness, public console and issuer discovery
to return 200 within 60 seconds before login, isolation, content and audit
assertions run once. Only named transient transport errors and HTTP 502/503/504
retry; TLS and other HTTP failures are immediate. Release reports bind all three
statuses, elapsed time and failed probe count. The local-evaluation case requires
the exact runtime-role epoch-preflight refusal, correct chart reapply and browser
login/logout under restricted admission and assigned identities.

A separate invocation-owned Kind probe observed a direct request timeout after
Service replacement and routing recovery at 11,782 ms. Proxy restart also
produced a connection refusal after rollout completion. This proves rollout
state alone is insufficient reachability evidence, not a product authentication
failure or a production latency bound. Its cluster/scratch were removed; retained
OPS-7 state was untouched. Targeted real HTTP probes passed temporary 503/socket
recovery, immediate 403 refusal and credential-free causal diagnostics.

Local validation passed 28 focused client/release/image-reuse tests, all 77
release-parity tests, fast/workflow gates, the six-mode starter contract,
JavaScript syntax and Rust/diff formatting. No Rust, schema or generated
contract changed in the final fixture correction. Independent verification
rehashed the original 31-file inventory before invoking repository validators,
then required the regenerated inventory to remain byte-identical. It checked
exact source/platform/image bindings, notice hashes, all seven launcher and
twenty consumer checks, migration-ledger retention and bounded route recovery.
Both local-evaluation reports passed restricted admission, assigned UID,
runtime-role preflight refusal, correct reapply and browser login/logout.

This source is qualified at `publish=false`, version assertion 0.4.3. Registry
copying and anonymous public-pull steps did not run; the final publisher was
skipped. No tag, public release or registry publication occurred. Native reports
explicitly leave real issuer login and harness use unqualified. Same-host logical
recovery does not establish encrypted off-host PITR; one-node Kubernetes,
restricted admission and an assigned UID do not establish HA or OpenShift SCC
support. The readiness verdict remains Not ready and OPS-11/OPS-12 stay open.
The next source slice addresses Rust SBOM content, followed by the remaining
release-security, published-client and support-policy gates.

### Published SBOM coverage inspection (2026-09-30)

[Read-only evidence](../../demos/evidence/ops12-published-sbom.json) binds
this inspection to the published v0.4.3 source, signed registry inventory and
immutable GHCR product digest. GitHub CLI verified the checksum attestation
under ADR-0132's fixed publisher policy; the registry manifest matched its
unique signed checksum before the image reference was used. No image was run
or rebuilt. Each architecture's SPDX 2.3 document contains 108 packages, zero
Cargo package URLs and no packages named Cedar or SQLx. The records identify
Syft 1.51.0 and BuildKit 0.32.2 and retain hashes of the rendered document bytes.
This is an inventory-coverage finding, not a vulnerability assessment.

[Docker documents](https://docs.docker.com/build/metadata/attestations/sbom/)
that default BuildKit scanning covers the final stage. The product currently
copies plain compiled Rust binaries into that stage without dependency
metadata. This explains the missing Rust package inventory as an inference
from the build recipe and inspected output. Image attestation descriptor parity
therefore does not prove complete dependency coverage. The next release-security
slice must choose a maintained, pinned dependency-inventory mechanism and
require expected runtime dependencies in the SBOM, then extend coverage across
native archives, bundled JavaScript, charts and third-party notices. A valid
SPDX envelope alone must not pass that gate. Record the architecture decision
before implementation; current image and archive coverage claims remain open.

### Rust-bearing OCI inventory source increment (2026-09-30)

[ADR-0133](../adr/adr-0133-embed-rust-dependency-inventory-in-release-binaries.md)
is Accepted for the first OCI slice. The product Dockerfile builds the dependency
cache and real CLI/gateway/core-worker/Apalis executables with locked
`cargo-auditable` 0.7.6. Product and browser final-stage scans use the immutable
BuildKit Syft 1.12.0 generator (Syft 1.51.0). OCI exports have fixed loopback image
names so their in-toto statements contain real native manifest subjects; no
image is pushed by naming an OCI export.

`docker-candidate.mjs` reads actual exported OCI JSON blobs, with regular-member,
size and SHA-256 checks. It requires one native platform, exact source/version
configuration, one matching attestation, a digest-bearing SPDX statement and
locked Synveda/Cedar/SQLx package content. Missing/wrong-version inventory,
foreign subjects, changed blobs, links and duplicates fail. The report retains
native/attestation/statement hashes, Cargo entry/identity counts and exact required
versions. Candidate publication and assembled/qualified release inventories
require these reports; registry copying retains the original descriptors.
This gate covers the two Rust-bearing images, not complete SBOM coverage.

The [local native ARM evidence](../../demos/evidence/ops12-oci-rust-sbom.json)
records actual final-stage exports: product 1,606 Cargo entries representing 351
distinct name/version pairs, browser 295 entries/identities. Both include the
expected roots, Cedar 4.11.2 and SQLx 0.8.6. Independent archive hashes and
attestation subjects pass. A named ordinary Rust OCI control has valid SPDX but
fails missing runtime content; a nameless control fails empty subjects. These
are dirty-checkout source candidates from parent `686b75b8`, not clean hosted
release evidence. No executable/deployment, real client, public registry or
production-readiness claim follows from this inspection.

For an independent source-checkout inspection, use the OCI archive from its
native producer and the exact version/source inputs:

```sh
node scripts/rust-image-sbom.mjs IMAGE.tar arm64 product VERSION SOURCE_SHA
```

Six focused OCI tests and release/CI integration refusals pass, including
percent-encoded Cargo versions. Fast/workflow gates, actionlint 1.7.7, release
parity, chart/starter/portability checks and all deployment-check components pass.
The aggregate deployment check initially hit the sandbox's denied loopback bind;
all 44 convergence tests passed with socket access, followed by the remaining
uninstall/convergence/chart components. Rust formatting and diff checks pass;
no Rust source, lockfile, product SQL or generated API contract changed.

Clean OCI source `3beeb1cbf88c492265e677e253b833c6b08609e3` passed all 23 jobs in
[full CI](https://github.com/synveda/synveda/actions/runs/36787337125) and its
[nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36787341370)
(15 jobs passed; final publication skipped). [Retained qualification](../../demos/evidence/ops12-oci-source-qualification.json)
hashes the original 31-payload inventory before the source-specific validator;
its regenerated checksum inventory is byte-identical. All six ordinary native
client archives, both image report sets and complete Compose/Helm evidence pass.
All five producer reports per architecture match their assembled bytes. The
product images report 354/351 Cargo identities on AMD64/ARM64; browser images
report 297/295, including expected roots, Cedar 4.11.2 and SQLx 0.8.6. Native
producers inspected exported OCI bytes and embedded SPDX content; independent
retention revalidated report bindings and content summaries without downloading
the large exports again. No earlier run supplies evidence. No image/chart copy,
anonymous public pull, tag or release publication ran. This qualifies the OCI
source slice only; native archive SBOM qualification follows on its own source,
then non-Rust dependency coverage, notices and vulnerability/incident policy.
OPS-12 and the production verdict remain open.

The next native archive slice is specified in ADR-0133 before implementation.
It requires a separate SPDX document for each final Rust executable and a
hash-bound producer report per archive, with the reviewed Syft 1.51.0 native
download for each of the six targets. Release assembly must read the actual
sidecar content and rehash its archive; a valid envelope or another executable's
metadata cannot supply the required CLI/gateway/worker inventory. Tools remain
outside the installed client. Current OCI runs remain tied to clean source
`3beeb1cb` and do not qualify the following native source slice.

### Native archive Rust inventory source increment (2026-10-01)

All six native client builds and both historical server builds now use locked
`cargo-auditable` 0.7.6 in private runner storage with a distinct cache key.
Server stripping failures are fatal. The native Syft 1.51.0 downloader checks
reviewed archive and executable SHA-256 pins before execution on its own host.
Each final CLI/gateway/worker archive member must be unique and regular with
the expected ELF/Mach-O/PE architecture. Separate SPDX documents require the
single scanned binary's SHA-256, expected Cargo root and locked Cedar/SQLx.
One binary's metadata cannot satisfy another's gate. No scanner or build tool
is added to client packages or installers.

Assembly requires eight clean source/target-bound archive reports and all twelve
actual SPDX documents, rehashes their archive/document bytes and reads Cargo
content before writing the inventory. All 20 sidecars are required producer,
assembly and final publisher outputs. The assembled inventory now has 51
payloads; final public qualification has 53 payloads plus the checksum inventory
and publisher bundle. The existing exact-source OCI/deployment, native client
execution and publisher-verification boundaries remain required.

[Local macOS ARM evidence](../../demos/evidence/ops12-native-rust-sbom.json)
binds its dirty-checkout probe to parent `98e7f62b` and tool/input hashes. The
actual client contains 295 Cargo identities; successfully stripped server
CLI/gateway/worker contain 295/327/327, each with Cedar 4.11.2 and SQLx 0.8.6.
Actual archive/SPDX hashes and per-binary checks pass. Private client
install/reinstall, packaged authentication and extracted Codex/Copilot replay
pass. The clean-source assembly gate refuses these local reports as intended.
Nine focused tests cover all six platform headers, regular/unique member
extraction, missing/misversioned/foreign-binary content, changed source/pins/
documents and pre-execution scanner download refusal. CI/workflow gates and
release parity pass. No Rust source, lockfile, SQL or generated API changed.
The complete deployment aggregate passes, including 173 Compose tests, all 44
convergence tests and chart/starter/portability gates. Actionlint 1.7.7, Rust
formatting and diff checks pass. No changed Rust crate requires an additional
local Clippy run; hosted workspace strict Clippy passed for the preceding OCI
source, with the native slice still awaiting its own clean qualification.

Next run full CI plus a nonpublishing release drill on this increment's clean
source. Independently hash the original expanded same-run
inventory and inspect all eight native report/document sets. Do not transplant
the preceding `3beeb1cb` native reports, whose ordinary binaries have no new
archive SBOM contract. Non-Rust dependency coverage, third-party notices,
vulnerability/incident policy and real issuer/harness use remain open. The
production verdict and feature state are unchanged.

Native implementation commit `c535d4f4` started CI
[`36792760622`](https://github.com/synveda/synveda/actions/runs/36792760622) and
queued nonpublishing Release
[`36792769482`](https://github.com/synveda/synveda/actions/runs/36792769482).
Both were deliberately cancelled after a copied-CRLF-lockfile test reproduced
the new reader's LF-only assumption; no report from either run qualifies the
corrected source. The reader now normalizes TOML line endings before selecting
the same locked package tables. All 26 CI-tooling tests pass, including the new
real-module CRLF case and existing content/source/hash refusals. The repository
lockfile and runtime requirements are unchanged.

Correction `668a3f96` started full CI
[`36793829307`](https://github.com/synveda/synveda/actions/runs/36793829307).
Both native Linux client/SBOM jobs passed. Independent ARM inspection rehashed
the actual archived CLI and its SPDX document: 295 Cargo identities, with the
required root, Cedar 4.11.2 and SQLx 0.8.6, matching its client-execution report.
The CNPG and macOS ARM jobs failed during crates.io transfers before their
changed gates. Windows x64 passed all 11 packaged-client checks, then refused
Syft 1.51.0's actual `\synveda.exe` filename; the validator had assumed a bare
POSIX-style name. The regression test reproduced that refusal before the fix.
It now requires the exact pinned Windows root-relative spelling, with nested,
drive-qualified, foreign and POSIX-style names refused. Binary hashes, Cargo
content and all POSIX filename checks remain required. All 27 CI-tooling tests
pass, including both Windows architecture fixtures and a foreign-hash refusal.

The queued same-source Release
[`36793836009`](https://github.com/synveda/synveda/actions/runs/36793836009) was
replaced by independent nonpublishing Release
[`36795598062`](https://github.com/synveda/synveda/actions/runs/36795598062) on
`codex/ops12-native-sbom-qualification`, also at `668a3f96`. These native runs are
superseded by the filename correction and cannot qualify its source. Next run
full CI and a nonpublishing drill on the correction commit, independently check
all 51 same-run payloads and eight archive/SPDX sets, and retain the preceding
`3beeb1cb` OCI drill separately. The temporary validation branch avoids serialized
drills without changing source or skipping gates; remove it after its retained
qualification. Non-Rust work follows this qualification.

The Windows filename correction is committed as
`aea91758eb7ce8fe20c7fff5525154138b4398ec`. Full CI
[`36796653523`](https://github.com/synveda/synveda/actions/runs/36796653523) and
nonpublishing Release
[`36796662979`](https://github.com/synveda/synveda/actions/runs/36796662979) are
bound to that exact source. The superseded `668a3f96` CI and independent Release
were deliberately cancelled; they supply no qualification for the correction.
Local CI tooling (27 tests), fast checks, actionlint, Rust formatting and diff
checks pass. This CI again failed during registry transfers: CNPG's builder
could not download `windows-link` for pinned cargo-auditable, and macOS ARM
could not fetch `bumpalo` before native CLI tests. Both reported HTTP/2 framing
errors, following the earlier same-job transfer failures on different packages.
Native build jobs and the product image builder now set
`CARGO_HTTP_MULTIPLEXING=false`; [Cargo's supported configuration](https://doc.rust-lang.org/cargo/reference/config.html#httpmultiplexing)
selects HTTP/1.1 while retaining TLS verification, locked dependencies and
existing checks. The setting is confined to these builds. No Rust crate,
runtime configuration or generated contract changed. The `aea91758` native runs
were deliberately cancelled after this correction; they cannot qualify it.

Transport correction
`1edb62557c7407cbf55aff22201fa0c04913ce89`: full CI
[`36798188473`](https://github.com/synveda/synveda/actions/runs/36798188473) and
independent nonpublishing Release
[`36798194065`](https://github.com/synveda/synveda/actions/runs/36798194065).
Both trigger SHAs were checked. All 27 CI-tooling tests, fast checks, actionlint,
Rust formatting and diff checks pass locally. Both Windows x64 jobs passed all
11 packaged-client checks, then correctly refused an SPDX file entry without
the required SHA-256 checksum. Native filename and required Cargo-content
checks were reached; binary binding remains unqualified. Both Windows ARM jobs
subsequently failed with the same missing SHA-256 checksum. CI completed with
20 successful jobs and the required result gate refusing the native failures.
The nonpublishing Release was deliberately cancelled after the correction was
committed. None of these reports qualify the correction. The preceding
`3beeb1cb` OCI qualification is retained separately and cannot supply native
archive SBOM evidence.

The [pinned scanner options](https://github.com/anchore/syft/blob/v1.51.0/cmd/syft/internal/options/file.go)
default to hashing package-owned file coordinates. The
[single-file indexer](https://github.com/anchore/syft/blob/v1.51.0/syft/internal/fileresolver/file_indexer.go)
indexes the source and its parent, and the
[SPDX encoder](https://github.com/anchore/syft/blob/v1.51.0/syft/format/common/spdxhelpers/to_format_model.go)
inserts a zero SHA-1 placeholder when file digests are absent. The correction
selects all file metadata within that single-file source, independently of
package ownership. It requires exactly the pinned synthetic directory root and
the expected executable. Only the root can contain its exact `OTHER` type and
zero SHA-1 placeholder; the executable still needs its actual, single SHA-256
checksum. Extra entries, foreign hashes, alternate root names/types and an
executable placeholder refuse. No SPDX bytes are rewritten or checksums supplied
by the validator.

[Local correction evidence](../../demos/evidence/ops12-native-file-digests-probe.json)
reuses the earlier dirty macOS ARM archive, reproduces both original-validator
refusals with actual pinned-scanner output, and runs the corrected production
producer on all three stripped CLI/gateway/worker members. Their binary hashes
and Cargo identities (295/327/327) are unchanged; each document binds its actual
member hash. Foreign hashes, executable placeholders and the dirty producer
report refuse. This control does not reproduce the Windows resolver defect on
macOS or qualify a clean source. Eleven focused archive tests cover both Windows
spellings and all six root contracts. All 29 CI-tooling and 77 release-parity
tests, fast checks, pinned actionlint, Rust formatting and diff checks pass
locally. No Rust source, dependency lock or generated contract changed.

File-metadata correction
`02023908ce95df5dc83991dbde6f9ed31b89e4cd`: full CI
[`36802763712`](https://github.com/synveda/synveda/actions/runs/36802763712) and
independent nonpublishing Release
[`36802769936`](https://github.com/synveda/synveda/actions/runs/36802769936).
Both dispatch trigger SHAs were independently checked. The Release branch
`codex/ops12-native-sbom-qualification` was advanced without force. These runs
are superseded by the root-spelling correction below and cannot qualify it.
The preceding OCI source is already retained separately; do not repeat or
transplant that qualification.

That source's Windows x64 CI job `110180571304` passed native Rust tests,
private-state checks and packaged-client execution, then refused the synthetic
root filename: Syft emitted an empty string, while the new Windows fixture
assumed `\`. The two-entry count and Cargo-content checks passed before this
refusal. Corrected fixtures reproduce that exact refusal before the validator
change, and also catch the formerly accepted backslash root. The validator now
requires the empty root on every target; the Windows executable still requires
exact `\synveda.exe` spelling and its actual, single SHA-256 digest. Root type,
placeholder isolation, extra-file, Cargo-content, scanner/source and publication
checks are unchanged. All 29 CI-tooling and 77 release-parity tests, fast checks,
Rust formatting, pinned actionlint and diff checks pass. Actual retained macOS
ARM CLI/gateway/worker archive and SPDX bytes pass the corrected validator,
while dirty-source refusal holds. No Rust source or generated contract changed.

The latest hosted native qualification source is root-spelling correction
`ef6d61dbf4183dcc39a02e549822cb8f76243d42`: full CI
[`36805252052`](https://github.com/synveda/synveda/actions/runs/36805252052) and
independent nonpublishing Release
[`36805260636`](https://github.com/synveda/synveda/actions/runs/36805260636).
Both dispatch trigger SHAs were independently checked. The obsolete
`02023908` Release is cancelled; CI cancellation was requested and its cleanup
was still completing at this checkpoint. Neither run supplies qualification for
the correction. The task-created Release branch was advanced without force;
remove it after retaining the corrected source's proof. Both Windows x64 jobs
(`110188331648` in CI and `110188115206` in Release) again correctly refused an
executable without SHA-256, despite full file-metadata selection. Native
execution, expected file spelling and Cargo-content checks passed before that
refusal. This source is not qualified. Both runs are now deliberately cancelled;
preserve their failures rather than retrying the unresolved scanner defect.

Assembly correction `31980069` follows review of a gap: coherent changes to a
producer's binary digest and SPDX file digest passed against an unchanged
archive. Assembly now re-extracts every required unique regular member and
checks its actual platform, size and SHA-256 before reading SPDX. Actual ZIP
fixtures cover both Windows targets; Linux verification uses libarchive rather
than GNU tar's unsupported ZIP reader. Twelve archive tests and all 30
CI-tooling / 77 release-parity tests pass, together with fast checks, Rust
formatting, pinned actionlint and diff checks. The downloaded same-run Linux ARM
client passes the stricter archive and client-execution checks: 295 Cargo
identities, correct CLI/Cedar/SQLx roots and its actual archived executable hash.
That partial producer inspection is not full qualification or evidence for a
future source.

[Retained blocker evidence](../../demos/evidence/ops12-native-windows-scanner-blocker.json)
binds both Windows x64 failures, the official upstream source comparison and
upstream Windows/Linux unit results. [Syft issue 5325](https://github.com/anchore/syft/issues/5325)
and [merged fix 5341](https://github.com/anchore/syft/pull/5341) identify the cause:
the file tree has POSIX volume-encoded keys, but native Windows path lookup
searched it before conversion. The digest cataloger silently found no location.
The exact merged upstream source is
`6ac7afb439c950ec2fc169103c4850c6dfe74c5a`; its Windows unit job passed. The
latest released scanner, 1.52.0, still has the same faulty resolver bytes as
pinned 1.51.0. No fixed release or prerelease was available at this checkpoint.
The upstream change also normalizes reported paths, so a new scanner needs
actual native filename/digest probes before changing the closed SPDX contract.
Do not manufacture SPDX checksums, remove Windows sidecars, widen accepted
paths or borrow POSIX evidence.

Native qualification is blocked on a fixed scanner distribution. The owner's
subsequent `continue` resumes the recommended independent non-Rust inventory/
notices slice while retaining this blocker. Official latest release remains
1.52.0 at the 2026-10-01 recheck. No custom upstream build is selected.
No new qualification source/run has been selected. The task-created validation
branch remains at failed `ef6d61db` until a corrected source is ready; no run
should resume from it. When a fixed scanner is released, inspect the selected
tool's immutable source/distribution and native path/digest behavior, preserve
all gates, then dispatch full CI and a nonpublishing Release on one clean
corrected source and record their exact IDs.
Before retaining qualification, independently verify its original 51-payload
inventory, six client reports, eight archive reports, twelve SPDX documents and
both complete OCI/Compose/Helm report sets. No earlier report can qualify that
source. The active independent slice is the console contract in ADR-0134;
Node/native libraries, other artifacts and security policy follow it.
Production readiness remains unclaimed.

### Next non-Rust mechanism probe (2026-10-01)

[Local evidence](../../demos/evidence/ops12-console-sbom-probe.json) binds an
in-memory Vite comparison to unchanged console/lockfile/font inputs from
`668a3f96`. Both builds reproduced the existing console JavaScript chunk hash.
Pinned Syft's JavaScript package scan found zero npm identities in the built
directory. Independent rendered-module inspection identified React 19.2.8,
React DOM 19.2.8 and scheduler 0.27.0. The isolated `rollup-plugin-sbom` 3.2.2
probe produced the same three identities, their dependency edges and complete
MIT licence bytes matching the installed packages' hashes.

[Syft's catalogue](https://oss.anchore.com/docs/capabilities/javascript/)
documents its package/lockfile evidence; [Rollup's output hook](https://rollupjs.org/plugin-development/#generatebundle)
exposes emitted chunks and module information. The
[upstream plugin](https://github.com/janbiasi/rollup-plugin-sbom/tree/v3.2.2)
is a candidate build-time reuse boundary. No project dependency or production
pin was added. Review the maintained release, helper attribution, complete
notice carriage and artifact/source/hash binding, then record the decision
before implementation. The probe's optional bundler tool auto-registration
warned because its disposable installation had no peer packages; component and
licence collection passed. Generated helpers, fonts, Node/native libraries,
charts and upstream image contents remain separate coverage requirements.

### Console dependency inventory source increment (2026-10-01)

The owner's continuation resumes the independent non-Rust slice while the native
Rust qualification remains blocked. ADR-0134 was recorded before implementation.
Exact build-only `rollup-plugin-sbom` 4.0.0 emits CycloneDX 1.6 and complete
package licence evidence in the existing Vite build. Independent positive
rendered-module checks require locked React 19.2.8, React DOM 19.2.8 and scheduler
0.27.0; omitted, duplicated, misversioned or foreign content refuses. Known
module-preload/CommonJS helpers carry the complete reviewed Vite notice, and the
emitted font/OFL must match source. Three named build-only licence exceptions are
recorded in ADR-0134; the shipped allowlist is unchanged.

The console carries its SBOM, complete notices and a source/input/hash-bound
inventory inside the existing archive and product-image directory. Release
preparation and assembly inspect the actual TAR, reject duplicate/linked/unsafe
members and rehash every output before checksum creation. Normal local builds
without a release source remain refused. No release payload is added; the
original expanded native source inventory still requires all 51 same-run payloads.

[Local source evidence](../../demos/evidence/ops12-console-inventory-candidate.json)
records actual macOS ARM and native Linux ARM Node 22.23.2 builds from the dirty
checkout at parent `2edfb4f8`. Both actual archives pass all content/source checks,
and all 12 built files match byte-for-byte across builders. JavaScript remains
491,865 bytes with SHA-256 `2b1f4f1030f5556068d0761916b2ad96644674d0784f1040ecac18e1983b4d7d`.
Eight focused refusal tests, all 38 CI-tooling and 77 release-parity tests,
258 console tests, the frozen build and npm licence gates pass. Formatting,
pinned actionlint and fast/diff checks pass. Deployment checks and strict chart
lint pass, including all 44 convergence and five uninstall tests. One convergence
fixture initially hit the sandbox's loopback socket denial; its unchanged rerun
with socket access passed. The optional unused Rolldown auto-registration warns;
all required tool metadata is checked. No Rust, SQL or public generated contract
changed. This is a source candidate, not a clean hosted/full release qualification.

The product-image console follow-up below owns the next action. Preserve the
native scanner blocker and prior OCI proof separately; inspect a fixed upstream
release before any new native qualification. Do not dispatch or resume failed
`ef6d61db` runs, remove sidecars or relax gates. No tag, main merge, public release
or registry publication is authorized.

### Product-image console content source increment (2026-10-01)

ADR-0134 was amended before implementation. The existing native image verifier
creates a stopped invocation-owned container from the inspected immutable
product image ID and reads its fixed console directory through Docker's bounded
TAR stream. The shared archive reader refuses duplicate, linked, foreign,
unsafe and oversized members; the same source/file/SBOM/full-notice validator
checks the actual bytes. No candidate code runs during that inspection. Failure
paths attempt removal of only the owned container and temporary stream,
preserve the first cause and cannot retain passing evidence after a cleanup failure.

Archive and image reports now carry the inventory SHA-256, which binds every
output file. Assembly requires equal console evidence in the standalone archive
and both native local-image reports; published qualification also requires it
for each registry and architecture. No sidecar is added and all 51 same-run
payloads remain required. Earlier OCI or native reports cannot qualify this
changed source.

[Local actual-image evidence](../../demos/evidence/ops12-console-image-candidate.json)
binds a full native Linux ARM product build to dirty parent
`334369effb0288e0e7046b9919dbc35052cc1606`, its source inputs, validator hashes
and inspected image ID. All 12 console files match the independent macOS ARM
archive; inventory SHA-256 is
`ff566eda988b3612e009e33b923b6dc6d7bf57e49e3a692c0eb888f4db053788`.
An actual image with altered HTML refuses the inventory check. A second control
updates HTML and its inventory coherently, retaining the same source/SBOM/notices:
internal inspection passes, but the production archive-binding validator refuses
it. Both control images and all inspection containers were removed; the task-owned
baseline image remains available for the next inventory probe. No retained
deployment was changed.

All 38 CI-tooling and 81 release-parity tests pass, including stopped-container,
actual TAR/content/source, resource-bound, cleanup and both-registry refusals.
All 258 console tests, deployment checks and strict chart lint pass, along with
Rust formatting, pinned actionlint and fast/diff checks. A sandboxed pnpm launcher
first failed registry signature verification because its registry fetches were
denied, before any tests started; the unchanged rerun with network access passed.
No signature or test gate was disabled. The local evidence is
console carriage only, not clean hosted/native/OCI or deployment qualification.
No Rust, SQL, public generated contract, runtime authority or publication gate
was changed.

Next probe actual bundled Node dependencies and complete upstream notices in the
product and six private client runtimes, and identify native C-library gaps left
by Cargo inventory. Review a maintained mechanism and record the architecture
choice before implementing its artifact/source/target/hash checks. Plugin/chart
and remaining upstream-image inventories/notices follow, then vulnerability and
publisher incident policy. The fixed Syft Windows distribution, operator S3
bucket and real issuer/harness credentials remain external blockers. Do not
resume failed `ef6d61db` runs, combine old proof with this source, weaken gates or
publish. Remove the temporary native qualification branch only after corrected
native proof is independently retained.

### Bundled Node mechanism probe (2026-10-01)

[Retained probe](../../demos/evidence/ops12-node-runtime-probe.json) uses clean
source `19ca39982b45d6ad64a1a64b43230f5fa9166d65`. It independently downloads and
checks all six already-pinned Node 24.21.0 distributions against both repository
pins and official checksums, then reads only their fixed unique regular
executables and complete licences. All executable headers match their target.
POSIX licence bytes match upstream source; Windows carries the same complete
text with exact CRLF bytes. No foreign executable is run.

Both official Linux Node 22.23.2 distributions are also checksummed and inspected.
The preceding task-owned native ARM product image from dirty parent `334369ef`
contains byte-identical upstream Node and complete licence files. Its source
label does not qualify the current probe source. The stopped inspection container
and isolated execution container were removed; no retained deployment changed.

The independently hash-verified Syft 1.51.0 binary classifier identifies Node
on POSIX but no embedded library versions. The original two Windows `node.exe`
filenames yield no Node package identity. Native macOS ARM Node 24.21.0 and the
actual isolated Linux ARM product runtime expose public dependency versions and
shared-library flags through Node's maintained API. ABI/data fields and empty
optional versions must not become software package claims. The upstream licence
builder includes runtime and build-tool notices, so its aggregate cannot itself
prove which packages ship. Both aggregate licences omit `deps/nbytes/LICENSE`;
that exact MIT notice is identical across the two upstream source commits.
SQLite's embedded upstream source disclaims copyright. Other unreported helpers
and transitive native components remain explicit coverage gaps.

Selected normal/build Cargo trees show non-vendored `openssl-sys` on Linux.
Its Rust binding version is not the C OpenSSL version. `libsqlite3-sys` appears
in the lockfile but is absent from the selected release dependency tree; do not
report it as shipped. These source observations are not platform execution or
complete C-library qualification.

ADR-0135 records the architecture choice before implementation: reuse Node's
built-in metadata, existing client manifest/native reports and image reports.
Next add reviewed executable/full-licence pins, carry the supplementary upstream
notices, check actual native metadata and independently inspect final archived
and stopped-image bytes. Keep the implementation inside the existing packaging
and verification path; no new scanner, service, package manager, formatter or
release sidecar is selected. Record runnable refusals and actual local acceptance
before committing the source increment. Then continue the remaining native,
plugin/chart/upstream-image coverage and security policy in the documented order.

This probe does not qualify six native client executions or a clean hosted
release. The fixed Syft Windows distribution, operator S3 bucket and real
issuer/harness credentials remain separate external blockers. No new native
qualification source/run is selected; never resume cancelled `ef6d61db` runs,
combine preceding OCI proof with this source, weaken gates or publish.

Independent retained-evidence checks rehash all eight downloaded distributions,
their actual executable/licence files and the reviewed upstream source files.
`make check-fast`, Rust formatting and diff checks pass. No production code,
workflow, dependency, generated contract or deployment was changed in this probe.

### Private-client Node inventory source increment (2026-10-01)

ADR-0135 preceded implementation. Existing runtime pins now include exact
executable sizes/hashes and full platform-specific licence sizes/hashes for all
six targets. The packager validates those actual bytes before executing Node,
copies the supplementary upstream nbytes MIT notice and SQLite copyright
disclaimer, then checks 21 reported dependency versions and twelve reviewed
shared-library flags. ABI/data values and empty optional versions do not become
library claims. Notice source bytes stay LF across checkouts; the full upstream
Windows licence remains its original CRLF. No runtime dependency or release
sidecar is added.

The existing `client.json` and native report retain the inventory. Both native
execution paths require the new dependency/notice check. Assembly independently
reads the final archive's unique regular Node, three notice files and manifest,
validates the executable platform, and checks actual bytes against reviewed pins
and dependencies. It reuses the existing bounded libarchive reader; the Rust
producer/validator retain that same reader and all Cargo/hash gates. Inventory
tools remain outside the client installer and hooks.

[Actual candidate evidence](../../demos/evidence/ops12-client-node-inventory-candidate.json)
binds dirty parent `f0681edb684c318e1436bbd263ef1cd833d415c6`, source-input hashes
and macOS ARM archive SHA-256
`9936a34e18e12d9e8afad3b491f3abd1bb415f7715aedfe9f03ddc6217630e7a`.
The CLI was built from unchanged Rust source using the local dev profile; this
is Node carriage/installation evidence, not Rust Cargo SBOM qualification.
All nine native archive checks pass, including isolated install/reinstall,
packaged authentication and extracted Codex/Copilot lifecycle replay. Actual
controls coherently alter a runtime byte or truncate its licence, updating both
manifest inventory and Node report: their internal client inventory is valid,
but the independent production archive checker refuses the original upstream
pin. Both controls were removed. Six-target fixtures exercise actual TAR/ZIP
readers and missing/truncated/duplicate/linked notices, foreign identities,
dependency substitutions and changed linkage; no foreign runtime is executed.

All 44 CI-tooling and 81 release-parity tests pass, together with 39 focused
Node/client/Rust-archive tests, fast checks, Rust formatting and diff checks.
No Rust, SQL, public generated contract, deployment or publication gate changed.
The local candidate does not qualify all six native targets, clean hosted source
or complete transitive native coverage. Native Rust remains externally blocked.

Next implement the ADR-0135 product follow-up: add the two reviewed Linux
Node 22.23.2 executable/licence pins, carry the supplementary notices, inspect
actual stopped-image Node/notice bytes before native metadata execution, and
require the existing image reports to retain the result through assembly and
both-registry qualification. Preserve console content, Cargo content, cleanup
and the original 51-payload gates. Then continue unreported/transitive native,
plugin/chart/upstream-image coverage before vulnerability and publisher incident
policy. The fixed Syft distribution, S3 bucket and real issuer/harness credentials
remain separate blockers; no new native qualification source/run is selected.
Never resume cancelled `ef6d61db`, borrow old proof, weaken gates or publish.

### Product Node inventory source increment (2026-10-01)

The ADR-0135 amendment preceded implementation. Both reviewed Linux Node
22.23.2 distributions now have executable/full-licence pins; the product carries
the supplementary nbytes MIT notice and SQLite copyright disclaimer. The
existing image verifier reads actual executable/notice files from an
invocation-owned stopped container through bounded TAR streams. Platform,
executable and complete notice bytes must pass before its isolated native Node
metadata command starts. Network, filesystem, privilege, ambient options and
resource limits remain explicit; cleanup failure cannot retain passing evidence
and preserves the original cause.

Existing local and both-registry image reports carry the inventory. Assembly
checks the reviewed upstream source, target, executable/notice hashes, 21
dependency versions and twelve linkage flags independently. No scanner, service,
package manager, SBOM formatter or release sidecar is added. Console content,
Cargo content, the separate 34 MiB console bound and original 51-payload checksum
gates remain required.

[Actual product candidate evidence](../../demos/evidence/ops12-product-node-inventory-candidate.json)
binds dirty parent `05f252527f4e381db35e4c557f9c753c43d29aef`, source-input hashes,
full native Linux ARM product build and immutable image ID. The actual Node
executable matches upstream SHA-256
`1a638b0fe2b68da0489276aca95526c5122fc61ba54d6a2d0d00c1c92ab7b876`;
all three complete notices, 21 dependency versions and linkage flags pass.
The same image's twelve console files pass their own current source contract.
Two actual control images preserve source labels but alter the Node executable
or truncate its nbytes notice; both refuse before metadata execution. All owned
inspection containers, control images and control storage were removed. No
retained deployment was changed. This is local Node/console carriage evidence,
not clean hosted, both-architecture, Cargo/OCI SPDX, registry or deployment
qualification. Reported Node versions do not prove complete transitive coverage.

All 44 CI-tooling and 84 release-parity tests pass. The six focused Node checks
also bind both product pins to the retained upstream probe. Deployment checks,
all 44 convergence and five uninstall tests, strict chart lint, fast checks,
Rust formatting, pinned actionlint and diff checks pass. One unchanged
convergence fixture initially hit the sandbox's loopback listener denial; its
rerun with socket access passed. No Rust, SQL, public generated contract or
publication path changed.

Next probe unreported/transitive native dependencies and full notice provenance
in the pinned Node distributions and actual product, then plugin/chart and
remaining upstream-image inventories/notices. Review maintained metadata before
choosing any necessary implementation; keep this inside existing packaging and
reports. Vulnerability and publisher incident policy follow that coverage work.
The Syft Windows distribution, operator S3 bucket and real issuer/harness
credentials remain separate external blockers. No new native qualification
source/run is selected. Never resume cancelled `ef6d61db` runs, borrow old OCI
proof, weaken gates or publish; retain the original same-run inventory before
future assembly validation and remove the temporary native qualification branch
only after corrected native proof is independently retained.

### Remaining native package/notice mechanism probe (2026-10-01)

[Retained probe](../../demos/evidence/ops12-native-package-notice-probe.json)
uses clean source `2fa50422dbc1ee9db476d6ffe2664fa1a87d5d9a` to inspect the
preceding task-owned product image from dirty parent `05f25252`. Image labels,
native configuration and every layer diff ID agree with that retained artifact;
the probe records the exported bytes/hash and scanner output hashes. This is
not qualification of the probe source or reuse of an earlier qualification.

The independently hash-verified existing Syft 1.51.0 dpkg and ELF catalogers
identify 106 installed Debian packages, including `libssl3`/OpenSSL
`3.0.20-1~deb12u2`, and the five Rust executables' dynamic `libssl.so.3` and
`libcrypto.so.3` imports. This distinguishes actual C-library identity from
Cargo's Rust binding version. The maintained file-content selection captures
104 package-root copyright documents, one shipped example copyright document
and fourteen common licence files. All 119 complete files, four actual C-library
files and the dpkg database match independent fixed stopped-image reads. Every
installed package maps to verified notice bytes, and all notice SHA-256 values
match the same scan's SPDX files. The container and temporary streams were
removed; no candidate code ran or retained deployment changed.

Parsed SPDX licence identifiers and extracted licence blocks do not retain
every full Debian copyright document. Preserve the original raw files and
their hashes; mixed source copyright sections cannot establish which compiled
library licence applies. The [maintained mechanism](https://oss.anchore.com/docs/capabilities/dpkg/)
and [file-content controls](https://oss.anchore.com/docs/reference/syft/configuration/)
can support a small follow-up in the existing image/SBOM reports. No scanner,
service, formatter, production dependency or release sidecar is added by this
probe, and no architecture choice for the production increment is recorded yet.

Six separate static probes recheck every private Node executable's platform,
size and original hash. Maintained metadata exposes macOS system/framework and
Linux C/C++ runtime imports. Both PE scans return no imported-library list;
this is missing evidence, not proof of zero Windows dependencies. No foreign
executable runs, host-library version is inferred or native target is qualified.
Unreported static Node/V8 components and ncrypto notice provenance remain open.

Independent evidence checks verify the original exported configuration blob,
source-input/scanner/configuration hashes, all 124 fixed image files and the six
original private executables. Fast checks, Rust formatting and diff checks pass.
This slice changes evidence and canonical status only; no production code,
dependency, workflow, generated contract or deployment changes.

Next retry official source reads after GitHub connectivity recovers, review the
pinned Node 24 and 22 build/notice sources and the Windows import-metadata gap,
then record only the necessary narrow choice before extending existing product
image/SBOM reports with installed-package identities, full raw-notice hashes and
dynamic-library checks. Keep the implementation in existing packaging/readers.
Plugin/chart and remaining upstream-image coverage follow, then vulnerability
and publisher incident policy. Two pinned source-tree downloads failed with TLS
connection timeouts; feature-branch push failed with a connection reset and its
bounded HTTP/1.1 retry also timed out. Retry that push when access returns; local
commits remain retained. These transport failures are separate from product
failures and the unchanged Syft Windows qualification blocker. The saved
automation remains paused; no new native source/run, merge, tag or publication
is authorized by this probe.

### Pinned Node/V8 notice correction (2026-10-01)

The [source review](../../demos/evidence/ops12-node-source-notice-review.json)
verifies both official Node source archives against their distribution checksums
and the previously verified release-tag commits. The Node/V8 build files select
Abseil, FP16, rapidhash and a separate V8 zlib copy in both distributions, plus
Highway in Node 24. Their complete upstream licence files now join the existing
supplementary-notice pins. ADR-0135 was amended before implementation; the client
copy loop and existing archive/image/report readers require those original
bytes without a new scanner, report format or release sidecar. The product
copies only its Node 22 set. No library version is inferred from a notice or
source directory. The installed Apple LLVM 21 tool also reads eleven imports
from each original Windows Node executable; original hashes and analysis hashes
are retained. This resolves the static import-list gap, without identifying host
versions or qualifying Windows execution.

[Actual candidate evidence](../../demos/evidence/ops12-node-helper-notices-candidate.json)
binds dirty parent `c66242cca819c2cf622634ff7303fad02076abb5`, source-input hashes
and the complete native ARM product build. Its seven Node notices and all twelve
console files pass. The macOS ARM client archive
`97a03902e9cbd955dec49c9a06812439537f0c7f9dea9de5aae44de6cedb1231`
has eight Node notices and passes all nine native client checks. It uses the
unchanged-source dev-profile CLI, not a Cargo SBOM-qualified release executable.
A coherently changed client Abseil notice and truncated product FP16 notice
refuse the original pins; the image refuses before metadata execution. Owned
controls and inspection containers were removed. No retained deployment changed.
This is local notice carriage, not clean hosted, six-target, Cargo/OCI SPDX,
registry, deployment or complete native coverage qualification.

All 54 focused archive/client/image checks, 44 CI-tooling and 84 release-parity
tests pass. Fast, deployment/chart, formatting and diff checks pass. No Rust,
SQL, generated public contract, workflow or publication gate changes.

Next resolve the pinned ncrypto and fast_float notice provenance when GitHub
access returns; both official source archives omit a separate ncrypto licence,
and Node 24's fast_float headers lack their full upstream licence. Absence does
not establish terms, and conditional GN components are not automatically Node
GYP build dependencies. Independently record the narrow installed-package and
raw-notice report choice from the retained Syft probe before implementing it.
Then continue plugin/chart and remaining image coverage, followed by vulnerability
and publisher incident policy. GitHub source reads time out and the feature-branch
push again failed with a connection reset; completed commits remain local pending
transport recovery. The saved automation stays paused. The native Rust Windows
scanner, operator S3 bucket and real issuer/harness credentials remain separate
blockers. No new qualification source/run is selected; preserve the original
51-payload same-run inventory and separate OCI proof, never resume cancelled
`ef6d61db`, and remove the temporary validation branch only after corrected
native proof is retained.

### Product system package and full-notice gate (2026-10-01)

[ADR-0136](../adr/adr-0136-verify-product-system-packages-and-notices.md) was
accepted before implementation. The existing hashed native product SPDX now
supplies installed Debian name/version/architecture/source identities, complete
notice paths and hashes, database provenance and four native-library hashes and
owners. Independent bounded stopped-image reads corroborate every installed
identity against the actual database, every selected raw file against its
original SPDX hash and each library's native ELF target. Local and both-registry
image reports retain that evidence; assembly requires its exact candidate
inventory. No scanner, service, formatter, workflow or release sidecar is added.
The original 51-payload and Cargo/console/Node/publication gates remain intact.

[Retained local evidence](../../demos/evidence/ops12-product-package-inventory-candidate.json)
binds the dirty reader candidate based on `7d1ab12e61fab348c49a5727ae2a2affc6b2c442`
to a newly built clean product artifact from that same commit:

- Native ARM manifest: `sha256:00018d7824029e1c3f0328652c2eeaadd4b1335604b4587326312e60e7acfe03`.
- Attestation manifest: `sha256:afa8f37a3f82f7f4642dbb0f153b32a9bc5c991687e25f45468024a34afe1221`.
- Original SPDX statement: `dcd1a03bd318d465602ada4c57b30f624897cda9056b909014e24bfc774e8a7a`.

All 106 Debian packages, 119 complete copyright/common-licence files, four
native libraries and the package database pass actual same-image reads. The
same image passes all twelve console files and seven pinned Node notices. A
truncated libssl copyright refuses its original hash. A changed glibc source in
the actual database still refuses after the control coherently updates that
file's expected digest. Owned control images and inspection containers are
removed; no unrelated retained deployment changes. This is local candidate
validation, not clean hosted, AMD64, six-client, registry, deployment or complete
third-party qualification. No full qualification source/run is selected.

The final unchanged release-parity gate exposed a real chart defect: Helm retained
wall-clock timestamps from private staging copies. Two packages across different
seconds had identical contents but different archive hashes. The packager now
fixes only private staged file timestamps; a real two-package regression checks
byte identity and complete LICENSE/NOTICE carriage. All 44 focused, 53 CI-tooling
and 85 release-parity tests pass. Fast, required deployment/chart, Rust formatting,
actionlint and diff checks pass. No Rust, SQL or generated public contract changes.

Current next action supersedes the preceding implementation checkpoint: resolve
pinned ncrypto/fast_float provenance and remaining Node/native coverage, then
plugin/chart and other-image inventories/full notices, followed by vulnerability
and publisher incident policy. Exact GitHub source reads still time out; the
bounded HTTP/1.1 feature-branch push also timed out. Retain local commits and retry
transport when it recovers, without inferring terms from current upstream heads.
The saved automation remains paused. The native Windows scanner, unavailable
operator S3 bucket and real issuer/harness credentials remain separate blockers.
When a fixed official scanner distribution passes actual native path/hash probes,
record the corrected source and dispatch fresh full CI/nonpublishing Release;
verify the original same-run 51-payload inventory before assembly. Preserve the
separate `3beeb1cb` OCI proof, never resume cancelled `ef6d61db` runs, and remove
the temporary qualification branch only after corrected native proof is retained.

### Plugin runtime closure and chart inventory probe (2026-10-01)

The [retained probe](../../demos/evidence/ops12-plugin-chart-inventory-probe.json)
checks actual plugin and chart packages at parent
`4ff8e6ac14788378ba56ac7b8537377835c4ade4`. It exposed tests, mock/driver modules,
declarations/maps and Darwin filesystem metadata in Claude's broad dist copy.
ADR-0065 amendment 14 was recorded before the correction: reuse the existing
shared list plus hook/MCP/skills as ordinary runtime files, and let native clients
consume that same payload without a second filtering path. All 73 retained plugin
files preserve their original bytes and every complete notice. Existing archive
checks inspect shipped files before adding private replay helpers. A broad-copy
control refuses the new gate; linked and missing runtime files refuse packaging.
All owned controls are removed.

Thirty-five configuration and Codex/Copilot replay tests and 21 client-package
tests pass. The actual dirty macOS ARM client archive
`7cbe6c6da318d8de9758cb5e69ccb20f34fffb52543a54164a28584a358ea170`
passes all nine native checks with the unchanged-source dev-profile CLI and
private pinned Node. This is not Rust SBOM, clean hosted, six-target, registry or
real issuer/harness qualification. Fast, 53 CI-tooling and 85 release-parity checks
pass; formatting/diff checks pass. The initial package-manager signature lookup
could not reach its registry in the sandbox; the network-enabled retry verifies
and builds all three adapters without ignoring signature checks.

The independently rehashed maintained Syft 1.51.0 probe finds zero packages from
its initial archive/directory modes. Explicit installed-package cataloging reads
four first-party npm occurrences, corroborated against their actual manifests.
It still does not identify the separate Claude plugin manifest or Helm components;
empty results cannot establish zero dependencies. All 28 locked `keycloakx` 7.3.2
members, its retained complete Apache licence and the chart's root notices match
actual packaged bytes. The upstream notice's version provenance and a complete
component/report contract remain open. No new production scanner, dependency,
workflow, report format or release sidecar is introduced by this probe/correction.

Current continuation: use this probe to record the smallest plugin/chart metadata
and full-notice inventory choice before implementing artifact/source/hash gates.
Resolve pinned ncrypto/fast_float notice provenance when exact GitHub byte access
returns, then remaining image inventory/notices before vulnerability and publisher
incident policy. A cached primary fast_float licence text is readable; the exact
raw-file download still times out and supplies no byte pin. Do not reconstruct
upstream notice bytes or infer ncrypto terms from current repository heads.
The feature push remains transport-blocked, and the saved automation stays paused.
No new full qualification source/run is selected. Preserve the separate `3beeb1cb`
OCI proof and original same-run 51-payload gates; native Windows scanner, operator
bucket and real issuer/harness blockers remain unchanged. Remove the temporary
qualification branch only after corrected native proof is retained.

### Feature-branch push checkpoint (2026-10-01)

The package/full-notice gate is committed as `4ff8e6ac`; the runtime cleanup and
plugin/chart probe are committed as `fe3e1e4f`. Their validation and next product
slice are recorded above. The worktree is clean before this checkpoint.

Automatic approval review initially rejected a bounded push before execution
because explicit owner authorization was missing. The owner subsequently
authorized the pending commits and checkpoint with `push it`. The destination
remains GitHub `synveda/synveda`, branch `codex/synveda-production-roadmap`;
the authorization block is resolved.

The authorized HTTPS push through `98e007a2` exceeded its 40-second transport
bound. Independent GitHub HTTPS and API reads failed during the TLS handshake;
IPv4 and verified TLS 1.2 probes also timed out. GitHub's SSH endpoint on port
443 responds, and its Ed25519 key matches the
[official fingerprint](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints).
Authentication with existing credentials, including the `github.com` host
configuration, is refused with `Permission denied (publickey)`. A temporary
pinned host-key file was used; no credentials, account or permanent SSH/remote
configuration changed. These attempts did not confirm a successful push.

The owner's subsequent requested retry succeeded over HTTPS: all seven pending
commits through `e665d486c85a39e951b302858cbe23372a1a1c52` reached the same feature
branch. An independent `git ls-remote` read confirmed that exact remote HEAD
matches local HEAD. The feature-branch push blocker is resolved; this establishes
no new product or hosted qualification.

The plugin/chart inventory choice remains the next implementation task. Retry
the exact pinned upstream notice downloads as needed; the earlier TLS failures
do not establish their current availability. The scanner, notice-provenance,
operator bucket and real issuer/harness gaps remain separate. The saved
automation stays paused; no merge, tag or public release is authorized.

### Draft PR and Linux packaging prerequisite (2026-10-01)

[Draft PR #67](https://github.com/synveda/synveda/pull/67) reviews this branch
against `main`. At initial PR head `aba43f11899d8f347119f61fd244ebc189bb792a`,
[CI 36908968804](https://github.com/synveda/synveda/actions/runs/36908968804)
deployment job `110526648564` failed the Windows ZIP evidence fixture with
`spawnSync bsdtar ENOENT`. The general check and release assembly install the
ADR-0133 Linux `libarchive-tools` prerequisite; the separate packaging job did
not. It now installs that same package before the unchanged deployment gates.
All 53 CI-tooling and 21 client-package tests and pinned actionlint 1.7.7 pass.

Next verify the corrected Linux packaging job on the PR, then continue the
plugin/chart inventory choice above. PR CI does not replace the pending clean
native/archive and nonpublishing Release qualification. The Windows scanner,
notice provenance, operator bucket and real issuer/harness gates remain open;
the separate OCI proof and paused automation remain unchanged.

### PR CI repair and fixed native scanner (2026-10-03)

PR #67 is now ready for review. At source
`854bc77fb6619385c2cfa651ff7e73c781137815`,
[CI 36919629883](https://github.com/synveda/synveda/actions/runs/36919629883)
passed the corrected Linux packaging job `110562237345` and all other required
jobs except Windows x64 `110562237690` and ARM64 `110562237816`. Both native
client execution paths passed, then the unchanged archive gate refused the
scanner's missing executable SHA-256. This is the retained Syft resolver defect,
not a transient failure or a Rust implementation error.

The official immutable
[Syft 1.54.0 release](https://github.com/anchore/syft/releases/tag/v1.54.0),
published 2026-10-01, now contains fix 5341: tag source
`cc326e45a6213360266dda4b30cc68095946d676` descends from merged
`6ac7afb439c950ec2fc169103c4850c6dfe74c5a`. All six archives match both the
official asset digests and checksum list. Extracted executables have independent
hashes/sizes and correct ELF/Mach-O/PE platform headers; ADR-0133 records the
native pin choice before implementation. OCI scanning remains separately pinned.

The temporary diagnostic at head
`03c096eb5eded1de1a6594a665dbcc27c4ba24b5` ran in
[CI 37117271010](https://github.com/synveda/synveda/actions/runs/37117271010),
checkout `9a32229397de13e3acabdfe6712dc8dbecf498b3`. The native probe step passed
in Windows x64 job `111186399230` and ARM64 job `111186399253`. The reviewed official
tool was copied as `synveda.exe` and scanned using the production single-file,
Rust cataloger and full-metadata options. Both report the exact basename
`synveda.exe`, unchanged empty/non-regular root, and SHA-256 matching the actual
reviewed tool bytes. [Retained evidence](../../demos/evidence/ops12-native-syft-release-probe.json)
binds each result to its original job/source and independent distribution pins.
This tool fixture cannot establish product Cargo coverage. The run was stopped
after both probes passed, so it is not full CI or product qualification.

ADR-0133 records the observed contract before replacing the old filename check.
The executable basename is now required on every target; the former Windows
backslash spelling refuses. The regression fails against the previous validator
before the correction. The temporary script and workflow diagnostic are removed.
Current action: verify fresh corrected PR CI on one source. No checksum
injection, custom build, path widening, archive-content or publication gate
exception is selected. Automatic approval review rejected the proposed separate
remote probe branch; no commit or push occurred there, and its task-created
local worktree/branch are removed.

Full clean-source native/archive and nonpublishing Release qualification remains
pending. Do not resume `ef6d61db` or borrow reports from another source/run;
independently verify the original 51-payload inventory before assembly and all
six client reports, eight archive reports, twelve SPDX documents and both full
OCI/Compose/Helm sets before retaining proof. Preserve the separate `3beeb1cb`
OCI qualification and temporary qualification branch until that corrected proof
is retained. The plugin/chart slice follows this CI repair; external operator,
issuer and notice-provenance gaps remain separate. Automation stays paused.
