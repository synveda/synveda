# Native consumer commands (OPS-12 candidate)

These commands are in the current source CLI and matching plugin candidate.
**They are not in published v0.4.0.** Keep using the
[published installation guide](../deploy/compose/PREBUILT.md) for that release.
Build the current CLI with `SQLX_OFFLINE=true cargo build -p synveda-cli`, or use
the client archive candidate below. [OPS-12](backlog/OPS-12.md) records native
execution reports and the remaining qualification gaps.

## Release downloads

The refactored CI and Release workflows build and test client packages natively
on Linux, macOS and Windows, each on x64 and ARM64. The
[release asset table](RELEASING.md#native-cli-release-artifacts) gives their exact
names. Every package and successful native report is required for stable
publication; a failing target cannot be omitted. Actions artifacts named
`binaries-TARGET` are available from successful validation runs. Public downloads
appear on the GitHub Release only after a new version is approved and published.

The existing v0.4.0 release has only the historical macOS ARM64 and Linux x64
server/CLI archives. It has no `synveda-client-*` assets or Windows packages.
Do not point the client installer at v0.4.0 public downloads. The source examples
below use locally built candidates matching the 0.4.1 workspace version.

For a future published client release, download your platform's archive,
`SHA256SUMS` and `SHA256SUMS.sigstore.json` into a private directory. Follow the
[publisher and checksum verification steps](RELEASING.md#artifacts-and-verification)
before installation. Use the installer from that exact tag and inspect it.
From the directory containing the verified downloads on Unix:

```sh
: "${RELEASE_VERSION:?set the published client version without its v prefix}"
curl -fL "https://raw.githubusercontent.com/synveda/synveda/v$RELEASE_VERSION/scripts/install.sh" \
  -o synveda-install.sh
# Inspect synveda-install.sh before executing it.
SYNVEDA_INSTALL_MODE=client SYNVEDA_VERSION="$RELEASE_VERSION" \
  SYNVEDA_BASE_URL="file://$PWD" sh synveda-install.sh
```

On Windows, place the verified ZIP and checksum inventory in a private local
directory, download `scripts/install.ps1` from the same tag, inspect it, then
run it with `-Version` and `-BaseUrl 'file:///C:/verified-synveda-assets'` pointing
to those files. Follow the Windows ownership and PowerShell requirements below.
Neither route requires a compiler, Docker, system Node or registry credentials.
The installer checks checksums; it does not perform the attestation step for you.

## Client archive candidate

`synveda-client-VERSION-TARGET.tar.gz` (Unix) or `.zip` (Windows) contains the CLI, Claude marketplace,
Codex/Copilot hooks and private Node 24.21.0. It contains no gateway, worker,
console or Compose bundle. Build targets are `darwin-arm64`,
`darwin-x86_64`, `linux-arm64`, `linux-x86_64` (glibc), `windows-arm64` and
`windows-x86_64`. These are candidate targets; each requires its own native
artifact execution report.
The CLI's Unix peer-witness code is isolated, and CLI/hooks share a tested
platform path contract. Windows credential storage now has a native candidate
for ACL, file-identity, bounded reads, locking and replacement checks. Private
receipt/spool storage, the Rust/Node bridge and PowerShell installation are now
implemented candidates; exact native storage, process-refresh and archive
qualification evidence is recorded in [OPS-12](backlog/OPS-12.md).
Windows setup/vendor configuration writers, deployment and diagnostic logs
still refuse. The commands in later sections require Unix unless stated otherwise.

On Unix, config uses absolute `XDG_CONFIG_HOME` or `HOME/.config`; state uses
absolute `XDG_STATE_HOME` or `HOME/.local/state`, each with a `synveda` child.
Relative XDG values are ignored. Missing or relative HOME refuses access; the
spool never falls back to the current repository. The Windows contract
uses `LOCALAPPDATA/synveda/config` and `LOCALAPPDATA/synveda/state`, or explicit
fully qualified local-drive XDG roots. Credentials require a fixed local drive,
ordinary directories/files and user-owned private ACLs; UNC/device paths,
junctions, hard links, alternate streams and ambiguous components are refused.
Missing directories are created only below an already private parent. Existing
ACLs are never repaired automatically; preserve refused files for inspection.
Credentials remain in the existing profile format, with no automatic migration.
Receipts and version-1 spools reuse this private storage boundary. Windows hooks
require `SYNVEDA_CLI` to name the absolute native `.exe` and use bounded local
pipes for state; no credential or gateway operation is exposed by that protocol.
Spool reads/writes are limited to 16 MiB and scans to 4096 entries. A stable lock
and digest comparison refuse stale replacement/removal. Unsafe files remain
held for inspection; see
[ADR-0117](adr/adr-0117-client-platform-boundaries.md).

The existing installer has an explicit client mode. For **locally built**
candidate assets and their `SHA256SUMS` in an absolute directory:

```sh
SYNVEDA_INSTALL_MODE=client SYNVEDA_VERSION=0.4.1 \
  SYNVEDA_BASE_URL=file:///absolute/path/to/candidate-assets \
  sh scripts/install.sh
```

No Docker, system Node or compiler is needed at installation time. The default
CLI launcher is `~/.synveda/bin/synveda`; add the printed directory to PATH.
On Linux, GNU tar also needs the host `gzip` utility to extract the archive.
`SYNVEDA_HOME` selects another installation root and `SYNVEDA_BIN` an explicit
CLI directory. No sudo, shell-profile edit or harness configuration happens.
Paths must be absolute, normalized and free of symbolic-link ancestors.
Checksums detect corruption; this installer does not yet enforce publisher
attestations. Published v0.4.0 has no client archive, so do not run this mode
against its public downloads.

On native Windows x64 or arm64, use the source PowerShell installer with
**locally built** ZIPs and `SHA256SUMS`. Inspect the script and use a host whose
existing PowerShell policy permits it; the installer never changes that policy:

```powershell
& ./scripts/install.ps1 -Version 0.4.1 -BaseUrl 'file:///C:/candidate-assets'
```

PowerShell 5.1+ and a private local fixed-drive parent are required. The default
root is `$env:LOCALAPPDATA\SynvedaClient`; `-InstallRoot` and `-BinDirectory`
select explicit absolute directories. It downloads the ZIP and checksums,
validates the bounded archive and native PE identities, then runs private Node.
It creates `bin/synveda.ps1` and atomically selects an immutable release with
`client/current.json`. The launcher checks the selected manifest/CLI digests.
Run that launcher directly or add its printed directory to PATH. Set
`SYNVEDA_CLI` to the printed absolute `.exe` for separately launched hooks and
use the printed private `node.exe` and hook paths for manual registration.
No harness files or credentials are changed by installation.

Immutable client releases live under `$SYNVEDA_HOME/client/releases/`, selected
by an atomic `client/current` link on Unix or `client/current.json` on Windows.
The installer verifies the content inventory,
native executable identity and private ownership receipt before switching.
Reinstallation retains earlier releases and all deployment state, credentials
and spools. Unowned launchers, modified releases and conflicting current links
are refused. An interrupted `.client-install.lock` is retained for inspection;
verify no installer is running before deliberately removing that empty lock.
Do not modify files in an installed release. Automatic artifact removal remains
unimplemented; retain releases referenced by a vendor marketplace or manual hook
configuration. Adapter removal below remains separately available.

On Unix, `synveda plugin install` and `synveda adapter install --client claude-code`
discover the installed client marketplace. Its copied Claude plugin includes
private Node and names it in both hooks and MCP. Codex/Copilot still require
manual registration and vendor trust: in their existing recipes replace the
Node executable with the printed `client/current/plugin/synveda/runtime/node`
and use the printed client hook path. Keep the entire installed client tree.
A runtime update does not update a vendor-owned cached plugin until its native
registration is deliberately reconciled. Managed receipts can still name an
earlier immutable release; retain those bytes. Cross-release adapter upgrades
and removal of old marketplace artifacts need separate qualification.

For maintainers, `scripts/node-runtimes.json` pins each
[upstream archive](https://nodejs.org/dist/v24.21.0/SHASUMS256.txt) by SHA-256.
The packager retains Node's complete licence and bundled notices, omitting npm
and Corepack. Build the existing adapters first, download the selected pinned
archive, then run on that target's native host:

```sh
node scripts/package-client.mjs 0.4.1 darwin-arm64 target/debug/synveda \
  /absolute/path/to/node-v24.21.0-darwin-arm64.tar.gz \
  /absolute/path/to/candidate-assets "$(git rev-parse HEAD)"
node scripts/check-client-package.mjs 0.4.1 \
  /absolute/path/to/candidate-assets/synveda-client-0.4.1-darwin-arm64.tar.gz \
  /absolute/path/to/client-report.json
```

Generate `SHA256SUMS` over the candidate archive before using the shell installer.
The artifact check uses an isolated home and restricted PATH, then replays the
existing Codex/Copilot lifecycle fixtures with the extracted private runtime.
It also reruns the credential-refresh and platform process tests against the
installed CLI. CI and Release package release-profile binaries on each native
runner; the debug binary above is only a local packaging example.
Reports include archive bytes, local install/reinstall timings, source identity
and dirty-tree state. Local file-copy timing is not network-download timing;
replay is not real issuer login or native vendor loading.

On each native Windows host, build the same adapters and CLI, then run:

```powershell
node scripts/windows-client-candidate.mjs target/debug/synveda.exe 0.4.1 windows-arm64 C:/candidate-assets (git rev-parse HEAD)
```

Use `windows-x86_64` on x64. This downloads the pinned Node ZIP, packages the
native candidate and runs `check-windows-client-package.mjs`. That check uses
Windows PowerShell, a restricted PATH, extracted private Node/CLI, Rust/Node
storage interoperability and malformed ZIP/reinstall refusals. CI carries both
native architectures and their exact archive/report bytes; configuration alone
does not qualify installation. The local macOS PowerShell parser check is syntax
evidence only.

## Local deployment

Use the extracted [plain-Compose candidate](../deploy/compose/CONSUMER.md), a
local Linux-container Docker engine and Compose 2.35+. The CLI and bundle must
have matching versions. Startup lets Compose pull the bundle's pinned images
when absent; the CLI does not build images.
From a terminal, using the built CLI on PATH:

```sh
synveda doctor --bundle /absolute/path/to/synveda-reference-0.4.1
synveda up --bundle /absolute/path/to/synveda-reference-0.4.1
synveda status --bundle /absolute/path/to/synveda-reference-0.4.1
synveda logs --bundle /absolute/path/to/synveda-reference-0.4.1 --tail 100
synveda down --bundle /absolute/path/to/synveda-reference-0.4.1
```

Without `--bundle`, the commands look in `$SYNVEDA_HOME/reference/current`
(default `~/.synveda/reference/current`). An older host-state bundle is refused;
its launcher is a different layout. `--dry-run` prints arguments without Docker
access or file writes. `doctor` checks prerequisites, file identity and ownership;
it does not establish authentication or recoverability.

The project defaults to `synveda-local`. Tests can explicitly select a fresh
`synveda-local-acceptance-<name>` using `--project-name`. On first startup, any
retained containers, networks or volume names for that project cause refusal.
A private receipt binds the original engine, project, canonical bundle path and
runtime files before Compose starts. The same command can resume a partial
start. A moved/changed bundle or different engine is refused; do not delete the
receipt to force adoption. Preserve the original artifact and inspect the
reported difference. This is not a cross-release upgrade command.

`down` stops the graph with every profile selected and retains all volumes,
identity and encryption keys. It never passes `--volumes`. Follow the candidate
guide for deliberate password retrieval, sample use and paired recovery. Direct
Compose and recovery operations must not run concurrently with these commands.

## Select a project and observation choice

Log in using the existing gateway/profile flow. Creating workspaces and projects
remains in the console's Getting started flow; it uses the same public APIs.
Run setup from the target Git repository (a subdirectory is also accepted):

```sh
synveda login --gateway http://localhost:8080 --profile evaluation
synveda setup --profile evaluation
synveda setup --profile evaluation --project PROJECT-UUID --observation off
```

Bare `setup` lists policy-visible projects. Selecting one verifies that exact
active project and its active workspace through authenticated public APIs before
writing `.synveda/config.json` at the Git root. It preserves other settings and
refuses conflicting unmanaged selection. It never stores a bearer or gateway
override in the repository. `--dry-run` verifies the API and prints the selection
without changing project configuration; ordinary credential refresh still applies.

Observation defaults to **off**. Choose `--observation on` explicitly to permit
the hooks to record this project's transcript and tool events. Setup records the
choice privately and marks the project configuration as managed. The updated
hook runtime checks both against the project, workspace and selected profile.
A copied managed config, changed selection, wrong profile, unreadable receipt
or missing config cannot enable recording. Context requests and explicit MCP
tool calls remain separately governed runtime actions.

Hooks launched below the Git root use that root's configuration. A nested Git
repository is a separate project. Select the same `SYNVEDA_PROFILE` and any
isolated `XDG_CONFIG_HOME` in the harness environment; project files do not choose
credentials. Existing manually configured adapters retain their prior contract.
Managed consent requires the matching updated CLI **and** plugin runtime.

## Register an existing adapter

For Claude Code, build/package the updated plugin as described in the
[adapter guide](../adapters/claude-code/README.md), then:

```sh
synveda adapter install --client claude-code --scope project --from /absolute/path/to/plugin
synveda adapter status --client claude-code --scope project
synveda adapter uninstall --client claude-code --scope project
```

This drives Claude's native installer (scope protocol verified with 2.1.241).
The managed route supports `project` and `local`; user-wide hooks cannot use
one project's consent for every repository. The bundle must carry the checked
receipt-aware hook contract; historical same-version bundles are refused.
Registration preserves other scopes, the shared marketplace and persistent
plugin data. Review the vendor's trust prompts and start a new session to verify
loading. Enabled registration alone is not loading or live-client evidence.

For existing registry-backed MCP clients, `--scope user` uses the registry's
user config. Project scope requires an explicit file inside the current repository:

```sh
synveda adapter install --client zed --scope project --config .zed/settings.json
synveda adapter status --client zed --scope project --config .zed/settings.json
synveda adapter uninstall --client zed --scope project --config .zed/settings.json
```

Use these relative paths from the repository root. Setup's profile and exact
workspace/project enter the MCP arguments, with `--writes tool`. Shared servers
remain unbound: provide the Synveda Session ID with each tool call. JSONC edits
preserve unrelated entries and comments. A configuration recipe is not a new
support claim; the [client matrix](CLIENT_SUPPORT.md) remains authoritative.
Codex and Copilot hook installation and trust retain their respective
[Codex](integrations/codex.md) and [Copilot](integrations/copilot-cli.md) guides.
Their compiled shared runtime understands managed observation when updated.

Every mutation retains a private ownership receipt; an interrupted install can
be retried with the same selection. An existing entry without that receipt is
not silently adopted. Removal refuses a changed entry or native marketplace
source, and preserves unrelated configuration, credentials, setup consent and
spools. Use `--dry-run` on install/removal to inspect the intended action.

Receipts live under `$XDG_CONFIG_HOME/synveda/consumer` (default
`~/.config/synveda/consumer`) with private directory/file permissions. They hold
selection and hashes, never tokens or transcripts. A bounded OS lock serializes
setup/lifecycle and both new and older plugin/MCP mutations. Do not remove its
`operations.lock` file: the OS releases the lock on process exit. Direct vendor
commands and editors do not participate; keep them idle during configuration
edits. Changed files are refused when detected before atomic replacement.
