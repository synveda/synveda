# Canonical Docker Compose deployment (CPR-45)

This directory contains Synveda's canonical single-host Compose graph. The
current checkpoint has an executable, convergent lifecycle for bundled
PostgreSQL with either bundled Keycloak or external OIDC. It is still
development/reference implementation evidence, not controlled-use acceptance:
the committed no-capture browser PKCE fixture has not run against live
containers, and a clean Linux run, reference HTTPS, backup, restore and upgrade
remain open.

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

Before a clean-Engine run, prepare the immutable candidate in a private state
root outside the checkout. Supply a collision-reviewed private `/24`; the tool
generates a 128-bit run identity and derives the exact 24-hex acceptance suffix.
The default state root is
`$HOME/.local/state/synveda/compose-acceptance`; an explicit
`SYNVEDA_CLEAN_ENGINE_STATE_BASE` must also be absolute, mode 0700, owned by the
current user and physically outside the repository.

```sh
SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24 \
  make compose-clean-engine-plan
make compose-clean-engine-status
make compose-clean-engine-verify
```

Planning performs no Docker, Colima or privileged hosts-file action. It writes
one canonical content-free candidate, immutable plan receipt and private
synthetic-proxy template, all mode 0600. The candidate records both the tracked
index and the actual effective Docker context, including modes, file bytes and
symlink targets; included untracked or empty directories fail closed. The
complete fsynced run publishes one no-replace hard-linked `active` receipt, and
a second plan refuses it. Do not edit or remove that link or receipt-owned run.
An uncatchable interruption before active-receipt publication can retain a validated
inert staging directory; it has no provider authority, does not block a later
plan and must be removed by the future final cleanup before success evidence.
More than eight retained inert directories fail closed.
The internal version-3 receipt state machine closes every intent/result,
failure-cleanup and success-only environment-manifest transition. Canonical
receipts and the manifest use no-replace publication. Its mutation journal is
append-only: permanent `.mutation-slot-SS`, `.mutation-close-SS` and
`.mutation-recovery-SS-RR` records are never deleted or reused. Each slot binds
the exact source receipt/environment endpoints and previous close digest; each
close binds the exact result endpoints, owner or recovery authority and a
schema-v2 operation-evidence digest. Provider success is explicitly classified
and binds its intent contract. Receipt v1/v2 and mutation-close v1 are
fresh-plan hard-cut refusals. Only unguessable `.mutation-stage-*` aliases are
retired. Every close link follows revalidation of the same authority, result
endpoints, operation evidence and staged inode/bytes. If a cooperative verifier
removes a live pre-link alias, publication retries only after those checks. An
unrelated one-link alias confers no authority and cannot block the authorised
close; its eventual final link loses to the permanent no-replace name. Append
and finalization close their short slot on a catchable no-effect error. Generic
append cannot publish preflight, provider-create, provider-cleanup or
finalization evidence.

The deterministic fixture finalizer can emit only the explicitly non-live
`synveda.clean-engine.synthetic-environment.v1` schema. Controlled-fake
evidence is structurally ineligible; a future live provider must introduce its
own reviewed environment schema before any acceptance claim.

Only the state-integrated provider-create seam has mutation-journal recovery.
The synchronous deterministic fake remains the rollback; the controlled fake
path holds the same slot across
an immutable root plan/reservation, a mirrored external-root owner, durable
launch/witness/one-way decision, one fixed child effect, outcome and whole
process-group settlement. The supervisor reasserts the actual slot before its
decision, while the actor validates the complete root inventory and
digest-bound plan/owner/launch/witness. Only the actor signals its group;
directly owned children exit on parent-IPC loss. The settlement binds exact
optional effect and outcome digests. The exact effect is then mirrored into
append-only state and a controlled-fake provider identity binds the slot,
intent, contract, root plan/owner and settlement; the passing receipt and
post-settlement close bind that identity. Explicit recovery requires
`recover:<fixture>:<two-digit-slot>:<slot-sha256>`, refuses a live or
unidentifiable owner and appends a gap-free claim chain. The open slot and
newest live claim block cooperative writers until a terminal receipt and close
are durable. Recovery never signals a stored process-group identifier and never
replays a durable start. A reservation-bound external marker can repair an
interrupted internal mirror; effect-mirror and identity-publication crashes
converge without replay. A durable launch without a witness remains a blocker.
A claim linked after an already-durable close is inert history and
cannot reopen that generation. Eight failed recovery attempts or 64 slots
exhaust the bounded journal and require operator inspection and fresh-plan
regeneration. The reusable `.mutation-lease` layout is refused rather than
reclaimed; an abandoned append or finalization slot still requires operator
inspection.

