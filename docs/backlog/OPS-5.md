---
title: "OPS-5: Backup/restore & DR"
labels:
  - epic:OPS
  - phase:4
size: M
---

# OPS-5: Backup/restore & DR

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** M

## Problem and evidence

No encrypted off-host application recovery has been proved. Disposable and
local drills establish tooling and mechanics only. The local KEK, or an
externally owned Helm Secret, must be restored with the database. This is a
P0 gap in [production readiness](../PRODUCTION_READINESS.md).

### Selected recovery mode (2026-10-22)

[ADR-0154](../adr/adr-0154-joint-pitr-recovery-and-kek-custody.md) selects
pgBackRest PITR for bundled Compose PostgreSQL 17, the first supported
self-installed shape, with local file-backed key custody. The `synveda` and
`keycloak` databases share that cluster and its WAL stream, so one recovery
target restores product and identity state at the same point without pausing
writers. The paired logical tools from ADR-0112 remain a planned-interruption
fallback and are never called PITR. CNPG/Barman
([ADR-0122](../adr/adr-0122-provider-owned-object-store-backup-seam.md))
applies only to an actual CNPG deployment. Customer-operated PostgreSQL keeps
its own recovery service and owner.

### Authoritative recoverable state

| State | Where | Recovery |
| --- | --- | --- |
| Schema lineage and head, migration ledger | `synveda` database `_sqlx_migrations` | bound in the custody header (head and ledger SHA-256); `migration-check` after restore |
| Tenants, scopes, grants, Knowledge items/revisions/sources, Sessions and events, Capture batches/candidates, VedaFlow objects/proposals/refs, operations/outbox, Configuration and policy packs | `synveda` database, forced RLS | WAL replay to the target |
| Tenant audit chains | `synveda` database | frozen prefix compared after restore; new appends verified |
| Deployment and tenant data-key envelopes | `deployment_keys`, `tenant_keys` | every generation must name and open under the escrowed KEK |
| KEK and reference | project secret files | custody bundle |
| Identity provider state (realm, users, clients, sessions) | `keycloak` database, same cluster | same WAL stream and target |
| Keycloak convergence credential and cluster role credentials | project secret files | custody bundle (a physical copy carries the source roles) |
| Repository cipher passphrase | `pgbackrest.conf` | custody bundle; bucket credentials are reissued by the bucket owner |

Rebuildable or non-authoritative state is not restored as truth. That covers
pgvector HNSW indexes and embedding sidecars (rebuilt or converged by the
worker), planner statistics, the optional Apalis transport database (product
operation/outbox rows stay authoritative), browser-acceptance receipts and
Prometheus data. External OIDC providers and directories remain external
recovery dependencies with their own owners.

### Implemented (2026-10-22, source)

- `pitr-backup` takes the encrypted full backup without automatic expiry. It
  then seals a per-generation `custody.svrc` under a separate custodian key
  using the existing envelope (`synveda kms custody seal|open|verify`, purpose
  `recovery.custody`). The clear header holds identifiers only: source,
  backup, image references, repository locator, backup label and LSN/WAL
  range, system identifier, server, pgvector, migration head and ledger digest,
  KEK references and custodian label. The header SHA-256 is the envelope row
  key. The command reopens the bundle before publishing it.
- `pitr-mark` records a named restore point once its WAL is archived.
- `pitr-restore` targets only a fresh confirmed project. Before any target
  change it proves custody, image and tenant binding, repository identity, a
  clean `verify` report, and every WAL segment from the base backup to the
  target. It then installs the escrowed credentials, restores with
  `archive-mode=off`, waits for promotion, and reruns authority convergence.
  After that it runs the read-only `migration-check`, the tenant audit/key
  verifier and wrong-key refusal, then the private graph and browser login. A
  restore marker blocks later archiving into the source repository.
