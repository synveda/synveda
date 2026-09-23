# CI and release guide

FND-1 / OPS-8 / OPS-12; [ADR-0108](adr/adr-0108-ci-gate-ownership-and-build-caching.md).
The refactor starts from clean commit `e3cb694e88710eb971d2e3676b82a82982924064`.
The pipeline refactor initially left product versions, lockfiles, generated APIs,
test baselines and published releases unchanged. A subsequent 0.4.1 release
candidate coordinates those versioned contracts; published v0.4.0 remains
immutable. CI, Release and Extended Tests are the entry points; Pages remains
independent. Only CLI and Docker share reusable jobs between CI and Release.

## Before / after coverage

| Existing job | Purpose | Current stage / job | Retained coverage |
| --- | --- | --- | --- |
| CI `layering` | Static contracts | Check; deployment Check | Dependencies, types/OpenAPI/client parity, docs, backlog/ADRs, adapters/security boundaries, corpora, benchmarks; chart checks move to deployment Check |
| CI `licences` | Rust dependency policy | Check — dependency policy | Same cargo-deny sources, advisories and licence policy |
| CI `rust` | Rust validation | Build and Test — Rust | Formatting, strict Clippy, workspace tests **and** separate production-feature build; executable docs |
| CI `typescript` | JS/console/adapters | Build and Test — TypeScript | Same non-SDK tests, console build, plugin packaging, npm licences |
| CI `sdk-interop` ×2 | Source/installed SDKs | Test — SDKs ×2 | Node 22/Python 3.11 with real interop; Node 24/Python 3.14; hash-locked wheels |
| CI `claude-replay` | Captured native frames | Test — Claude replay | Same exact-role database and governed replay |
| CI `eval` | Deterministic quality/security | Test — deterministic evaluation | Same corpus, baselines and 400 security variants; reports on failure |
| CI `beta-demo` | Product walkthrough | Test — product walkthrough | Same PulseBoard database acceptance |
| CI `release-install` | Packaging/deployment contracts | Check — deployment and packaging | Same `check-deploy`; chart lint/render/locked dependencies and Kubernetes/OpenShift schemas |
| CI `kind` | CNPG installation | Helm — CNPG install and failover | Same live install, failover, RLS/PDP and OIDC assertions |
| CI `kind-operations` ×2 | Operations | Helm — operations ×2 | CNPG/packaged and external/external interruption, upgrade/recovery; external TLS/issuer negative cases |
| CI `windows-client-boundary` ×2 | Native Windows boundary | CLI shared jobs | All native Clippy/storage/refresh/private-state tests; release-profile ZIP and installed-binary auth added |
| Release `binaries` ×4, `windows-clients` ×2 | Native packaging | CLI shared jobs in CI and Release | Linux/macOS/Windows x64 and ARM64, private Node checksums, packaged adapter/auth lifecycle; Release retains both historical server archives |
| Release `version` | Source/version choice | Check release source | Exact checkout/version plus successful full main CI for that exact commit before publishing |
| Release `bundles` | Frontend/plugin/chart | Build release bundles | Same locked console, plugin and chart packages; existing vendor validator remains advisory |
| Release `images` ×2 | Six native images, two registries | Docker candidate validation; Publish Docker candidates | Same Dockerfiles, targets, labels, SBOM/provenance; tested OCI archives copied to Docker Hub and GHCR without rebuilding |
| Release `assemble` | Indexes and asset inventory | Assemble release assets | Independently inspected registry digests, reference bundle/overlays, byte-identical OCI chart; all six client reports mandatory |
| Release `verify-images` ×2 | Anonymous distribution/install | Test published Docker and Helm artifacts | Both registries, exact digest smoke, extracted Compose lifecycle/recovery and all four independent PostgreSQL/Keycloak modes; anonymous chart comparison |
| Release `publish` | Attest and announce | Publish verified release | Exact-source attestation and checksum inventory; draft stays unpublished until all 31 assets upload successfully |
| Eval `eval`, `retrieval`, `security` | Scheduled deeper evaluation | Extended Tests (same `eval.yml`) | Same nightly deterministic, real TEI retrieval and 10,000-variant security gates |
| Pages `build` ×2, `deploy` | Website | Pages unchanged | Native amd64/arm64 site/brand/docs checks; main-only deployment |

**Verified duplication removed:** chart lint/render ran in `layering` and both
operations jobs; it now runs once in deployment Check. CI and Release now share
the actual CLI job definitions. Docker publication reuses its own run's tested
OCI files. The old dry run built images but skipped executable installation;
the new dry run tests local candidates and never labels them public evidence.

**Suspected duplication retained:** Kind builds overlap Docker candidate builds,
but CNPG failover and TLS/issuer negatives are distinct from bundled PostgreSQL
qualification. Rust tests enable features absent from the production build;
source and installed SDK checks differ. Those checks remain. Cache compatibility
across these jobs has not been demonstrated.

## Selection and the required gate

