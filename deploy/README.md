# Deployment

Synveda has one context-platform runtime. Direct binaries, source/release
Compose services and Helm Deployments use the same product commands, schema
epoch, generated `/v1` contract, embedded Cedar PDP, VedaFlow effects and
hash-chained audit path (CPR-36, ADR-0095, ADR-0102). The gateway is the public
request process; the private core worker owns scheduled Capture, Knowledge
index, relaxation-expiry and optional directory-pull work.

`personal`, `team` and `enterprise` are not deployment editions. They are
canonical Configuration documents copied into immutable governed versions and
bound to scopes after login. Deployment files may choose infrastructure size,
OIDC wiring, supported model implementations, secret references and telemetry;
they do not select policy, capture rules, context budgets, trace retention,
freshness or Skill/Tool advertisement.

- `compose/` contains the additive canonical Docker reference graph and its
  executable `up`, `smoke`, `down` and exact-confirmation `reset` lifecycle.
  Deterministic lifecycle tests are implementation evidence, not a validated
  reference claim: clean-volume browser/Keycloak acceptance is still open.
  The separate `make dev-up` contributor stack retains Rauthy/Temporal residue
  and is not the reference product lifecycle.
- `release/` is the pull-only transitional artifact manifest installed under
  `~/.synveda/profile`. It is retained for cutover evidence but is no longer
  advertised as a turnkey single-host install.
- `helm/` is the Kubernetes infrastructure: separate gateway and worker
  Deployments from the same image, CloudNativePG, optional TEI, ingress and
  external IdP/secret wiring. The CloudNativePG operator is deliberately a
  separately installed cluster dependency.

## Bootstrap boundary

Deployment-owned bootstrap and the Helm install job do only the operations for
which no authenticated product principal exists yet:

1. provision the exact migrator, gateway and worker roles and extensions;
2. prove database/peer isolation and apply the current schema chain;
3. optionally admit the first tenant;
4. establish deployment key and issuer material.

The `synveda init` verb is closed by a CPR-45 cutover gate before profile
discovery or mutation. It neither infers nor provisions database authority.
It reopens only when the canonical Compose lifecycle owns the same bounded
file inputs and complete startup deadline.

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

Compose supplies role-scoped files. Helm renders separate migrator, gateway
and worker Secrets and the same explicit role contract; runtime Deployments do
not receive the database owner or superuser credential. Its bootstrap,
preflight and migration stages are bounded and ordered. Remaining Helm gaps
include file-mount parity for issuer/KMS material and full promotion
acceptance, not gateway-owner credential reuse.

The closed `synveda init` implementation retains explicit URL validation but
is not the canonical bootstrap entry point. Compose now invokes the explicit
database, migration, tenant, identity and issuer-diagnostic commands. There is
no implicit bundled fallback or host/container endpoint claim.

The worker's default supervised join is 75 seconds. Both transitional Compose
manifests give it an 85-second outer stop grace; the installed release also
uses `restart: unless-stopped` so a deliberate non-zero critical-task exit is
visible and restarted. Helm derives its termination grace as the configured
worker join plus ten seconds.

`make check-deploy` renders both transitional Compose manifests and Helm,
asserts distinct process commands/credentials and private worker probes,
packages the release twice and checks the upgrade-shaped replacement. The
CPR-36 database acceptance test also proves a runtime login with no tenant GUC
cannot read tenant data. The kind acceptance script is written to assert the
worker role before a governed round trip and repeat private readiness after
CloudNativePG primary failover, but that script has not been rerun since the
current three-credential install-job cutover.

## Legacy host-gateway residue

The retained Rauthy profile used `http://localhost:8100/auth/v1/`. An OIDC
issuer identifier must be the same URL for the browser, discovery document,
token and gateway, while RFC 6761 resolves `localhost` to each caller's own
loopback. That forced its gateway onto the host. The CPR-45 cutover gate now
refuses the lifecycle before it can start either the bundled or external-issuer
legacy shape. Canonical Keycloak Compose removes the workaround by running both
product processes in containers behind one browser/container-reachable proxy
issuer name. That graph is executable; exact browser issuer/PKCE acceptance is
still required before the legacy stack can be deleted.

## Embeddings

`deterministic` is a lexical-only development implementation. `tei` serves
BGE-M3 and is the meaningful semantic option. Upstream's amd64 image is version
tagged; its arm64 image is pinned by commit because no versioned arm64 tag is
published. The two tested builds produce the same 1024-dimensional model output
to float32 rounding (cosine `1.000000000`, maximum absolute difference `7e-8`,
measured 2026-07-26).

Knowledge embedding rows retain model and dimension. A model change converges a
separately labelled sidecar; an old vector is never reinterpreted as output from
a new model. TEI's cache is persistent in Compose and Helm because a cold
BGE-M3 download is about 2.3 GB.

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
- The chart has no Qdrant, Temporal consumer, backup promise, external HSM or
  customer-managed-key implementation. Provider credentials are Secret
  references; rendered diagnostics must not contain values.
- Release binaries are unsigned and un-notarized; shipped binaries are macOS
  arm64 and Linux x86_64 only. There is no Windows build, zero-downtime gateway
  upgrade guarantee or old-schema translator.
