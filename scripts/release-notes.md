This is a **controlled evaluation release**. It adds governed business links
between Knowledge items, which recall shows and context-run expansion can
follow. Graph expansion now follows a relation only where its step would hold
on recheck and replay. It also clears a Rust security advisory.

**Upgrading from v0.4.10 is a planned maintenance interruption.** The gateway,
worker and bundled Keycloak stay stopped from writer stop until the new
release is ready. There is no zero-downtime, mixed-version or down-migration
path. The [upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md)
proposes a 15-minute window for the bundled single-host route; no maximum has
been set, and larger databases are not measured.
- Schema epoch 3 is unchanged. The required head moves from `0019` to `0021`:
  `0020` adds the business-link tables and `0021` lets a retained graph step
  name a link. Both are identical on the published and portable lineages.
- This is the first published pair whose migrator opens a transition. Before
  its first schema change it rewrites the head as `0019..0021`, which every
  earlier release refuses.
- From the extracted v0.4.11 bundle, `./synveda-compose upgrade before-0-4-11`,
  with `SYNVEDA_UPGRADE_FROM` naming the installed v0.4.10 bundle and
  `SYNVEDA_CONFIRM_UPGRADE=upgrade:synveda-evaluation:before-0-4-11`, backs up
  and restarts the loopback evaluation on the new release. A rerun resumes from
  the last completed phase; once `migrating` has started, rerun rather than
  restart v0.4.10.
- Rollback means restoring the pre-upgrade set with the v0.4.10 bundle.
- The guide's table supports v0.4.10 → v0.4.11 only. From v0.4.9, upgrade to
  v0.4.10 first.

**The v0.4.10 → v0.4.11 pair has not been drilled from published bundles, and
Kubernetes upgrades are not yet qualified.**

**Business links** (ADR-0164).
- An entity Knowledge item can declare a class in `metadata.synveda_entity`,
  as a compact IRI from schema.org, W3C ORG, PROV-O or a product term.
  Configuration's `entity_classes` enables a subset.
- `POST /v1/knowledge/{id}/links` joins two items with one of eleven governed
  predicates, each with the endpoints it admits, as a reviewed Knowledge
  command. `POST /v1/knowledge/{id}/links/{link_id}/end` ends a link, and
  `GET /v1/knowledge/{id}/links` lists them. Configuration's
  `entity_predicates` enables a subset. An item has at most 1,000 links in its
  lifetime, ended ones included.
- Recall hits at summary and full depth carry up to 16 current links whose
  other end the caller may read. `synveda recall` and the MCP recall text
  list them.
- With Configuration's `context.graph.include_entity_links` on, context-run
  expansion follows links as well as relations, under the same bounds.
- All three settings are off by default and left out of canonical bytes, so
  existing Configuration hashes do not move.

**Behaviour changes.**
- Graph expansion follows a relation only when the caller may read the
  revision that asserted it and the supporting end's current revision holds
  at the run instant. Meeting an unreadable asserting revision raises the
  run's policy flag.
- An anchor keeps a `contradicts` warning only when the warning's other end
  is current active Knowledge at the run instant.
- Both changes stop a fresh delivery from carrying a step that its own replay
  would refuse.
- A revision's `metadata.synveda_entity` is now validated. A value that is not
  a version 1 class declaration, a class on an item that is not an entity, or
  a class the scope's Configuration does not enable is refused.

**Security.** url is upgraded to clear RUSTSEC-2026-0592.

PostgreSQL 14–18, pgvector 0.8.7 and the dated legacy pgvector profiles are
admitted as in v0.4.10. For a retained database still on an earlier pgvector,
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