This remains an internal fake-only contract, not a provider runner. The
state-integrated adapter accepts no function, command, path, environment or
provider selector.
The controlled actor executes only the repository-fixed fake command under the
private receipt-owned root; it has no Docker or Colima command. Provider
artifacts now record the plan, reservation, owner mirror, launch, witness,
decision, optional outcome, ESRCH settlement, state-owned effect mirror and an
explicitly controlled-fake provider identity. The fixed effect contains only
opaque binding metadata. Owner challenges and PGID probes are cooperative
same-user/PID-namespace evidence; journal hashes do not protect against a
hostile same-user writer.

A separate lifecycle-unexposed fake canary now models the background provider
truthfully. Before root mutation, a recoverable fixture-only create authority
binds the exact base/evidence directory identities, intended receipt/slot-shaped
inputs and ownership nonce. A controller-launch decision is durable before the
controller process, and a separate start decision is durable before the host
agent. The v3 process contract inserts a synchronous veto-only authority
checkpoint before root publication, controller spawn, start-decision
publication, start delivery and terminal provider identity. Each checkpoint
compares the full ordered evidence and private-root effect frontier with
launcher-owned identities before the next effect; the terminal checkpoint runs
only after both provider sockets are freshly reauthenticated and the identity
candidate is prevalidated. Root, controller-readiness and PID files use private
fsynced stages plus no-replace hard links, so interruption cannot expose a
partially written final. A separate non-mutating inspector validates causal
evidence/root prefixes, distinguishes unattested/observed/proved-absent process
state and retains partial or linked stages for the future state recovery path.
The controller and host agent reopen their exact digest-bound configuration
through no-follow descriptors. The controller also revalidates that exact
authority, root owner, launch/witness chain, start decision and host-agent
configuration before it can spawn. Its explicitly authorised fixture launcher
starts only a digest-bound controller and separately detached host agent,
and authenticated host-agent and Engine sockets plus the Docker-context
endpoint remain healthy after controller-group `ESRCH`. Its proposed live
contract pins the Colima 0.10.3 and Lima 2.2.0 source revisions and exact
Darwin/arm64 VZ command, explicit short roots, closed helper path and unchanged
inherited `HOME`, but refuses start while its OS, Docker, helper and disk-image
closure is unresolved. Provider identity binds the complete creation-time
fake-root inventory; retirement requires its exact match before publishing the
leaf-first actions. It then stops the exact host agent through its
authenticated socket protocol, fsyncs socket absence and applies individual
identity-checked unlink/rmdir steps. Append-only progress resumes an exact
complete stage/link, discards only its reserved same-target/digest partial
stage, and recovers shutdown and delete-before-progress interruptions. Any
already-absent deletion target has its exact parent fsynced and is rechecked
before recovered progress can become durable.
When the recorded host-agent PID is already absent before planning, every
non-socket creation identity must remain exact and only the two exact recorded
sockets may be stale or absent before a recovery plan is published.
Wrong-digest or foreign-linked stages, settlements, links, symlinks, inodes,
leaves and recreated roots are preserved and refused. Create and cleanup
evidence are distinct.

No supported lifecycle target exposes either test-only seam; there is no live
provider mutation or environment manifest. The background canary explicitly
records `state_integration: not-authorized`, while the older state-integrated
provider root remains unretired. Controlled runs are therefore non-finalizable,
and their synthetic cleanup vocabulary grants no receipt deletion authority.
`registry/`, `runtime/` and `evidence/` remain empty. The next slice must add a
state-born wrapper that reasserts the real mutation journal around this
fixture-only inner chain and recovers every partial create prefix before it can
bind the canary's distinct cleanup head. It must then finish the live
Docker/helper/disk closure before exposing a supported lifecycle target or
making any Docker claim.
A receipt-v1/v2 plan or mutation-close v1 is discarded and regenerated.
The manual browser ceremony below remains usable for fixture development but
cannot be labelled a clean-Engine result.

The dedicated development browser fixture is deliberately a fresh-project
one-shot, not an ordinary smoke extension. The hosts manager owns at most one
global block, and ownership includes the exact Compose project. The ordinary
project and a suffixed acceptance project therefore cannot coexist in that
block. First stop the ordinary project, then remove its mapping (or prove it
already absent):

```sh
make compose-down
SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
  make compose-hosts-remove
make compose-hosts-status
```

Flush the active resolver cache after removal. Install the mapping for the
exact fresh acceptance project. Hosts actions deliberately take the suffix but
no Compose profiles or IPv4 pool:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development-acceptance-browser:app.synveda.test:auth.synveda.test \
  make compose-hosts-install
