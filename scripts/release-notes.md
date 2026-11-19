This is a **controlled evaluation release**. It makes an interrupted upgrade
safer to recover: the next schema change leaves the database in a state no
earlier release will serve, and a failed Keycloak realm convergence is retried
instead of needing `stop`. It also makes progressive context disclosure
cheaper to read, refuses a bad query-embedding deadline, fixes a worker
shutdown race and clears a Rust security advisory.

**Upgrading from v0.4.9 is a planned maintenance interruption.** The gateway,
worker and bundled Keycloak stay stopped from writer stop until the new
release is ready. There is no zero-downtime, mixed-version or down-migration
path. The [upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md)
proposes a 15-minute window for the bundled single-host route; no maximum has
been set, and larger databases are not measured.
- Schema epoch 3 and the required head `0019` are unchanged. This upgrade runs
  no migration.
- From the extracted v0.4.10 bundle, `./synveda-compose upgrade before-0-4-10`,
  with `SYNVEDA_UPGRADE_FROM` naming the installed v0.4.9 bundle, backs up and
  restarts the loopback evaluation on the new release. A rerun resumes from
  the last completed phase.
- Rollback means restoring the pre-upgrade set with the v0.4.9 bundle.
- The guide's table supports v0.4.9 → v0.4.10 only. From v0.4.8, upgrade to
  v0.4.9 first.

**The v0.4.9 → v0.4.10 pair has not been drilled from published bundles, and
Kubernetes upgrades are not yet qualified.**

**Upgrade recovery.** The published v0.4.8 → v0.4.9 drill found two gaps that
this release closes.
- Before its first schema change, the migrator now rewrites the head as an
  open transition, such as `0019..0020`, which every earlier release refuses.
  In the drill, the v0.4.8 gateway reported ready on an interrupted upgrade
  whose stamp trailed the committed ledger. Resume an interrupted upgrade with
  the new release's migrator. `migrate --check` now names an open transition,
  a part-way ledger and a trailing stamp. It takes effect from the next
  upgrade that runs a migration.
- The Keycloak realm convergence service retries a failed convergence against
  the same Keycloak start after 10 seconds, doubling the delay up to 5
  minutes. In v0.4.9 it did not retry until Keycloak restarted. Each
  failure's log line adds the attempt number and the next delay, and a failed
  final check names its step, as in
  `open projection was refused at stage client-read`.

**Progressive context disclosure.**
- A manifest entry is one plain line: id, type, sensitivity, title,
  description and token cost. Entries no longer carry a revision, kind or
  channel.
- `knowledge-expand` always serves the current revision and marks an item
  changed since the manifest offered it with `revised_since_offer`.
- The MCP instructions say when to recall and expand, and every miss gives
  one recovery sentence.
- The console's Configuration editor and the Claude Code adapter can ask for
  a manifest.

**Behaviour changes.**
- `knowledge-expand` no longer takes `resolve`, and the MCP `expand` tool
  refuses unknown arguments.
- A manifest saved in v0.4.9's JSON form no longer replays; send it again
  with a new idempotency key.
- The gateway refuses to start when `SYNVEDA_CONTEXT_EMBED_TIMEOUT_MS` is not
  a whole number from 1 to 30,000. v0.4.9 silently used 100 ms. Bounded
  settings and database connection limits now refuse a value that is not
  UTF-8 instead of treating it as unset.

**Operations.**
- The chart gains `gateway.queryEmbeddingTimeoutMs` (1 to 30,000 ms, unset by
  default). The configuration guide explains the 100 ms default deadline for
  a CPU embedding server: how a miss shows, how to measure it and how to fix
  it. The product default is unchanged.
- The gateway and worker install their signal handlers before they bind, so
  a stop signal that arrives just after start is no longer fatal.

**Security.** zip is upgraded to clear RUSTSEC-2026-0548. chrono's
unmaintained advisory, RUSTSEC-2026-0543, is excepted: it reports no
vulnerability, and Cedar, SQLx and other dependencies require chrono.

PostgreSQL 14–18, pgvector 0.8.7 and the dated legacy pgvector profiles are
admitted as in v0.4.9. For a retained database still on an earlier pgvector,
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
and no published N-1 data upgrade is qualified by these checks. The
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
