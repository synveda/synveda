# Run with Docker

Run the versioned prebuilt bundle with bundled dependencies for a local
evaluation, or select the reference HTTPS configuration for infrastructure you
operate. Both use the same Compose services, database authority checks,
Cedar, forced RLS, VedaFlow and audit.

<!-- installation-version: 0.4.0; publication: unreleased -->
**Publication: 0.4.0 is unreleased.** These are candidate instructions. The
owner's v0.3.0 build is separate; do not combine its images with this bundle.
The [release manifest](../../docs/installation.json) owns this status.

## Requirements

An ordinary account with access to a **local Docker daemon**, Compose 2.33.1+
(the minimum for the existing merge contract), curl, tar and a SHA-256 utility.
Actual candidate testing uses macOS/OrbStack, Engine 29.4.0, Compose 5.1.2 and
Apple Silicon. Other host/architecture combinations remain unqualified until
their recorded runtime gates pass. Use at least 6 GiB available to Docker for
PostgreSQL, Keycloak, gateway, worker and private telemetry; this is an
operational starting allocation, not a measured capacity guarantee.

No Git, Rust, host Node/OpenSSL, Make, Buildx selection, DNS edits, external
account, paid model or public wildcard DNS is required for local evaluation.
Preparation runs briefly in the product image with no network or Docker socket.
The localhost endpoint is plaintext and binds only 127.0.0.1. Use reference
HTTPS for remote users. Do not forward the local port onto an untrusted network.

## Download and verify

The following download is **pending publication**. After 0.4.0 is approved:

```sh
version=0.4.0
release_url="https://github.com/synveda/synveda/releases/download/v$version"
mkdir "synveda-$version"
cd "synveda-$version"
curl -fLO "$release_url/synveda-reference-$version.tar.gz"
curl -fLO "$release_url/SHA256SUMS"
awk -v file="synveda-reference-$version.tar.gz" \
  '$2 == file { count++; print } END { if (count != 1) exit 1 }' \
  SHA256SUMS > reference.sha256
# macOS:
shasum -a 256 --check reference.sha256
# Linux alternative: sha256sum --check reference.sha256
tar -xzf "synveda-reference-$version.tar.gz"
cd "synveda-reference-$version"
cat environment.json
./synveda-compose up
```

Stop on a checksum failure. Checksums detect corruption; unsigned assets do
not authenticate their own download channel. `environment.json` binds source,
version, image digests and dependencies. The launcher never builds source or
selects `latest`. No registry login should be required for a published bundle;
an anonymous-pull failure is a release defect, not a reason to supply a
publisher's token.

The first download is separate from startup and may be large. `up` waits for
health, completed bootstrap/migrations and issuer diagnostics. It prints the
console URL without passwords. Normal logs contain no secret values.

Before the first `up`, edit the small `evaluation.json` file if port 8080 is
occupied or its private /24 overlaps a VPN. Supported keys are `port`, `subnet`
and the explicit `demoAccounts` choice. Their identity is fixed in private
state after preparation; an accidental later change is refused. Use paths
without spaces or shell metacharacters. Do not set source-development
`SYNVEDA_*` variables to override the prepared contract.

## First workspace and sample

Open **http://localhost:8080/console/**. Retrieve a generated password deliberately
in your terminal:

```sh
./synveda-compose credential author
```

Sign in as `synveda-demo-admin` (Avery Author). The generated accounts are local
evaluation identities, never universal passwords. Create an empty workspace
through Getting started, or opt into fictional sample content:

```sh
./synveda-compose sample
```

The optional browser/CLI image signs in the four distinct generated identities
and reuses the existing governed demo through public APIs. It creates **Northstar
Delivery → Ingestion API**, a synthetic source Session, a baseline Knowledge
revision, a proposed retry learning and a proposed Skill version. The command
never approves a review and requires no model API. Repeat it safely: its durable
receipt/idempotency keys preserve existing resources and reviewer edits.

1. In **Sessions**, open the fictional ingestion retry session and inspect its
   synthetic evidence. **New Learnings** shows the captured candidate and its
   proposed-change state.
2. Sign out. Run `./synveda-compose credential reviewer`, then sign in as
   `synveda-demo-member` (Riley Reviewer). Open **Reviews** and inspect the
   learning's source, proposed content and required reviewers before deciding.
3. Approve and apply the learning using the normal review controls. Approval
   and application are separate states; pending content is not active Knowledge.
4. In **Context**, select the Ingestion API scope and request context about
   ingestion retries. Inspect the selected immutable Knowledge revision and
   source evidence. The sample's Skill version requires the two distinct
   configured administrators; use `credential approver` only for a deliberate
   second evaluation decision, then create its binding through **Skills**.

These are your evaluation actions, not verified historical human reviews.
The automated acceptance fixture can perform explicit test acts; those are
reported separately. Live-agent verification is owned by the
[client support matrix](../../docs/CLIENT_SUPPORT.md).

Console **Sign out** removes the Synveda session and makes subsequent API reads
unauthenticated. Keycloak's SSO session is separate: use a private browser window
per identity, or end the provider session before switching accounts. No provider
logout is silently implied.

