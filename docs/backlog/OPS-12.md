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
refusal. This source is not qualified; preserve its failure rather than retrying
the unresolved scanner defect.

Independent review also reproduced an assembly gap: coherent changes to a
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

Next investigate the pinned scanner's native Windows digest omission, reproduce
the failure with bounded native diagnostics and correct the actual defect
without supplying or relaxing SPDX checksums. Then dispatch full CI and a
nonpublishing Release on one clean corrected source and record their exact IDs.
Before retaining qualification, independently verify its original 51-payload
inventory, six client reports, eight archive reports, twelve SPDX documents and
both complete OCI/Compose/Helm report sets. No earlier report can qualify that
source. Non-Rust inventory/notices follow; production readiness remains unclaimed.

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