- `kek-rewrap` stops the gateway and worker. It runs `synveda kms rewrap`,
  which rewraps every deployment and tenant generation by compare-and-set and
  records content-free `tenant.key.rewrapped` events, then resumes and smokes.
- `recovery-verify` now refuses any key generation on a KEK reference the
  custody set does not hold.
- `deploy/compose/scripts/pitr-drill.sh` runs one disposable restore drill. It
  writes a JSON record and a Prometheus textfile with result, duration,
  generation age and last success.
- A latent fault in the existing `pitr-backup` was fixed: pgBackRest defaulted
  to a nonexistent `postgres` role in the bundled cluster.

### Evidence

- Deterministic: Rust unit tests for custody sealing (wrong key, replaced
  header, transplanted body, truncation, closed names) and rewrap planning.
  A store DB test covers tenant-scoped compare-and-set under RLS. The planner
  unit tests use captured pgBackRest 2.59.1 output. Fake-Docker lifecycle
  tests cover backup/seal, mark, restore order and pre-mutation refusals, the
  restored-source archive guard and `kek-rewrap`. The full Compose gate passes
  (see the [record](../../demos/evidence/ops5-pitr-custody-rehearsal.json)).
- Native: on this Windows host, `synveda kms rewrap` against a fresh
  PostgreSQL 17.11 fixture rewrapped two tenant generations, refused a third
  KEK reference, was idempotent and appended one `tenant.key.rewrapped` event.
  `recovery-verify` then refused the retiring KEK.
- Local two-point rehearsal (`scripts/ops5-joint-custody-drill.sh`,
  2026-10-22, Docker Desktop on Windows 11). It used the source `pitr` image,
  a pinned local S3 server over private-CA TLS, the real product schema
  through a source CLI image, tenant keys and audit, and a stand-in identity
  database in the same cluster. Restores to `point-1`, `point-2` and `latest`
  reproduced 1/2/2 tenants and identity rows `1`/`1,2`/`1,2,3`. The frozen
  tenant audit prefix matched byte for byte, a new append verified, and the
  unarchived identity row 4 was absent. Rewrap on the restored copy passed
  and the retiring KEK was refused. Eight faults were refused with
  content-free diagnostics before any target state changed: wrong custodian
  key, wrong passphrase, expired credential, untrusted CA, unrecorded mark,
  missing WAL, corrupt object, and reuse after an interrupted restore. No
  restored target archived to the source repository. Custody open to
  promoted-and-verified took 26 seconds per target on this tiny database.
  That is the database-layer interval only. The repository and restore
  one-shots ran with the Compose service's read-only, dropped-capability
  hardening.
- Local application drill (2026-10-22, same host, source-built images; see
  the [record](../../demos/evidence/ops5-pitr-application-drill.json)). It
  used the real `compose.sh` actions against Docker Engine 29.7.2 and a local
  private-CA S3 server. The source was brought up with the full graph
  (gateway, worker, Keycloak, proxy), and a public-API seed was run.
  `pitr-backup` sealed custody gen-1, then `pitr-mark` recorded `point-1`
  (tenant audit head 245) and `point-2` (head 309). Further writes reached
  head 377 before the source PostgreSQL was killed and the graph taken down.
  `pitr-restore` then ran into two fresh projects. `name:point-1` restored
  head 245 exactly and reached browser login in 391 seconds. `latest`
  restored head 309 exactly in 439 seconds, so the 14 seconds of unarchived
  writes (heads 310–377) were lost, as the RPO definition predicts. On each
  target the key opened and the wrong key was refused, and `migration-check`
  passed without writes. The original administrator signed in, the six
  applied proposals were listed, and an audited workspace scope was created
  through the public API; `audit verify` stayed valid. Wrong custodian key,
  unrecorded mark and wrong confirmation were refused live before any target
  volume existed. `down` and confirmed `reset` worked on a restored target.
