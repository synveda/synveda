# Canonical source-checkout Docker Compose guide

This is the sole detailed source-checkout guide for Synveda's canonical
single-host deployment. It runs the gateway and worker as separate
processes with PostgreSQL, bundled Keycloak, a reverse proxy and a private
OpenTelemetry Collector. Optional settings add a bounded local Prometheus
operator view, external trace export or one experimental Apalis-backed
Skill-validation worker.

For source prerequisites, focused tests and the code map, start with the
[developer guide](../../docs/DEVELOPMENT.md). This guide owns the complete
source-deployment lifecycle and optional modes.

To run a server without a checkout or compiler, use the
[prebuilt Docker guide](PREBUILT.md). It uses this same deployment graph and a
release-pinned launcher. The published v0.4.0 bundle includes the reference
archive. The v0.4.1 digest check and v0.4.2 public-verifier timeout prevented
their publication; v0.4.5 is the current published release. Earlier pre-epoch-3
databases are deliberately refused.

The [CI/release guide](../../docs/CI.md) describes candidate validation and
publication. Shared Docker jobs build each native AMD64/ARM64 image once, then
test the extracted bundle against those exact OCI candidates. Release copies
qualified bytes to Docker Hub and GHCR, then checks anonymous digest pulls and
executable assets on both native architectures.
Published consumers need no source build or publisher token. The existing
v0.4.0 downloads retain their original GHCR/checksum contract. v0.4.1 intended
attested checksums and both registries, but its tagged verification failed.

It supports development and reference configuration. A clean development
acceptance passed on macOS 26.6.2 arm64 with OrbStack Docker Engine 29.4.0 and
Compose 5.1.2 on 2026-09-12. The published bundle separately passed native Linux
AMD64/ARM64 lifecycle, browser login and paired recovery. Docker Desktop,
Windows/WSL2, a supported cross-release upgrade window and Apalis live evidence
remain open. The source and release evidence do not establish HA,
disaster-recovery, hosted-SaaS or enterprise certification.

Use deploy/compose/scripts/compose.sh through the Make targets. Do not assemble
Compose fragments manually.

## Prerequisites

- Docker Engine 28 or newer, reached through its local Unix socket;
- Docker Compose 2.33.1 or newer;
- for development source builds, Docker Buildx with the running embedded
  `default` builder and local `docker` driver;
- Node.js 22 or newer;
- OpenSSL when generating bundled-provider secrets or running restore;
- GNU Make for the documented targets;
- a non-root Unix operator.

Hosts install/remove additionally requires root-owned, non-writable,
ACL-free Node 22 at /usr/bin/node or /usr/local/bin/node. Linux also requires
the fixed system getfacl binary from the acl package.

The lifecycle refuses remote Docker contexts because its loopback publication
and resolver checks describe the Engine host. Development also requires the
selected .test hostnames to resolve only to 127.0.0.1.

The default names are app.synveda.test and auth.synveda.test. The browser,
gateway, discovery document and tokens use the same issuer authority.

The reviewed checkout, fixed root-owned Node binary, Docker/Compose/Buildx
binaries, credential helpers, daemon mirrors, daemon proxy/CA and embedded
BuildKit policy are part of the trusted host. The lifecycle does not sandbox a
checkout from its owner. It empties ambient Docker client proxy variables for
runtime services and development builds; explicit outbound-proxy and custom-CA
support are not implemented.

