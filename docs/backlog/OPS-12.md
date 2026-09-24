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
The source CLI now implements `up/down/status/logs/doctor/setup/adapter` under
[ADR-0116](../adr/adr-0116-native-consumer-commands.md); these are not in the
published binary. The [candidate command guide](../CONSUMER_CLI.md) owns their
exact scope, observation and receipt contracts.
The published reference launcher prepares host files before Compose. This
working tree adds a plain-Compose candidate with locally qualified named-volume
recovery. Release CI now packages it and requires its separate native lifecycle
and recovery reports before publication; no new release has been authorized.

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
client artifact removal and publisher-attestation enforcement remain open.

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
attestation enforcement and Homebrew/WinGet remain unimplemented; configuration
of GitHub attestations alone does not authenticate the current shell installer.

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

Continue OPS-12 from the published v0.4.0 bytes. The owner authorized a new
v0.4.2 release from the latest mainline source. Its candidate includes the
Docker Hub RepoDigest verifier fix and coordinated workspace, chart, adapter,
console, OpenAPI and SDK version metadata. Next: merge it through CI Result,
wait for full exact-source main CI, run the nonpublishing Release drill, then
push the unused v0.4.2 tag for native anonymous pull, Docker, Helm and final
asset verification. Preserve existing v0.4.1 image tags and the source tag.
For another local candidate, use the
existing `package-release.sh` arguments with the explicit candidate flag, extract
the archive, then run `node scripts/qualify-release.mjs --consumer-candidate
EXTRACTED_BUNDLE REPORT.json`. It creates an absent random acceptance project,
leaves source/restore databases, installation state and the paired recovery set
retained. Release CI runs the same gate on both native architectures and requires
its checksummed reports. The first hosted dry run stopped before consumer
qualification because AMD64 Docker failed; the final hosted run completed it
on both native architectures. Next OPS-12 work is artifact-based real
issuer/harness acceptance, installer attestation enforcement, OS
signing/notarization and a supported platform/version policy. Future releases
still require exact-source main CI, owner settings and version coordination.
Do not reuse an unrelated deployment or replace published artifacts.
