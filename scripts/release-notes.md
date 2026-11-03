This is a **controlled evaluation release**. It lets the first administrator
choose the organisation's review profile, adds an upgrade command for the
Docker evaluation bundle, and adds request limits, tenant lifecycle commands,
signed-in session management and bounded Capture retries.

**Upgrading from v0.4.7 is a planned maintenance interruption.** The gateway,
worker and bundled Keycloak stay stopped from writer stop until the new
release is ready. There is no zero-downtime, mixed-version or down-migration
path. In the v0.4.6 → v0.4.7 drill, on a small dataset, writers were down for
378 seconds. The [upgrade guide](https://github.com/{{repository}}/blob/{{source}}/docs/UPGRADING.md)
proposes a 15-minute window for the bundled single-host route; no maximum has
been set, and larger databases are not measured.
- Schema epoch 3 is unchanged. Six additive migrations move the required head
  from `0012` to `0018` in place, on both the published and portable lineages.
- From the extracted v0.4.8 bundle, `./synveda-compose upgrade before-0-4-8`,
  with `SYNVEDA_UPGRADE_FROM` naming the installed v0.4.7 bundle, backs up,
  migrates and restarts the loopback evaluation. A rerun resumes from the last
  completed phase.
- The migrator refuses schema changes while gateway or worker sessions are
  still connected, after waiting up to `SYNVEDA_MIGRATION_WRITER_DRAIN_SECONDS`
  (default 120). Lock waits are bounded, so stop long readers such as a
  concurrent dump first.
- After the schema change, rollback means restoring the pre-upgrade set with
  the v0.4.7 bundle; v0.4.7 refuses the upgraded database.

**The v0.4.7 → v0.4.8 pair has not been drilled from published bundles, and no
published chart pair has been drilled on a cluster.**

**Organisation profile.** Onboarding's first step offers the `enterprise`,
`team` and `personal` profiles (the `regulated-strict`, `standard` and
`open-collaboration` packs) and binds the choice at the tenant root, so the
whole tenant follows it. Skipping the step leaves the `regulated-strict`
fail-safe with the choice open. It is offered again on Home and under Advanced
▸ Configuration, and `/v1/me` reports it as `onboarding.profile`. Under
`standard`, a later Configuration change needs an administrator other than its
author; publishing a Skill or tool still needs two approvers other than its
author.

A tenant with no profile bound at its root, including one onboarded under an
earlier release, can still make this first choice. Any one root administrator
can make it without review, including `personal`, and audit records it. To keep
the fail-safe against a single administrator, bind `enterprise` at the root,
which closes the choice.

**Request limits and usage budgets.** The gateway bounds request rates and
concurrent work per principal, tenant and cost class. It refuses excess work
with HTTP 429 `rate_limited` and a `Retry-After`. **Enforcement is on after the
upgrade** (`SYNVEDA_ADMISSION_MODE=enforce`, Helm `gateway.admission.mode`);
`observe` counts without refusing and is the incident rollback. Tenants gain
durable daily meters for session event bytes and ContextRuns. A tenant owner or
administrator reads them at `GET /v1/usage` and sets budgets under
`/v1/usage/budgets/{meter}`.
- The counters live in one gateway process: keep one gateway replica.
- Behind a proxy, set `SYNVEDA_TRUSTED_PROXIES`. The bundled Compose proxy now
  removes client-written forwarding headers and forwards the client address;
  that path has not had a live acceptance run.
- The default limits and budgets are provisional, not service objectives.

**Signed-in sessions.** The console's Settings ▸ Signed-in sessions, and
`synveda auth sessions`, `revoke-session` and `revoke-all-sessions`, list and
end your own console sessions. `synveda auth logout --revoke token|family`
revokes the CLI's credential. Issuer-side revocation is opt-in per issuer; the
bundled Compose and Helm issuer documents do not enable it, and its 30-second
bound is unqualified.

**Tenant lifecycle.** Operators can:
- `synveda tenant suspend` and `resume` a tenant, which refuses its requests,
  sign-ins, stored credentials and later work;
- `synveda tenant archive export`, `verify` and `import` a sealed archive that
  restores the tenant, suspended, into fresh storage;
- `synveda tenant erase` under interim fixture rules.

Erasure is **not production-qualified**, and restoring a backup from before an
erasure brings the erased rows back. The Compose one-shots for these commands
need a source checkout; they are not in this release's reference bundle. See
the [tenant lifecycle guide](https://github.com/{{repository}}/blob/{{source}}/docs/TENANT_LIFECYCLE.md).

**Capture.** Capture now retries only transient provider failures (timeouts,
lost connections and 408, 429, 500, 502, 503, 504 and 529 responses), on a
backoff scheduled in the database. Any other failure closes the batch after one
attempt with a `terminal_reason`. Provider calls are capped across the
installation (`SYNVEDA_CAPTURE_PROVIDER_CONCURRENCY`, default 3); give every
worker the same settings. `synveda session retry-capture` reopens a stopped
batch for one more bounded cycle. The retry values are a proposal, and
multi-node runs are still open.

**Skills and context.**
- `synveda skill import-git FOLDER --scope SCOPE_ID` imports every skill in a
  Git folder from a commit, never the working tree, with Git provenance. It
  needs `git` on PATH.
- The Procedure improvement scan no longer rejects evidence that contains a
  capture event ID or a whole timestamp.
- Dense Knowledge search keeps a stable plan across repeated runs.
- New metrics report database pool waits, pool connections and Knowledge index
  lag. The new [capacity guide](https://github.com/{{repository}}/blob/{{source}}/docs/CAPACITY.md)
  is a planning proposal measured on one host; the 24-hour soak has not run.

**Windows client.** `install.ps1` now:
- names any refused folder and account;
- accepts `-InstallRoot` and the `install.sh` environment names;
- answers authenticated proxies and mirrors;
- without a GitHub CLI of 2.102.0 or later, verifies with a hash-pinned GitHub
  CLI 2.102.0 that it deletes afterwards.

Browser sign-in from the CLI now opens the complete URL. Neither change has
been run on a Windows host.

**Fixes.**
- The OpenAPI document declares the tool-registry list and diff filters as
  query parameters.
- The browser-acceptance fixture's seccomp profile allows `faccessat2`, which
  Docker Engine 29.8 needs to start it.
- RUSTSEC-2026-0387 (crypto-bigint 0.5) is an accepted exception, not a fix:
  only an elliptic-curve path that Synveda's RSA-only token verification never
  runs reaches it.

PostgreSQL 14–18, pgvector 0.8.7 and the dated legacy pgvector profiles are
admitted as in v0.4.7. For a retained database still on an earlier pgvector,
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
Source tests advance both v0.4.7 lineages from `0012` to `0018`; the
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
