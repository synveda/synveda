# Plain Compose consumer bundle

This 0.4.3 bundle is part of the matching published GitHub Release; the earlier
v0.4.0 launcher remains available. This bundle uses the
existing evaluation services with named private state; it cannot adopt the
launcher's retained host state. Its artifact gate includes paired backup/restore;
native Docker Desktop qualification remains separate.

The refactored CI and Release workflows test this extracted bundle against
the exact native AMD64/ARM64 OCI candidates, including paired recovery. The
tagged release separately verifies anonymous pulls, source labels and executable
smoke for the copied Docker Hub and GHCR digests. The public gate does not
repeat the full recovery drill. Public installation needs no publisher token
and never builds images from source.

The matching source CLI offers native lifecycle commands over this graph,
with engine/project/bundle ownership receipts (`docs/CONSUMER_CLI.md` in the
source checkout). Start that route
with a fresh project; it cannot adopt an existing direct-Compose installation.
For a published client release, use its matching native `synveda-client-*`
archive from the GitHub Release; all six OS/architecture packages are required
by the pipeline. Use the published installation instructions in
`docs/CONSUMER_CLI.md`.

For a disposable evaluation on a local Linux-container Docker engine, extract
the verified release archive into a directory and run:

```sh
docker compose up -d --wait --wait-timeout 900
docker compose run --rm --no-deps credentials
```

Open `http://localhost:8080/console/` and sign in as `synveda-demo-admin` with
the private password printed by the deliberate retrieval command. Startup logs
do not contain passwords. Docker Compose 2.35 or later and an engine supporting
named-volume subpaths are required. No Git, Rust, Node, npm, pnpm, Make, host
OpenSSL, host mappings or installed certificates are used at runtime. Use a
local Docker context with access to the extracted files; remote daemons are
outside this bundle's qualification.

The optional fictional sample uses real browser sign-in and public workflows:

```sh
docker compose run --rm --no-deps --entrypoint node browser-acceptance product-demo.mjs sample
docker compose ps --all
docker compose logs --tail 100
docker compose down
docker compose up -d --wait --wait-timeout 900
```

`node` in the sample command is inside the pinned optional browser image; it is
not a host dependency. The sample leaves review and approval to the user. The
four evaluation accounts are synthetic and their passwords are unique to the
installation. This is a loopback HTTP evaluation, not a team-access deployment.
Use the existing reference configuration for HTTPS, external PostgreSQL or
generic OIDC.

Normal `down` preserves PostgreSQL, identity, credentials, encryption keys and
the optional browser fixture's private state. Keep the installation and database
volumes together. Do not use `down --volumes`, remove a key, change the project,
or change issuer/port options beside retained data. Initialization refuses
missing or changed private material; it never repairs that by generating new
keys. The release and issuer configuration are fixed for that installation.
There is no implicit cross-release database upgrade or layout migration.

The default project is `synveda-local`. Test automation may select an isolated
`synveda-local-acceptance-<name>` project before its first run. The initializer
has no network and no Docker socket; it completes before the existing database,
identity and application gates. Each runtime receives only its own secret
subset. Readiness still requires database preflight, migrations, tenant and
issuer checks, and the Keycloak generation gate.

## Paired logical recovery

Use the bundle's `synveda-recovery` command from the extracted archive. The
host-state launcher's recovery commands do not operate these named volumes.
Do not run direct Compose commands concurrently with recovery. For the default
`synveda-local` installation:

```sh
./synveda-recovery backup checkpoint-1
./synveda-recovery verify checkpoint-1
```

Backup first verifies the installation's original secret seal and reserves a
new immutable ID. It stops all profiles, runs only PostgreSQL for the native
logical dumps, and validates both databases against the linked key set and
original private configuration. Success leaves the source **down**, with all
volumes retained. Restart it with `docker compose up -d --wait --wait-timeout
900` when a restore drill is not in progress. An existing backup ID is refused.

The separate `synveda-local_recovery` volume contains
`checkpoint-1/database/{manifest.json,synveda.dump,keycloak.dump}`, the existing
`keys/` recovery set, and `configuration/` plus `configuration.json`. Keep the
whole set together with the exact bundle. Configuration includes the original
issuer, installation identity, secret seal, passwords and encryption keys.
The database and key manifest formats are unchanged. Neither a retained
database volume nor a Keycloak realm export substitutes for this set.

Restore uses a **different, empty project** and exact source:backup:target
confirmation. It refuses retained target containers, volumes or networks before
allocating state. The source must remain down because the restored identity
uses the original issuer and network options:

```sh
export COMPOSE_PROJECT_NAME=synveda-local-acceptance-recovery
export SYNVEDA_RECOVERY_SOURCE=synveda-local
export SYNVEDA_CONFIRM_RESTORE=synveda-local:checkpoint-1:synveda-local-acceptance-recovery
./synveda-recovery restore checkpoint-1 synveda-local
```

The restore reads the recovery volume without needing the source database or
installation volume. It verifies archive hashes, the paired keys, configuration
inventory and original seal before copying anything. Only the explicit target
project changes in the installation identity. Both target databases must be
empty and idle. Existing database bootstrap commands reconverge their authority
after restore; the ordinary gateway-role verifier checks the source tenant,
audit chain and tenant-key unwrap before tenant convergence. A separate probe
must refuse an unrelated encryption key.

The restored proxy has **no published port**. Use the private browser fixture
to verify real sign-in, API access and logout, then stop the restored graph:

```sh
docker compose --project-directory . -f deploy/compose/consumer-runtime.yaml -f deploy/compose/consumer-restore.yaml run --rm --no-deps --entrypoint node browser-acceptance evaluation.mjs
docker compose --project-directory . -f deploy/compose/consumer-runtime.yaml -f deploy/compose/consumer-restore.yaml down
```

Keep `COMPOSE_PROJECT_NAME` and `SYNVEDA_RECOVERY_SOURCE` as above for those
commands. Unset them before using the source's ordinary lifecycle. The artifact
gate additionally checks the original sample receipt against restored product
rows and compares every original secret hash. It leaves both projects stopped
and retains their installation, database, browser and recovery volumes.

An interrupted operation leaves `.recovery-operation` in its installation
volume; normal initialization refuses while it exists. Inspect the exact
project's containers and the recovery command's process before doing anything
else. Once all recovery processes/containers have stopped, keep the incomplete
backup or target for diagnosis. For an interrupted **backup**, remove only the
empty marker with `docker compose run --rm --no-deps recovery-state unlock
checkpoint-1`, then use ordinary startup and a new backup ID. An interrupted
**restore** must not start the application: retain it and choose another empty
target for a fresh restore. Do not remove database volumes to make a retry pass.

These archives and private configuration are unencrypted and same-host by
default. Hashes detect alteration; they are not signatures. Canonical writers
are stopped, but independent database clients are not fenced. This is a
planned-interruption full logical recovery check, not WAL/PITR, encrypted
off-host custody or a production RPO/RTO guarantee. OPS-5 owns those requirements;
OPS-12 tracks remaining consumer and platform qualification.
