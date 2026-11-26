This is a **controlled evaluation release**. Its read-only migration check
refuses every open transition, and `upgrade` runs that check after a failure
in its `migrating` phase. Both close gaps that the published v0.4.10 →
v0.4.11 upgrade drill found.

**Upgrading from v0.4.11 is a planned maintenance interruption.** The gateway,
worker and bundled Keycloak stay stopped from writer stop until the new
release is ready. There is no zero-downtime, mixed-version or down-migration
path. The [upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md)
proposes a 15-minute window for the bundled single-host route; no maximum has
been set, and larger databases are not measured.
- Schema epoch 3 and the required head `0021` are unchanged. This upgrade runs
  no migration.
- From the extracted v0.4.12 bundle, `./synveda-compose upgrade before-0-4-12`,
  with `SYNVEDA_UPGRADE_FROM` naming the installed v0.4.11 bundle and
  `SYNVEDA_CONFIRM_UPGRADE=upgrade:synveda-evaluation:before-0-4-12`, backs up
  and restarts the loopback evaluation on the new release. A rerun resumes from
  the last completed phase. A failure after `migrating` still says the schema
  change began, although this upgrade runs no migration; rerun to resume.
- Rollback means restoring the pre-upgrade set with the v0.4.11 bundle.
- The guide's table supports v0.4.11 → v0.4.12 only. From v0.4.10, upgrade to
  v0.4.11 first. Finish an interrupted v0.4.10 → v0.4.11 upgrade with v0.4.11,
  even though v0.4.12's check names its own migrate.

**The v0.4.11 → v0.4.12 pair has not been drilled from published bundles, and
Kubernetes upgrades are not yet qualified.**

**Behaviour changes.**
- `synveda db migrate --check`, the bundle's `migration-check`, exits 1 for
  every open transition, such as `0019..0021`, and names the interrupted
  migration to finish. v0.4.11 exited 0 when none of the transition's
  migrations had committed. A pass now means a completely stamped head. The
  reference runtime's image switch and PITR restore verification run the
  check, so they stop at an open transition before starting services. A script
  that runs the check before a planned upgrade sees the same refusal.
- When `upgrade` fails in its `migrating` phase, it runs the new release's
  read-only check. If the check passes, no migration is open, and the message
  directs a rerun without saying the schema change began. If it fails, or
  PostgreSQL does not start, the message says a migration may be open and
  keeps the restore alternative.

PostgreSQL 14–18, pgvector 0.8.7 and the dated legacy pgvector profiles are
admitted as in v0.4.11. For a retained database still on an earlier pgvector,
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
