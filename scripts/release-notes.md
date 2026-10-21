This is a **controlled evaluation release**. Customer-operated PostgreSQL
14, 15, 16, 17 and 18 are now admitted with the same build and installation
workflow:
- the server major is read before any version-sensitive SQL, and any other
  major is refused before a change;
- a fresh database installs one byte-pinned portable baseline, while an existing
  PostgreSQL 17 database converges through the append-only `0011` migration
  without rewriting its rows;
- `synveda db setup-sql`, or the product image's `database-setup-sql`
  command, renders the DBA's rerun-safe provisioning script;
- PostgreSQL 14 and 15 cannot represent ADMIN-only role memberships, so declare
  none there and manage the roles from a declared administrator.

The bundled quick start, CNPG image and backup/PITR stay on PostgreSQL 17.11.
Compose external mode with the bundled Keycloak is qualified on 17 only;
managed providers remain unqualified. PostgreSQL 14 reaches upstream end of
life on 12 November 2026; nothing refuses by date.

Bundled, CNPG and fresh installations now use **pgvector 0.8.7**, which fixes
CVE-2026-103484 in the native library. Earlier admitted versions (0.8.2–0.8.6)
remain a legacy exception until 2027-01-31, and `synveda db migrate` warns
while one is in use. For a retained database, follow the
[retained pgvector transition](https://github.com/{{repository}}/blob/{{source}}/docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition).
Ordinary startup never updates the extension catalogue. After the DBA's
`ALTER EXTENSION vector UPDATE TO '0.8.7'`, earlier releases refuse the
database, so keep the pre-transition recovery set.

Knowledge reads no longer use views; their row security is evaluated as the
caller. Invitation acceptance and multi-user onboarding are repaired.
Cedar, RLS, VedaFlow and audit behaviour are otherwise unchanged.

Version: `{{version}}`. Exact release source: `{{source}}`.

Local OrbStack acceptance passed namespace-scoped installation, real browser
and native CLI login/context, retained reinstall and the governed first-use
walkthrough. Its exact source/runtime evidence is recorded in
[OPS-11](https://github.com/{{repository}}/blob/{{source}}/docs/backlog/OPS-11.md#orbstack-local-acceptance-checkpoint-2026-10-04).
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
Source tests converge a retained published ledger through `0011`; see
[OPS-11](https://github.com/{{repository}}/blob/{{source}}/docs/backlog/OPS-11.md#postgresql-1418-compatibility-checkpoint-2026-10-18)
for the PostgreSQL 14–18 evidence and limits.
Keep the same runtime selection when updating an existing deployment.
Back up both databases and matching keys first. Application/Helm
rollback does not reverse database migrations.

To install only the CLI and adapters on Linux or macOS:

```sh
curl -fL https://raw.githubusercontent.com/{{repository}}/{{source}}/scripts/install.sh -o synveda-install.sh
# Inspect synveda-install.sh before running it on a client machine.
SYNVEDA_INSTALL_MODE=client SYNVEDA_VERSION={{tag}} \
  SYNVEDA_SOURCE_SHA={{source}} sh synveda-install.sh
```

All six native archives are attached to this release:

| System | x64 | ARM64 |
|---|---|---|
| Linux (glibc) | `synveda-client-{{version}}-linux-x86_64.tar.gz` | `synveda-client-{{version}}-linux-arm64.tar.gz` |
| macOS | `synveda-client-{{version}}-darwin-x86_64.tar.gz` | `synveda-client-{{version}}-darwin-arm64.tar.gz` |
| Windows | `synveda-client-{{version}}-windows-x86_64.zip` | `synveda-client-{{version}}-windows-arm64.zip` |

The installers require a trusted GitHub CLI for publisher verification. They
need no compiler, Docker, system Node or registry credentials. They verify the
inventory's publisher, release workflow, tag and source commit before fetching
or executing code, then check each archive against its unique checksum entry.
The four native Unix client archives include private Node; their
checksummed `synveda-client-report-*.json` files record restricted-PATH
installation, packaged authentication lifecycle and extracted hook replay. These checks do not establish
native vendor loading or real issuer login from the client archives.
The two native Windows ZIPs include private Node and PowerShell
installation. Their reports require native Rust/Node private storage,
retained-state reinstallation and unsafe ZIP/installer refusals.
On Windows, download and inspect the matching PowerShell installer:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/{{repository}}/{{source}}/scripts/install.ps1 -OutFile synveda-install.ps1
# Inspect this script; the installer does not change execution policy.
& ./synveda-install.ps1 -Version {{version}} -SourceSha {{source}}
```

On Windows, Claude plugin registration prints the user-level `SYNVEDA_CLI`
assignment that its hooks need. Codex/Copilot registration and vendor trust
remain manual, and the Compose deployment commands remain Unix-only.

Then give an AI client your team's governed memory:

```sh
synveda plugin install                        # Claude Code
synveda mcp install --client claude-desktop   # or cursor, or zed
```

The historical macOS ARM64 and Linux x64 server/CLI archives, console archive
and system-Node plugin archive remain available as separate assets. The shell
installer's default `reference` mode retains that historical two-platform
contract; use explicit `client` mode for the six-platform packages above.

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