```

Flush the active resolver cache after installation, then prove the exact
acceptance mapping and real resolver result:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
  make compose-hosts-status
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
  make compose-resolver-check
```

Supply that same bounded suffix and a distinct non-overlapping `/24` to the
browser target:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24 \
  make compose-browser-acceptance
```

The target fixes the selected profiles to exactly `demo,browser-acceptance`
and invokes `up --initial-assets absent`. It refuses the unsuffixed project,
reference/external-provider modes, every pre-existing exact project container,
network or volume, and every additional profile. After the normal bundled
graph is healthy, a sandboxed non-root Playwright 1.62.1 container completes
one authorization-code/PKCE S256 administrator admission and logout. The
wrapper waits for that exact container to exit zero before the ordinary
runtime smoke; no browser service is accepted in ordinary smoke. The fixture
does not record screenshots, HTML, HAR, trace, video, storage state,
credentials, codes, tokens or cookies. Its sole secret is the mounted demo
administrator password, which is read through a bounded no-follow descriptor;
the mutable read and return buffers are zeroed, and the value is never logged
or captured. A live run must still prove the effective secret uid/mode on each
supported Docker platform. The target leaves the fresh project running for
inspection. Teardown and disposal must repeat the exact suffix, pool and
profiles selected by the target:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24 \
SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance \
  make compose-down
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24 \
SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance \
SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-browser \
  make compose-reset
```

The confirmed reset preserves generated secrets and issuer inputs under the
current lifecycle contract. Remove the acceptance mapping separately, without
profiles or the pool, flush the active resolver cache, and prove it is absent:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development-acceptance-browser:app.synveda.test:auth.synveda.test \
  make compose-hosts-remove
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-browser \
  make compose-hosts-status
```

Reinstall the ordinary project's mapping before returning to its default
lifecycle. A refusal during any handoff requires recovery or inspection; do
not override it or install a second block manually.

Neither `compose-down` nor confirmed `compose-reset` removes the host-wide
mapping. For the ordinary unsuffixed project, remove only the helper-owned
block after the exact project is stopped:

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
file and the isolated database-test harness are outside this canonical wrapper,
but their source-build declarations supply the same exact empty proxy arguments
so those deployment callers cannot reopen the image-stage boundary.

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
containers and networks to be absent. `absent` is narrower: it is accepted only
for a suffixed development acceptance project and requires all exact project
containers, networks and volumes to be missing before the first build. A
deterministic post-create contract
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
9. `compose.browser-acceptance.yaml` only for the exact fresh browser fixture;
10. the remaining optional profile fragments.

`make compose-config` runs the complete deterministic matrix without starting
or pulling images. The accepted profile vocabulary is `semantic`,
`observability`, `apalis-board`, `demo`, `backup-test` and
`browser-acceptance`; profiles without an implemented service remain
configuration-only. The demo profile requires both bundled providers, and
`browser-acceptance` is a fixture-only profile valid only together with `demo`
under the fresh-project command above.

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

This checkpoint contains a deterministically tested browser-login fixture, but
is not proof that a browser login has completed against live containers. It is
also not proof of desktop/Linux parity, reference HTTPS, backup/PITR, isolated
restore, upgrade/rollback, HA, host-loss tolerance, hosted SaaS readiness or
enterprise certification. The synthetic Docker-client proxy, canary remote
builder, authenticated private registry and exact clean-Engine teardown remain
open. Candidate planning now binds the source and selection closure, prepares
the private synthetic proxy template and contacts no Docker endpoint. The
append-only receipt grammar, cleanup-only failure branch, permanent mutation
journal, synchronous rollback fake, controlled actor/process-group witness,
mirrored external-root ownership, lifecycle-unexposed background-process retirement
canary and success-only manifest finalizer are deterministic fixed-fake
contracts, not a live provider claim. The supervisor
reasserts the actual slot before the actor validates its digest-bound authority
and full root inventory. Exact optional effect/outcome digests and group ESRCH
are bound into settlement and close evidence. The separate process canary
proves exact fake-root retirement but has no mutation-journal authority. No
supported lifecycle target exposes the test fixtures; state-integrated cleanup
and exact live-provider retirement remain prerequisites for a real effect.
Separately, every development source build proves the embedded local builder
grammar before mutation; none of that is live provider evidence. The core
Collector remains private but
currently exports to `nop`; the bounded observability profile and Operations UI
are open. Do not delete the legacy Rauthy/Temporal assets or change the
production-readiness verdict until replacement acceptance passes.