CI runs on PRs, main pushes, manual dispatch and `merge_group`. Feature-branch
pushes do not create duplicate CI runs. Only superseded PR runs are cancelled.
Main, manual and merge-group runs select everything. PR selection uses the full
Git merge-base diff, including both sides of renames/deletions; failed comparison,
an empty diff or any unknown/shared input selects everything.

The small allowlist in [ci-select.mjs](../scripts/ci-select.mjs) skips expensive
jobs only for identified prose; website changes still select TypeScript and
Pages; console/assets select Rust, TypeScript, deployment, Docker and Helm.
Check and dependency policy always run. CI Result requires their success and
every selected job's success. Missing outputs, cancellation, failure, unexpected
skips or a changed dependency inventory fail the gate.

Caches are optional: lockfiles remain required; cache hits never skip tests.
Rust/Kind/candidate build caches are saved only by main pushes. Candidate jobs
reclaim their dedicated builder layers after OCI export, before installation.
Keep production
build and tests on one runner; do not transfer incompatible Cargo build trees.

## Running and releasing

```sh
make check-fast check-deps check-ci
actionlint -shellcheck=  # actionlint 1.7.7; YAML/expressions, ShellCheck separate
make check-deploy chart-lint
KUBECONFORM=/path/to/kubeconform make chart-schema  # v0.7.0
cargo fmt --all --check
SQLX_OFFLINE=true cargo clippy -p synveda-cli --all-targets --locked -- -D warnings
SQLX_OFFLINE=true cargo test -p synveda-cli --locked --test credential_refresh --test client_platform
```

Release dispatch on a chosen ref is a **nonpublishing** packaging drill. Its
optional version must equal the workspace version. Both native Docker jobs
build six OCI archives, import them into a loopback registry, test immutable
digests and run the documented Compose and four-mode Helm install/upgrade/recovery
commands. All six native client packages must pass. Public registry inventories
are synthetic in dispatch; public pulls, registry writes, signing and release
creation do not run. Full deployment drills require fresh disposable hosts.

A `v*` tag must match the workspace version and identify an ancestor of main
with a successful **main-push CI run for that exact SHA**, including CI Result
and no skipped jobs. Wait for that CI run before tagging. Publication downloads
only artifacts produced in the same release run; no PR/cross-run artifact is
promoted. Registry jobs copy hash-checked OCI files, then assemble both platforms.
Fresh runners retain anonymous distribution checks. All 29 payloads enter the
attested SHA256SUMS; the inventory and its attestation make 31 public assets.
Upload failure leaves a draft. `verified-release-assets` retains the signed set
for 30 days; candidate OCI files retain seven days and diagnostic reports 14.
Follow [RELEASING](RELEASING.md) for reviewed recovery and immutable-tag rules.

## Evidence and remaining qualification

Read-only Actions inspection on 2026-09-22 found no merge queue or required status
checks: the active `main` ruleset only prevents deletion and force pushes.