- Defects the application drill found and this branch fixes:
  - Docker Engine 29 stamps `enable_ipv4`/`enable_ipv6` on networks, which
    made the network preflight refuse every action on an existing project.
  - Only one of the two bootstrap services that build the shared PostgreSQL
    tag selected the `pitr` target, so PostgreSQL could start without
    pgBackRest.
  - The restored-source archive guard also blocked `config`, `down` and
    `reset`.
- pgBackRest findings that shaped the design: `verify` reports neither a
  missing middle WAL segment nor fails for some corrupt objects. A restore to
  `latest` across a gap promotes silently at the gap. A named target beyond a
  gap fails only after the target volume is filled.

### Limits of this evidence

The two-point rehearsal did not start the gateway, Keycloak or browser; it
drove the planner, restore wrapper, CLI and metadata SQL directly. The
application drill ran `pitr-backup`, `pitr-mark`, `pitr-restore`, `down`,
`reset` and `pitr-drill.sh` against real Docker. `kek-rewrap` as a Compose
action has still run only against fake Docker; the rehearsal rewrapped
through the CLI. The application drill is a partial pass with these
unresolved results:

- The product's own public-API demo verifier (`product-demo.mjs seed` and
  `verify`) failed on both restored targets, at the `product-demo` and
  `retry-review` stages. The cause is not isolated. Login, `whoami`,
  proposal listing, an audited scope create and `audit verify` passed
  instead.
- The one live `pitr-drill.sh` run failed after its pgBackRest restore. Compose
  `up --wait` ended with the one-shot `keycloak-database-bootstrap` exiting 0,
  and the script recorded the failure correctly. The same step passed in the
  two direct restores.
- Keycloak realm convergence took about 4.5 minutes, past its 4.25-minute
  health budget. The drill raised the retries in its copy only.
- No Knowledge revision or source read, context compose, VedaFlow governed
  mutation or index readiness check ran on a restored target.

The local S3 server is not off-host custody. No
owner-selected RPO/RTO, retention or drill cadence exists. RPO is defined as
commits after the last archived WAL segment at the loss instant, bounded by
`archive_timeout` (60 seconds in Compose, 10 in the rehearsal). RTO is defined
from the first restore command to promoted and verified, then on to browser
login for the application drill. Both exclude fetching the custody bundle and
reconciling identity. Neither is a target.

### Remaining live gate

Inputs that only the owner can supply:

1. one approved S3-compatible bucket and region (AWS S3 or compatible), a
   backup credential and a read-only restore credential, and its CA if
   private;
2. a retained-key custodian who holds the custodian key and moves each
   `custody.svrc` off-host;
3. approved RPO, RTO, retention and drill cadence;
4. an authorised isolated restore host or project with no access to the
   source storage.

The published reference and consumer bundles do not yet carry the `pitr`
PostgreSQL image, its overlays or `pitr-recovery.mjs`, so this path runs from
a source checkout. Shipping it in a release is separate OPS-12 work that needs
publication approval.

Then, on a supported Linux host with the source `pitr` image and a product
image from this source:

```sh
export SYNVEDA_PITR_ENABLED=true SYNVEDA_RECOVERY_CUSTODIAN=<custodian>
export SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/custodian/synveda-custody.key
SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance make compose-acceptance   # source with product state
SYNVEDA_BACKUP_ID=gen-1 make compose-pitr-backup
# write product state 1 through the public API, then
SYNVEDA_PITR_MARK=point-1 make compose-pitr-mark
# write product state 2, then
SYNVEDA_PITR_MARK=point-2 make compose-pitr-mark
make compose-down                                  # source storage unavailable from here
# on the isolated target, once per point (fresh suffix and /24 each time):
SYNVEDA_PITR_TARGET=name:point-1 SYNVEDA_RESTORE_SOURCE_PROJECT=<source> SYNVEDA_BACKUP_ID=gen-1 \
  SYNVEDA_CONFIRM_RESTORE=<source>:gen-1:<target> make compose-pitr-restore
```

