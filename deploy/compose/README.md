# Canonical Docker Compose deployment (CPR-45)

This directory contains Synveda's canonical single-host Compose graph. The
current checkpoint has an executable, convergent lifecycle for bundled
PostgreSQL with either bundled Keycloak or external OIDC. It is still
development/reference implementation evidence, not controlled-use acceptance:
a clean browser PKCE exchange, a clean Linux run, reference HTTPS, backup,
restore and upgrade remain open.

The contributor-only `make dev-up` stack still contains Rauthy and Temporal
residue. It is not an alternative reference deployment and will be removed
only after the replacement acceptance is complete.

## Prerequisites

- Docker Engine 28 or newer, reached through a local Unix socket. Remote Docker
  contexts are refused because loopback publication and host resolver checks
  would describe the client host rather than the Engine host. The lifecycle
  captures that validated socket, clears `DOCKER_CONTEXT`, and pins every later
  inventory and mutation to the same `DOCKER_HOST` value.
- Docker Compose 2.33.1 or newer.
- Node.js 22, OpenSSL and a non-root Unix operator.
- Development host mappings that resolve each selected `.test` name to exactly
  `127.0.0.1`, with no IPv6 or additional address.

Print the exact development hosts-file block without changing the host:

```sh
make compose-hosts-plan
```

Install that marked block using the host's normal administrator procedure, then
prove the resolver and Docker prerequisites:

```sh
make compose-resolver-check
```

The default names are `app.synveda.test` and `auth.synveda.test`. `.localhost`
is deliberately not used across container namespaces. The browser, gateway,
CLI, discovery document and tokens use the same issuer authority.

## Canonical lifecycle

The default is development with bundled PostgreSQL and bundled Keycloak:

```sh
make compose-config
make compose-up
make compose-smoke
make compose-restart-gateway
make compose-down
```

`compose-up` performs the host/Docker-network preflights, creates or validates
the project-scoped secret set and issuer input, validates the rendered graph,
and converges the containers. Re-running it does not rotate credentials. The
ordered one-shot services bootstrap database authority, migrate the schema,
converge the exact tenant, converge the Keycloak realm, and diagnose the public
issuer before the gateway becomes ready. `compose-smoke` requires every exact
service and one-shot result, then probes the real host resolver path for public
health, console, issuer and management/metrics refusal. It is not a browser
login test. `compose-restart-gateway` first requires that same complete smoke,
restarts only the existing gateway container under the exact-project lock,
waits up to 120 seconds for its declared health without recreating it, requires
the full container identity to remain unchanged, and repeats the complete
smoke. Its non-replenishing lifecycle deadline retains a 40-second postflight
reserve across all final checks plus a five-second orchestration margin. The
command provides the bounded lifecycle action for a separate live test that an
already completed browser session survives a gateway process restart; it does
not itself prove browser state. An in-flight login is intentionally
process-local and is not expected to survive.

`compose-down` stops containers while retaining the database volume and all
project input. The generated files live under
`runtime/synveda-development/` by default; secret files are mode 0600 beneath a
mode-0700 project directory and their values are never printed by the
lifecycle.

Every mutating lifecycle action and authority-file generator holds one
operator-owned, exact-project lock across preparation and Docker mutation.
The whole operation shares one monotonic 240–3600 second elapsed-time budget
(900 seconds by default); signals are forwarded to the active process group,
followed by a five-second termination grace and a bounded forced-stop check.
A child process group that cannot be proved gone returns an uncertainty status
instead of allowing another lifecycle to overlap it. The lifecycle also
inventories the complete rendered network/IPAM contract and every retained
project network before startup. It refuses a concurrent owner, a stale or
drifted network, an overlapping foreign network, or an asset that changes
between validation and use; it never repairs those states by deletion.

An uncatchable operator/process death, an unclean bounded child, a missing
private completion witness, or any failed, timed-out or interrupted Docker
mutation intentionally leaves the lock in a fail-closed state. A catchable
signal during read-only preparation releases the lock only after its complete
child process group is proved gone. Recover only
the exact project after proving that its owner PID is gone, the local Engine is
reachable, and no lifecycle or Docker mutation for that project is active:

```sh
PROJECT=synveda-development
LOCK_ROOT="/tmp/.synveda-compose-locks-$(id -u)"
LOCK_FILE="$LOCK_ROOT/$PROJECT.lock"
cat "$LOCK_FILE"                      # must be exactly PROJECT:PID
ps -p PID                              # substitute the recorded decimal PID
rm "$LOCK_FILE"                       # only after ps proves it is absent
```

Do not remove the root, glob across projects, or remove a lock when the PID,
child process group or Engine mutation state is uncertain.

Reset is deliberately separate and destructive. It refuses unknown state,
validates the exact Compose volume labels, and requires the exact project name:

```sh
SYNVEDA_CONFIRM_RESET=synveda-development make compose-reset
```

Reset removes only the exact PostgreSQL data volume and generated
database-authority/Keycloak-gate state. It preserves the secret set, issuer
input and Synveda KMS key so the next convergence can prove stable key custody.
It is not tenant erasure, backup or credential rotation.