Development builds also refuse ambient BuildKit, Buildx and Bake selectors,
pin the validated local Engine, use private Buildx state outside the repository
and start the graph with `--no-build` only after the explicit build succeeds.
Registry authentication is retained opaquely. If `DOCKER_CONFIG` is not set,
an accessible `HOME` is required; any existing `config.json` must be a regular
file rather than a symlink. The complete boundary and residual risks are in
[the security model](../../docs/SECURITY.md#docker-reference-boundary).

## Development hostname setup

Preview and inspect the owned hosts-file block:

    make compose-hosts-plan
    make compose-hosts-status

Install exactly the displayed block after reviewing the checkout and the
confirmation printed by the plan. For the default project:

    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-install

Run only that target with the required privilege escalation; do not run Docker,
the lifecycle or Make generally as root. Flush the active resolver cache. On
macOS:

    sudo dscacheutil -flushcache
    sudo killall -HUP mDNSResponder

On a Linux host using systemd-resolved:

    sudo resolvectl flush-caches

For another local resolver, use that resolver's documented cache-flush action.
Then verify both the owned text and operating-system resolution:

    make compose-hosts-status
    make compose-resolver-check

The manager owns at most one marked block in `/etc/hosts`, refuses unmarked,
duplicate, foreign or drifted ownership, and keeps a root-only recovery copy
without printing existing host-file content. It modifies only a terminal
managed suffix and preserves the existing inode metadata. Noncanonical,
ACL-bearing, multiply linked or incorrectly owned targets are refused.
Interrupted append recovery requires a newly confirmed exact-prefix action;
this is a recovery contract, not an old-or-new power-loss atomicity claim.
External-OIDC development owns only the application hostname. Reference mode
uses operator DNS and never reads or edits `/etc/hosts`.

If status refuses a saved backup witness after its filesystem device number
changes, rerun the exact confirmed `compose-hosts-install` for the same project.
The privileged helper renews the public receipt only when every other witness
field, the protected recovery copy and the installed hosts content still match.
It preserves hosts content and the backup; unknown drift remains refused.
An interrupted receipt renewal is retried with the same confirmed install.

### Removing development hostname ownership

`compose-down` and confirmed `compose-reset` retain the host-wide hostname
prerequisite. When the development project is no longer needed, stop it first,
then ask the removal action for its configuration-bound confirmation. The first
call is expected to refuse without mutation:

    make compose-down
    make compose-hosts-remove

Inspect the exact value it prints, then rerun with that value and the same
selectors:

    SYNVEDA_CONFIRM_HOSTS_REMOVE="<exact value printed above>" make compose-hosts-remove

Flush the resolver cache again and run `make compose-hosts-status` to prove the
owned block is absent. The helper never removes unrelated hostname rows.

## Default lifecycle

Development with bundled PostgreSQL and bundled Keycloak is the default:

    make compose-config
    make compose-up
    make compose-smoke
    make compose-restart-gateway
    make compose-down

For a ready-to-use local demo, select the existing demo profile before start:

    export SYNVEDA_COMPOSE_PROFILES=demo
    make compose-up

Successful startup prints the actual `/console/` browser URL, the four demo
account names, the protected password-file paths, and exact status, bounded-log
and stop instructions. It never prints a password. Keep the same `SYNVEDA_*`
selectors for every later lifecycle command.

compose-up:

1. validates the host, Docker socket, network range and inputs;
2. generates or validates project-scoped secrets and issuer configuration;
3. renders and validates the exact Compose graph;
4. builds development images when required;
5. creates the databases and roles;
6. runs Synveda migration and tenant convergence;
7. starts production-mode Keycloak and converges its realm;
8. diagnoses the exact public issuer;
9. starts separate gateway and worker processes.

Re-running compose-up is convergent and does not rotate secrets.

compose-smoke checks the expected services and completed jobs, public health
and console routes, OIDC discovery and the refusal of management, metrics
and version routes. It is not a browser-login test.

`make compose-acceptance` is the fresh-project gate. It requires the explicit
acceptance suffix and private `/24`. Development uses the four-principal
ingestion-retry walkthrough below: real Keycloak browser and CLI login,
idempotent seed with preservation of a browser edit, worker-completed Capture,
distinct review and apply, strict two-person Skill approval, exact Knowledge
provenance, redacted Context revision links and direct restricted-viewer
denials. Reference HTTPS retains the two-principal public-API PulseBoard team
scenario; its private Knowledge and Skill decisions may remain honestly
`pending_review` under strict policy. Neither fixture advertises pending
changes as published or bypasses review. The gate then restarts
PostgreSQL, Keycloak, Collector, worker, gateway and proxy one at a time, runs
the full smoke after each, repeats browser login and verifies the existing
receipt against live product rows. Transient network or HTTP 5xx recovery is
re-probed for at most 180 seconds after each restart; an exposed refusal route,
wrong issuer document or other contract failure is immediate. One project lock
and one bounded deadline cover the run. Identity admission, Capture and
receipt-owned product rows are persisted-state witnesses across the matrix.
Success leaves the stack running for inspection and later recovery/upgrade
gates; reset remains separately confirmed.

compose-down stops containers and preserves the PostgreSQL volume, any selected
Apalis transport volume and generated project inputs. When the
browser-acceptance profile is selected, down removes
that profile's disposable credential-and-receipt volume after checking its
exact ownership; it does not retain login tokens at rest. Confirmed reset with
the same profiles also removes it when stopping a running project. The default
ignored state directory is deploy/compose/runtime/synveda-development.

## Optional local metrics

The observability profile adds one private metrics path without changing the
application telemetry contract:

    export SYNVEDA_COMPOSE_PROFILES=observability
    make compose-up
    make compose-smoke

The Collector scrapes the gateway and worker privately. Prometheus reads only
the Collector fan-in and publishes its operator UI at
`http://127.0.0.1:${SYNVEDA_PROMETHEUS_PORT:-9090}`. Smoke requires the gateway
authority and worker readiness gauges to equal one and the worker heartbeat to
be no more than five seconds old, all from samples newer than the smoke start.
The combined worker also reports console credential-custody retention through
`synveda_console_session_purge_sweeps_total` with `ok` and `error` outcomes, and
`synveda_console_sessions_purged_total`. A healthy empty table still advances
the `ok` sweep counter once per minute; rising `error` or a stopped `ok`
counter warrants worker/database investigation. Each sweep deletes at most
256 expired rows; it does not change the existing expiry-time refusal.
The profile applies 72-hour and 1-GB TSDB block-retention thresholds (whichever
triggers first). WAL, head-block and compaction overhead mean this is not a
volume or disk quota. The disposable `prometheus-data` volume is not part of
product recovery.

Use the same profile selector for `make compose-down` or confirmed
`make compose-reset`. Down preserves the metrics volume; reset validates and
removes it before the product database volume. On a remote host, reach the UI
only through an operator-controlled loopback tunnel. This profile is separate
from `compose-acceptance`, is not the tenant-safe Operations page, and carries
no production monitoring or alerting claim.

## Experimental Apalis canary

The disabled-by-default `apalis` profile changes only the
`skill_validation@1` execution transport. Synveda's forced-RLS operation,
attempt and transactional outbox rows remain authoritative; the ordinary core
worker remains the default rollback path. The profile adds one private
PostgreSQL queue, one bounded migration job and one private Apalis worker. It
has no public port or board:

    export SYNVEDA_COMPOSE_PROFILES=apalis
    make compose-up
    make compose-smoke

To execute the canary through the full fresh-project gate and include its
worker in the restart matrix:

    export SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance,apalis
    make compose-acceptance

The queue payload contains only an untrusted tenant routing identifier, the
Synveda operation ID and operation version. The worker rechecks tenant
association and current authority under forced RLS before applying the normal
Skill-validation effect. Duplicate delivery is fenced by the Synveda
operation lease and immutable test-result link; stale acknowledged delivery is
eligible for bounded resubmission.

The queue cluster's bootstrap owner is an isolated superuser used only by its
PostgreSQL process and one-shot migration service. The long-running worker
receives only the converged queue runtime password and Synveda's ordinary
worker DSN. The `apalis-data` volume is disposable transport state and is not
included in logical backup. Backup, restore, upgrade and standalone gateway
restart actions refuse this experimental profile. Select the default `postgres`
provider and stop the profile to roll back; there is no automatic queue
reconstruction or recovery claim. If a consumer remains unavailable, stale
deliveries are resubmitted at a bounded rate and may grow the disposable queue;
disable the profile until the consumer is repaired.

## Secrets

Generate secrets explicitly when preparing a bundled-PostgreSQL deployment:

    make compose-secrets
    make compose-issuer

The generator writes mode-0600 secret files below a mode-0700 project
directory, refuses overwrite unless explicitly forced and never prints secret
values. It refuses external PostgreSQL because those credentials and its CA
are operator-owned. The lifecycle's `--if-missing` preparation adds a missing
complete Apalis secret pair to a pre-Apalis set without rotating existing
credentials, but refuses a partial pair. The checked-in .env.example contains
no usable credential.

Do not put credentials, database URLs, KMS key material or client secrets in
Compose YAML, a committed environment file, an image layer or a shell command.
The canonical graph consumes mounted secret files.

## Bundled identity

Keycloak 26.7.2 is built as an optimized, pinned image and runs start
--optimized. Health and metrics are enabled on its private management port.
Only the required realm discovery, login, token, JWKS, logout, account and
static-resource routes are available through the public proxy.

The realm convergence service creates or reconciles:

- realm synveda;
- public authorization-code clients with PKCE S256;
- exact redirect origins;
- sub, email, name, groups and API-audience claims;
- the synveda-admins group;
- bounded token/session and brute-force settings.

The group seeds only the first Synveda administrator. Synveda grants, Cedar,
forced RLS, VedaFlow and audit remain authoritative.

Keycloak uses database keycloak and role keycloak. Synveda uses database
synveda with separate migrator, gateway and worker roles. Neither product role
has access to the other product's database.

## Demo and browser acceptance

The demo profile adds four convergence-owned users for the fictional Northstar
Delivery team:

- `synveda-demo-admin` — Avery Author, initially admitted through the existing
  `synveda-admins` bootstrap boundary;
- `synveda-demo-member` — Riley Reviewer, granted the existing `reviewer` and
  `administrator` roles at the demo workspace by the fixture;
- `synveda-demo-approver` — Morgan Approver, granted the existing
  `administrator` role at the demo workspace so strict Skill approvals use a
  second person;
- `synveda-demo-viewer` — Vera Restricted Viewer, granted only the existing
  `viewer` role at the demo workspace.

Start the opt-in profile without resetting an existing project:

    SYNVEDA_COMPOSE_PROFILES=demo make compose-up
    SYNVEDA_COMPOSE_PROFILES=demo make compose-smoke

Their passwords are generated into the mode-0600
`keycloak_demo_admin_password`, `keycloak_demo_member_password`,
`keycloak_demo_approver_password` and
`keycloak_demo_viewer_password` files below the printed mode-0700 project
secret directory. Read them only through a local password-input mechanism.
Only Avery belongs to the Keycloak `synveda-admins` group; Riley, Morgan and
Vera receive no Keycloak domain role. Synveda's Cedar-governed grants remain
the application authority.

### Governed ingestion-retry walkthrough

This walkthrough uses public APIs and the normal deterministic Capture and
retrieval paths. The Session and events are synthetic replay; no model, paid
service or live coding-agent subscription is involved. Set the explicit HTTP
relaxation only for this loopback development origin, then create four stored
login profiles. For each command, open the printed URL in a fresh private
browser session and sign in as the named user with that user's password file:

```sh
export SYNVEDA_GATEWAY=http://app.synveda.test:8080
export SYNVEDA_INSECURE_DEVELOPMENT_HTTP=true

synveda login --gateway "$SYNVEDA_GATEWAY" --profile author --no-browser
# Avery Author: synveda-demo-admin
synveda login --gateway "$SYNVEDA_GATEWAY" --profile reviewer --no-browser
# Riley Reviewer: synveda-demo-member
synveda login --gateway "$SYNVEDA_GATEWAY" --profile approver --no-browser
# Morgan Approver: synveda-demo-approver
synveda login --gateway "$SYNVEDA_GATEWAY" --profile viewer --no-browser
# Vera Restricted Viewer: synveda-demo-viewer
```

Seed is deliberately incomplete. It creates or reopens the stable
`northstar-delivery-demo` workspace and `ingestion-api` project, verifies the
four grants, creates one already-approved Knowledge revision with repository
provenance, opens a versioned Skill install proposal, and appends one synthetic
finding to a Session. It does not capture that Session:

```sh
synveda demo retry-review seed \
  --author-credentials author \
  --reviewer-credentials reviewer \
  --approver-credentials approver \
  --viewer-credentials viewer \
  --confirm-target "$SYNVEDA_GATEWAY"
synveda demo retry-review inspect --author-credentials author
synveda demo retry-review capture \
  --author-credentials author \
  --confirm-target "$SYNVEDA_GATEWAY"
```

The expected finding says that retries reuse the original `Idempotency-Key`,
return 409 while the first ingestion runs, and replay its stored response once
complete. Capture prints two exact IDs: `<learning-change-id>` and
`<skill-change-id>`. Riley inspects each proposal. Vera's attempted learning
approval must exit non-zero without granting or applying anything. Riley alone
can approve the Knowledge change. Skill changes require Riley and Morgan as two
distinct approvers before Avery applies each typed change:

```sh
synveda proposal show <learning-change-id> --profile reviewer
synveda proposal approve <learning-change-id> --profile viewer \
  --comment "Not authorised to review"
# expected: denied; no state transition
synveda proposal approve <learning-change-id> --profile reviewer \
  --comment "Retry contract matches the reviewed Session evidence"
synveda proposal apply <learning-change-id> --profile author

synveda proposal show <skill-change-id> --profile reviewer
synveda proposal approve <skill-change-id> --profile reviewer \
  --comment "Skill instructions preserve idempotency and content-free evidence"
synveda proposal approve <skill-change-id> --profile approver \
  --comment "Distinct administrator approval for the reviewed Skill"
synveda proposal apply <skill-change-id> --profile author
```

Now open the project binding, substitute the printed
`<binding-change-id>`, and keep review and effect execution separate:

```sh
synveda demo retry-review bind-skill \
  --author-credentials author \
  --confirm-target "$SYNVEDA_GATEWAY"
synveda proposal show <binding-change-id> --profile reviewer
synveda proposal approve <binding-change-id> --profile reviewer \
  --comment "Pin the reviewed Skill version at the ingestion project"
synveda proposal approve <binding-change-id> --profile approver \
  --comment "Distinct administrator approval for the exact binding"
synveda proposal apply <binding-change-id> --profile author

synveda demo retry-review verify \
  --author-credentials author \
  --reviewer-credentials reviewer \
  --confirm-target "$SYNVEDA_GATEWAY" \
  --json
synveda demo retry-review status --author-credentials author --json
```

`verify` first reads the applied Knowledge head and its Session-event
provenance, then requests Riley's authorised context, checks that it selected
that exact revision, reads the enabled pinned Skill version, and finally reads
content-free Knowledge, Skill, Session and context audit pages plus the chain
verification result. The team's redacted Context trace does not retain task
or Knowledge text; its selected-item links still address the exact immutable
Knowledge revisions. `status` reopens the recorded Knowledge, Session,
Capture, Skill, Context and audit addresses through the public API. Re-running
`seed` uses stable idempotency keys and must retain the same addresses; it
refuses a different gateway, different identities, a
different effective curator file or changed receipt ownership. There is no
fixture reset endpoint and no business-table SQL.

### Governed context optimisation in this source checkout

Governed context optimisation adds an opt-in `conservative` context mode to the same Configuration
document and Session context route. This source change is not in the published
v0.4.3 images or CLI. Start the source deployment above and build the local CLI
from this checkout once; the deployed product does not need an extra model,
Python service or GPU:

```sh
cargo build -p synveda-cli --bin synveda
export PATH="$PWD/target/debug:$PATH"
export SYNVEDA_GATEWAY=http://app.synveda.test:8080
export SYNVEDA_INSECURE_DEVELOPMENT_HTTP=true
synveda login --gateway "$SYNVEDA_GATEWAY" --profile author --no-browser
```

In the console, choose the project, open Configuration, select **Conservative**
in the existing context editor, and publish the complete document under the
ordinary VedaFlow review and binding rules. The equivalent CLI path for an
already bound artifact is:

```sh
synveda configuration effective <scope-id> --profile author --json > /tmp/context-effective.json
node -e 'const fs=require("fs"); const x=JSON.parse(fs.readFileSync(process.argv[1],"utf8")); x.document.context.optimization_mode="conservative"; fs.writeFileSync(process.argv[2],JSON.stringify(x.document,null,2)+"\n")' /tmp/context-effective.json /tmp/context-conservative.json
synveda configuration publish <artifact-id> --expected-version <version-id> \
  --file /tmp/context-conservative.json --profile author
synveda configuration effective <scope-id> --profile author --json
```

Use `artifact_id` and `version_id` from the first effective response. If its
binding pins an older version, update that binding through the normal
`synveda configuration update-binding` command. A pending review must be
approved and applied before the new version is effective. To roll back,
publish an otherwise identical document with `optimization_mode: "off"` or
rebind an earlier off version. Do not edit a stored version in place.

After an agent has opened a Session, preview and inspect with its exact IDs:

```sh
synveda context preview <session-id> --query 'traceparent correlation header' \
  --tokenizer-encoding o200k_base --profile author --json
synveda context preview <session-id> --query 'traceparent correlation header' \
  --require-knowledge <item-id>@<revision-id> --profile author
synveda context preview <claude-session-id> --restart --query 'resume the pending task' \
  --profile author
synveda context inspect <context-run-id> --profile author
synveda context detail <item-id> <revision-id> --profile author
```

The first command is a preview only. Request a real ContextRun in the console
Context page, then use its returned ID with `context inspect`. `context detail`
pages through the existing reauthorised Knowledge history route for one exact
revision. `--restart` includes the latest available Claude checkpoint chain and a bounded
later event window when current SessionRead permits it. The checkpoint's
coverage label describes only the server-observed window, not the entire host
transcript. It is a preview, never a host compaction rewrite.
`--tokenizer-encoding o200k_base` counts that local text encoding;
without a declared compatible encoding the count is labelled an estimate.
These counts cover the Synveda-rendered contribution, not host history,
provider framing, output tokens or a billable request. The expandable console
preview shows visible selected and omitted sources, exact excerpts and authored
title indexes. Denied source details remain absent.

The deterministic source demo runs against a disposable exact-role database:

```sh
SYNVEDA_DB_TEST_TASK=demo bash scripts/db-test.sh demos/ctx-8-context-optimisation.sh
SYNVEDA_DB_TEST_TASK=demo bash scripts/db-test.sh demos/ctx-6-checkpoint-restart.sh
```

It compares the same authorised fixture in off and conservative modes, fetches
the original detail, and checks that preview creates no delivery. Its token
numbers are a local rendered-text comparison; no provider request or monetary
savings are measured.

The second demo replays Claude's public Session event and compact/restart
requests against a disposable exact-role database. It checks checkpoint
identity, incomplete coverage, duplicate delivery and source-labelled recent
events. It is an API and adapter-frame fixture, not a live Claude invocation.

The existing isolated browser acceptance starts from an explicitly fresh,
suffixed project. If the ordinary development block is installed, stop that
project and hand off the one owned hosts-file block first:

    make compose-down
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove

Then select and install the acceptance project:

    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    make compose-hosts-plan
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development-acceptance-local:app.synveda.test:auth.synveda.test \
      make compose-hosts-install
    make compose-hosts-status
    make compose-resolver-check
    make compose-acceptance

It replays the same staged scenario through four real authorization-code
logins, PKCE S256, issuer/audience claims and first-administrator admission
through the same proxy authority used by containers. The automated reviews
are labelled synthetic acceptance replay; they are not evidence of a human
reviewing a live agent execution. Redirect hops are validated from Playwright request events while
routable foreign requests are aborted. The fixture keeps every capability
dropped and uses the reviewed Playwright seccomp profile with one documented
local change: `chroot` is unconditional so Chromium can enter its unprivileged
sandbox after Docker has dropped `CAP_SYS_CHROOT`. The profile bytes, digest
and notice are checked before use. It requires a real Docker Engine;
deterministic fixture tests are not live evidence. After inspecting the result,
stop and reset that exact project, then remove its owned hosts block:

    SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance make compose-down
    SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance \
      SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-local make compose-reset
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development-acceptance-local:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove

Unset the acceptance selectors before returning to the ordinary project.
Reinstall its hosts block only when continuing development:

    unset SYNVEDA_COMPOSE_PROJECT_SUFFIX SYNVEDA_COMPOSE_IPV4_POOL
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-install
    make compose-resolver-check

## Encrypted off-host PITR recovery

[ADR-0154](../../docs/adr/adr-0154-joint-pitr-recovery-and-kek-custody.md)
selects pgBackRest PITR for bundled PostgreSQL. Physical base backups and
continuous WAL go to one operator-owned S3-compatible bucket, including AWS S3,
over verified TLS and with client-side `aes-256-cbc` encryption. The Synveda
and Keycloak databases share that cluster and its WAL stream, so one recovery
target restores product and identity state at the same point without pausing
writers. This is the source/reference lifecycle. The published consumer bundle
keeps the paired logical recovery below until a release ships the `pitr`
PostgreSQL image. Live off-host qualification is still open: see the
[backup and recovery brief](../../docs/backlog/OPS-5.md) for evidence and inputs.

### Inputs held apart

| Input | Holder | Never stored with |
| --- | --- | --- |
| Bucket, region, endpoint and a backup credential | object-store owner | the custody bundle |
| `pgbackrest.conf` (contains the credential and cipher passphrase) | the project runtime directory, mode 0600 | Git, Compose environment, command lines |
| Custodian key: 64 hex characters from `synveda kms keygen` | retained-key custodian | the deployment host's project inputs and the bucket |
| `custody.svrc` per generation | retained-key custodian, off-host | the bucket |

The custody bundle seals the KEK and its reference, the Keycloak convergence
credential, the source cluster's database role credentials and the repository
passphrase. A physical copy carries the source roles, so the restore needs
them. The bundle's clear header holds identifiers only. It is bound into the
seal, so a replaced header does not open.

### Configure the repository

First create the normal project inputs with `make compose-secrets` and
`make compose-issuer`, using the same runtime, project suffix and other
selectors that will start the deployment. Place a mode-0600
`pgbackrest.conf` owned by the runtime UID/GID in that project's private
runtime directory, beside its `secrets` directory. The lifecycle checks the
`[global]` and `[synveda]` sections without printing their values and mounts
the file only into PostgreSQL and its one-shot repository utilities:

```ini
[global]
repo1-type=s3
repo1-path=/synveda/synveda-development
repo1-s3-bucket=<operator-owned-bucket>
repo1-s3-endpoint=<TLS-hostname>
repo1-s3-region=<bucket-region>
repo1-s3-key=<access-id>
repo1-s3-key-secret=<secret-key>
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass=<at-least-32-random-characters>
repo1-storage-verify-tls=y

[synveda]
pg1-path=/var/lib/postgresql/data
```

For supported AWS instance credentials, replace the two static key lines with
`repo1-s3-key-type=auto`. Use a unique non-root `repo1-path` per source
project. A private S3-compatible endpoint may also need
`repo1-s3-uri-style=path` and `repo1-storage-port`. For a private CA, put a
mode-0600 `pgbackrest-ca.pem` beside `pgbackrest.conf` and add
`repo1-storage-ca-file=/var/run/postgresql/pgbackrest-ca.pem`. TLS
verification cannot be disabled. Azure/GCS configuration remains follow-on
work. Set `SYNVEDA_PITR_ENABLED=true` on every lifecycle call for the project.
Development builds the `pitr` image target. Reference mode requires an
operator-supplied digest-bound image built from that target.

### Back up and seal custody

```sh
export SYNVEDA_PITR_ENABLED=true
export SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/custodian/synveda-custody.key
export SYNVEDA_RECOVERY_CUSTODIAN=ops-escrow-1
export SYNVEDA_BACKUP_ID=gen-20261022
make compose-up
make compose-pitr-backup
```

`pitr-backup` creates or checks the stanza, checks WAL archiving and takes a
full base backup without automatic expiry. The backup requests an immediate
checkpoint, so expect a short I/O burst rather than a wait of up to
`checkpoint_timeout`. It then reads the repository and
cluster identifiers, seals `custody.svrc` with the custodian key, reopens it to
prove the seal, and publishes `custody.svrc` and the clear `binding.json` under
`deploy/compose/backups/secrets/<project>/<backup-id>`. Use
`SYNVEDA_RECOVERY_SECRETS_ROOT` for another mode-0700 root. A generation ID is
never reused. The custodian key must be absolute, mode 0600 and outside the
repository, the project inputs and the recovery roots. Move the bundle
off-host by the custodian's own route, never to the bucket. A KEK replacement
needs a new generation (see below).

Record named recovery points while the source runs:

```sh
SYNVEDA_PITR_MARK=before-change make compose-pitr-mark
```

`pitr-mark` creates a PostgreSQL restore point, switches WAL and waits up to
120 seconds until the archive holds that segment. It then records the mark's
LSN and segment under `deploy/compose/backups/marks/<project>`, or
`SYNVEDA_PITR_MARK_ROOT`. A mark name is never reused. If the wait expires,
check `pg_stat_archiver` and use a new name.

Monitor `pg_stat_archiver` (`last_archived_time`, `failed_count`), local
`pg_wal` capacity and backup age. A failing repository makes PostgreSQL keep
WAL locally until the disk fills. The fixed `archive_timeout=60s` bounds how
long a committed transaction can wait before its segment is pushed. It is
not an RPO objective.

### Restore into a fresh project

Restore always targets a new suffixed project and private `/24`, created with
`make compose-secrets`/`make compose-issuer` for the target. Give the target
its own `pgbackrest.conf` naming the same repository and passphrase. A
read-only object-store credential is preferred. Then:

```sh
export SYNVEDA_PITR_ENABLED=true
export SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/custodian/synveda-custody.key
export SYNVEDA_RESTORE_SOURCE_PROJECT=synveda-development-acceptance-local
export SYNVEDA_BACKUP_ID=gen-20261022
export SYNVEDA_PITR_TARGET=name:before-change   # or latest
export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-restore1
export SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24
export SYNVEDA_CONFIRM_RESTORE=synveda-development-acceptance-local:gen-20261022:synveda-development-acceptance-restore1
make compose-pitr-restore
```

Before it changes any target state, the command:

1. proves the target project is absent and opens the custody bundle;
2. requires the selected product and PostgreSQL images to match those that
   took the backup (restore first, then upgrade);
3. reads repository `info`, `verify` and the WAL listing;
4. requires a clean verify report, the recorded backup set and system
   identifier, and every WAL segment from the base backup to the target.
   pgBackRest neither reports a missing middle segment nor fails its exit
   status for a corrupt object, and a restore to `latest` would otherwise stop
   silently at a gap.

It then installs the escrowed credentials into the target, restores into the
empty volume with `archive-mode=off`, starts PostgreSQL with archiving off and
waits for promotion. Next it reruns the database authority convergence and the
read-only `migration-check` (schema lineage, authority and forced RLS). It runs
the tenant audit/key verifier, which requires every key generation to name and
open under the escrowed KEK, and the wrong-key refusal. Finally it starts the
private graph and completes browser login. The proxy publishes no host port.

The target records `pitr-restored.json` beside its configuration. A restored
cluster keeps the source system identifier, so every later PITR action that
can start PostgreSQL on that project is refused until its `pgbackrest.conf`
names another repository path. `config`, `down` and `reset` stay available.
A second restore into the same project is refused: reset it with its exact
confirmation and choose a fresh project. Refusals name the cause without
credentials or content:

| Condition | Refusal |
| --- | --- |
| missing or deleted WAL segment | `archived WAL is incomplete: segment … is missing` |
| corrupt object | `repository verification found corrupt or unreadable backup material` |
| wrong passphrase | `repository metadata did not open; the cipher passphrase is wrong …` |
| expired or revoked credential | `repository credentials were refused; renew …` |
| untrusted CA | `repository TLS certificate was not trusted …` |
| wrong custodian key or replaced header | `recovery custody bundle did not open under this custodian key` |
| different image, tenant or repository | names the mismatched binding |
| KEK reference not held by the custody set | `recovery verification found tenant key reference …` |

An interrupted restore leaves the target refused for reuse, and pgBackRest
restores `pg_control` last, so a partial copy cannot start. Restore never
writes to the source project or its repository.

### Before admitting restored traffic

A restore reproduces identity and policy as they were at the target point.
Before you publish the target, disable or revoke every account, client,
session or grant revoked since then in Keycloak and the external directory,
let realm convergence and directory pull complete, and confirm the selected
policy packs and Configuration. Then reconnect clients. A historical
database restore cannot prove present-day authorization.

### Recurring drill and retention

`deploy/compose/scripts/pitr-drill.sh` restores one generation into a
disposable `acceptance-drill-*` project, then removes only that project. It
writes a JSON result and a Prometheus textfile `pitr-drill.prom` into
`SYNVEDA_PITR_DRILL_RESULT_ROOT`. The textfile has result, duration
(first command to browser login), generation age and last-success time.

```sh
SYNVEDA_RESTORE_SOURCE_PROJECT=synveda-reference-acceptance-prod \
SYNVEDA_BACKUP_ID=gen-20261022 \
SYNVEDA_PITR_DRILL_SUFFIX=acceptance-drill-20261023 \
SYNVEDA_PITR_DRILL_IPV4_POOL=10.231.47.0/24 \
SYNVEDA_PITR_DRILL_RESULT_ROOT=/var/lib/synveda-drills \
SYNVEDA_PITR_DRILL_REPOSITORY_CONFIG=/custodian/restore-pgbackrest.conf \
SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/custodian/synveda-custody.key \
deploy/compose/scripts/pitr-drill.sh
```

Schedule it with the host's scheduler, for example a systemd timer or a cron
entry that sets a new suffix per run. Alert when the textfile reports failure
or when no drill has passed within the owner's interval. A monthly drill and a
35-day staleness alert are proposals, not approved commitments. Base backups
never expire automatically. Run an explicit `pgbackrest expire` only after a
newer generation has passed a drill, and never expire the newest verified
generation.

### KEK replacement

1. Copy the project's current `synveda_kms_key` and `synveda_kms_key_ref` to
   two mode-0600 files outside the repository and the project secrets. These
   files are the retiring KEK. Keep them with every older custody generation.
2. Write a new key from `synveda kms keygen` and a new `local:` reference into
   the project's `synveda_kms_key` and `synveda_kms_key_ref`, keeping mode 0600.
3. Run the ceremony:

   ```sh
   export SYNVEDA_KMS_PREVIOUS_KEY_FILE=/custodian/retiring/kms_key
   export SYNVEDA_KMS_PREVIOUS_KEY_REF_FILE=/custodian/retiring/kms_key_ref
   make compose-kek-rewrap
   ```

`kek-rewrap` refuses an unchanged key or reference. It stops the gateway and
worker and runs `synveda kms rewrap` once as the ordinary gateway database
role. Then it recreates both services and repeats the smoke. The command
rewraps every deployment and tenant key generation, retired ones included, by
compare-and-set. It records `tenant.key.rewrapped` audit events with
references and counts, refuses any row on a third KEK and resumes after
interruption. Then take a new `pitr-backup` generation. Older generations open
only with the retiring KEK, and `pitr-restore` refuses a custody set whose
reference the restored database does not name.

### Local mechanics drill

`sh scripts/ops5-local-pitr-drill.sh` checks only the pgBackRest mechanics on
a disposable Docker network with a pinned local S3-compatible server and
private CA. It does not test off-host custody, the Synveda application,
Keycloak or KMS.

## Logical backup and isolated restore

Recovery currently supports the bundled PostgreSQL and bundled Keycloak modes
only. Keep the exact source runtime, project suffix, profiles, hosts and image
selectors used to start the stack. If the source uses a non-default
`SYNVEDA_BOOTSTRAP_TENANT_ID`, retain that exact UUID for backup and restore;
the manifest binds it and a mismatch is refused. A backup verifies and smokes
the running graph, pauses the gateway, worker and Keycloak writers, creates separate
PostgreSQL 17 custom archives, snapshots the KMS key/reference and surviving
Keycloak convergence credential into a separate root, resumes the writers and
smokes again:

    export SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    export SYNVEDA_BACKUP_ID=20260908-reference-1
    make compose-backup

If `SYNVEDA_BACKUP_ID` is omitted, the lifecycle uses a UTC-second identifier.
An ID is immutable and is never overwritten. Default mode-0700 roots are:

    deploy/compose/backups/database/<source-project>/<backup-id>
    deploy/compose/backups/secrets/<source-project>/<backup-id>

Both roots are ignored by Git and the image build context, but they still live
on the same host. For operator storage, set absolute existing mode-0700
`SYNVEDA_DATABASE_BACKUP_ROOT` and `SYNVEDA_RECOVERY_SECRETS_ROOT` paths owned
by the runtime UID/GID. They must be canonically non-overlapping with one
another and with active project inputs. Preserve both roots: the second
contains the KMS key and Keycloak credential needed to use the database pair.

Restore always targets a new suffixed project and private `/24`. It refuses
in-place recovery and requires the exact source, backup ID and derived target
project in one confirmation:

    export SYNVEDA_RESTORE_SOURCE_PROJECT=synveda-development-acceptance-local
    export SYNVEDA_BACKUP_ID=20260908-reference-1
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-restore1
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24
    export SYNVEDA_CONFIRM_RESTORE=synveda-development-acceptance-local:20260908-reference-1:synveda-development-acceptance-restore1
    make compose-restore-smoke

The command verifies both linked sets and the exact PostgreSQL image reference
before Docker mutation. It installs the recovered KMS key/reference and
Keycloak convergence password into the fresh target's normal secret set,
restores both databases, rebuilds planner statistics and reruns the existing
database authorities. Before tenant convergence can create product state, it
verifies the source bootstrap tenant and full audit chain through the ordinary
gateway database role, opens its current tenant data key, and proves an
unrelated key gets the expected cryptographic refusal. It then converges the
normal graph and completes the private browser login. The proxy publishes no
host port in this mode. Success leaves the target running for inspection.

For a reference-HTTPS restore, select `reference` and the target suffix first,
run `make compose-secrets`, and install the target certificate files exactly as
described below before invoking the restore. The source recovery manifests
must still match the selected immutable PostgreSQL image.

To remove the isolated target, retain its target suffix, private pool and
profiles, then use the ordinary exact-project commands:

    make compose-down
    SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-restore1 make compose-reset

Reset removes the restored volume but deliberately retains the target secret
set. Remove retained inputs separately only after confirming they are no
longer needed.

This is a planned-interruption logical recovery check, not an online backup or
cross-database atomic snapshot. It pauses only the canonical application
writers; independently connected database writers are not fenced. The
archives and recovery-secret set are sensitive and are not encrypted by this
tool. SHA-256 links detect accidental alteration, not malicious replacement,
and are not signatures. The KMS check proves tenant-data-key unwrap; Synveda
does not claim application-level encryption of Knowledge bodies. Same-host
storage is validation evidence, not disaster recovery. The PITR path above
does not change this logical path's limits.

If interruption reports that the exact-project lock was retained during
writer stop or backup, assume the four writers may still be stopped. Do not
delete the lock while its recorded PID or a bounded lifecycle child is alive.
Inspect the named Docker project and host process tree; once the recorded owner
and its children are conclusively gone, remove only
`/tmp/.synveda-compose-locks-<uid>/<source-project>.lock`. With the original
profiles still selected, run `make compose-down`; this removes the uncertain
containers/networks and the disposable browser credential-and-receipt volume,
but preserves the PostgreSQL volume and project inputs. For the
acceptance-project example above, then set `SYNVEDA_COMPOSE_PROFILES=demo`, run
`make compose-up`, and require `make compose-smoke` to pass. Never recursively
remove the lock directory. If process or Docker mutation state is uncertain,
leave the lock in place and escalate to the host operator.

## Patched pgvector on a retained bundled database

Bundled PostgreSQL images built with ADR-0153 carry pgvector 0.8.7. On a
volume created by v0.4.6 or earlier, the new image replaces the native
library at its next start. The bootstrap and `migrate` leave the 0.8.6
catalogue in place, and `migrate` prints a legacy-profile warning. Follow the
[retained pgvector transition](../../docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition):
take and verify a logical backup with writers stopped first, and run the
owner's `ALTER EXTENSION vector UPDATE TO '0.8.7'` through the PostgreSQL
service's administrator session only after the application upgrade passes.

## Same-schema product upgrade smoke

`make compose-upgrade-smoke` exercises a bounded application-image change
against an already accepted reference project. It requires bundled PostgreSQL
and Keycloak, the exact `demo,browser-acceptance` profiles, a project suffix,
reference TLS inputs and immutable image digests. Keep every ordinary
reference selector identical to the running project and set:

    export SYNVEDA_COMPOSE_RUNTIME=reference
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    export SYNVEDA_PRODUCT_STARTING_IMAGE=registry.example/synveda/product@sha256:<starting-digest>
    export SYNVEDA_PRODUCT_IMAGE=registry.example/synveda/product@sha256:<candidate-digest>
    make compose-upgrade-smoke

The references must resolve to different local image IDs. Under one project
lock and deadline, the command proves the running starting image, public
smoke, Keycloak browser login and existing product receipt. It pulls the
candidate by exact digest and runs `synveda db migrate --check` in a read-only
repeatable-read transaction. That check verifies the current schema epoch,
embedded migration ledger, database authority and forced-RLS catalogue without
running the SQLx migrator or changing persistent database state. This smoke
transitions product images only and requires an already current migration head;
an upgradeable older head needs the separate backup, writer shutdown and
migration sequence before the new image can serve.

Only the long-running product services gateway and worker are
image-transitioned; the disposable browser-acceptance service is rerun at
each checkpoint. The command verifies candidate, starting rollback and final
candidate in turn; every checkpoint repeats exact container image identity,
public smoke, browser login and the existing product receipt. Success leaves
the candidate running. An ordinary failed transition
attempts to restore the last fully verified image but still returns failure.
A timeout, signal uncertainty or failed restoration retains the exact-project
lock for operator inspection.

This is planned-interruption, current-schema application compatibility
evidence. It does not upgrade PostgreSQL, Keycloak, volumes or schema; it does
not establish a general N-1 support window, downgrade safety, zero downtime or
Helm/release parity. Those remain open upgrade and rollback work.

## Reference HTTPS

Reference mode publishes public traffic only on ports 80 and 443 and requires
real operator DNS, an explicit private /24, mounted certificate files and
immutable image references. The optional metrics UI remains loopback-only:

    export SYNVEDA_COMPOSE_RUNTIME=reference
    export SYNVEDA_PUBLIC_SCHEME=https
    export SYNVEDA_APP_HOST=app.example.com
    export SYNVEDA_AUTH_HOST=auth.example.com
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24
    make compose-secrets

Place a leaf-first PEM chain in the selected project's tls_cert file and the
matching unencrypted PEM private key in tls_key. Both must be mode 0600 and
readable by the configured runtime UID/GID. Then supply matching
digest-addressed product, PostgreSQL, Keycloak and proxy image references. Use
one published environment manifest once release publication provides it:

    make compose-config
    make compose-up
    make compose-smoke

Certificate preflight checks format, chain adjacency, key match, hostnames and
remaining validity. It does not establish public trust, DNS ownership or
revocation status. ACME and automatic renewal are not implemented.

## External OIDC

The same product image runs without bundled Keycloak:

    make compose-down
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove
    export SYNVEDA_OIDC_MODE=external
    export SYNVEDA_OIDC_ISSUER=https://identity.example.com/realms/example
    export SYNVEDA_OIDC_ISSUERS_FILE=/absolute/path/to/issuers.json
    make compose-hosts-plan
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:- \
      make compose-hosts-install
    make compose-hosts-status
    make compose-resolver-check
    make compose-config
    make compose-up

The mounted issuer document owns the provider-neutral client ID, API audience,
supported algorithms, login scopes and static bootstrap-tenant binding. The
issuer diagnostic validates discovery, exact issuer, PKCE S256, JWKS and the
configured algorithm/audience contract before gateway readiness.

Create the mounted file as mode 0600 with the provider's exact values. This is
the minimal single-tenant shape; add provider-required scopes only when they
are needed to place the configured group claim in tokens:

```json
[
  {
    "issuer": "https://identity.example.com/realms/example",
    "client_id": "synveda",
    "audience": "synveda-api",
    "algorithms": ["RS256"],
    "tenant": {
      "static": {
        "tenant_id": "019b53c0-7c00-7000-8000-000000000045"
      }
    },
    "groups_claim": "groups",
    "login_scopes": ["openid", "profile", "email", "groups"]
  }
]
```

The API audience must differ from the login client ID. The provider must emit
the configured audience and group claim; `synveda-admins` is used only for the
one-time bootstrap described below, not ongoing Synveda authorisation.

In development, the hosts manager owns only the application .test name in this
mode. The external provider retains its own DNS name. If no bundled hosts block
exists, skip its removal; if one exists, remove it before changing selectors so
the exact ownership confirmation remains valid. Reference mode uses operator
DNS and does not run the hosts targets.

## External PostgreSQL

The executable external mode pairs external PostgreSQL with external OIDC.
The provider/operator must first provide PostgreSQL 14, 15, 16, 17 or 18
(the product detects the major), create database `synveda`, install
`btree_gin` 1.3 and an admitted `vector` profile for that major in `public`,
and create the database owner plus the exact protected role shape: NOLOGIN
capability role `synveda_app` and the least-privilege migrator, gateway and
worker logins with the memberships, ownership and ACLs required by the
selected role contract. It must also provide a server certificate for the
database DNS authority. Compose validates this authority and applies Synveda
migrations; it does not provision, reset, back up or restore an externally
owned cluster.

Render the DBA's setup script from the same role contract instead of writing
SQL by hand: `SYNVEDA_DATABASE_ROLES_FILE=<roles.json> synveda db setup-sql >
setup.sql`, or run the product image's `database-setup-sql` command with that
file mounted. The script selects PostgreSQL 14/15 or 16+ membership syntax by
itself and is safe to rerun. On PostgreSQL 14 and 15 the role contract must
declare no `administrative_memberships`.

The external-PostgreSQL row with bundled Keycloak places Keycloak's own
database on the same customer server through `SYNVEDA_POSTGRES_BOOTSTRAP_URL`
and `SYNVEDA_KEYCLOAK_DATABASE_URL`. That shared-server bootstrap depends on
PostgreSQL 17 startup-lock behaviour and stays qualified on PostgreSQL 17
only. With PostgreSQL 14, 15, 16 or 18 for Synveda, use external OIDC, or use
Helm, whose packaged Keycloak takes an independent `keycloak.database`
server.

Read the [PostgreSQL compatibility contract](../../docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility)
before provisioning: exact versions, schemas, trusted owners and catalogue
proof are required even for lexical operation. Azure Flexible Server's listed
PostgreSQL 17/pgvector 0.8.2 combination is version eligible in the source
candidate but remains provider-unqualified; released v0.4.4 still refuses it.
The CLI preflight proves roles/target identity; migration and full runtime
authority perform the complete extension proof. Keep those checks enabled.

Prepare an absolute mode-0700 secret directory owned by the configured runtime
UID:GID. Its path must end in the exact selected project and `/secrets`, for
example `/absolute/private/synveda-development/secrets`; a reference or
suffixed acceptance project uses its own exact project name. Create a
non-symlink mode-0700 `oidc-directory` child for the product's empty directory
secret. Put these non-symlink mode-0600 files in the secret directory:

- `synveda_migrator_database_url`;
- `synveda_gateway_database_url`;
- `synveda_worker_database_url`;
- `synveda_kms_key` and `synveda_kms_key_ref`; and
- `postgres_root_ca`, currently one PEM root certificate.

Each role URL has this exact TLS shape, with the password percent-encoded for a
URL and the role name varied per file:

    postgresql://synveda_gateway:<encoded-password>@database.example.com:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/postgres_root_ca

Prepare the external issuer document separately as described above. Its parent
must also be a non-symlink mode-0700 directory owned by the runtime UID:GID and
the file must be non-symlink, nonempty and mode 0600. It must not sit inside
the secret directory. Then set:

    export SYNVEDA_POSTGRES_MODE=external
    export SYNVEDA_OIDC_MODE=external
    export SYNVEDA_SECRETS_DIR=/absolute/private/synveda-development/secrets
    export SYNVEDA_DATABASE_ROLES_FILE=$PWD/deploy/compose/configs/database/roles.external-oidc.json
    export SYNVEDA_DATABASE_EXPECTED_HOST=database.example.com
    export SYNVEDA_DATABASE_EXPECTED_PORT=5432
    export SYNVEDA_DATABASE_EXPECTED_NAME=synveda
    make compose-config
    make compose-up
    make compose-smoke

The preflight checks all three URLs against the declared endpoint and requires
hostname-and-CA verification before migration or either runtime starts. Native
TLS adds the mounted CA to normal platform trust; it is not exclusive
certificate pinning. A deterministic graph check is not a live external-
database claim. External acceptance, reset and recovery remain explicitly
refused.

## External OTLP traces

The application always sends traces to the private Collector. To forward them
to an external public-PKI OTLP/gRPC receiver, set one non-secret DNS authority:

    export SYNVEDA_OTLP_MODE=external
    export SYNVEDA_OTLP_EXPORT_ENDPOINT=otel.example.com:4317
    make compose-up

The Collector uses TLS, bounded batching, an in-memory queue and a one-minute
retry window. The endpoint accepts no scheme, path, credentials or IP address.
Private CA, authentication-header and mTLS delivery are not implemented in
this slice. Metrics remain local when the observability profile is selected,
and normal smoke proves Collector configuration rather than remote receipt.

## Network and edge contract

The selector divides one operator-selected private /24 into fixed internal and
egress networks and refuses foreign overlap. Only Caddy publishes public host
ports; the optional Prometheus operator UI binds only to host loopback.

The proxy removes caller-supplied Forwarded, X-Forwarded-*, X-Real-IP,
identity, original-path and tracing/baggage headers before adding its own
bounded forwarding values. It applies request-body, header and upstream
timeouts. PostgreSQL, Keycloak management, worker health, application metrics,
OTLP receivers and recovery jobs remain private. Prometheus is the only
profile-owned operator UI and is loopback-only.

Containers use non-root users where supported, read-only roots where
compatible, dropped capabilities, no-new-privileges, PID/resource bounds and
init/signal handling. Nothing is privileged and no service mounts the Docker
socket.

## Reset

Reset is destructive and requires the exact confirmation emitted by the
lifecycle:

    make compose-reset

It acts only on the validated project containers, networks, PostgreSQL volume,
profile-selected browser credential-and-receipt, Prometheus or Apalis transport
volume and transient database-authority/Keycloak-gate state. It deliberately
retains the project's secrets, issuer document and KMS key. Use the same
profile selector that created profile-owned volumes, review the target and
supply the requested confirmation. Use compose-down when product/database
state must be retained; the disposable browser-acceptance credentials are
still removed.

## Current validation gaps

The Docker reference implementation covers the source,
deterministic-contract and packaged-release boundaries. The 2026-09-10
macOS/OrbStack clean-volume development run proved packaged-console login,
public-API product use, worker Capture completion, the native restart matrix,
persisted receipt verification and a non-destructive down/up that retained the
product-data volume and generated key set. Confirmed reset separately retained
the project keys. It still requires repetition on Linux and Docker Desktop,
reference HTTPS, live Apalis execution, paired logical backup and isolated
restore, and the same-schema product upgrade.

A general dashboard platform, ACME, HA, Helm promotion, signed provenance,
S3/WAL-PITR recovery and enterprise controls remain later production work.
Same-host logical recovery does not establish disaster recovery or an owned
RPO/RTO.
