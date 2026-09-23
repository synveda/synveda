# Deployment

Public entry points: [Run with Docker](compose/PREBUILT.md) and
[Deploy to Kubernetes](helm/synveda/README.md). The published v0.4.0 remains
immutable. The refactored [CI and Release workflows](../docs/CI.md) prepare the
next release; Docker Hub distribution, attested checksums and six native
[CLI packages](../docs/RELEASING.md#native-cli-release-artifacts) require a new
qualified publication.

This file is the infrastructure-shape overview. Source-checkout operator steps
live in the [canonical Compose guide](compose/README.md); the normative mapping
across deployment shapes lives in the
[deployment contract](../docs/DEPLOYMENT_CONTRACT.md), and unproved operational
claims remain in [production readiness](../docs/PRODUCTION_READINESS.md).

For the next release, see the [Kubernetes release contract](#kubernetes-release-contract).
The [portable chart guide](helm/synveda/README.md) covers the implemented external-services
installation and starter; the feature brief records remaining release/platform work.

Synveda has one context-platform runtime. Direct binaries, source/release
Compose services and Helm Deployments use the same product commands, schema
epoch, generated `/v1` contract, embedded Cedar PDP, VedaFlow effects and
hash-chained audit path (CPR-36, ADR-0095, ADR-0102). The gateway is the public
request process; the private core worker owns scheduled Capture, Knowledge
index, relaxation-expiry and optional directory-pull work. A disabled-by-default
Apalis leaf can transport one non-executing Skill-validation operation without
owning its tenant or business state.

`personal`, `team` and `enterprise` are not deployment editions. They are
canonical Configuration documents copied into immutable governed versions and
bound to scopes after login. Deployment files may choose infrastructure size,
OIDC wiring, supported model implementations, secret references and telemetry;
they do not select policy, capture rules, context budgets, trace retention,
freshness or Skill/Tool advertisement.

- `compose/` contains the additive canonical Docker reference graph and its
  executable `up`, `smoke`, full `acceptance`, gateway-only `restart-gateway`,
  paired logical `backup`/fresh private `restore-smoke`, `down` and
  exact-confirmation `reset` lifecycle, plus optional observability and Apalis
  canary profiles.
  Published v0.4.0 passed native Linux AMD64/ARM64 installation and recovery.
  Separate source-development evidence includes a macOS/OrbStack clean-volume
  run; Docker Desktop and Windows/WSL2 remain unqualified. The new plain-Compose
  candidate and refactored pipeline still require their own hosted qualification.
  This is also the only source-development product topology. Evaluation-only
  dependencies use isolated fixtures and do not define another Synveda stack.
- `helm/` is the Kubernetes infrastructure: separate gateway and worker
  Deployments from the same image, bundled or external PostgreSQL, with optional CloudNativePG,
  optional TEI, explicit HTTPS ingress and file-mounted external IdP/Secret wiring.
  Only CNPG mode requires a separately installed operator. The release workflow packages this
  chart and a digest-bound reference bundle using one versioned six-image plan:
  product, single-host and CloudNativePG PostgreSQL, optimized Keycloak,
  reference proxy and browser acceptance. Published v0.4.0 passed anonymous
  pulls and installation in all four bundled/external database and identity
  combinations. The next release additionally requires exact OCI candidate
  testing before publication to Docker Hub and GHCR, followed by public pulls
  and installation using the final destination digests.

## Bootstrap boundary

Deployment-owned bootstrap and the Helm install job do only the operations for
which no authenticated product principal exists yet:

1. provision roles/extensions for bundled databases; external providers do this separately;
2. prove database/peer isolation and apply the current schema chain;
3. optionally admit the first tenant;
4. establish deployment key and issuer material.

The reserved `synveda init` verb is a permanent, side-effect-free refusal. It
neither discovers profiles nor reads configuration. Canonical Compose owns the
deployment lifecycle; explicit CLI commands remain available for bounded
database migration, tenant admission and recovery operations.

The first `synveda-admins` login creates the tenant root, the caller's principal
scope and its root `administrator` grant. Workspaces, projects, sessions,
capture decisions, Knowledge and Configuration are public-API/PDP/VedaFlow/
audit acts after that. No deployment script inserts those tables directly.

## Runtime database roles and forced RLS

Deployment bootstrap creates `synveda_app` as a NOLOGIN capability role. The
ordinary `synveda_migrator` owns only the selected database and public
application objects. Distinct `synveda_gateway` and `synveda_worker` LOGINs
inherit only `synveda_app`; they own no database, schema or object and carry no
elevation, database-wide setting or other membership.

Gateway and worker continuously re-prove the same epoch, catalog authority,
forced-RLS contract, peer isolation and database identity. Authority closure
withdraws readiness and governed work; conclusive refusal terminates the
process. This is process enforcement, not only a readiness probe.

Compose and Helm reference separate migrator, gateway and worker files and the
same explicit role contract; runtime Deployments receive no database owner or
superuser credential. Helm also mounts issuer/KMS/extractor Secrets. External
PostgreSQL mode provisions no administrative objects, requires verify-full,
and uses the same bounded preflight and migration implementation. Full
production promotion, real OpenShift execution and provider-specific recovery
remain open; the bounded native Kind installation/recovery evidence does not
establish those claims.

Direct-binary database commands require explicit `DATABASE_URL` or
`DATABASE_URL_FILE`; there is no implicit development credential. Compose
invokes explicit database, migration, tenant, identity and issuer-diagnostic
commands rather than a second bootstrap implementation.

The worker's default supervised join is 75 seconds. Canonical Compose gives it
an 85-second outer stop grace and uses `restart: unless-stopped` so a deliberate
non-zero critical-task exit is visible and restarted. Helm derives its
termination grace as the configured worker join plus ten seconds.

`make check-release-parity` validates the closed release-version boundary,
exact six-image workflow plan, repeatable Helm chart and digest-bound Docker
reference package without contacting Docker or a registry. The reference
environment manifest pairs the source SHA with every image identity.
`make check-chart-images` requires every external deployment-image base to
carry a readable tag and full SHA-256 digest. `make check-deploy` includes both
gates, renders canonical Compose and Helm, asserts distinct process commands,
credentials and private worker probes, packages the reference twice and checks
upgrade-shaped replacement. The CPR-36 database acceptance test also proves a
runtime login with no tenant GUC cannot read tenant data. Current live Kind
acceptance proves Keycloak login, a governed product round trip and worker
readiness after CloudNativePG primary failover. That is Kubernetes source-image
evidence. Published v0.4.0 has separate native Linux reports for Docker and the
four operator-free Helm ownership modes; its optional CNPG path still needs
qualification with the published images.

## Embeddings

`deterministic` is a lexical-only development implementation. `tei` serves
BGE-M3 and is the meaningful semantic option. Upstream's amd64 image is version
tagged; its arm64 image is pinned by commit because no versioned arm64 tag is
published. The two tested builds produce the same 1024-dimensional model output
to float32 rounding (cosine `1.000000000`, maximum absolute difference `7e-8`,
measured 2026-07-26).

Knowledge embedding rows retain model and dimension. A model change converges a
separately labelled sidecar; an old vector is never reinterpreted as output from
a new model. The isolated evaluation fixture and Helm retain a TEI cache
because a cold BGE-M3 download is about 2.3 GB. A canonical Compose semantic
profile remains pending.

## Honest operating limits

- Helm runs one gateway and one core-worker replica with `Recreate`. Pending
  login state and cross-process cache invalidation have not passed OPS-7; the
  chart refuses replica settings. CloudNativePG provides a replicated data
  plane, not request or worker HA, and an application upgrade has a brief
  outage. Worker SIGTERM has bounded cancellation/join evidence at idle;
  interruption during claimed Capture work and two-worker execution remain
  open.
- Compose is a single-node shape with generated file-mounted development
  credentials. It is not a production secret-management example.
- The chart has no Qdrant, workflow scheduler, backup promise, external HSM or
  customer-managed-key implementation. Provider credentials are Secret
  references; rendered diagnostics must not contain values.
- Native binaries have no OS code signature or notarization. Public v0.4.0
  contains macOS ARM64 and Linux x64 archives; the next release requires all
  six native client packages, including Windows x64/ARM64. Configured jobs do
  not establish hosted qualification. The new checksum attestation and dual
  registry publication have not yet shipped. There is no zero-downtime gateway
  upgrade guarantee or old-schema translator.

<a id="small-team-kubernetes-release-contract"></a>

## Kubernetes release contract

[OPS-11](../docs/backlog/OPS-11.md) owns this milestone. The former baseline
audit and implementation plan are retained in Git; current operator instructions
are consolidated in the [Kubernetes installation guide](helm/synveda/README.md),
[configuration reference](helm/synveda/CONFIGURATION.md) and
[operations runbook](helm/synveda/OPERATIONS.md). Architecture remains governed
by ADR-0109 through ADR-0112 and the deployment contract above.

| Boundary | Current implementation and evidence |
|---|---|
| Providers | Bundled or external PostgreSQL, independently bundled Keycloak/existing OIDC; both operator-free and CNPG matrices have local Kind evidence |
| Team admission | Explicit initial owner, invitation/direct grant, viewer/member/service access and unchanged-token revocation; no email-based authority |
| Runtime | One product image, gateway-served console, separate worker and existing PostgreSQL jobs; mandatory authority readiness separate from optional provider diagnostics |
| Portability | Restricted-ID Kind simulation and Kubernetes/OpenShift structural schemas; real SCC/router/CNI/CSI/cloud execution remains unqualified |
| Recovery | Writer-quiesced native PostgreSQL archives, original key/issuer custody and clean-namespace functional verification; actual measurements are recorded in OPS-11, not inferred from retained PVCs |
| Upgrade | Same-epoch migration reruns/locking and retained reinstall; no general N-1 release window is qualified, and the retired v0.2.0 schema is refused |
| Distribution | v0.4.0 chart/images and digest overlays have anonymous retrieval evidence; the refactored workflow requires six native CLI packages, qualified OCI candidates, Docker Hub/GHCR parity and attested checksums before a new stable release |

The next release needs hosted qualification of this refactor and the owner
settings in the [release guide](../docs/RELEASING.md#owner-setup). A general
supported N-1 upgrade window and real target-platform qualification remain
separate readiness work. External administrators own provider compatibility
and backup custody. No operator, workflow engine, service mesh, schema
translator or application replica knob is introduced.
