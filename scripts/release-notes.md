This is a **controlled evaluation release**. It adds progressive context
disclosure: a team can deliver Knowledge to agents as a manifest of short
entries, and an agent opens an entry's full text only when it asks and is
still allowed to read it. It also fixes the Codex adapter install and the
Docker launcher's operation lock on Ubuntu 26.04, names why withheld
context-run detail is withheld, and clears four Rust security advisories.

**Upgrading from v0.4.8 is a planned maintenance interruption.** The gateway,
worker and bundled Keycloak stay stopped from writer stop until the new
release is ready. There is no zero-downtime, mixed-version or down-migration
path. The [upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md)
proposes a 15-minute window for the bundled single-host route; no maximum has
been set, and larger databases are not measured.
- Schema epoch 3 is unchanged. One additive migration moves the required head
  from `0018` to `0019` in place, on both the published and portable lineages.
- From the extracted v0.4.9 bundle, `./synveda-compose upgrade before-0-4-9`,
  with `SYNVEDA_UPGRADE_FROM` naming the installed v0.4.8 bundle, backs up,
  migrates and restarts the loopback evaluation. A rerun resumes from the last
  completed phase.
- After the schema change, rollback means restoring the pre-upgrade set with
  the v0.4.8 bundle.
- The guide's table supports v0.4.8 → v0.4.9 only. From v0.4.7, upgrade to
  v0.4.8 first.

**The v0.4.8 → v0.4.9 pair has not been drilled from published bundles, and no
published chart pair has been drilled on a cluster.**

**Progressive context disclosure.** It is opt-in: every built-in Configuration
template still delivers full text. To turn it on, publish a Configuration
version whose `context` section sets `"disclosure_mode": "manifest"`. It goes
through review like any other Configuration change.
- A manifest ContextRun gives one entry per Knowledge item the reader may
  see: title, summary, type, sensitivity marker and the body's estimated token
  cost. A denied item leaves no entry, count or marker. Entries come from the
  stored, reviewed revision; there is no model call.
- `POST /v1/sessions/{session_id}/knowledge-expand` returns the full text of
  1–32 ids and authorises each one again. A missing, denied, withdrawn or
  changed id gets the same `not_found_at_as_of`, with no reason. Each
  expansion that serves an item appends one `context.knowledge_expanded`
  Session event that names items, revisions and tokens, never content.
  `synveda mcp` offers it to agents as the `expand` tool.
- After compaction, a `restart` run reads up to 32 expanded items again under
  current policy.
- A single ContextRun request can ask for `"disclosure": "manifest"`. A
  request can narrow `full` to `manifest`, never the reverse.

On the same 10 of LongMemEval's 500 questions, with the first 8 entries opened
by a fixed rule, manifest delivery reached 9 of 14 evidence sessions against 8
under `full`, and used 1,805.6 mean tokens against 1,413.7. In one judged run
per mode, manifest answered 4 of 10 and `full` 2. That sample does not show
that manifest delivery answers better.

**Behaviour changes.**
- `knowledge-query`, the MCP `recall` tool and `synveda recall` now return
  summaries by default. Ask for `depth` `full` (`--depth full`) for the body.
  There is no compatibility switch.
- Every Knowledge command now applies the effective policy pack's redaction
  ladder to personal data in the title, summary, body, tags, metadata, sources
  and lifecycle reasons. A credential is refused under every pack. A Knowledge
  write that v0.4.8 stored unchanged may now be redacted or refused.

**Console and CLI.**
- Recent context no longer prints tokens and selections that its listing never
  carries. Open a run for its size and selections.
- A run whose detail is withheld now says why, and shows the withheld values
  as withheld rather than as zeros. `synveda context inspect` does the same,
  and the run detail gains `delivery_withheld`.

**Agent adapters.** `synveda adapter install --client codex` no longer fails
with `` `hooks` must be an object `` when `.codex/hooks.json` does not exist,
including on every reinstall after an uninstall.