| Before evidence | Observed duration / result |
| --- | --- |
| [CI 35655475135](https://github.com/synveda/synveda/actions/runs/35655475135), starting SHA | 30m01s overall; external operations 29m56s, Windows ARM64 25m47s, CNPG 17m23s, Rust 6m54s |
| [Release 35525132082](https://github.com/synveda/synveda/actions/runs/35525132082), older published SHA | 72m56s; upload-only failure, native installation verification passed (64m40s / 65m45s) |
| [Dry run 35522831120](https://github.com/synveda/synveda/actions/runs/35522831120), older workflow | 19m16s; native image builds 18m12s / 15m54s, verification jobs 11s because installation was skipped |

Local refactor evidence: actionlint and workflow mutation checks; representative
prose/console/website/shared/unknown/rename/deletion/350+ path selections; an
actual intentionally failing required test makes CI Result exit 1; release
source, missing-platform, changed-archive, failed-report and partial-upload
refusal tests. A disposable ARM64 OCI transport fixture retained its image and
BuildKit attestation descriptor through two digest-preserving registry copies
and a native Docker pull. This checks transport, not full product installation.
The macOS ARM64 candidate archive passed restricted-PATH install,
repeat installation, adapter replay and seven auth/platform process tests against
the installed binary. It used a debug CLI and a dirty checkout, so it is local
test evidence, not releasable provenance.

`make check-fast check-deps check-ci` and the complete
`make check-deploy chart-lint` run passed after updating the image-discovery
and release-note checks to follow their new files. HTTP fixtures ran with
loopback access. All pinned Kubernetes/OpenShift schema renders passed.
Formatting, strict CLI Clippy and the focused native Rust tests also passed.

The first hosted [PR CI run](https://github.com/synveda/synveda/actions/runs/35827387823)
and [nonpublishing Release run](https://github.com/synveda/synveda/actions/runs/35827409973)
on `34355d7` exercised the new jobs. macOS and Windows client packages and the
ARM64 Docker/Helm candidate passed. Linux x64/ARM64 packaging refused Cargo's
hard-linked executable input; the workflow now packages a detached copy without
relaxing that check. AMD64 Docker reached Compose installation, where bundled
Keycloak realm convergence became unhealthy; bounded, redacted diagnostics were
added before exact-project cleanup. CI Result failed and Release's downstream
publication jobs skipped as designed.

The second hosted attempt on `3a4da43` passed Linux archive packaging but
found that the restricted installer test PATH omitted `gzip`, which GNU tar
invokes for `.tar.gz`. That standard decompressor is now included while system
Node, Docker and build tools remain excluded. The third
[PR CI run](https://github.com/synveda/synveda/actions/runs/35840850084)
passed both Linux client archive checks but again reached an unhealthy AMD64
realm gate after roughly ten minutes. At failure, generation capture and the
Keycloak management network probe both passed; the supervisor had no logs.
The evaluation healthcheck's five-minute start period plus retries ended around
the old 600-second launcher limit. The launcher and derived plain-Compose
candidate now use a 900-second wait; their evaluation healthcheck remains in
`starting` through that bound. This extends time for CPU-constrained first-run
identity reconciliation without changing the complete generation readiness
gate. The documented direct-Compose and recovery commands use the same bound.
The fixture retains bounded process-name diagnostics if it still fails.

The final [PR CI run](https://github.com/synveda/synveda/actions/runs/35844879556)
on `47fb126` passed **CI Result**, all six native CLI packages, both Docker
Compose and four-mode Helm candidates, Rust, TypeScript, SDKs, replay, evaluation,
walkthrough and deployment checks. It ran 09:46–11:53 UTC (2h07m). PR artifacts
were built from GitHub's synthetic merge commit and cannot be released. The
[nonpublishing Release dispatch](https://github.com/synveda/synveda/actions/runs/35845344195)
on the exact clean source commit `47fb1262d616cb417774ff9d9cc988a535e48b39`
also passed all six packages and both Docker/Helm candidate jobs (09:50–12:18
UTC, 2h27m). Each native candidate report records seven launcher Compose checks,
20 plain-consumer/recovery checks and all four Helm modes. The dry-run
`release-assets` artifact contains 21 payloads plus `SHA256SUMS`; all 21
checksums were verified after download. Its registry inventory explicitly says
`published: false`. The eight public-distribution verification reports and
checksum attestation only exist on a successful tagged publication path; that
path must verify all 31 assets before making the draft stable.

The final PR ARM64 fresh launcher took 577s, and AMD64 took 769s; the exact-source
Release dry run took 588s and 769s respectively. An older published run's
fresh launcher took 574s and 538s. Runner load, source and candidate scope
differ, so these data do not establish a performance saving. The prior 19-minute
dry run skipped installation; it is not comparable with the new 2h27m full
candidate drill. No cold-cache, fork-PR or real registry-write performance
claim is made. A tagged publication and fresh anonymous public pulls remain
unexercised by instruction; local retained evaluation volumes were untouched.

No new release, tag, registry write or protection change was performed. The
existing v0.4.0 public release does not contain the six new native client-only
archives; download the validated dry-run packages from Actions until a new
release is separately approved.

## Manual owner settings

1. After a successful hosted run, add required status check **CI Result** (GitHub
   Actions) to the main ruleset/branch protection. Require branches up to date,
   or deliberately enable merge queue; `merge_group` already selects all stages.
   Do not require individual matrix job names. No required checks currently exist.
2. Repository variables: `DOCKERHUB_NAMESPACE` (owned namespace) and
   `DOCKERHUB_USERNAME` (publisher). Both variables were configured on
   2026-09-23. Create or verify the six public Docker Hub repositories named in
   [RELEASING](RELEASING.md#owner-setup). Follow the exact
   [Docker Hub token setup](RELEASING.md#docker-hub-token-setup) for an expiring
   read/write token without delete permission and the correct publisher username.
3. Environment **release** now has an owner reviewer, a `v*` tag rule and the
   **DOCKERHUB_TOKEN** secret. Protect creation/update/deletion of release tags
   with an owner-controlled tag ruleset before publication. **release-dry-run**
   needs no secrets or approval and was exercised separately.
4. Give this repository's Actions token write access to the six GHCR packages
   and `ghcr.io/synveda/charts/synveda`; make all public for anonymous verification.
   Permit pinned Actions and workflow-scoped `packages: write`, `contents: write`,
   `id-token: write` and `attestations: write`. Approve the documented GitHub OIDC
   publisher identity; no signing key or PAT is needed for GitHub publication.
5. Keep Linux ARM64, macOS Intel/ARM64 and Windows x64/ARM64 hosted runners
   available. A disabled/unavailable runner blocks its required platform; never
   change it to an allowed failure. Existing OS signing, notarization, installer
   attestation enforcement and real client issuer/harness gaps remain OPS-12 work.

The [native CLI asset table](RELEASING.md#native-cli-release-artifacts) names
every required public package. The existing v0.4.0 release has neither those
six client-only archives nor the new authenticated checksum inventory; the next
approved publication must satisfy the complete asset gate.
