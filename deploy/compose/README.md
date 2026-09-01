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
- Node.js 22 or newer, OpenSSL and a non-root Unix operator. Hosts mutation additionally
  requires a root-owned, non-writable, ACL-free Node binary and path at
  `/usr/bin/node` or `/usr/local/bin/node`; Linux requires the fixed
  root-controlled `getfacl` from the `acl` package. The lifecycle selects
  Node's bundled CA set explicitly for every host-side validator.
  Fixed `/usr/bin` and `/bin` `sudo`, `env`, `find`, `uname`, `awk`, `ls` and
  `getfacl` binaries and their root-owned system paths are part of the host OS
  trust base; the repository does not attest that base.
- Development host mappings that resolve each selected `.test` name to exactly
  `127.0.0.1`, with no IPv6 or additional address.
- A regular, single-link, root-owned `/etc/hosts` with exact mode `0644`, and an
  ACL-free file and physical parent directory. Noncanonical modes, access/default
  ACLs, bind mounts and externally managed host files are refused.

Print the exact development hosts-file block without changing the host:

```sh
make compose-hosts-plan
make compose-hosts-status
```

Install it with the repository-owned helper. The confirmation binds the
action, fixed loopback address, exact project and both selected names:

```sh
SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
  make compose-hosts-install
```

Run `make` and the Compose lifecycle as the ordinary operator. The mutation
target invokes only `manage-hosts-file.mjs` under an empty environment with a
root-owned, non-writable and ACL-free Node 22-or-newer binary and path at
`/usr/bin/node` or `/usr/local/bin/node`; it refuses caller-selected runtimes. This remains a full
administrator trust decision because the reviewed checkout script itself is
operator-writable. Use only a clean checkout whose diff and source you trust;
the empty environment and closed argv reduce accidental ambient input but are
not a sandbox against that operator. Never use `sudo make`, `sudo compose.sh`,
or run Docker, generators or browser acceptance as root. After installation,
flush only the host's active resolver cache. On macOS:

```sh
sudo dscacheutil -flushcache
sudo killall -HUP mDNSResponder
```

On Linux, use the command for the cache implementation that is actually
active, such as `sudo resolvectl flush-caches` or `sudo nscd -i hosts`. Then
prove textual ownership, the real resolver result and the local Docker
prerequisites in order:

```sh
make compose-hosts-status
make compose-resolver-check
```

The default names are `app.synveda.test` and `auth.synveda.test`. `.localhost`
is deliberately not used across container namespaces. The browser, gateway,
CLI, discovery document and tokens use the same issuer authority.

The manager hardcodes `/etc/hosts`, owns at most one development block, refuses
unmarked equivalent rows and every foreign, partial, duplicate or drifted
marker, and never prints the existing file. It keeps a mode-0600 root-owned
recovery record and a raw-content-free ownership record adjacent to the
physical host file. The ACL-free-parent preflight runs before any recovery bytes
are staged. The ownership record contains a full-file integrity digest; both it
and the required target are mode `0644`, while the full recovery copy is
root-owned mode `0600` with no access ACL. Installation appends only the terminal managed block and
removal truncates only that recorded suffix through the same open descriptor.
The inode, unrelated bytes, POSIX metadata, extended attributes, security
labels and file flags are therefore retained; ACL-bearing targets are refused.
A killed append may leave an
exact strict prefix of the block rather than an old-or-new atomic result; the
next exactly confirmed install or removal completes or truncates that proved
prefix. This does not promise preservation of modification/change timestamps,
power-loss atomicity beyond filesystem `fsync`, exclusion of a separate root
editor, or support for bind-mounted, network, immutable or externally managed
host files. Do not copy the recovery record, integrity digest or host-file
contents into deployment evidence.

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

Neither `compose-down` nor confirmed `compose-reset` removes the host-wide
mapping. After the exact project is stopped, remove only the helper-owned block
with the same runtime, OIDC, suffix and hostname selectors used to install it:

```sh
SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
  make compose-hosts-remove
```

Flush the active resolver cache again, then `make compose-hosts-status` must
report `absent`. Removal is idempotent only when both the selected mapping and
its ownership records are absent; drift or an unowned exact-looking block is a
refusal, not a global hostname deletion. A valid stale cooperative lock is
removed automatically only after its recorded PID is absent. A malformed lock
requires administrator inspection; never delete it while a helper process may
still be active.

Host-side validators start only through
`deploy/compose/scripts/run-node-closed`. Reference
`config`, `up`, `smoke` and `restart-gateway` refuse an ambient Node/OpenSSL
trust override, even when it is set to an empty value, before the first Node
process or project lock. Development and the recovery-oriented `down` and
confirmed `reset` actions scrub those controls from lifecycle children instead.
The wrapper selects Node's fixed bundled CA snapshot and disables ambient Node
proxy activation; ordinary proxy URL variables therefore do not reroute the
host smoke. This is not a custom-CA or outbound-proxy interface.

Docker client proxy configuration is not a canonical deployment input either.
Every selected service explicitly sets the upper- and lower-case HTTP, HTTPS,
NO, FTP and ALL proxy environment variables to the empty string. Every
development build sets the same ten build arguments to the empty string;
reference mode has no source builds. The rendered-contract gate rejects a
missing or non-empty value. After `compose-up`, and before smoke or either side
of a gateway restart, the asset gate requires the complete service, network and
volume inventory and inspects every exact container's immutable `Config.Env`
for one empty entry per name. This closes Docker CLI proxy auto-injection; it
does not add supported outbound-proxy routing. The legacy contributor Compose
file and the isolated database-test harness are outside this canonical wrapper.

