# Native consumer commands

This is the reference for the `synveda` client: how it is installed, where it
keeps its files and how its setup commands behave. To connect an agent, start
with [Connect an AI coding agent](AGENT_SETUP.md). The
[client support matrix](CLIENT_SUPPORT.md) records tested agent versions and
platform limits.

## Release downloads

The [release asset table](RELEASING.md#native-cli-release-artifacts) names the
Linux, macOS and Windows archives for x64 and ARM64. The installers pick the
archive for your system and install it without a compiler, Docker, system Node
or registry credentials:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
```

```powershell
powershell -NoProfile -Command "irm https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.ps1 | iex"
```

Before they fetch or run any archive, both installers use the GitHub CLI to
verify that the release's `SHA256SUMS` was signed by this repository's release
workflow for that tag, then check the archive against its unique checksum
entry. Missing or failed proof stops installation
([ADR-0132](adr/adr-0132-verify-publisher-before-installing-release-code.md),
[ADR-0155](adr/adr-0155-one-step-client-installation.md)).

| Setting | Shell (`install.sh`) | PowerShell (`install.ps1`) |
| --- | --- | --- |
| Release | `SYNVEDA_VERSION=0.4.7` (default: latest) | `-Version 0.4.7` |
| Also pin the release commit | `SYNVEDA_SOURCE_SHA=<40-hex commit>` | `-SourceSha <40-hex commit>` |
| Install root | `SYNVEDA_HOME` (default `~/.synveda`) | `-InstallRoot` (default `%LOCALAPPDATA%\SynvedaClient`) |
| Launcher directory | `SYNVEDA_BIN` (default `$SYNVEDA_HOME/bin`) | `-BinDirectory` (default `<root>\bin`) |
| Leave PATH alone | `SYNVEDA_NO_MODIFY_PATH=1` | `-NoModifyEnvironment` |
| Mirror or offline directory | `SYNVEDA_BASE_URL=https://...` or `file:///abs/dir` | `-BaseUrl` |
| GitHub CLI | `gh` on PATH | `-GhPath` or `SYNVEDA_GH` (full path of `gh.exe`), else a current `gh` on PATH, else a pinned copy for this run |

Each PowerShell parameter defaults to the environment variable in the shell
column (and `SYNVEDA_GH`), so the one-line command can be configured without
parameters. `SYNVEDA_VERSION` may carry a leading `v` in both installers.

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\Synveda" | Out-Null
$env:SYNVEDA_HOME = "$env:USERPROFILE\Synveda\Client"
powershell -NoProfile -Command "irm https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.ps1 | iex"
```

Unless told not to, the shell installer appends one PATH line to your shell's
profile, and the PowerShell installer adds the launcher directory to your user
`Path` and sets the user `SYNVEDA_CLI` variable that agent hooks use. For an
offline installation, download the platform archive, `SHA256SUMS` and
`SHA256SUMS.sigstore.json` into a private local directory and point the base
URL at it; set the source commit too, so the signature is still checked. A
local directory without a source commit is treated as an unverified
development candidate.

`SYNVEDA_INSTALL_MODE=reference` makes the shell installer install the server
binaries and Docker reference bundle instead of the client.

### Managed Windows machines

The PowerShell installer needs no write access outside your profile and no
elevation:
- **Proxy.** Downloads use the Windows proxy settings and answer an
  authenticated proxy with your Windows sign-in.
- **Internal mirror.** Copy a release's platform ZIP, `SHA256SUMS` and
  `SHA256SUMS.sigstore.json` to a folder on GitLab, Bitbucket, Artifactory or
  another HTTPS server, then set `SYNVEDA_BASE_URL` to that folder and
  `SYNVEDA_VERSION` to the release. Unless every machine has a current `gh`,
  also copy the pinned GitHub CLI ZIP (`gh_2.102.0_windows_amd64.zip` and
  `gh_2.102.0_windows_arm64.zip`, from the
  [GitHub CLI release](https://github.com/cli/cli/releases/tag/v2.102.0)) to
  the same folder. When the mirror asks for Windows sign-in
  (Negotiate or NTLM), the installer answers for that host only; no other host
  or redirect target receives credentials. URLs carrying a user name are
  refused.
- **GitHub CLI.** The installer uses `SYNVEDA_GH` when set. Otherwise it uses
  `gh` from PATH if it reports version 2.102.0 or later. Otherwise it
  downloads GitHub CLI 2.102.0 from its GitHub release (or the mirror). It
  checks the download against the SHA-256 pinned in the script, runs it from
  the installer's private download folder and deletes it afterwards. That
  copy writes no configuration, cache or telemetry identity to your profile.
  Neither path needs a GitHub account or `gh auth login`. Verification still
  needs HTTPS access to `tuf-repo-cdn.sigstore.dev` and `tuf-repo.github.com`
  for Sigstore's trusted roots. `gh` reads only `HTTPS_PROXY` for its proxy
  and does not perform Windows (NTLM) proxy sign-in. No step needs UAC or
  administrator rights, including the client's bundled Node runtime.
  Application-control policies (AppLocker, App Control for Business) must
  still allow executables in the install root and download folder.
- **Install location.** If the installer reports that a folder lets another
  account write, delete or change entries, choose an install root outside that
  folder, for example `$env:USERPROFILE\Synveda\Client` after creating
  `$env:USERPROFILE\Synveda`. When the reported folder is `AppData`, also set
  `XDG_CONFIG_HOME` and `XDG_STATE_HOME` as described in
  [Windows ACL refusals](#windows-acl-refusals) so sign-in state avoids it too.

The older v0.4.3 installer requires the manual attestation verification in its
[versioned guide](https://github.com/synveda/synveda/blob/v0.4.3/docs/CONSUMER_CLI.md).
The [v0.4.5 install probe](../demos/evidence/ops12-045-clean-install.json) also
checks the published macOS ARM64 CLI's real PKCE login to bundled Keycloak.
OS code signatures, native vendor loading and external issuer qualification
remain open. See [production readiness](PRODUCTION_READINESS.md) for those limits.

<a id="client-archive-candidate"></a>
## Client archive contents

`synveda-client-VERSION-TARGET.tar.gz` (Unix) or `.zip` (Windows) contains the CLI, Claude marketplace,
Codex/Copilot hooks and private Node 24.21.0. It contains no gateway, worker,
console or Compose bundle. Build targets are `darwin-arm64`,
`darwin-x86_64`, `linux-arm64`, `linux-x86_64` (glibc), `windows-arm64` and
`windows-x86_64`. The v0.4.7 release includes all six archives. See the
[readiness assessment](PRODUCTION_READINESS.md) for platform limits.
On Windows, credentials, receipts, hook spools and hook diagnostic logs use
private local storage, through the CLI's native ACL backend. These commands
work natively:
- `setup --project`;
- `mcp install|uninstall`;
- `plugin install|uninstall`;
- `adapter`;
- demo and skill commands.

Only the deployment commands (`up`, `down`, `status`, `logs`, `doctor`) refuse
by name. The Windows client connects to a deployment running elsewhere.

Windows Claude registration has three requirements:
- It runs only Claude Code's native `claude.exe` found in an absolute `PATH`
  entry. npm `.cmd`/`.ps1` shims are refused.
- It finds the installed release through `SYNVEDA_HOME`, else
  `%LOCALAPPDATA%\SynvedaClient`, and the installer's checked
  `client\current.json`.
- Hooks need a user-level `SYNVEDA_CLI` naming the installed `synveda.exe`.
  The PowerShell installer sets it on every install or upgrade; restart Claude
  afterwards. With `-NoModifyEnvironment`, `synveda plugin install` prints the
  `[Environment]::SetEnvironmentVariable(...)` line to run yourself.

Hook diagnostics are batched into the private
`%LOCALAPPDATA%\synveda\state\adapter.log`, or the same file under
`XDG_STATE_HOME`. The log rotates at 4 MiB and keeps one generation. Raw-frame
capture for the live gate stays Unix-only.

The commands in later sections require Unix unless stated otherwise.

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
held for inspection.

Windows project and vendor configuration edits keep the file's existing DACL:
1. An exclusive temporary in the same directory receives that DACL before any
   byte is written.
2. The temporary is flushed.
3. An atomic rename replaces the file.

A new file is created private to you, SYSTEM and Administrators, like the Unix
0600 temporary.

These are refused without changing the file:
- links, directories and hard-linked files;
- files larger than 2 MiB;
- a null or unreproducible DACL;
- a file that changed during the edit.

Another program holding the file without delete sharing delays the edit by at
most 0.5 s, after which the edit fails and the original is kept. The name
always shows a complete version. Survival of the directory entry across power
loss is not promised.

<a id="windows-acl-refusals"></a>
An ACL refusal names the object and the account SID that broke the rule, for
example `C:\Users\you\AppData lets S-1-5-21-...-1016 write, delete or change
entries`. Synveda never repairs existing ACLs. To recover:
1. Set `XDG_CONFIG_HOME` and `XDG_STATE_HOME` to directories on a local fixed
   drive. Their ACL may grant access only to you, SYSTEM and Administrators,
   and no ancestor may let another account write or delete entries.
2. Set the same variables for each harness, so that hooks resolve the same
   roots.

For **locally built** candidate assets and their `SHA256SUMS` in an absolute
directory:

```sh
SYNVEDA_VERSION=0.4.7 \
  SYNVEDA_BASE_URL=file:///absolute/path/to/candidate-assets \
  sh scripts/install.sh
```

No Docker, system Node or compiler is needed at installation time. The default
CLI launcher is `~/.synveda/bin/synveda`, and the installer adds its directory
to PATH in your shell profile unless `SYNVEDA_NO_MODIFY_PATH=1` is set.
On Linux, GNU tar also needs the host `gzip` utility to extract the archive.
`SYNVEDA_HOME` selects another installation root and `SYNVEDA_BIN` an explicit
CLI directory. No sudo or harness configuration happens.
Paths must be absolute, normalized and free of symbolic-link ancestors.
Without an expected source commit, an absolute local asset directory is a
development candidate: the installer prints that publisher identity was not
verified. Supplying `SYNVEDA_SOURCE_SHA` instead requires its signed inventory
and the same publisher verification as remote assets. Local candidates retain
checksum and archive validation. Published v0.4.0 has no client archive, so do
not run this mode against its public downloads.

On native Windows x64 or arm64, use the source PowerShell installer with
**locally built** ZIPs and `SHA256SUMS`. Inspect the script and use a host whose
existing PowerShell policy permits it; the installer never changes that policy:

```powershell
& ./scripts/install.ps1 -Version 0.4.7 -BaseUrl 'file:///C:/candidate-assets'
```

PowerShell 5.1+ and a private local fixed-drive parent are required. The default
root is `$env:LOCALAPPDATA\SynvedaClient`; `-InstallRoot` and `-BinDirectory`
select explicit absolute directories. The folder containing the install root
must already exist, must not be a drive root, and must be owned by you and
accessible only to you, SYSTEM and Administrators; no folder above it may let
another account write or delete entries. Downloads are staged in that folder
and removed afterwards. It downloads the ZIP and checksums,
validates the bounded archive and native PE identities, then runs private Node.
It creates `bin/synveda.ps1` and atomically selects an immutable release with
`client/current.json`. The launcher checks the selected manifest/CLI digests.
The installer adds that launcher directory to the user `Path` and sets the
user `SYNVEDA_CLI` to the selected `.exe` unless `-NoModifyEnvironment` is
given. No harness files or credentials are changed by installation.

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

`synveda adapter install` finds the installed client release. For Claude Code
it registers the release's marketplace, whose copied plugin includes private
Node and names it in both hooks and MCP. For Codex and Copilot it writes hook
entries that run the release's private Node and hook, and registers the MCP
server with the vendor's own `mcp add`. Keep the entire installed client tree.
On Unix those hook entries name the `client/current` link, so they follow an
upgrade; on Windows they name the release, so rerun `synveda adapter install`
after upgrading. A runtime update does not update a vendor-owned cached plugin
until its native registration is deliberately reconciled. Managed receipts can still name an
earlier immutable release; retain those bytes. Cross-release adapter upgrades
and removal of old marketplace artifacts need separate qualification.

For maintainers, `scripts/node-runtimes.json` pins each
[upstream archive](https://nodejs.org/dist/v24.21.0/SHASUMS256.txt) by SHA-256.
The packager retains Node's complete licence and bundled notices, omitting npm
and Corepack. Build the existing adapters first, download the selected pinned
archive, then run on that target's native host:

```sh
node scripts/package-client.mjs 0.4.5 darwin-arm64 target/debug/synveda \
  /absolute/path/to/node-v24.21.0-darwin-arm64.tar.gz \
  /absolute/path/to/candidate-assets "$(git rev-parse HEAD)"
node scripts/check-client-package.mjs 0.4.5 \
  /absolute/path/to/candidate-assets/synveda-client-0.4.5-darwin-arm64.tar.gz \
  /absolute/path/to/client-report.json
```

Generate `SHA256SUMS` over the candidate archive before using the shell installer.
The artifact check uses an isolated home and restricted PATH, then replays the
existing Codex/Copilot lifecycle fixtures with the extracted private runtime.
It also reruns the credential-refresh and platform process tests against the
installed CLI. CI and Release package release-profile binaries on each native
runner; the debug binary above is only a local packaging example.
Reports include archive bytes, install results, source identity and dirty-tree
state.

On each native Windows host, build the same adapters and CLI, then run:

```powershell
node scripts/windows-client-candidate.mjs target/debug/synveda.exe 0.4.5 windows-arm64 C:/candidate-assets (git rev-parse HEAD)
```

Use `windows-x86_64` on x64. This downloads the pinned Node ZIP, packages the
native candidate and runs `check-windows-client-package.mjs`. That check uses
Windows PowerShell, a restricted PATH, extracted private Node/CLI, Rust/Node
storage interoperability and malformed ZIP/reinstall refusals. CI runs both
native architectures.

## Local deployment

Use the extracted [plain-Compose bundle](../deploy/compose/CONSUMER.md), a
local Linux-container Docker engine and Compose 2.35+. The CLI and bundle must
have matching versions. Startup lets Compose pull the bundle's pinned images
when absent; the CLI does not build images.
From a terminal, using the built CLI on PATH:

```sh
synveda doctor --bundle /absolute/path/to/synveda-reference-0.4.5
synveda up --bundle /absolute/path/to/synveda-reference-0.4.5
synveda status --bundle /absolute/path/to/synveda-reference-0.4.5
synveda logs --bundle /absolute/path/to/synveda-reference-0.4.5 --tail 100
synveda down --bundle /absolute/path/to/synveda-reference-0.4.5
```

Without `--bundle`, the commands look in `$SYNVEDA_HOME/reference/current`
(default `~/.synveda/reference/current`). An older host-state bundle is refused;
its launcher is a different layout. `--dry-run` prints arguments without Docker
access or file writes. `doctor` checks prerequisites, file identity and ownership.
Run `up` and sign in to check the deployment.

The project defaults to `synveda-local`. Tests can explicitly select a fresh
`synveda-local-acceptance-<name>` using `--project-name`. On first startup, any
retained containers, networks or volume names for that project cause refusal.
A private receipt binds the original engine, project, canonical bundle path and
runtime files before Compose starts. The same command can resume a partial
start. A moved/changed bundle or different engine is refused; do not delete the
receipt to force adoption. Preserve the original artifact and inspect the
reported difference. This is not a cross-release upgrade command.

`down` stops the graph with every profile selected and retains all volumes,
identity and encryption keys. It never passes `--volumes`. Follow the bundle
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

## Register an agent client

`synveda adapter install|status|uninstall --client NAME` connects an agent to
the project chosen by `synveda setup`, from the repository root:

```sh
synveda adapter install --client claude-code     # or codex, copilot-cli
synveda adapter status --client claude-code
synveda adapter uninstall --client claude-code
```

Claude Code, Codex and Copilot default to `--scope project`; MCP clients default
to `--scope user`. `--from` selects an extracted client archive (or, for Claude,
a packaged marketplace built from a checkout as described in the
[adapter guide](../adapters/claude-code/README.md)) instead of the installed
client.

This drives Claude's native installer (scope protocol verified with 2.1.241).
The managed route supports `project` and `local`; user-wide hooks cannot use
one project's consent for every repository. The bundle must carry the checked
receipt-aware hook contract; historical same-version bundles are refused.
Registration preserves other scopes, the shared marketplace and persistent
plugin data. Review the vendor's trust prompts and start a new session to check
that the client loads the adapter.

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
For Codex the command merges its entries into `.codex/hooks.json` and keeps
other hooks; for Copilot it owns `./.github/hooks/synveda.json`. Both also run the
vendor's `mcp add synveda` once and record that in the receipt, so uninstall
runs `mcp remove synveda` only for an entry it added. The
[Codex](integrations/codex.md) and [Copilot](integrations/copilot-cli.md)
guides describe the hooks and the vendor trust prompts.

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


## Capture retry in the current source build

A Capture batch that stopped retrying can be reopened once more for one bounded
cycle when its source and Configuration are unchanged. Inspect it first with
`GET /v1/capture-batches/BATCH_UUID`; `terminal_reason` says why it stopped.

```sh
synveda session retry-capture BATCH_UUID --idempotency-key RETRY_REQUEST_KEY
```

Reuse the same key on a retry of the request. The gateway decides
`session.write` on the source session, keeps the attempt count, allows two
manual retries within seven days of failure and refuses a batch whose frozen
source was withdrawn or whose effective Configuration has changed; capture the
session again in that case. Published client archives up to v0.4.7 do not
include this command.

## Session source controls in the current source build

The source build adds governed withdrawal and forgetting for one inspected
Session event. Obtain the event id and full `payload_hash` from the authorized
Session event API or console payload inspector. Use the public API through the
CLI; published v0.4.6 client archives include these commands, v0.4.5 does not.

```sh
synveda session source control SESSION_UUID EVENT_UUID \
  --mode withdraw --expected-payload-hash ORIGINAL_PUBLIC_HASH \
  --reason 'Retire this source evidence' --idempotency-key SOURCE_REQUEST_KEY
synveda session source status SESSION_UUID EVENT_UUID
```

Reuse the same key and body on a retry. A pending review removes nothing. An
applied withdrawal makes the event and its dependent copies unavailable to
future delivery while retaining their storage contents. Its durable receipt is
`blocked` with `source_retained`. Upgrade the same source to `--mode forget`
with a new request key to queue physical cleanup; forgetting also requires the
current KnowledgeForget permission. Review and apply use the existing VedaFlow
workflow and current approval matrix.

Inspect logical removal separately from cleanup: `removed_at`, `mode`,
`cleanup_state`, `cleanup_stage` and the content-free `error_code` are returned
by source status. `/v1/operations/OPERATION_UUID` retains the addressed receipt,
including after a later Forget upgrade or Session disposal. It requires current
SessionRead authority reconstructed from the immutable source address. No
Skill-specific target or plaintext is included.

The worker checks holds before scrubbing and processes at most 100 indexed
targets per page. Retries retain suppression; a live fifth attempt may finish
multiple pages. A hold reports `blocked/retention_hold` and leaves content
inaccessible but retained. A legacy nonempty delivery without producer proof
reports `blocked/lineage_unproven`. Blocked operations remain terminal; further
retention disposal belongs to the owning workflow and does not make the source
eligible again. This build does not silently clear a permanent source suppression.

Suppression binds the original tenant/Session, client-event-id digest and raw
payload hash. Retrying the original event, changing only its client id, or
returning an old worker result cannot restore its old derivatives. A separate
new Session and deliberately authored new Knowledge are separate governed
materializations; they do not restore the old source, receipts or approvals.
Independent later revisions remain available. Flat multi-source revisions are
withheld when any supporting root is withdrawn because that model cannot prove
which statement remains independently supported.

Already admitted responses may finish after removal. External harness history,
independent exports, remote Git history and backups follow their own retention
contracts. Source controls do not retract bytes already delivered there.

## Imported artifact source controls in the current source build

Inspect an authorized OKF import plan to obtain its job id, artifact id and
original `content_hash`. The console offers the same control on each visible
artifact and retains the resulting operation receipt after the artifact becomes
unavailable. The source CLI uses these public API routes:

```sh
synveda okf source control JOB_UUID ARTIFACT_UUID \
  --mode withdraw --expected-content-hash ORIGINAL_CONTENT_HASH \
  --reason 'Retire this imported evidence' --idempotency-key ARTIFACT_REQUEST_KEY
synveda okf source status JOB_UUID ARTIFACT_UUID
```

Reuse the exact key and body on retry. Pending review removes nothing. Applied
withdrawal hides all copies dependent on the inspected artifact, including
accepted revisions, candidates and saved context. Unrelated artifacts and
independently sourced revisions remain usable. `--mode forget` additionally
queues physical cleanup through the same bounded SourceCleanup worker. Current
project-scoped KnowledgeRead governs inspection; KnowledgeWrite and
ProposalOpen govern the intent; KnowledgeForget additionally governs Forget.
The existing approval matrix governs publication.

Suppression binds scope, admitted source identity, path and original content
hash. Changing the bundle, import job or upstream commit cannot restore the
same source bytes. Managed imports use the server-established repository
identity, which survives detach/reattach. Changed content may produce a new
reviewable candidate. Plain uploads bind their admitted kind and locator;
labelling an upload as Git does not establish a managed repository identity.
Replaying a frozen plan with unavailable required source rows returns a
conflict. An explicitly erased item mapping remains inert under item Forget.

The worker checks every dependent revision's holds before erasing content and
processes at most 100 targets per page. It clears affected service-owned
delivery, candidate, pending command, revision, embedding, relation, descriptor,
mapping and artifact copies. Erased artifacts retain hashes and ids with an
inert `erased/ARTIFACT_UUID` path. Import notices are hidden at logical removal
and cleared during cleanup. Unprovable legacy import lineage blocks cleanup
with `lineage_unproven`; complete legacy producer tuples are normalized during
upgrade. `/v1/operations/OPERATION_UUID` retains the exact addressed receipt,
even after a later Forget upgrade. Its status does not claim erasure of remote
Git history, independent exports, backups or already delivered harness bytes.