**Docker launcher.** On a host whose `mkdir` comes from uutils coreutils, such
as Ubuntu 26.04, two `synveda-compose` commands started together could both
pass the operation lock, and one of them then failed. The launcher now takes
the lock with the shell's exclusive file create. v0.4.8 and earlier keep the
defect on such hosts, so run their commands one at a time, and let every one
finish before starting the upgrade. After a killed command,
`state/.evaluation-operation` is now a file holding the process ID, not an
empty directory.

**Security.** rmcp is upgraded to clear RUSTSEC-2026-0457, 0461 and 0462, and
rustix to clear RUSTSEC-2026-0486.

PostgreSQL 14–18, pgvector 0.8.7 and the dated legacy pgvector profiles are
admitted as in v0.4.8. For a retained database still on an earlier pgvector,
follow the
[retained pgvector transition](https://github.com/{{repository}}/blob/{{source}}/docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition).

Version: `{{version}}`. Exact release source: `{{source}}`.

Local OrbStack acceptance passed namespace-scoped installation, real browser
and native CLI login/context, retained reinstall and the governed first-use
walkthrough. Its exact source/runtime evidence is recorded in
[the Kubernetes release record](https://github.com/{{repository}}/blob/{{source}}/docs/backlog/OPS-11.md#orbstack-local-acceptance-checkpoint-2026-10-04).
**The three unfamiliar engineers' guide-only trials are still pending.**
No setup-time, additional platform-support or production-readiness claim follows
from that local probe. Shared first-human/HTTPS admission and real vendor/model
use remain separate acceptance work.

Native client artifacts for Linux, macOS and Windows on x64 and ARM64,
plus the digest-bound Docker reference deployment. Client installation does
not start containers or install a server. For a server, extract the separate
reference bundle and use its `synveda-compose` launcher with the loopback
evaluation or documented HTTPS configuration.

Verify `SHA256SUMS` with its `SHA256SUMS.sigstore.json` publisher
attestation before verifying/extracting downloads. See the versioned
[verification guide](https://github.com/{{repository}}/blob/{{tag}}/docs/RELEASING.md).
Native binaries have no OS code signature or notarization. The attested
checksum inventory authenticates these release bytes and registry digests.

For a server, download only `synveda-reference-{{version}}.tar.gz`
plus `SHA256SUMS` and `SHA256SUMS.sigstore.json`: no source checkout or compiler is needed. Follow
the extracted bundle's `INSTALL.md` and `CONSUMER.md` for matching download
verification, localhost first sign-in and lifecycle operations.

Both Linux AMD64 and ARM64 jobs anonymously pulled the manifest-bound
image set and passed isolated executable/asset smoke checks. Their
checksummed `release-images-*.json` reports are attached. These
reports distinguish executable checks from the required Docker startup,
real OIDC browser/CLI login, sample/recreation and paired recovery drill.
Both native runners also exercised all four provider ownership modes
using the same candidate bytes and packaged chart in disposable Kind.
Both runners also qualified plain Compose and its private named-volume
recovery through the extracted archive's CONSUMER.md ceremony.
The checksummed release-docker, release-consumer and release-kubernetes reports are attached.
Docker reference live clean-host acceptance is tracked separately
in those deployment reports from the isolated image smoke checks.
macOS Docker Desktop, Windows/WSL2 and real OpenShift remain unqualified.
This is a controlled single-host reference, not HA, production SaaS
or enterprise certification.

The database baseline is schema epoch 3; earlier epochs are refused
and no published N-1 data upgrade is qualified by these checks.
Source tests advance both v0.4.8 lineages from `0018` to `0019`; the
[upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md#compatibility-table)
lists the supported pairs.
Keep the same runtime selection when updating an existing deployment.
Back up both databases and matching keys first. Application/Helm
rollback does not reverse database migrations.

To install only the CLI and agent adapters on Linux or macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/{{repository}}/{{source}}/scripts/install.sh |
  SYNVEDA_VERSION={{tag}} SYNVEDA_SOURCE_SHA={{source}} sh
```

On Windows (PowerShell):

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/{{repository}}/{{source}}/scripts/install.ps1 -OutFile synveda-install.ps1
& ./synveda-install.ps1 -Version {{version}} -SourceSha {{source}}
```

These commands pin this release and its source commit. The installers need a
trusted GitHub CLI for publisher verification (on Windows, the hash-pinned copy
above when none is current), but no compiler, Docker, system Node or registry
credentials. They verify the inventory's publisher, release
workflow, tag and source commit before fetching or executing code, then check
each archive against its unique checksum entry. They add `synveda` to your
PATH; on Windows they also set the user-level `SYNVEDA_CLI` that agent hooks
need and never change the execution policy. The Compose deployment commands
remain Unix-only.

All six native archives are attached to this release:

| System | x64 | ARM64 |
|---|---|---|
| Linux (glibc) | `synveda-client-{{version}}-linux-x86_64.tar.gz` | `synveda-client-{{version}}-linux-arm64.tar.gz` |
| macOS | `synveda-client-{{version}}-darwin-x86_64.tar.gz` | `synveda-client-{{version}}-darwin-arm64.tar.gz` |
| Windows | `synveda-client-{{version}}-windows-x86_64.zip` | `synveda-client-{{version}}-windows-arm64.zip` |

The four native Unix client archives include private Node; their
checksummed `synveda-client-report-*.json` files record restricted-PATH
installation, packaged authentication lifecycle and extracted hook replay. These checks do not establish
native vendor loading or real issuer login from the client archives.
The two native Windows ZIPs include private Node and PowerShell
installation. Their reports require native Rust/Node private storage,
retained-state reinstallation and unsafe ZIP/installer refusals.

Then sign in to your server as the
[agent setup guide](https://github.com/{{repository}}/blob/{{source}}/docs/AGENT_SETUP.md)
describes and, from the root of the repository you work in, give an agent your
team's governed memory:

```sh
synveda setup --project PROJECT_ID --observation on
synveda adapter install --client claude-code   # or codex, copilot-cli, cursor
```

Add `--dry-run` to `adapter install` to preview its changes.

The historical macOS ARM64 and Linux x64 server/CLI archives, console archive
and system-Node plugin archive remain available as separate assets. The shell
installer's `reference` mode retains that historical two-platform contract;
the default `client` mode installs the six-platform packages above.

Consumer images: `docker.io/{{namespace}}/product:{{version}}`
and the other five images recorded in the attested registry inventory.
Both registries passed anonymous native pulls. Full deployment checks ran on
the exact native OCI candidates before their digests were copied to Docker Hub
and GHCR. The published OCI chart was downloaded anonymously and compared byte
for byte with the release asset. GHCR mirrors include
`ghcr.io/synveda/product:{{version}}`,
`ghcr.io/synveda/postgres:{{version}}`, and
`ghcr.io/synveda/cnpg-postgres:17.11-synveda-{{version}}`.
Additional GHCR copies are
`ghcr.io/synveda/keycloak:{{version}}` and
`ghcr.io/synveda/proxy:{{version}}`.
Helm chart: `synveda-{{version}}.tgz`.

Kubernetes uses the checksummed `synveda-images-{{version}}.yaml`
overlay; CNPG additionally uses `synveda-cnpg-image-{{version}}.yaml`.
After publisher/checksum verification, extract the chart and follow its
`README.md` for this release's Kubernetes installation and first-use guides.
The archive includes its locked dependency, customer presets and preparation
tools; installing a release requires no chart compilation or dependency download.
BuildKit emits image provenance and SBOM attestations. Publication retains
their descriptors when copying the tested OCI candidates; the attached native
deployment reports separately record installation and recovery qualification.

See [docs/INSTALL.md](https://github.com/{{repository}}/blob/{{tag}}/docs/INSTALL.md).
