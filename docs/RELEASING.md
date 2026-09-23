# Publishing a consumer release

OPS-8 / OPS-12; [ADR-0115](adr/adr-0115-prebuilt-container-release-verification.md).
The [CI/release guide](CI.md) owns the workflow map, source gate, local checks
and current manual repository settings.
The two-registry and attestation changes are configured, **not published or
qualified**. The published v0.4.0 remains unchanged and uses GHCR, its original
installer and its original checksum-only trust boundary. Never rerun publication
against that version. The [installation guide](../deploy/compose/PREBUILT.md)
continues to describe those existing downloads.

Published v0.4.0 uses `e59284619567d6a13b70ce3f3b3e81121b7621e6`.
Its original 15 public assets and OCI chart matched the qualified bytes on
anonymous download. Its upload-only failure was recovered from the original
workflow artifacts; that workflow remains failed while its installation jobs
and recovered release are verified. v0.3.0 also remains immutable.

## Native CLI release artifacts

The refactored Release workflow requires every archive below and its native
execution report before it can publish a stable release. `VERSION` is the
workspace version without the `v` prefix. None of these client-only archives is
present in the existing v0.4.0 release; they become public assets only after a
new version completes qualification and owner-authorized publication.

| Operating system | CPU | Required GitHub Release asset |
|---|---|---|
| Linux (glibc) | x64 | `synveda-client-VERSION-linux-x86_64.tar.gz` |
| Linux (glibc) | ARM64 | `synveda-client-VERSION-linux-arm64.tar.gz` |
| macOS | x64 | `synveda-client-VERSION-darwin-x86_64.tar.gz` |
| macOS | ARM64 | `synveda-client-VERSION-darwin-arm64.tar.gz` |
| Windows | x64 | `synveda-client-VERSION-windows-x86_64.zip` |
| Windows | ARM64 | `synveda-client-VERSION-windows-arm64.zip` |

CI and Release dry runs also retain these packages as Actions artifacts named
`binaries-TARGET`; those are validation outputs, not public releases. A missing
runner, archive or successful report blocks publication; no target is optional.