Before that run, isolate the demo-verifier failure on restored targets and
the intermittent `up --wait` failure in `pitr-drill.sh`.

The narrow acceptance criterion still unmet is this. Against the owner's
bucket and custodian, restore both marks into isolated targets whose source
storage is unavailable. Then, through ordinary public APIs, log in, open
sealed data, read the exact Knowledge revisions and sources, compose context
and complete one governed mutation. Verify forced RLS, the frozen audit
prefix, new appends and index readiness. Record achieved RPO and RTO under the
definitions above, then run `pitr-drill.sh` on the owner's schedule. OPS-5
stays open, and no DR claim is made, until that passes. The Helm/CNPG route
keeps its separate live qualification below.

## Scope

- Encrypt and retain PostgreSQL base backups plus WAL in an owner-selected
  object store for the installed-host and Helm deployment shapes in scope.
- Restore to a chosen point into an isolated deployment with the exact schema
  epoch, deployment key and tenant-key custody material.
- Verify tenant admission, immutable Knowledge heads/history, session and
  capture state, VedaFlow refs, forced RLS inventory and the complete audit
  prefix after restore.
- Rebuild or verify derived pgvector indexes and report readiness only after
  the restored service passes its post-restore checks.
- Automate a recurring drill and measure achieved RPO/RTO.

## Non-goals

- No invented RPO/RTO, retention or legal-hold policy.
- No multi-region active/active or residency failover.
- No backup of an unrecoverable database without its key material.
- No destructive retention cleanup before at least one restore has verified
  the retained generation.
- No KMS/HSM framework until a customer selects a provider.

## Architecture seam

Postgres remains the durable source of truth. Deployment tooling schedules and
restores backups. Key custody stays outside the database and the bucket, as
[ADR-0094](../adr/adr-0094-context-platform-key-and-secret-plane.md) and
ADR-0154 require. Post-restore verification uses existing schema, RLS,
Knowledge, audit and key checks. It must not bypass the gateway for
application acceptance.

## Acceptance criteria

- Encrypted base backup plus WAL restores to selected instants before and
  after committed writes, within documented RPO/RTO.
- Correct database/key pairs start and open sealed data; missing, stale or
  wrong keys fail closed with actionable diagnostics.
- A frozen audit prefix verifies byte-for-byte before backup and after restore,
  and new events append to the same valid chain.
- Public-API acceptance proves Sessions, Knowledge, context and governed
  mutation against the restored deployment.
- Restore recreates no cross-tenant visibility and every tenant table remains
  enabled and forced RLS.
- A scheduled isolated drill publishes age, duration and result metrics and
  alerts on missed or failed evidence.

## Required tests

- PITR tests across multiple write points and a failed/incomplete backup.
- Joint database/KMS restore, wrong-key and lost-key tests.
- Restore followed by RLS adversarial, audit verification and index search
  checks.
- Corrupt archive, expired credential, object-store outage and retention-race
  fault injection.
- Monthly production-shaped drill outside the source cluster.

## Rollout and rollback

Enable backups in a non-production deployment, restore them independently, then
stage retention. Keep at least one previously verified generation while
changing format, provider or keys. Rollback disables new scheduling without
deleting valid backups; restoration never writes into the source deployment.

### Helm/CNPG route

ADR-0122's opt-in CNPG chart seam binds an operator-owned Barman Cloud
ObjectStore as WAL archiver and renders a ScheduledBackup. Static render and
refusal tests cover it. It has not been installed or exercised in a live
cluster. The OPS-11 / ADR-0112 Kind drill proved only quiesced logical
archives with original KEK/issuer custody. Qualify it separately with the
same custody and verification checks.

## Dependencies

The owner must choose object store/region, encryption custody, retention,
RPO/RTO, drill frequency and incident ownership. OPS-3 constrains residency;
AUD-3 constrains WORM audit retention; TEN-5 constrains tenant erasure and
legal holds. OPS-15 consumes the drill signals.
