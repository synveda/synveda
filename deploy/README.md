# Deployment

This file is the infrastructure-shape overview. Source-checkout operator steps
live in the [canonical Compose guide](compose/README.md); the normative mapping
across deployment shapes lives in the
[deployment contract](../docs/DEPLOYMENT_CONTRACT.md), and unproved operational
claims remain in [production readiness](../docs/PRODUCTION_READINESS.md).

For the next release, see the [small-team Kubernetes contract and audit](#small-team-kubernetes-release-contract).
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
  Deterministic lifecycle tests are implementation evidence, not a validated
  reference claim. One macOS/OrbStack clean-volume development run passed;
  Linux, Docker Desktop, reference HTTPS and recovery acceptance remain open.
  This is also the only source-development product topology. Evaluation-only
  dependencies use isolated fixtures and do not define another Synveda stack.
- `helm/` is the Kubernetes infrastructure: separate gateway and worker
  Deployments from the same image, external PostgreSQL or optional CloudNativePG,
  optional TEI, explicit HTTPS ingress and file-mounted external IdP/Secret wiring.
  Only CNPG mode requires a separately installed operator. The release workflow packages this
  chart and a digest-bound reference bundle using one versioned six-image plan:
  product, single-host and CloudNativePG PostgreSQL, optimized Keycloak,
  reference proxy and browser acceptance. No tagged candidate has yet proved
  publication, authenticated pulls or installation from those artifacts.

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
production promotion, starter identity, recovery and OpenShift remain open.

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
evidence, not a published Helm or Docker-reference release claim.

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
- Release binaries are unsigned and un-notarized; shipped binaries are macOS
  arm64 and Linux x86_64 only. There is no Windows build, zero-downtime gateway
  upgrade guarantee or old-schema translator. The release workflow has no
  completed tagged run for the aligned chart/image set, captured OCI
  descriptors, signatures or provenance. The generated environment manifest
  has deterministic static coverage but no published-registry evidence.

## Small-team Kubernetes release contract

[OPS-11](../docs/backlog/OPS-11.md) owns this milestone. The former baseline
audit and implementation plan are retained in Git; current operator instructions
are consolidated in the [Kubernetes installation guide](helm/synveda/README.md),
[configuration reference](helm/synveda/CONFIGURATION.md) and
[operations runbook](helm/synveda/OPERATIONS.md). Architecture remains governed
by ADR-0109 through ADR-0112 and the deployment contract above.

| Boundary | Current implementation and evidence |
|---|---|
| Providers | External PostgreSQL/OIDC and independently optional CNPG/Keycloak; four ownership combinations passed local Kind acceptance |
| Team admission | Explicit initial owner, invitation/direct grant, viewer/member/service access and unchanged-token revocation; no email-based authority |
| Runtime | One product image, gateway-served console, separate worker and existing PostgreSQL jobs; mandatory authority readiness separate from optional provider diagnostics |
| Portability | Restricted-ID Kind simulation and Kubernetes/OpenShift structural schemas; real SCC/router/CNI/CSI/cloud execution remains unqualified |
| Recovery | Writer-quiesced native PostgreSQL archives, original key/issuer custody and clean-namespace functional verification; actual measurements are recorded in OPS-11, not inferred from retained PVCs |
| Upgrade | Same-epoch migration reruns/locking and retained reinstall; public v0.2.0 predates epoch 3, so no supported published N-1 pair exists |
| Distribution | Existing native/image/chart workflow, digest-bound Helm overlays, checksums and prepared BuildKit provenance/SBOM; no new artifacts published in this work |

The release remains blocked on a pull-verified candidate, supported upgrade
baseline and real target-platform qualification. External administrators own
provider compatibility and backup custody. No operator, workflow engine,
service mesh, schema translator or application replica knob is introduced.