OPS-12 adds `synveda-client-VERSION-TARGET.tar.gz` for native macOS/Linux x86_64
and arm64, plus `.zip` for Windows x86_64/arm64. Each contains the existing CLI and adapters plus private Node pinned
by upstream SHA-256 in [the runtime inventory](../scripts/node-runtimes.json),
with Synveda and complete Node licence notices. It contains no server binaries
or deployment bundle. The [candidate guide](CONSUMER_CLI.md#client-archive-candidate)
owns its explicit `SYNVEDA_INSTALL_MODE=client` and PowerShell instructions.

Each native job executes its extracted archive with no Node/Docker on the
installer PATH. Unix jobs replay existing Codex/Copilot fixtures using private
Node. Windows jobs require native Rust/Node storage interoperability, PowerShell
install/reinstall, hook entry-point smoke and unsafe ZIP/installer refusals.
Every platform also reruns the credential-refresh and platform process tests
against the installed executable, using the same tests as source validation.
Assembly requires all six `synveda-client-report-TARGET.json` reports
to match archive digests, source, target and runtime pin. Tagged builds also
require a clean source tree and matching CLI version. Reports enter SHA256SUMS
before attestation and accompany the release assets. A configured job is not
hosted execution evidence. [OPS-12](backlog/OPS-12.md) records local macOS arm64
archive evidence and the exact hosted Windows qualification results. Real
issuer/harness use of these archives and cold network download measurements
remain open. The two historical server binary archives
and system-Node plugin archive retain their existing contract.

## Owner setup

1. Select a Docker Hub namespace you own. Do not assume `synveda` is available.
   Create public repositories named `product`, `postgres`, `cnpg-postgres`,
   `keycloak`, `proxy` and `browser-acceptance`. The last image is required by
   the opt-in sample and acceptance/recovery tour. Keep the upstream Collector
   and Prometheus at their existing reviewed digest pins.
2. Enable immutable version tags on Docker Hub where available. Treat all
   version and architecture tags on both registries as write-once even where
   the registry does not enforce that policy. Restrict writers and protect
   GitHub `v*` tags. There is no `latest` or rolling alias in this release plan.
3. Create an expiring Docker Hub read/write credential without delete permission.
   Use a dedicated publisher with access to the six repositories, or an
   organization access token restricted to those repositories when available.
   A personal access token inherits its user's repository access; its permission
   selector does not scope it to six named repositories. Follow the
   [token setup steps](#docker-hub-token-setup) below and record its owner and
   rotation reminder outside this repository.
   Never paste the credential into an issue, shell argument or committed file.
4. In GitHub repository variables set `DOCKERHUB_NAMESPACE` and
   `DOCKERHUB_USERNAME`. Create the protected environment named `release`,
   require an owner reviewer, restrict its deployment tags, and store
   `DOCKERHUB_TOKEN` as an environment secret. Make GHCR packages public and
   grant this repository write access. `release-dry-run` requires no secret and
   must not hold publisher credentials. Namespace is a repository variable
   because the read-only version job resolves it before publishing jobs begin.
5. Review the pinned `actions/attest` revision and approve GitHub OIDC as the
   publisher identity for `.github/workflows/release.yml` on protected tags.
   The final publishing job alone has attestation and OIDC permissions. No
   signing private key is stored in GitHub. This authenticates the checksum
   inventory; it is not macOS notarization, Authenticode or a complete archive
   SBOM. Those native distribution requirements remain OPS-12 work.
6. Authorize a new unused coordinated version and update the existing versioned
   product/chart/package contracts, including console/adapters and starter image
   versions. Regenerate OpenAPI, console client and SDK contract metadata after
   changing the workspace version. Require normal CI on the intended committed
   source; release qualification does not replace it. The tagged commit must
   have a successful full main-push CI run, including CI Result and no skipped
   jobs. Run the nonpublishing `workflow_dispatch` first. Its optional version
   must match the workspace; it tests native client archives and exact OCI
   candidates through a loopback registry, Compose and Helm. It never logs in,
   publishes to an external registry, signs or creates a Release. Its public
   inventory has `published: false` and cannot pass anonymous release verification.
7. After reviewing local and hosted results, separately authorize the new tag
   push. The existing `v*` workflow is the only publisher. Confirm the protected
   environment approvals, both native image reports, Docker and all four Kind
   ownership-mode reports, and final attestation verification. Only then update
   the publication manifest, README, site and UI to name the new release.

The workflow checks every existing image tag before writing. A missing secret,
private pull, authentication/rate-limit/network error, incomplete architecture,
lost BuildKit attestation or different mirror descriptor fails the release.
An interrupted publication can leave unannounced artifacts. Inspect and retain
those bytes; do not delete/rebuild/overwrite them automatically. Recover using
the original verified artifacts under an explicitly reviewed owner procedure,
or authorize a new version. Do not move a published Git tag.
Stable announcement now follows a draft upload and verification of all 31
expected asset names, sizes and completed upload states. An upload failure
leaves the draft unpublished. For upload-only recovery, use the original
`verified-release-assets` artifact (30-day retention), which includes the
qualified reports and authenticated final checksum inventory, and verify all
checksums/source/image identities. The old checksum-only
v0.4.0 recovery is not a signing bypass for a new release.

Docker documents [multiple registry exports](https://docs.docker.com/build/ci/github-actions/push-multi-registries/),
[personal tokens](https://docs.docker.com/security/access-tokens/personal-access-tokens/)
and [organization tokens](https://docs.docker.com/security/access-tokens/organization-access-tokens/).
The workflow retains the existing reviewed Docker action pins rather than
upgrading them as part of this change.

## Docker Hub token setup

**A Docker Hub token is required for tagged publication.** CI and Release's
nonpublishing manual dry run need no publishing credentials. Consumers pull
public images anonymously and must not receive this token. A successful dry
run does not prove Docker Hub write access.

1. In [Docker Home](https://app.docker.com/), sign in as the publishing Docker
   user. Open your avatar → **Account settings** → **Personal access tokens** →
   **Generate new token**. Give it a description such as `synveda-release`, set
   an expiration date, and select **Read & Write**, without Delete. Generate and
   copy the token once. The user must have push access to the public `product`,
   `postgres`, `cnpg-postgres`, `keycloak`, `proxy` and `browser-acceptance`
   repositories in your selected namespace. See Docker's
   [personal token instructions](https://docs.docker.com/security/access-tokens/personal-access-tokens/).
2. In [Synveda's Actions variables](https://github.com/synveda/synveda/settings/variables/actions),
   use **New repository variable** to add:

   | Variable | Value |
   |---|---|
   | `DOCKERHUB_NAMESPACE` | The Docker Hub user or organization owning the six repositories; no `docker.io/` prefix |
   | `DOCKERHUB_USERNAME` | The Docker ID of the user who created the personal token; this can differ from the namespace |

   Keep these at repository scope: the source/version job reads the namespace
   before any protected publishing job starts. If you instead use an
   [organization access token](https://docs.docker.com/security/access-tokens/organization-access-tokens/),
   set `DOCKERHUB_USERNAME` to the organization name and grant that token
   read/write access to the six repositories.
3. In [repository Settings → Environments](https://github.com/synveda/synveda/settings/environments),
   create **release**. Add an owner as a required reviewer. Under deployment
   branches and tags, select **Selected branches and tags**, add a **Tag** rule
   matching `v*`, and save it. Under **Environment secrets**, choose **Add
   secret**, name it **DOCKERHUB_TOKEN**, and paste the token directly there.
   Do not put it in a variable, repository file, command argument or chat.
   GitHub documents [environment protection](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments)
   and [environment secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets).
4. Make the six GHCR image packages and `ghcr.io/synveda/charts/synveda` public,
   and grant this repository Actions write access. GHCR and GitHub Release
   publication use the automatic `GITHUB_TOKEN`; no additional GitHub PAT is
   needed. Complete the tag protection, CI Result and permissions settings in
   [the CI guide](CI.md#manual-owner-settings).

After the environment exists, the equivalent CLI commands for the variables
and secret are below. Replace the two nonsecret placeholders. The last command
prompts privately for the token; it does not configure environment protection:

```sh
gh variable set DOCKERHUB_NAMESPACE --repo synveda/synveda --body YOUR_NAMESPACE
gh variable set DOCKERHUB_USERNAME --repo synveda/synveda --body YOUR_DOCKER_ID
gh secret set DOCKERHUB_TOKEN --repo synveda/synveda --env release
```

Rotate the token before expiry by replacing this environment secret. Keep
`release-dry-run` free of publishing credentials. Authorize a new coordinated
version and tag only after the source CI and dry-run results have been reviewed;
never use a real release to test whether the secret was added correctly.

## Artifacts and verification

The complete stable inventory is 31 assets: six native client archives and six
matching reports; the two historical server archives; console, plugin and
reference archives; the Helm chart and two image overlays; the two-registry
inventory; eight native image/Compose/consumer/Helm qualification reports; and
`SHA256SUMS` plus `SHA256SUMS.sigstore.json`. The executable inventory is
[`scripts/check-release-assets.mjs`](../scripts/check-release-assets.mjs).

Each native AMD64/ARM64 image is built once per run from the same source and
lockfiles into an OCI archive with source/revision/version labels, SBOM and
provenance. Read-only jobs test those exact archives before publishing jobs
copy them with preserved digests to both registries. Artifacts must come from
the same release run; PR artifacts and cross-run promotion are not accepted.
Each registry's final multi-platform index is independently
assembled, inspected and hashed. The versioned `synveda-registry-images-*.json`
records both destinations and their complete child descriptors. The consumer
reference archive, its `environment.json`, launcher and Helm overlays use
the **Docker Hub destination digests**. The chart's unconfigured source defaults
remain GHCR because the owner namespace is unknown until release assembly.
Use the release's immutable overlay when installing the packaged chart.

Native verification starts with an empty Docker credential store. It checks
both registries, including descriptor parity, anonymous digest pulls, labels
and executable assets. The full existing Docker lifecycle/browser login/sample/
paired-recovery and Kind checks then consume the Docker Hub bundle. The archive
also includes the plain-Compose candidate and `synveda-recovery`. Both native
jobs must run `qualify-release.mjs --consumer-candidate` against those extracted
bytes. The required `release-consumer-{amd64,arm64}.json` reports cover stopped
writers, a private empty-target restore, original sealed keys, audit/key
verification, real browser access and the original sample receipt. They join
the checksummed, attested release inventory; a missing or failed report blocks
publication. The host-state Docker and Kind gates remain required. These
reports distinguish image smoke from full deployment evidence; they do not
qualify Windows, Docker Desktop, OpenShift or an N-1 migration.

The full drills are `scripts/qualify-release.mjs` and
`scripts/qualify-kubernetes-release.mjs`. They require Docker, Helm 4.2.3,
Kind 0.32.0, kubectl 1.36.1, Node, Ruby, Python and OpenSSL on the qualification
runner. They do not build images. Configure public visibility and repository
Actions access for `ghcr.io/synveda/charts/synveda` too: anonymous OCI retrieval
must be byte-identical to the downloadable chart. The source checkout's
`assets/brand` and `assets/product` directories are never upload candidates.

After those gates, GitHub attests the final `SHA256SUMS`, which covers all
archives, overlays, destination digests and native reports. The downloadable
`SHA256SUMS.sigstore.json` carries that attestation. For a future approved
release, verify it with a current trusted GitHub CLI before trusting its
checksums. Supply the exact approved tag and source commit from the release:

```sh
gh attestation verify SHA256SUMS --bundle SHA256SUMS.sigstore.json \
  --repo synveda/synveda \
  --signer-workflow synveda/synveda/.github/workflows/release.yml \
  --source-ref "refs/tags/$RELEASE_TAG" --source-digest "$SOURCE_SHA" \
  --deny-self-hosted-runners
```

Then verify the desired downloaded archive against exactly its checksum entry
before extracting it. A checksum from an unauthenticated download does not
establish publisher identity. Failed or absent attestation is not permission to
skip verification. GitHub documents [attestation verification](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

The current installer does not yet enforce this new attestation boundary;
integrating verification without imposing GitHub CLI or Docker on client-only
installation remains open. Do not advertise this source installer as an
authenticated consumer installer. Artifact authentication also does not prove
OS code signing, absence of vulnerabilities or safe data migration. An owner
incident response must revoke publishing access, identify affected immutable
digests/tags and publish a reviewed replacement; never silently replace bytes.

Homebrew and WinGet publication remain blocked until the native direct artifact
flow is qualified. Owners must supply signing/notarization identities where
needed and authorize package-manager submissions separately. No package-manager
install command or Docker Hub release is advertised before it exists.
