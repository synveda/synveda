# Plain Compose candidate

This is an unpublished OPS-12 qualification artifact. The released v0.4.0
launcher remains the supported installation path. This candidate uses the
existing evaluation services with named private state; it cannot adopt the
launcher's retained host state. Paired backup/restore and native Docker Desktop
qualification are still required before publishing this path for consumer use.

For a disposable evaluation on a local Linux-container Docker engine, extract
the verified candidate archive into a directory and run:

```sh
docker compose up -d --wait --wait-timeout 600
docker compose run --rm --no-deps credentials
```

Open `http://localhost:8080/console/` and sign in as `synveda-demo-admin` with
the private password printed by the deliberate retrieval command. Startup logs
do not contain passwords. Docker Compose 2.35 or later and an engine supporting
named-volume subpaths are required. No Git, Rust, Node, npm, pnpm, Make, host
OpenSSL, host mappings or installed certificates are used at runtime. Use a
local Docker context with access to the extracted files; remote daemons are
outside this candidate's qualification.

The optional fictional sample uses real browser sign-in and public workflows:

```sh
docker compose run --rm --no-deps --entrypoint node browser-acceptance product-demo.mjs sample
docker compose ps --all
docker compose logs --tail 100
docker compose down
docker compose up -d --wait --wait-timeout 600
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

Do not store valuable data in this candidate until its paired recovery path is
qualified. The host-state launcher's recovery commands do not operate these
named volumes. The canonical progress and remaining acceptance requirements
are tracked by OPS-12 in the repository.