## Demo identities

The optional `demo` profile extends the existing, generation-fenced realm
convergence job; it does not add a second identity controller:

```sh
SYNVEDA_COMPOSE_PROFILES=demo make compose-up
SYNVEDA_COMPOSE_PROFILES=demo make compose-smoke
```

It owns two target-realm users, `synveda-demo-admin` and
`synveda-demo-member`. Their strong passwords are generated into the
project-scoped `keycloak_demo_admin_password` and
`keycloak_demo_member_password` secret files. Read those files only through a
local password-input mechanism that does not copy values into shell history or
logs. The administrator is a member of `synveda-admins`; the member has no
group. Neither user receives a Keycloak domain role. Synveda's first-admission
rule may use the group claim, after which Synveda grants, Cedar, forced RLS,
VedaFlow and audit remain authoritative.

The image carries one review-locked Keycloak 26.7.2 user-profile contract. It
retains the four upstream built-in attributes and permits the two Synveda demo
ownership attributes only to administrators, with exact lengths and option
sets. `unmanagedAttributePolicy` is absent: in the pinned implementation,
missing/null is the disabled state. Convergence closes the realm before a full
no-merge profile replacement, proves the exact readback before using ownership
markers, and refuses to adopt or delete marked identities when a previously
drifted profile makes their provenance untrustworthy.

Every demo convergence resets both passwords with non-temporary credentials,
removes other credentials, direct roles and unexpected groups, proves that the
bootstrap group contains only the owned demo administrator, and refuses to
adopt a same-named user that lacks the Synveda demo ownership marker. A later
`compose-up` without `demo` closes the realm and deletes only those exactly
owned demo users before reopening it; stopping containers or merely changing an
environment variable does not itself mutate persisted identity state.

## File selection and provider modes

The wrapper owns file and profile selection in this order:

1. `compose.yaml`;
2. exactly one of `compose.dev.yaml` or `compose.reference.yaml`;
3. development image-build fragments for selected bundled providers;
4. `compose.postgres.yaml` when PostgreSQL is bundled;
5. `compose.keycloak.yaml` when OIDC is bundled;
6. the matching Keycloak/PostgreSQL bridge;
7. external-provider egress fragments when selected;
8. `compose.demo.yaml` when `demo` is selected;
9. the remaining optional profile fragments.

`make compose-config` runs the complete deterministic matrix without starting
or pulling images. The accepted profile vocabulary is `semantic`,
`observability`, `apalis-board`, `demo` and `backup-test`; profiles without an
implemented service remain configuration-only. The demo profile requires both
bundled providers.

Bundled PostgreSQL with external OIDC uses the same product image and requires
an operator-supplied, mode-0600 issuer file plus `SYNVEDA_OIDC_ISSUER`. The
file must contain exactly one issuer entry with the exact configured issuer
and one static binding to `SYNVEDA_BOOTSTRAP_TENANT_ID`; the product diagnostic
rejects a different or non-static tenant mapping before the gateway starts.
The file also owns that provider's client ID, closed API audience and login
scopes under the common issuer schema. Fully external
PostgreSQL rows render and validate only: canonical `up` and `reset` refuse
them until the authenticated-TLS bootstrap and compiled SQLx transport
contract is implemented.

Reference mode requires operator DNS, HTTPS certificate/key files, an explicit
private `/24`, and digest-addressed product/provider/proxy images. Only ports 80
and 443 are published. It does not manage `/etc/hosts`:

```sh
SYNVEDA_COMPOSE_RUNTIME=reference \
SYNVEDA_APP_HOST=app.example.invalid \
SYNVEDA_AUTH_HOST=auth.example.invalid \
SYNVEDA_PUBLIC_SCHEME=https \
SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24 \
deploy/compose/scripts/compose.sh config
```

Replace the example names and pool; `.invalid` is documentation-only. Put the
certificate and key in the selected project secret directory as `tls_cert` and
`tls_key` before starting. ACME is not implemented in this checkpoint.

## Network and edge contract

The selector divides one canonical private `/24` into ten fixed `/28` networks.
Before startup it inventories Engine networks, refuses every foreign overlap or
stale/drifted project network, and accepts only an exact retained project
contract. It never deletes a conflict. A suffix of the form
`acceptance-<name>` isolates project names and volumes but still requires a
distinct explicit pool.

Only Caddy publishes host ports. It removes untrusted `Forwarded`, every
`X-Forwarded-*`, `X-Real-IP`, identity headers, original-path headers and
tracing/baggage inputs before installing its own bounded forwarding headers.
PostgreSQL, Keycloak management, worker health, OTel receivers, metrics,
backups and operator UIs remain private.

## Current limits

This checkpoint is not proof of browser login, desktop/Linux parity, reference
HTTPS, backup/PITR, isolated restore, upgrade/rollback, HA, host-loss tolerance,
hosted SaaS readiness or enterprise certification. The core Collector remains
private but currently exports to `nop`; the bounded observability profile and
Operations UI are open. Do not delete the legacy Rauthy/Temporal assets or
change the production-readiness verdict until replacement acceptance passes.
