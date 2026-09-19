# Deployment

This file is the infrastructure-shape overview. Source-checkout operator steps
live in the [canonical Compose guide](compose/README.md); the normative mapping
across deployment shapes lives in the
[deployment contract](../docs/DEPLOYMENT_CONTRACT.md), and unproved operational
claims remain in [production readiness](../docs/PRODUCTION_READINESS.md).

For the next release, see the [small-team Kubernetes contract and audit](#small-team-kubernetes-release-contract).
The [portable chart guide](helm/synveda/README.md) covers the implemented external-services
installation; the audit distinguishes it from the remaining starter/platform work.

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

The audit below records the pre-implementation checkout. OPS-11 now implements
the [portable external-services chart](helm/synveda/README.md) and
[persistent starter](helm/synveda/STARTER.md); its current
validation and remaining work live in [the feature brief](../docs/backlog/OPS-11.md).

Audit: 2026-09-19, source `1194c0a348733ab58aaa5e1984eedccfd3eff25e`.
Tracked by [OPS-11](../docs/backlog/OPS-11.md) and
[ADR-0109](../docs/adr/adr-0109-small-team-kubernetes-release.md).
The owner reports the Docker MVP validated. This session independently ran
smoke against the existing local development images; it did not rebuild them
or repeat full browser acceptance. The following is the smallest release
target, not a declaration that the Kubernetes programme is delivered.

### Baseline audit evidence (before the portable chart increment)

`Proven` is limited to the named observation; `partial` means implementation
exists with a release gap; `missing` means the requested capability is absent;
`unverified` means the required runtime or artifact evidence was not obtained.
Tests listed as inspected were not run in this session.

| Requirement | Status | Code/configuration | Test evidence and remaining limit |
| --- | --- | --- | --- |
| One product image, gateway-served UI, separate worker | Proven | `compose/product/Dockerfile`, `compose/product/synveda-container`; `../crates/synveda-gateway/src/console.rs`; `helm/synveda/templates/{gateway,worker}.yaml` | Local smoke serves `/console/`; chart checks retain distinct commands. No UI server, Node runtime or UI PVC is needed. |
| Exact database roles, extensions and schema | Partial | `compose/postgres/synveda-database-bootstrap`; `../crates/synveda-store/src/{runtime_role,epoch,database_url}.rs`; `helm/synveda/templates/install-job.yaml` | Helm role/preflight contracts pass; live smoke reports ready. Epoch 3, baseline revision 3; old baselines are refused. Fresh exact-role DB suite was not rerun; external providers remain unqualified. |
| Independent database/identity modes | Missing | `helm/synveda/templates/{_helpers.tpl,postgres-cluster,install-job}.yaml`; `compose/scripts/compose.sh` | Chart always emits CNPG and derives its Secrets; no bundled Keycloak chart path. Existing Compose tests explicitly refuse external PostgreSQL plus bundled Keycloak. |
| First owner, membership and service access | Partial | `../crates/synveda-gateway/src/{provision,authz}.rs`; `../crates/synveda-identity/src/oidc.rs` | Inspected `jit_provisioning`, `access_api`, `service_identities` and `console_session` tests. Reuse the one-time admin group, JIT principal scope, workspace-owner creation, invitations and grants. Helm tenant/issuer ordering still needs an operator-friendly path. |
| Secret files, public URLs and provider transport | Partial | `../crates/synveda-gateway/src/runtime_config.rs`; `compose/compose.external-postgres.yaml`; chart gateway/worker templates | 20 configuration unit tests pass. DB Secrets are mounted; Helm issuer/KMS/Claude credentials still use Secret-backed environment values. External DB verify-full/CA exists in Compose; general HTTP custom CA/proxy wiring is absent. |
| TLS edge and private management/metrics | Partial | `compose/configs/caddy/`; `helm/synveda/templates/ingress.yaml` | Local smoke checks public refusal routes. Helm's `/` ingress also routes unauthenticated `/metrics`; it has no NetworkPolicy, TLS termination enforcement or OpenShift Route. |
| Single-instance correctness and process lifecycle | Partial | `../crates/synveda-identity/src/flow.rs`; `../crates/synveda-gateway/src/{authority,main,worker,shutdown}.rs`; chart helpers | Replica refusals and private worker probes are checked. `/healthz` is process liveness; `/readyz` includes schema/role authority. Inspected worker subprocess tests; claimed-work SIGTERM and multi-worker execution remain unverified. |
| Persistent recovery and upgrade | Partial | `compose/scripts/{compose.sh,recovery-set.mjs}`; `../crates/synveda-store/src/epoch.rs` | Logical recovery and same-schema rollback have deterministic fixtures. This session ran no live restore/upgrade. Helm has no backup path; joint database/KEK recovery, off-host retention and declared recovery objectives remain release work. |
| OpenShift restricted execution | Unverified | `helm/synveda/values.yaml`, `templates/{install-job,tei}.yaml`; product/Keycloak Dockerfiles | Kubernetes 1.34 render accepts removal of UID/fsGroup and addition of RuntimeDefault seccomp. No SCC admission run. Defaults fix UID/fsGroup 65532; TEI lacks the product security settings and binds port 80. |
| Publishable chart/images | Partial | `helm/synveda/Chart.yaml`; `../.github/workflows/release.yml`; `../scripts/check-release-parity.mjs` | Source packaging is tested. GitHub's published `v0.2.0` asset list contains no chart/current reference archive; anonymous product/CNPG manifest requests return 403, which does not prove absence. |

### Actual runtime dependencies

| Dependency | Class | Required behaviour / evidence when absent |
| --- | --- | --- |
| PostgreSQL 17, `btree_gin` 1.3, `vector` 0.8.6 in `public` | Mandatory | Required even with deterministic embeddings. Bootstrap and `synveda-store` extension fingerprints verify them; built-in `plpgsql` is also checked. FTS, graph relations, VedaFlow objects, jobs and audit all live here. No active Redis, Kafka, Temporal, PGMQ, Tantivy, Qdrant, Elasticsearch or graph-server consumer was found. |
| OIDC identity and local KMS key/reference | Mandatory for the deployed console | Keep Keycloak and standard code+PKCE. An external Keycloak replaces the bundled provider; identity itself is not optional. Keycloak needs its own durable database and role. KMS absence disables sessions/secrets, so it is not an acceptable console preset. |
| TEI/BGE-M3 endpoint | Optional | `SYNVEDA_EMBEDDER=deterministic` leaves authorised lexical Context/Knowledge retrieval available; `context_api.rs` explicitly reports `deterministic_embedder_is_not_semantic`. This is not semantic search. TEI failure also has bounded lexical fallback. |
| Claude or vLLM extraction endpoint | Optional | Deterministic Capture runs locally without credentials; 13 filtered ingest tests pass. It uses rules/truncated summaries, not LLM quality. An extractor implementation is required: there is no `off` enum. Claude needs an API key; vLLM needs URL/model. |
| Apalis 0.7.4 and separate queue PostgreSQL | Optional experiment | Native PostgreSQL Skill validation remains the default. Local smoke ran without Apalis; only opaque Skill-validation routing references go in its queue. Retain the leaf, disabled in both release presets. |
| Telemetry | Optional external backend | Compose requires its private Collector service, including discard mode; smoke passed with no external backend or Prometheus. The chart currently ships no Collector and an empty `otel.endpoint` exports to localhost. Product state is independent of telemetry delivery; receiver outages are not yet live-qualified. |
| Directory providers, remote Tool discovery, import URLs | Optional feature-specific endpoints | JIT login and explicit invitations/grants do not need SCIM or scheduled pull. Configure credentials/egress only for selected integrations. No Tool execution runtime is required on the server. |
| Object storage and SMTP | Not active dependencies | VedaFlow object bytes are in PostgreSQL. Invitations return a link; there is no Synveda mail transport. S3-compatible backup/WAL retention is future OPS-5 work. Keycloak mail is only needed if the operator enables its email workflows. |
| Demo users, Playwright browser fixture | Demo-only | Explicit acceptance/demo profiles, not application services. Build-stage Node/Rust and test-client pods are also not production runtimes. |

The inspected Context/Capture integration tests use deterministic providers.
Their end-to-end workflows were not rerun; no-model release acceptance remains
required alongside the unit and live health/console evidence above.

### Presets, topology and operator inputs

Use the two documented presets in the **existing** chart. Use one organisation (one admitted
tenant) per release and namespace/project, with one exact static issuer binding.
Workspace, project and personal-scope isolation remain unchanged. Multiple
issuers per tenant stay outside this release because identity lookup currently
keys by tenant and subject, not issuer (MEM-7).

| Preset / combination | PostgreSQL | Identity | Persistent application footprint |
| --- | --- | --- | --- |
| External-services | Operator-owned conforming PostgreSQL 17 | Existing Keycloak through generic OIDC | No chart database, CNPG CRD requirement or application PVC; retain external DB/IdP backups and KMS Secret. |
| Persistent starter | Existing CNPG integration, one instance with PVC | One optimized Keycloak in a locked upstream chart, native initial realm import (ADR-0110) | Separate `synveda` and `keycloak` databases/roles on that instance; shared failure domain and joint recovery. Requires an already-installed CNPG operator. See the [starter guide](helm/synveda/STARTER.md). |
| Mixed: in-cluster DB, external identity | Same CNPG/PVC | External OIDC | Only Synveda database required locally. |
| Mixed: external DB, bundled identity | External PostgreSQL; operator also supplies a separate Keycloak database/login | Packaged Keycloak, native initial realm import | Identity persistence is external too; Synveda roles cannot enter its database. This Helm selection does not change Compose's existing supported combinations. |

The supported application topology remains **one gateway and one core worker**,
both `Recreate`, from the same immutable product image. No HPA or rolling surge.
Preserve existing optional three-instance CNPG and its primary-failover test;
database replication does not provide application HA. Retain the private
Collector seam (local discard by default or declared external receiver).
Use the cluster's ingress/router for the public edge; no separate UI service.
No operator, CRD installer or generic infrastructure provisioner is added.

| Operator input | Current names / required release mapping |
| --- | --- |
| Namespace/project, image identity and pull access | Helm `--namespace`, `image.repository`, `image.tag`, `imagePullSecrets`; publish an explicit digest input rather than rely on mutable defaults. |
| Public DNS and TLS | `gateway.publicUrl` → `SYNVEDA_PUBLIC_URL`; `ingress.host`, `ingress.className`, `ingress.tls` or an operator TLS router. Bundled Keycloak also needs an exact public issuer hostname. HTTPS is required; `gateway.insecureDevelopmentHttp` remains disposable-development only. |
| Database endpoints and existing Secrets | Distinct migrator, gateway and worker URLs; existing `gateway.databaseExistingSecret` and `worker.databaseExistingSecret` with configurable URL/password keys. Add external migrator/CA/role-contract references and endpoint assertions, reusing `DATABASE_URL_FILE`, `SYNVEDA_DATABASE_ROLES_FILE`, `SYNVEDA_DATABASE_EXPECTED_{HOST,PORT,NAME}`. External operators pre-provision roles/extensions; no superuser credential enters the product. Managed-service compatibility must be proved against the exact authority contract. |
| Identity trust and first owner | `oidc.existingSecret`/`secretKey` → `SYNVEDA_OIDC_ISSUERS_FILE`. JSON names `issuer`, `client_id`, distinct API `audience`, `algorithms` (RS256 default), `tenant.static.tenant_id`, `groups_claim`, `login_scopes` and optional `service_audiences`. Register precisely `<publicUrl>/auth/callback`; browser, gateway and CLI must resolve the same issuer. Map only the intended first administrator to `synveda-admins` before login. This creates a tenant `administrator`, not a new role vocabulary; workspace creation grants `owner`. |
| Bundled identity credentials | Helm's upstream Keycloak boundary uses existing Secret references for its separate database login and bootstrap administrator. Native initial import needs no convergence account (ADR-0110). Compose retains its existing file adapter and realm convergence. Do not generate credentials into rendered values or reports. |
| Tenant and key custody | Current `install.tenant.slug/name` admits a generated ID; reuse `tenant-converge` with an explicit UUIDv7 to remove the current admit/read-ID/create-issuer-Secret ordering. `kms.existingSecret`, `secretKey`, `keyRefSecretKey` must mount `SYNVEDA_KMS_KEY_FILE` and `SYNVEDA_KMS_KEY_REF_FILE`. Preserve the key and stable reference independently of DB backups. |
| Storage and resource sizing | `postgres.storage.{size,storageClass}` only for CNPG; `tei.cache.{size,storageClass}` only if local TEI is selected. Gateway/worker need no durable volume. Keep pool limits (`gateway.dbMaxConnections`, `worker.dbMaxConnections`) within database capacity with migration/identity headroom. |
| Optional providers and telemetry | `embedder`, `embedderModel`, `tei.{enabled,url,image,model}` map to `SYNVEDA_EMBEDDER`, `SYNVEDA_EMBEDDER_MODEL`, `SYNVEDA_TEI_URL`. `extractor.{kind,model,existingSecret,baseUrl}` maps to `SYNVEDA_EXTRACTOR`, `SYNVEDA_EXTRACTOR_MODEL`, `ANTHROPIC_API_KEY_FILE` or `SYNVEDA_VLLM_BASE_URL`. No provider credential is needed for the lexical/rule-based preset. `otel.endpoint` maps to `OTEL_EXPORTER_OTLP_ENDPOINT`; optional service identities require IdP client credentials plus provisioned Synveda grants and bounded token TTL. |

The shared direct/file loader rejects ambiguous inputs and bounds reads; it
supports Kubernetes Secret projections. External PostgreSQL already requires
`sslmode=verify-full` and `/run/secrets/postgres_root_ca` (one PEM root is the
documented contract). This does **not** imply custom CA/proxy support for
OIDC, TEI, extraction or OTLP. Compose deliberately clears ambient proxies;
external OTLP supports public PKI only, without authentication headers/mTLS.
Keep those limits explicit until a common tested configuration path exists.

### State, ports and writable paths

| State | Durability and scaling consequence |
| --- | --- |
| Sessions/events, Capture leases/results, Knowledge/revisions, objects, grants, configuration, audit, operations/outbox, console sessions, directory reconciliation evidence | Durable PostgreSQL state (`synveda-store` epoch-3 migration). Console bearer/refresh material is sealed; recovery also requires KMS material. |
| Keycloak users, client/realm configuration and signing state; KMS and operator credentials | Durable identity DB plus separately retained Secrets. Realm export or regenerated passwords are not a substitute for this recovery set. |
| FTS/index structures and Knowledge embedding sidecars | Rebuildable from retained database revisions with the same model; `knowledge_index.rs` retries missing rows. Production rebuild/cutover timing is unverified. TEI `/data` is a model-download cache, not product truth. |
| Pending PKCE logins (10 minutes), CLI handoffs (60 seconds), JWKS/PDP/entity caches, worker timers/in-flight calls and telemetry buffers | Process-local (`flow.rs`, `oidc.rs`, `pdp.rs`, `worker.rs`). Restart loses unfinished logins and reschedules loops from durable state. Capture is fenced, but no complete two-worker or cross-pod authority proof exists. |
| Keycloak authority/generation files, Apalis queue and local metrics history | Rebuildable operational state. Compose retains its Keycloak converger and public gate. Helm's locked upstream chart uses native initial import and database readiness (ADR-0110); it does not run that Compose sidecar. Apalis queue loss uses native execution/outbox recovery. |

Gateway port **8120** serves API, UI and probes; its writable path is `/tmp`
(`HOME` and XDG cache point there). Worker health is loopback **8121** with exec
probes; optional Apalis health is **8122**. Product Secrets/configs and console
assets are read-only. Helm currently uses unbounded disk `emptyDir` for product
`/tmp`; Compose bounds it to 64 MiB. The install job has memory-backed private
`/run/secrets` (64 KiB) and `/tmp` (16 MiB) for projected-secret copies.

Bundled PostgreSQL persists `/var/lib/postgresql/data` in Compose; CNPG owns
its data PVC layout. PostgreSQL uses **5432** and temporary `/tmp` and
`/var/run/postgresql`. Keycloak uses **8080**, private management **9000**,
temporary `/tmp`, `/opt/keycloak/data/tmp` and shared
`/run/synveda/keycloak-public-gate`; these files are not its user database.
Collector OTLP is **4317/4318**, health **13133**; optional Prometheus is **9090**.
Only edge HTTP/HTTPS is public. Optional TEI currently uses **80** and `/data`;
its restricted-port/security configuration must be corrected before OpenShift.

Worker shutdown defaults to 75 seconds with Helm grace 85. Gateway defaults
to a 30-second application bound, while Helm omits an explicit outer grace
(Kubernetes defaults to 30); add headroom and prove readiness withdrawal before
drain. No live claimed-Capture interruption or recovery test ran in this audit.

### Upstream and platform qualification

Primary upstream checks on 2026-09-19 retain existing versions; no dependency
upgrade is selected by this audit. Registry manifest resolution proves tag and
platform availability, not successful pulls or runtime compatibility.

| Existing input | Upstream evidence and release consequence |
| --- | --- |
| Chart/application 0.2.0, no Helm subchart dependencies | Local chart packages/renders. [Published release assets](https://github.com/synveda/synveda/releases/tag/v0.2.0) are the older native/console/plugin/profile set, not the current chart/reference closure. Product and CNPG GHCR checks returned anonymous 403. All six release images still need authenticated pull and digest evidence. |
| PostgreSQL 17.11 + pgvector 0.8.6; CNPG 1.30.0 | [PostgreSQL](https://www.postgresql.org/docs/17/release-17-11.html) and [pgvector](https://github.com/pgvector/pgvector/blob/master/CHANGELOG.md) confirm the versions. [PostgreSQL licence](https://www.postgresql.org/about/licence/) and [pgvector licence](https://github.com/pgvector/pgvector/blob/v0.8.6/LICENSE) are permissive PostgreSQL terms. Exact PostgreSQL/CNPG base tags resolve for linux/amd64 and arm64. |
| Preinstalled CNPG operator | [1.30 support policy](https://cloudnative-pg.io/docs/1.30/supported_releases/) lists Kubernetes 1.34–1.36 and PostgreSQL 14–18; only the latest patch is supported. It does not claim community OpenShift support. [CNPG licence](https://github.com/cloudnative-pg/cloudnative-pg/blob/v1.30.0/LICENSE) is Apache-2.0. Preserve the digest-checked operator fixture; the product chart installs no operator. |
| Keycloak 26.7.2 | [Release](https://github.com/keycloak/keycloak/releases/tag/26.7.2), [licence](https://github.com/keycloak/keycloak/blob/26.7.2/LICENSE.txt) (Apache-2.0) and [optimized container contract](https://www.keycloak.org/server/containers) checked; amd64/arm64 manifest entries resolve. Reuse the optimized Synveda image, not a new third-party identity chart. |
| TEI cpu-1.8.1; BGE-M3 | [TEI release](https://github.com/huggingface/text-embeddings-inference/releases/tag/v1.8.1) and [Apache-2.0 licence](https://github.com/huggingface/text-embeddings-inference/blob/v1.8.1/LICENSE) checked; [BGE-M3 model card](https://huggingface.co/BAAI/bge-m3/blob/main/README.md) declares MIT. The chart's CPU tag resolves **amd64 only**; existing `cpu-arm64-sha-4150561` resolves arm64. Select architecture deliberately and pin the model revision before semantic release qualification. |
| Caddy 2.11.4, Collector Contrib 0.159.0, Prometheus 3.13.3-distroless | Exact repository tags resolve amd64/arm64 manifests. Versioned upstream licences are Apache-2.0: [Caddy](https://github.com/caddyserver/caddy/blob/v2.11.4/LICENSE), [Collector](https://github.com/open-telemetry/opentelemetry-collector-contrib/blob/v0.159.0/LICENSE), [Prometheus](https://github.com/prometheus/prometheus/blob/v3.13.3/LICENSE). Keep the existing digest inventory; no new observability stack is required. |
| Apalis / apalis-sql 0.7.4 | [Published apalis](https://docs.rs/crate/apalis/0.7.4) and [apalis-sql](https://docs.rs/crate/apalis-sql/0.7.4) source/docs exist; permissive MIT/Apache terms are recorded in the deployment contract. Exact Cargo pins remain; do not adopt the newer prerelease job engine as release preparation. |

Existing update controls are Cargo/pnpm locks, Dockerfile tag+digest checks,
the image inventory and release-parity tests. There is no automated container
advisory/remediation SLA. Release work must refresh upstream support/advisories,
review licence and model changes, update pins/inventory together, and rerun
acceptance; never silently float tags. PostgreSQL's [support policy](https://www.postgresql.org/support/versioning/)
keeps major 17 supported through November 2029 and recommends current minors.
The image inventory's old unselected-Synveda-licence text and omission of
built-in `btree_gin` also need alignment with the current Apache-2.0 repository
and executable extension contract before publication.

| Platform/shape | Evidence level | Qualification boundary |
| --- | --- | --- |
| macOS arm64 + OrbStack Engine 29.4.0 / Compose 5.1.2; Linux arm64 containers | Runtime-tested | Existing local development stack passed smoke this session; owner-reported MVP and earlier full acceptance remain separately attributed. HTTPS and restore were not retested. |
| Existing Kind/CNPG chart fixture | Runtime-tested, historical repository evidence | Current test is retained; it covers PKCE/product work, forced RLS and DB-primary failover. Not run this session. Kind node version is unpinned, so it does not establish a Kubernetes minor support range. |
| Kubernetes 1.35, linux/amd64 initial target | Render/security-checked, partial | This session rendered the current chart for 1.35 and passed Helm contract checks; no new-preset live acceptance or full network/security admission evidence. The chart's `>=1.29` constraint is not a support claim. |
| OpenShift 4.21 (Kubernetes 1.34), linux/amd64, external services first | Documented-only target | [Red Hat release mapping](https://docs.redhat.com/en/documentation/openshift_container_platform/4.21/html/release_notes/ocp-4-21-release-notes) and [arbitrary-UID requirements](https://docs.redhat.com/en/documentation/openshift_container_platform/4.21/html/images/creating-images) checked. A 1.34 render with UID/fsGroup removed is possible; restricted SCC, router, SELinux and PVC execution remain unverified. |
| Kubernetes arm64, OpenShift starter, other distributions/versions | Unverified | Upstream image architectures alone do not qualify Synveda. No automatic extension of the support matrix. |

### Implementation order and required gates

1. **Core chart / external services (OPS-11).** Add a closed external/CNPG
   database choice, operator-owned migrator/CA/role references and mounted
   issuer/KMS secrets. External renders must contain no CNPG resource,
   superuser bootstrap or PVC. Reuse database-preflight, migrations, authority
   sentinel and issuer diagnostic. Retain the existing CNPG branch and its
   failover test; test invalid combinations without weakening role checks.
2. **Persistent starter / onboarding (OPS-11).** ADR-0110 maps the existing
   optimized Keycloak image and database isolation to a locked upstream chart,
   with native initial realm import and private administration. Keep stable
   tenant admission, explicit first-admin mapping and public membership APIs.
   Qualify all four Helm combinations, no-model workflows, unchanged-token
   revocation and retained reinstall; preserve Compose's existing contract.
3. **OpenShift / portability (OPS-11).** Supply an assigned-UID preset, bounded
   writable mounts and projected-secret permissions; cover install jobs,
   packaged Keycloak and optional TEI, not only the gateway. Use restricted
   admission without `anyuid`, privileged pods or new SCCs. Add the public TLS
   ingress/Route contract, metrics/admin refusal and explicit dependency/DNS
   NetworkPolicies. Validate private CA/proxy requirements consistently or
   retain an explicit unsupported boundary. Run an authorised disposable
   OpenShift target and retain vanilla Kubernetes acceptance.
4. **Operations / docs / release tests (OPS-11 with OPS-5/6/8).** Prove drain
   during real work, credential rotation/restart, joint DB/identity/KMS restore,
   compatible upgrade/rollback and data-preserving uninstall. Define backup
   ownership/retention and measured recovery objectives; keep PITR/DR claims
   separate until proven. Publish the exact chart/image set and test pulls from
   an empty target on each claimed architecture. Update this guide and the
   existing image/readiness inventories; add no parallel documentation tree.

Preserve `make ci`, `make db-test`, `make chart-lint`, `make check-deploy`,
`make compose-config`, `make claude-acceptance`, `make eval-check` and the live
`make compose-acceptance`, `make compose-backup`, `make compose-restore-smoke`,
`make compose-upgrade-smoke` gates. Extend `demos/ops-2-helm-install.sh`; it
creates/deletes a Kind cluster and changes kubectl context, so it must never be
aimed at a shared cluster. Live client acceptance remains credential-dependent.

Session validation: Helm lint (minimum/full) and role contracts passed; both
explicit Kubernetes renders passed; 13 filtered ingest tests and 20 runtime
configuration tests passed with no ignored tests. Local smoke used
`SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-interop`,
`SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24`, `SYNVEDA_COMPOSE_PROFILES=demo` and
`make compose-smoke`. No services were restarted or cluster resources changed.
All deployment-gate components passed: 352 Node tests, Compose/Helm contracts
and release/image packaging checks. The initial `make check-deploy` stopped
at a sandbox-denied loopback bind; its 44-test convergence group, five uninstall
tests and final static check then passed with local socket access. This is not
a claim that the first invocation passed. Backlog, ADR-status and documentation
checks, `cargo fmt --all --check` and `git diff --check` also passed.
Full CI, fresh DB tests, Kind/OpenShift, live provider/semantic,
browser/restart, restore, upgrade and published-installation tests were not run.

Concrete remaining blockers are the chart branches and security gaps above,
conforming external-provider fixtures, a disposable Kubernetes/OpenShift target
with DNS/TLS/storage, and a publishable, pullable candidate artifact set. No
credential values are required in this guide or acceptance reports.