`demoAccounts:false` creates no evaluation users or sample. An identity
administrator must provision users and the initial `synveda-admins` mapping;
for real identities follow [existing infrastructure](#use-existing-infrastructure).
No external deployment is automatically seeded.

## State and ordinary lifecycle

Use one `SYNVEDA_HOME` (default `$HOME/.synveda`) throughout this deployment.
The private `state/synveda-evaluation` contains the matching issuer, database
credentials, Keycloak authority and encryption keys. PostgreSQL's named volume
contains **both** the Synveda and separate Keycloak databases, with different
owners and runtime roles. The optional sample's receipt lives in its own named
volume. Never delete keys while retaining encrypted data.

```sh
./synveda-compose status
./synveda-compose logs
./synveda-compose stop
./synveda-compose up
./synveda-compose restart   # gateway and worker only
./synveda-compose down      # removes containers/networks; preserves volumes/keys
./synveda-compose up
```

Mutation commands claim a private operation lock. A concurrent command refuses
with exit 75. After a killed host process, confirm that its Docker operation and
containers have stopped before removing the **empty** `state/.evaluation-operation`
directory. Preparation uses an additional bounded lock and atomic files; do not
repair concurrency by deleting secrets. Missing keys beside retained data cause
a refusal with recovery guidance.

## Backup and restore

The existing PostgreSQL 17 logical tooling quiesces gateway, worker and Keycloak,
dumps both databases, validates the archive/key pair and resumes services. The
command temporarily interrupts service. Choose a new lowercase backup id:

```sh
./synveda-compose backup before-update
```

Keep the entire private `state/backups/before-update` directory together. It
contains database dumps, hashes and the matching keys, credentials and issuer
configuration. Copy it to separately protected storage. This command does not
provide encryption at rest for backup media, off-host storage or disaster recovery.
An incomplete backup is retained for diagnosis and cannot be restored as complete.

Restore only into an **absent evaluation database volume**, with the matching
bundle, `evaluation.json`, state and original canonical localhost port. It refuses
an existing volume and incompatible/mismatched private state:

```sh
SYNVEDA_CONFIRM_RESTORE=restore:synveda-evaluation:before-update \
  ./synveda-compose restore before-update
```

For a new host, securely copy the backup under the same relative state path
first. Restoring into the current disposable evaluation requires the separate
explicit reset below. Restore checks the tenant, audit prefix and encryption
key before normal startup. Sign in and inspect the expected workspace afterwards.
A logical data restore is different from rolling an application image back.

## Update and removal

Download and verify the complete next bundle; retain the old bundle and private
state. Back up first. Use the new bundle only when its notes explicitly support
your schema epoch. Keep the same runtime selection: an existing HTTPS reference
deployment must retain `SYNVEDA_COMPOSE_RUNTIME=reference`. Switching to the
loopback evaluation creates a separate deployment; it does not migrate the
reference volume, realm or issuer. The current baseline is **epoch 3**. Earlier epochs fail with
reset guidance; there is no compatibility migrator or supported v0.2.0 → current
data upgrade. Local same-epoch reapply is evidence only for the tested revisions.
Helm/application rollback never reverses SQL migrations.

Ordinary removal is `down`, followed by deliberate removal of the extracted
archive if desired. Data and secrets survive. For a disposable destructive reset:

```sh
SYNVEDA_CONFIRM_RESET=delete:synveda-evaluation:postgres-data \
  ./synveda-compose reset
```

This deletes exactly the named evaluation database volume, including Keycloak
identity state; it retains the generated keys and backups. Remove the optional
sample receipt volume only when deliberately abandoning that fixture. Global
Docker pruning is never part of installation, upgrade or removal.

## Use existing infrastructure

Select `SYNVEDA_COMPOSE_RUNTIME=reference` explicitly and follow the existing
[reference prerequisites](README.md#reference-https) and
[external PostgreSQL](README.md#external-postgresql) /
[external OIDC](README.md#external-oidc) inputs. This advanced route retains its
host Node 22+/OpenSSL, operator DNS and trusted TLS prerequisites. Replace its
`make compose-*` calls with the extracted `./synveda-compose <action>` launcher.
It is a different exposure/configuration selection of the same Compose graph.
For bundled providers on that HTTPS route, prepare the private inputs after
setting the required hostnames:

```sh
export SYNVEDA_COMPOSE_RUNTIME=reference
export SYNVEDA_APP_HOST=app.example.com
export SYNVEDA_AUTH_HOST=auth.example.com
./synveda-compose secrets
./synveda-compose issuer
```

Install the required TLS certificate/key files using the reference guide before
`up`. External mode requires supplied files instead of these bundled helpers.

| PostgreSQL mode | OIDC mode | Database ownership |
|---|---|---|
| bundled | bundled | Synveda and Keycloak databases in the bundled server, separate owners |
| external | external | Provider-owned application DB; provider-owned identity |
| external | bundled | Supply the application DB **and a distinct durable Keycloak DB** |
| bundled | external | Bundled application storage; external realm is not mutated |

Never hand an application runtime a PostgreSQL superuser or realm administrator
credential. Use the [existing service contract](../helm/synveda/CONFIGURATION.md)
for roles, CA/hostname verification, issuer/client/audiences and initial admission.
Generic OIDC support does not qualify every provider.

## Troubleshooting

- **Port already allocated:** choose an unused port in `evaluation.json` before
  preparation. A persisted issuer cannot be silently moved to another port.
  If allocation fails after preparation, free the configured port, then run
  `down` followed by `up` to recreate its binding while preserving data and keys.
- **Network overlap:** select a private unused /24; do not remove unrelated
  Docker networks to make room.
- **Identity unreachable or wrong issuer:** `up` fails its bounded diagnostic.
  Check the printed endpoint and state; never disable issuer/audience checks.
- **Missing/unsafe private files:** restore the original private state. The
  launcher refuses permissive files and replacement credentials beside data.
- **Storage exhausted:** free or increase storage on the selected Docker host,
  retain the volume and keys, then rerun preparation/startup. A failed write is
  not evidence of completed bootstrap.
- **Interrupted backup or restore:** retain its files, inspect `status` and
  non-secret logs, and follow the fresh-target rule. Do not reset useful data
  to bypass a diagnostic.

[Readiness and platform evidence](../../docs/PRODUCTION_READINESS.md) ·
[Build from source](../../CONTRIBUTING.md#local-deployment)