Development `up` also refuses every recognised ambient BuildKit, Buildx and
Bake control before starting a helper or taking the project lock. After the
local Unix Engine endpoint is pinned, the wrapper requires the resulting
Docker context to be exactly `default`, creates a fresh mode-0700 Buildx state
directory outside the repository, and runs an explicit
`compose build --builder default`. Startup then uses `up --no-build`; reference
startup and gateway recovery are always no-build paths. Other lifecycle
actions scrub the same controls. Entering the build marks Docker mutation state
uncertain. A failed, timed-out or catchably interrupted build therefore retains
the exact project lock for operator recovery even though cleanup removes the
private Buildx scratch directory; only a cleanly settled build advances to the
separate startup mutation phase. The canonical child environment disables
optional Compose Bake selection and Bake environment-variable lookup, while
preserving `DOCKER_CONFIG` opaquely for private-registry authentication and
passing `DOCKER_AUTH_CONFIG` through unchanged where the installed client
supports it. The lifecycle never opens or parses credential content and never
rewrites or prints either environment value. It resolves only the effective
Docker config directory's physical path metadata and refuses development builds
when that directory or the lifecycle temporary root is inside the repository,
so neither credentials nor temporary evidence can enter the source context.
When `DOCKER_CONFIG` is unset, a non-empty accessible `HOME` is required and the
prospective `.docker` path is checked even when it does not exist. An existing
`config.json` must be a non-symlinked regular file; the lifecycle checks metadata
but never opens it. Private Buildx state is removed on ordinary and catchable
cleanup; an uncatchable process death cannot make that cleanup guarantee.

The installed Docker CLI, Compose and Buildx plugin binaries, plugin discovery
configuration, credential helpers, registry authentication and local daemon
policy remain operator-trusted inputs. The daemon's mirrors, proxy, CA and
embedded BuildKit policy are not isolated by this wrapper. A clean live build
with canary remote-builder state and private-registry authentication remains
required before this deterministic boundary is deployment evidence. Hardlinks,
bind mounts and hostile same-user path replacement remain trusted-host limits.

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

Asset state is explicit: `existing` permits an absent or partial exact project
so recovery can proceed, `converged` requires every rendered container, network
and volume plus the closed runtime proxy environment, and `stopped` requires
containers and networks to be absent. A deterministic post-create contract
refusal releases the lifecycle lock so the exact project can be taken down or
force-recreated; an unavailable, timed-out or otherwise uncertain inspection
retains the fail-closed lock.

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

In development external-OIDC mode the hosts manager owns only the selected
application name; the provider issuer keeps its own DNS. Use the same exported
selectors for plan, status, install, resolver check and removal. The exact
confirmation ends in `:<app-host>:-` because no identity hostname belongs to
the block. A different project suffix or hostname produces a different
confirmation and never shares ownership with the default block.

Reference mode requires operator DNS, a leaf-first PEM fullchain containing the
leaf and any intermediates but not the trust root, a matching unencrypted PEM
private key, an explicit private `/24`, and digest-addressed
product/provider/proxy images. Only ports 80 and 443 are published. It does not
manage `/etc/hosts`.

First replace the documentation-only names and export the reference settings:

```sh
export SYNVEDA_COMPOSE_RUNTIME=reference
export SYNVEDA_APP_HOST=app.example.invalid
export SYNVEDA_AUTH_HOST=auth.example.invalid
export SYNVEDA_PUBLIC_SCHEME=https
export SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24
make compose-secrets
```

The secret generator creates the mode-0700 project directory but never invents
a certificate. Copy the operator fullchain and key into the selected
`synveda-reference` project's ignored runtime secret directory, naming them
`tls_cert` and `tls_key`. Set both files to mode 0600 and the configured runtime
UID:GID, then supply the digest-addressed images from the matching environment
manifest and run `make compose-config` before `make compose-up`.

The preflight accepts one through eight leaf-first `CERTIFICATE` blocks and one
matching unencrypted `PRIVATE KEY`, `RSA PRIVATE KEY` or `EC PRIVATE KEY`
block. It refuses an included self-signed trust root. Every supplied certificate
must be valid through the remaining bounded lifecycle. Bundled mode requires
DNS SAN coverage for both hosts; external OIDC requires the application host
only. A conventional one-label wildcard SAN is accepted, but CN fallback,
partial wildcards and multi-label wildcards are not.
The preflight proves parsing, adjacent chain signatures and key/hostname
coherence. It does not prove public trust, revocation, DNS ownership or what a
live endpoint serves. The reference runtime smoke separately requires HTTPS
for both configured URLs. It validates the application routes and, in bundled
OIDC mode, the issuer routes with Node's bundled CA set; external-OIDC host
smoke enforces only the issuer scheme. Passing the probes therefore requires
their served chains to be trusted by that set, but it still does not prove
browser trust or public-PKI ownership independently. There is no automatic
renewal, and ACME is not implemented in this checkpoint. Certificate validity
never blocks canonical `down` or `reset`.

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
