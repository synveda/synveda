# Installing Synveda during the Docker reference cutover

Synveda has one context-platform runtime: separate gateway and worker
processes, PostgreSQL, generic OIDC, one public API and the same governed
configuration semantics in direct binaries, Compose and later Helm. Personal,
team and enterprise are Configuration documents, not deployment editions.

The CPR-45 canonical Compose graph now has an executable, bounded lifecycle for
development with bundled PostgreSQL and either bundled Keycloak or external
OIDC. Deterministic gates cover file selection, private inputs, exact-project
locking, network preflight, tenant/realm convergence and smoke predicates. It
has not yet passed clean-volume browser acceptance on the supported desktop
and Linux platforms, so it is not a supported controlled-use deployment.
`synveda init` remains closed by a cutover gate and refuses before profile
discovery, Compose, secret-file or database mutation. The old Rauthy profile
cannot establish the new exact database-authority, endpoint and file-secret
contract safely and is not an alternative installation path.

The target contract and current limits are in
[DEPLOYMENT_CONTRACT.md](DEPLOYMENT_CONTRACT.md). From a checkout, print and
install the exact marked development host mapping using the host's normal
administrator procedure, then run:

```sh
make compose-hosts-plan
make compose-resolver-check
make compose-config
make compose-up
make compose-smoke
make compose-down
```

`compose-up` creates or validates project-scoped secret files, converges the
bundled authorities and keeps gateway and worker in separate containers.
`compose-smoke` probes the public host route and private-route refusals, but is
not a browser authorization-code exchange. Clean browser login, reference
HTTPS, backup/restore and upgrade acceptance remain open. External PostgreSQL
bootstrap deliberately refuses before secret reads or SQL until the
authenticated-TLS contract is implemented.

The remaining sections describe product use only after a gateway has been
started through separately validated development/test infrastructure. They are
not deployment instructions or evidence that the reference is complete.

### Bootstrap policy retained for the accepted lifecycle

The resumed reference bootstrap will create **no organisation**. After tenant
admission the tenant contains one row and the audit chain contains one
break-glass event to say so:

```
1  tenant.created  BREAK-GLASS
```

There are no scopes, identities, grants, Configuration bindings or Knowledge
items, because
everything the product has a governed surface for is created *through* that
surface, by a person the PDP can decide about. An installer runs once, as
root-equivalent, before anybody is watching — it is the worst place in this
product to keep a shortcut past the policy engine (seed §2.2). See ADR-0055.

## Log in — this is where the organisation starts to exist

```sh
synveda login --gateway http://127.0.0.1:8120
```

Use credentials provisioned by the deployment's identity operator; no current
`init` path prints demo credentials. The browser opens, you sign in, and
**that login is where the tenant starts to exist**: on a fresh tenant whose
administrator bootstrap remains unclaimed, the tenant
root scope is minted from the tenant's own slug and name, your identity gets
its own `principal`-shaped scope under it, and you are granted
`administrator` **at the tenant root** because yours is the first qualifying
`synveda-admins` login (CPR-45, ADR-0102). All three are chained under *your*
subject, not an installer's:

```
2  access.granted         <your subject>
3  identity.provisioned   <your subject>
```

## Build your scope tree

```sh
root=$(curl -sH "authorization: Bearer $TOKEN" http://127.0.0.1:8120/v1/admin/scopes \
        | python3 -c 'import json,sys;print(json.load(sys.stdin)["parent"]["id"])')

synveda scope create --parent $root --kind org_unit --slug eng      --name Engineering
eng=<the id the tree shows>
synveda scope create --parent $eng  --kind workspace --slug platform --name Platform

synveda scope tree
```

Each of those is a `ScopeCreate` decision the PDP takes at the *parent*
scope, creation takes a required `Idempotency-Key` (the CLI mints one), and
each chains its own `scope.created` carrying that decision. There is no
bulk import and no seeding shortcut; scopes are governed objects — and
there is no delete: retiring one is `synveda scope move`-shaped
administration plus a status transition through the PATCH route.

Personal scopes are not created here — each person gets their own when they
first log in. While bootstrap remains unclaimed, the tenant's first qualifying
`synveda-admins` login gets the initial `administrator` grant at the tenant
root; later administrators must receive a governed Synveda grant.

## Workspaces, projects and the grants that decide

The scope tree above is the one tree (CPR-7): workspaces and projects are
product-level subtypes of a governed scope, and grants — not role
bindings — are what let people act:

```sh
curl -H "authorization: Bearer $TOKEN" http://127.0.0.1:8120/v1/me
```

`/v1/me` is the one call a client makes first. It answers who you are, what
exists, what is missing, and — the part worth reading — **where you stand and
what you may do there**:

```json
"anchors": [
  {"scope_id": "…", "kind": "principal", "source": "principal_scope",
   "direct": false, "roles": [], "actions": {"memory.write": true, …}},
  {"scope_id": "…", "kind": "workspace", "source": "grant",
   "direct": true,  "roles": ["owner"], "actions": {"workspace.update": true, …}}
]
```

Every `actions` entry is a **real PDP decision** taken at that scope under
that scope's effective immutable Configuration — a forecast of what an act
would answer, never a
grant and never a shape read off a plan. Three things follow from the model
that are worth knowing before you hand somebody a role key:

- **A grant reaches downward.** Give somebody a workspace and they reach its
  projects, with no row written at any of them. Give somebody one project and
  they reach that project and **nothing above it**.
- **Your own scope is yours.** `/v1/me` mints a `principal`-shaped scope for
  every caller the first time they call it. Nothing above it reaches in — not
  a tenant-wide grant, not an administrator, under no profile. The only way
  somebody else reaches it is a grant written **at** it, by you.
- **Revocation is immediate.** Access is resolved on every request, so
  revoking a grant is refused on the very next one. Nothing has to run.

### The first grant

While the marker remains unclaimed, a tenant's first qualifying member of the
IdP's `synveda-admins` group gets an `administrator` grant at the tenant root —
that is the one-time operator door. Any earlier governed root-administrator
grant consumes the same marker. The marker survives revocation, so neither
that revocation nor a later group login reopens IdP authority; later
administrators must receive governed Synveda grants. A fresh tenant
admitted with `synveda tenant create` for dev-token use has no IdP group
to read, so seed the same row by hand, once, at the store level (CPR-7
deleted `role bind` with the bindings; this is its replacement, as SQL,
because a governed route that hands out the first authority in a tenant
is the shortcut past the policy engine ADR-0055 refuses — where that
grant *should* come from is admission's, and it is recorded as standing
work rather than solved):

```sh
docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
  psql -U synveda -d synveda -c "
  insert into scopes (id, tenant_id, kind, slug, display_name)
  values (gen_random_uuid(), '<tenant id>', 'tenant', '<slug>', '<name>');
  insert into scope_grants
        (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
  select gen_random_uuid(), tenant_id, id, 'principal', '<subject>',
         'administrator', 'automation'
  from scopes where tenant_id = '<tenant id>' and kind = 'tenant';"
```

Every grant after the first goes through `/v1/admin/grants` under the
PDP.

## Governed runtime configuration

One complete Configuration document selects the Cedar pack and narrows the
runtime at a governed scope: capture triggers and bounds, context token budget
and channels, trace retention, type-aware freshness, Skill/Tool advertisement
and allowed external-provider families. Resolution walks the scope chain
nearest first. A tenant-root binding is the ordinary tenant selection; with no
binding, the built-in enterprise document is the conservative fail-safe.

`personal`, `team` and `enterprise` are templates, not editions. Choosing one
copies its document into an ordinary stable aggregate and immutable version.
Every create, publish, bind, pin, enable/disable and rollback still opens a
typed VedaFlow change, passes the PDP and leaves content-free audit evidence.
A permissive decision may apply it immediately; a stricter one reports
`pending_review`. Capture batches and context runs retain the exact version ID
and digest they used, so a later publication cannot rewrite their history.

Inspect and operate the public surface from **Advanced → Configuration**, or
with the HTTP-only CLI:

```sh
synveda configuration templates
synveda configuration effective <scope-id>
synveda configuration create --scope <scope-id> --name project-runtime --template team
synveda configuration show <configuration-id>
synveda configuration compare <configuration-id> --from <version-id> --to <version-id>
synveda configuration bind --scope <scope-id> --artifact <configuration-id>
synveda configuration rollback <binding-id> --expected-revision 2 --version <version-id>
```

Publishing a hand-edited document uses `configuration publish ... --file
document.json --expected-version <version-id>`. Binding mutations require the
exact binding revision. The deleted `/v1/policy/default` and per-scope policy
assignment routes have no aliases; `/v1/policy/packs` remains a read-only Cedar
source catalogue and local `synveda policy apply|clear` remains documented
operator break-glass for source installation, not runtime selection.

## Unified reviews

**Advanced → Reviews** is the comprehensive VedaFlow queue for Knowledge,
Skills, Tool servers and bindings, Configuration, policy relaxations,
OKF-sourced publication, prompts and context packs. Filter by artifact family,
then inspect the stable artifact id, operation, exact version or digest,
stale-head precondition, immutable effect, inherited approval requirement and
opened/reviewed/closed timeline. New Learnings remains the lightweight capture
decision page; the session-event quarantine remains a separate secret-admission
control.

Approve and Reject send the exact proposal commit currently displayed. If the
proposal changed, the gateway returns a conflict and records no verdict.
Rejection requires a reason. Where the live profile forbids author review, the
author must Cancel or ask another authorised reviewer. Under
`regulated-strict`, a person who authored or counted as a reviewer cannot also
apply or publish the effect; a separately authorised actor completes it. The
`standard` profile requires a reviewer distinct from the author but permits the
author to execute after that review. Personal auto-apply is unchanged: an empty
live requirement still creates the proposal, object/commit, typed command and
audit evidence before applying it.

API clients use `expected_commit` in both verdict bodies. CLI
`synveda proposal approve|reject` first reads the proposal and supplies that
precondition automatically. Cancel uses the existing withdrawal operation—no
second lifecycle or alias exists—and Apply/Publish repeat Cedar, the live
matrix, separation and artifact revision checks.

## Governed policy relaxations

A relaxation temporarily widens one provisioned subject's ability to read
current Knowledge at one non-personal scope. It is not a Cedar bypass: create,
revision and early revocation each open a typed `Policy/apply` VedaFlow change,
the live matrix returns `applied`, `pending_review` or `rejected`, and Cedar
still makes every Knowledge decision. The first release supports only the
closed `knowledge.read` action.

The effective Configuration at the target must enable that action and caps the
window. The stored hard expiry is calculated when the change applies; database
time ends authority even if the expiry bookkeeping worker is unavailable.
Changing Configuration may narrow a standing relaxation immediately. Personal
principal scopes cannot be targets, and quarantine, sealing and service-token
confinement remain overriding forbids.

Inspect these under **Advanced → Scopes**, or with the public-HTTP CLI:

```sh
synveda relaxation list --scope <scope-id>
synveda relaxation show <relaxation-id>
synveda relaxation create --scope <scope-id> --subject <identity-id> \
  --start 2026-08-25T12:00:00Z --end 2026-08-25T14:00:00Z \
  --reason "bounded incident investigation"
synveda relaxation revise <relaxation-id> --expected <current-version-id> \
  --subject <identity-id> --start 2026-08-25T12:00:00Z \
  --end 2026-08-25T13:00:00Z --reason "narrowed investigation window"
synveda relaxation revoke <relaxation-id> --expected <current-version-id> \
  --reason "investigation complete"
```

The subject flag takes the identity UUID returned by the authenticated
identity surface, not a free-form user name. Revisions require the exact
current immutable version. Ordinary API, console, CLI, log and audit responses
carry identifiers, hashes and bounded reasons, never Knowledge content or a
second permission token. The predecessor routes and command have no aliases;
an old development database is refused by the schema-epoch guard rather than
translated.

## Check it works

```sh
synveda scope tree                          # your organisation
synveda recall --query "..."                 # a governed read
synveda audit tail --limit 20 # policy-visible recent activity
synveda audit verify         # the caller's tenant chain
synveda audit events --context-run-id <uuid> # content-free exact evidence
synveda audit export --output audit-chain.json # frozen public-API prefix
synveda audit verify-export audit-chain.json # offline; no profile needed
```

Traces are at <http://localhost:16686>.

Audit query and export require tenant-wide `audit.read`; a grant below the
tenant root is refused rather than served a misleading partial chain. The
export command never accepts a tenant or database URL, verifies every canonical
hash input before its atomic no-overwrite write, and contains identifiers,
hashes, decisions and provenance—not Knowledge bodies, Skill files, Tool
credentials or Configuration documents.

## Connect an AI client

Two commands, because the two kinds of client are genuinely different.

### Claude Code

```sh
synveda plugin install              # --dry-run to see it first
```

The release carries a plugin — a **marketplace**, which is the unit Claude
Code installs — and this adds it and installs the one plugin in it by running
`claude plugin` itself. That gets you more than an MCP server: four hooks, so
a session composes a watermarked context block at `SessionStart` and every
turn is recorded back at `Stop`, `PreCompact` and `SessionEnd`. Start a new
session to pick it up, and check it loaded:

```sh
claude plugin list          # synveda@synveda … Status: ✔ enabled
```

Run it again after every upgrade. It compares what Claude Code has installed
against the bundle the release put on disk: the same version is left alone,
a different one is **replaced**, and `--force` replaces regardless. That
comparison is the point — Claude Code keeps its own copy of a plugin, so
until you re-run this an upgraded release still has the *old* plugin running,
reporting itself enabled and healthy.

Nothing is written outside Claude Code's own plugin state, and the `claude`
CLI has to be on your `PATH` — this drives it rather than editing the three
JSON files it keeps.

It needs a login to do anything: `synveda login` stores the bearer, and the
plugin reads it per call. There is no other configuration.

#### What happens when the gateway is unreachable

Valid spooled events survive a gateway outage, and ordinary outage recovery is
automatic.

Every event the plugin records is written to a **local spool** first — one
file per session under `$XDG_STATE_HOME/synveda/spool/` (or
`~/.local/state/synveda/spool/`) — and only then delivered.
A write is a temp file, an `fsync` and a rename, so a machine that dies
mid-write leaves the previous state or the new one and never half of either.
An event is deleted only once the gateway has acknowledged it.

Delivery happens on the lifecycle hooks: `Stop` and `PreCompact` synchronously
record to the local spool and return before credential or network work;
`SessionEnd` flushes what it can inside a bounded budget; and the **next**
`SessionStart` retries whatever is still unacknowledged. So a session worked
on a plane, or against a gateway that was down for the afternoon, delivers
itself the next time you start Claude Code with a network.

Three commands if you want to look, or to hurry it along:

```sh
synveda session spool status                # what is held, and how old
synveda session flush                       # deliver everything now
synveda session spool purge --acknowledged  # reclaim the delivered
```

`purge` **requires** `--acknowledged` and there is no `--all`. It will not
delete an observation the gateway has not confirmed.

The adapter validates the spool version, structure, event ordering and each
payload hash before either automatic or manual delivery. A malformed,
unreadable, future-version or hash-mismatched file is **held in place** rather
than treated as absent, overwritten or sent. A spool is also pinned on first
authenticated use to its canonical gateway origin; changing profiles to a
different gateway holds the old run rather than sending its transcript across
deployments. `synveda session spool status` reports held state, and the
adapter log records only a fixed reason class — never the rejected payload or
credential-bearing exception text.

The SHA-256 payload hash detects accidental local corruption. It is not a MAC
and does not protect against a process that already has arbitrary write access
to your account and can replace both payload and hash. Synveda does not claim
to preserve trustworthy client evidence after full local-account compromise.

Once events reach the gateway, a terminal session freezes the exact eligible
event snapshot as a durable capture batch. An explicit client can request the
same operation with
`POST /v1/sessions/{session_id}/capture-batches`; retrying either path resolves
to the same snapshot rather than calling the extractor twice. Extraction
creates candidates, not current Knowledge. Accept, edit-and-accept, merge or
replace calls the governed Knowledge/VedaFlow command layer, and a strict
profile can retain the result as `pending_review`. Dismissal publishes
nothing. Candidate content needs both access to its source session and
Knowledge-read authority at its proposed destination, so a private preference
derived during a shared run is not a shared draft.

> **The one thing that is lost.** If the host client is killed outright —
> SIGKILL, a kernel panic, a battery dying — before any lifecycle hook can
> run, the events since the last `Stop` go with it. No hook fires, so nothing
> writes.
>
> Claude Code fires `Stop` at the end of every turn, so the window is one
> turn, not one session: usually seconds. Closing it entirely would mean
> writing to disk on every token, which costs more than it saves. What is
> guaranteed is the other half — **a valid event that reached the spool is
> retained until the gateway acknowledges it.** Refused spool bytes are held
> for explicit recovery; they are not silently discarded or delivered.

### Everything else — MCP clients

Connection support and lifecycle verification are different claims. The
generated [client support matrix](CLIENT_SUPPORT.md) records the exact level,
tested versions, authentic fixture digests and limitations for every built-in
client. In particular, Cursor configuration is available but its lifecycle is
currently experimental and has not been run by a real Cursor client here.

`synveda mcp` serves governed context to any MCP client over stdio: `recall`
uses the ordinary Knowledge query on the caller's public session, and
`remember` appends an assertion event to that session for later capture in
your own personal scope. Recall returns exact immutable revision and source
addresses; it neither consumes a context token budget nor opens the separately
authorised diagnostics enumeration lens. The deleted tenant-global
`/v1/recall` route has not returned. You do not have to write the config by
hand —

```sh
synveda mcp install --client claude-desktop   # or: --client cursor
synveda mcp install --client cursor --dry-run # see it first
```

It changes one key in the client's own config file and writes everything else
back as it found it, so your other MCP servers are untouched. An existing
`synveda` entry that differs is reported rather than replaced; pass `--force`
if you meant to replace it. Restart the client afterwards.

For a client this release does not know, `synveda mcp install --print` gives you
the entry to place yourself, and `--config <path>` writes a config kept
somewhere unusual — a project-level `.cursor/mcp.json`, say.

**Claude Code needs none of this** — use `synveda plugin install` above. Its
plugin carries its own MCP entry, and launches the server with the write tool
switched off, because its `Stop` hook is already recording your turns and the
tool would store each one a second time.

If a client will not connect, the server's diagnostics are on its stderr, which
is where clients collect them — Claude Desktop keeps them in
`~/Library/Logs/Claude/mcp-server-synveda.log`. It is quiet by default; add
`RUST_LOG` to the entry's `env` to turn it up:

```json
"synveda": {
  "command": "/usr/local/bin/synveda",
  "args": ["mcp", "--writes", "tool"],
  "env": { "RUST_LOG": "synveda=debug,rmcp=debug" }
}
```

`rmcp` is the protocol SDK, so including it shows the frames themselves — which
is what you want when the handshake is the thing failing.

### Trusting an external MCP server for a project

The `synveda mcp` command above is Synveda's own thin client adapter. The
trusted MCP catalogue is a different plane: it records which exact external
server source, transport, authentication shape and tools/resources/prompts a
project reviewed. The public `/v1/tool-servers` and `/v1/tool-bindings`
operations import metadata, retain immutable raw and normalised discovery
snapshots, compare versions and pin one approved version to a project.

A changed schema, description, source, transport or authentication shape is a
new quarantined version. Approving it does not move an existing project
binding; repinning is a separate governed change. Authentication entries name
an opaque secret reference only. Do not put a token, header value or environment
credential in an imported manifest or client configuration: the API rejects
it and generated configuration never resolves the reference.

For a Synveda-custodied Tool credential, mint a stable local reference at the
Tool server's governing scope. Values come from a file or stdin—never argv:

```sh
synveda tenant secret put \
  --tenant <tenant-uuid> \
  --scope <governing-scope-uuid> \
  --kind tool_server \
  --label pulseboard.mcp \
  --provider remote_mcp \
  --from ./private-token
```

The command prints `synveda-secret://<uuid>`. Put that reference, not the
credential, in `secret_reference`. Registration, VedaFlow application and
every generated configuration recheck the exact tenant, scope, kind and active
state. A missing, revoked or foreign reference has one non-oracular refusal.
External references remain opaque adapter metadata and grant no permission.
Revoke a local reference with `synveda tenant secret revoke --tenant
<tenant-uuid> <secret-uuid>`; a revoked binding can still be removed, but it
cannot be rendered or re-enabled.

Local stdio commands are untrusted executable metadata. The gateway never runs
an imported command. A trusted local adapter may perform MCP `server/discover`
and the three list operations on the user's machine and report that bounded
evidence; catalogue tests refuse `tools/call` and are not an execution proxy.
The accepted external contract is the stateless MCP `2026-07-28` specification
over stdio or Streamable HTTP. Retired HTTP+SSE/session-shaped servers are not
translated.

In the console, select the target project and open **Tools**. The catalogue
links each stable server to its immutable versions, exact digests, transport,
authentication shape, tools/resources/prompts and JSON schemas. A quarantined
version is never offered in the binding picker; use its **Advanced Reviews**
link, then explicitly repin the project if that exact approved version is the
one it should advertise. Disable and remove preserve binding history. The
configuration preview masks opaque secret-reference identifiers, and its
health section labels the trusted adapter and exact read-only methods behind
each report. A `passed` row does not mean the gateway executed a tool.

### Exchanging project Knowledge with OKF v0.2

Synveda implements only the canonical Open Knowledge Format **v0.2** contract,
pinned to `GoogleCloudPlatform/open-knowledge-format@ad30107`. The public API
accepts already enumerated directory or checked-out Git files, or bounded zip,
tar and tar-gzip bytes. It never accepts server filesystem authority, runs Git,
follows a symlink, fetches a frontmatter URL or executes imported content.

The exchange is deliberately two-stage:

1. `POST /v1/projects/{project_id}/okf/imports` validates the bytes and creates
   an immutable dry-run plan. Supply `Idempotency-Key`; the same source and
   digest resolves to the same job.
2. Inspect it with `GET /v1/okf/imports/{id}`, then call
   `POST /v1/okf/imports/{id}/materialize` with another idempotency key. This
   creates ordinary capture candidates, not active Knowledge. Review them in
   **New Learnings**; Accept, Merge or Replace still creates a VedaFlow change.

`POST /v1/projects/{project_id}/okf/exports` deterministically renders selected
current project Knowledge, or all visible current project Knowledge when the
selection is empty. Every item, provenance source and retained relationship is
re-authorised before it enters the output. Unknown v0.2 types and extension
metadata survive; a declared v0.1 bundle is rejected rather than translated.
The generated OpenAPI document is the exact request/response reference. The
filesystem-owning client commands are:

```sh
synveda okf validate ./knowledge-bundle
synveda okf inspect ./knowledge-bundle --source-revision release-42
synveda okf import ./knowledge-bundle --project <project-id> --dry-run
synveda okf import ./knowledge-bundle --project <project-id>
synveda okf export --project <project-id> --output ./exported-knowledge
```

Validation and inspection are local. Import packages inert bytes and calls the
public project API; omitting `--dry-run` creates New Learnings only. Export
verifies the server-returned pin, paths and hashes before atomically publishing
a new local directory and refuses an existing output path. In the console,
select the project and open **Import / Export** for the same dry-run history,
classification, candidate and deterministic export views. Neither surface is
a scheduled Git synchroniser, database seeder or direct-publication shortcut.

## Choosing an embedder for semantic Knowledge search

A separately validated deployment selects `tei` for BGE-M3 or `deterministic`
for reproducible lexical-only evaluation through the ordinary runtime
configuration contract. The canonical Compose semantic profile and its
endpoint acceptance remain pending; the withdrawn `init` flags are not a
selection path.

The public Knowledge collection is always lexically searchable from its
immutable current revision. With `tei`, a restart-safe indexer also stores a
model-labelled revision vector and search fuses bounded lexical and semantic
candidates. A newly written revision is available lexically immediately and
joins the semantic leg after that asynchronous index converges.

The default `deterministic` embedder remains useful for reproducible functional
tests, but its BLAKE3 geometry has no semantic meaning. The Knowledge API never
queries or labels it as semantic: responses say `lexical` and report
`deterministic_embedder_is_not_semantic`. Use TEI/BGE-M3 for a quality or
semantic demonstration.

Vectors are keyed by immutable revision and model, so changing models does not
reinterpret old vectors. The indexer creates rows for the configured model as
it converges; no runtime reader falls back to the replaced aggregate.
Supported index dimensions remain 16 and 1024 (ADR-0024 decision 5), so adding
a third model shape requires an explicit schema decision.

## Using your own IdP

An external issuer remains part of the generic application contract, but the
withdrawn `init` verb is not an external-IdP setup path. A separately validated
deployment must mount the issuer configuration and provision a public
authorization-code client with PKCE S256, its exact deployment callback/origin,
and the `openid profile email groups` scopes. The issuer in discovery, tokens
and gateway configuration must be byte-for-byte identical.

One group claim is read: `synveda-admins` may seed the first tenant-root
`administrator` grant only while the tenant's insert-only bootstrap remains
unclaimed. It never governs later administrator assignment. There is no
placement convention — everybody arrives at their own scope and reaches
anything else through a grant (ADR-0074 decision 3). Issuer configuration does
not sync a directory.

Directory *synchronisation* — joiners, movers, leavers — is a separate,
deliberate step (AUTH-4, ADR-0059). Once the instance is up:

```sh
synveda scim token issue --label entra
```

prints a provisioning credential **once**. Paste it into Entra
(Provisioning → Admin Credentials → Secret Token) or Okta (Provisioning →
Integration → API Token) with the tenant URL `https://<your-host>/scim/v2`,
which is the same for every tenant — the credential names its own. Two
credentials may be live at once, so rotation never stops provisioning.

**For Entra, set `external_id_claim` to `oid` on the issuer.** Entra's `sub`
is pairwise per application and never equals the object id its provisioning
agent sends, so the default (`sub`) would match nothing and a person who
logged in before the directory reached them would end up with a second
identity. Okta needs no change.

Synchronisation projects users onto stable identities and principal scopes,
and directory groups onto the same Group and identity-keyed membership rows
used by the rest of the product. It can join, disable, rehire and change group
membership; it cannot name a scope, role, policy pack or governed artifact —
those are not in the wire format.

To let one directory group act in a governed subtree, an authorised operator
uses the dedicated public application command:

```http
POST /v1/directory/access-assignments
Idempotency-Key: <unique retry key>

{"scope_id":"<scope uuid>","group_id":"<directory Group uuid>","role":"member"}
```

That creates an ordinary source-bearing `scope_grants` row after the same
`membership.grant` Cedar decision used for manual access. Removing a member,
disabling their identity or archiving the directory group withdraws effective
access on the next request. Ordinary group/grant mutation routes refuse
directory-owned rows and tell the operator to change the directory or use the
dedicated assignment route. No live Entra or Okta verification is claimed by
the repository fixtures; they remain labelled captured or transcribed.

The retained bundled-Rauthy profile used a host gateway because its issuer was
`http://localhost:8100/...`; the closed `init` entrypoint means that shape is
cutover residue, not an executable or supported install. Canonical Keycloak
Compose removes the workaround only after its exact-issuer acceptance. ADR-0055
decision 8 has the localhost measurements; CPR-45 replaces it with one
proxy-routed Keycloak issuer name.

## PulseBoard product walkthrough

The removed ACME release seeder is not packaged or aliased: it depended on the
deleted hierarchy, policy-assignment and global observe/recall surfaces. Once
the runtime is initialised and the acting user has completed `synveda login`,
the packaged tour uses only the public application API:

```sh
synveda demo start --profile personal
synveda demo status
synveda demo reset --force
```

`--profile team` uses a separately logged-in `bob` credential when one exists,
or `--bob-credentials <profile>` when explicitly supplied. With no second
credential it returns a one-time invitation, does not store its token and says
that clean-session reuse ran as Alice. `--profile governed` selects the
canonical enterprise Configuration on the same binary/schema and reports
pending review honestly. The first exact canonical Configuration and matching
binding still create and apply typed VedaFlow changes; this is not an edition
switch or a bootstrap bypass.

The mode-0600 XDG receipt makes an interrupted run resumable. Reset archives
receipt-owned objects through public routes and preserves immutable/audit
history. For repository acceptance evidence, run
`sh demos/cpr-41-one-command-demo.sh`; it combines the CLI contract with the
database-backed Profile and PulseBoard scenarios.

## The admin console

`http://127.0.0.1:8120/console/`, served by the gateway from its own origin —
no second process and no second port. Sign in with credentials provisioned by
the deployment's identity operator; the session is an `HttpOnly` cookie, so
there is no token to paste.

**Since CPR-8 the console is the product rather than a review queue.** The
first sign-in on a fresh deployment goes to a six-step **getting started**
flow — create a workspace (just you, or a team), create the first project,
attach the repository it is about, choose your agent client, copy the two
commands that connect it, and run a connection check — because nobody is
asked to declare an organisation before they can hold a record.

After that the left-hand navigation is the product: **Home, Sessions,
Knowledge, New Learnings, Skills, Tools, People, Settings**, with a workspace
and a project switcher in the header that remember what you chose. **People**
is where you invite somebody (a one-time link you copy — this product emails
nobody), see who may act in a workspace and who has access only to one
project, and read *why* each of them does: granted here, inherited from a
scope above, through a group, or managed by your directory.

**Skills** is the immutable Skills Library. Its catalogue shows what the
selected personal or project session would actually receive; a Skill's detail
page shows exact versions and files, provenance, scanner evidence, bindings,
controlled tests and usage. Declared tools are metadata only. Installing,
updating, pinning, disabling or rolling back reports the VedaFlow outcome, so a
pending review is never presented as an active change.

**Tools** is the trusted MCP catalogue. It shows immutable source and
capability snapshots, quarantined changes and their approved-version diff,
exact project bindings, discovery-only adapter reports and generated client
configuration. Capability descriptions and requested permissions are review
metadata, not authorisation; imported commands are never launched by the
gateway and secret-reference values are masked in ordinary console output.

Governance lives under **Advanced** — Reviews (the proposals inbox), Scopes
(the scope tree, effective Configuration and standing relaxations),
Configuration, Audit and Service identities. Those five appear only if the
policy decision point says
you may read them, so a viewer who holds no governance role sees no Advanced
section at all. That is a forecast and not a permission: every act is decided
again at its own seam, and a page you reach anyway will show you the gateway's
own refusal.

**Sessions** is where you see what your agents have actually been doing.
Every run an agent opened against this deployment, newest first, narrowed by
state, project, client, who ran it and a range of days, a page at a time.
Open one and you get its whole timeline: the messages, tool calls, file
changes, commands and skill loads in the order the server assigned them,
beside the context blocks composed for that run. Each entry shows **both
clocks** — when the client says a thing happened, and when this deployment was
told — and an entry that did not arrive live is marked with how far behind it
was, because the agent clients here spool to disk when the gateway is
unreachable and flush later. An adapter warning gets a banner and a mark in
place. A run that never finished says which way it stopped and, when the
client said so, why.

Raw event payloads are **not** shown by default: a timeline says a message was
sent and summarises it, and the payload is what was actually said. Expanding
one takes `session.diagnostics` at that run's scope — a separate authority
from reading the timeline, so a team can follow what its agents did without
handing everybody a transcript of everybody's prompts. Where you hold it, each
entry gets a *Show raw payload* control; where you do not, the page says which
role it takes.

The public Knowledge Browser searches current active revisions and exposes
immutable history and independently authorised provenance. Its conflict queue
compares exact revisions and resolves keep-separate, support, duplicate,
supersede, future-transition or archive choices through VedaFlow. Conflicting
challengers remain `transitional` and absent from ordinary results until that
change applies; capture-backed challengers stay in New Learnings. **Valid at**
and **As known at** are separate controls, and history/transitional rows only
appear when explicitly requested. The staleness queue explains explicit or
configured due dates plus type-specific repository-change, failed-use and
source-freshness signals; verification creates a new immutable revision.
Session extraction
produces reviewable capture candidates; **New Learnings** groups them by batch
and lets you filter by project, session and decision state, inspect their exact
source-event summaries and current-Knowledge comparisons, and accept, edit,
merge, replace, change scope or dismiss. Private, project and workspace choices
are named distinctly and a scope you cannot publish into is not offered. An
applied decision links to its Knowledge item; a stricter profile's pending
change links to Advanced Reviews and remains explicitly unpublished. Raw source
payloads still require `session.diagnostics` at the run.

Signing in needs a **key plane**, because a console session seals its tokens
under the deployment's encryption key (TEN-4). Gateway and worker accept the
same mutually exclusive direct/file KMS settings; canonical Compose mounts a
mode-0600 key file. The deployment must generate, retain and back up that key
separately from PostgreSQL, since every tenant key in the database is wrapped
by it. Canonical Compose generates and retains the project-scoped file but has
not yet passed the required joint database/key restore acceptance. The
console ships with release artifacts; from a checkout it needs
`pnpm --filter @synveda/console build` first, and without a bundle the route
404s rather than failing boot, because a static asset must not be a dependency
of the audit log (CNSL-1, ADR-0056).

Each tenant has its own versioned data key. `synveda tenant key status
--tenant <uuid>` lists only credential-free secret metadata and durable
re-encryption jobs. `synveda tenant key rotate --tenant <uuid>` retains the
old generation for external archives, creates a retryable job, and advances
every active database-owned secret envelope without changing the secret's
stable identity or logical value revision. Directory configuration uses the
same aggregate through `synveda directory set-credential`; clearing it revokes
the stable row, so a stale credential cannot silently fall back to a deployment
directory.

`synveda tenant export --tenant <uuid> --out tenant.svexp` writes the hard-cut
context export: Knowledge heads and head history, immutable revisions,
normalised sources and relations, plus the audit chain. Its body is sealed
under a fresh archive key wrapped by that tenant's key. There is no Record
section, old-format reader, re-import or tenant-erasure claim.

The shipped provider is the local `SYNVEDA_KMS_KEY` boundary. Keep it outside
the database and back it up. The provider interface leaves room for later
custody integrations, but this release does not support cloud KMS, an HSM,
customer-managed keys or secret-manager resolution inside the gateway.

## Upgrading

There is no accepted Docker reference upgrade path yet. Re-running the release
installer replaces the downloaded CLI, gateway/worker binaries, console and
transitional profile, but it is not evidence that a database, issuer or
running service was upgraded. Do not follow it with implicit `synveda init`.

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
```

No transitional operator path substitutes for deployment validation. The
accepted reference upgrade/rollback smoke test remains CPR-45 work and must
cover the binaries, schema migration, role contract, issuer and rollback
limits together; zero downtime is not claimed.

### If the upgrade refuses to start: the schema epoch

Synveda is pre-1.0, and one upgrade in this product's life is a **hard cut**
rather than a migration. Since the context-platform redesign the database
carries a **schema epoch**, and a build serves exactly one of them. If your
database was written before the cut, the gateway will not start — it exits
with a message rather than serving rows in a model it does not implement:

```
this database carries no Synveda schema epoch marker, so it was written
before the context platform (epoch 3).

Synveda is pre-1.0 and the context-platform redesign is a hard cut: there is
no migration from the previous schema, no compatibility path, and nothing that
translates old rows into the new model. A database from before the cut is
refused rather than upgraded.

Reset it — this DESTROYS everything in that database:

    synveda reset --database --force
```

`synveda db migrate` refuses the same database, and writes nothing when it
does — your rows are left exactly as they were, so you have as long as you
like to export anything you want before running the reset.

**There is no migrator, deliberately.** Nothing translates old rows into the
new model; see ADR-0068 for why that is a decision rather than an omission.

```sh
synveda reset --database --force   # destroys the database, builds a fresh one
```

After a reset, re-run the deployment-owned database bootstrap and only then
use the complete explicit authority plan. The withdrawn implicit init command
cannot bring the deployment back up.

`reset` drops and recreates **the application database** — not the volumes,
not the installation. Your `kms.key`, the compose profile, the console bundle,
your stored logins and every other database on the same server (Temporal's
two live in the same volume) all survive. It stops the gateway first, installs
the extensions, migrates to the current epoch, removes the derived search
index, and is idempotent: running it twice leaves the same thing.

It requires both flags. `synveda reset --database` on its own tells you what
it would destroy and destroys nothing. It also refuses a `DATABASE_URL` or
`DATABASE_URL_FILE` target that points at another machine, and prints the two
statements to run there by hand instead — `--force` says "yes, destroy it", not
"and I checked which server I am pointed at".

If instead you are told the database is at a *newer* epoch than the build,
**do not reset it**: that database holds data this installation cannot read,
and the message says to upgrade the installation rather than destroy it.

**If you installed the Claude Code plugin, upgrade it too:**

```sh
synveda plugin install
```

The installer replaces the bundle under `~/.synveda/plugin`, but Claude Code
copies a plugin into a cache of its own when you install it — so the plugin
that actually *runs* stays on whatever release put it there until you say
otherwise. `synveda plugin install` compares the two and replaces the
installed one when they differ, so running it after every upgrade is right
and doing it twice costs nothing. `claude plugin list` shows the version it
ended on; start a new Claude Code session to pick it up.

## Stopping and starting

From a checkout, use the canonical project-scoped lifecycle:

```sh
make compose-down
make compose-up
```

`compose-down` preserves the database volume and every project input. Reset is
separate, destructive, and requires the exact project confirmation:

```sh
SYNVEDA_CONFIRM_RESET=synveda-development make compose-reset
```

Reset preserves the secret set, issuer input and KMS key; it is not tenant
erasure, backup or credential rotation. See
[`deploy/compose/README.md`](../deploy/compose/README.md) for exact lock-recovery
and provider-mode procedures.

The installed transitional profile and legacy host-gateway process remain
cutover residue only. No current reference claim is based on them; canonical
Compose keeps the gateway in its container. To remove installed artifacts
rather than stop a canonical checkout, see **Uninstalling** below.

## Uninstalling

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/uninstall.sh | sh
```

Fetched rather than installed on disk, for the reason a self-deleting script
is a bad idea: it removes the directory it would have lived in. From a
checkout it is `scripts/uninstall.sh`.

It stops the gateway and the containers, and removes exactly what the
installer wrote — the CLI from wherever the sudo fallback put it, plus
`~/.synveda/{bin,console,profile,plugin}` and the transient entries under
`~/.synveda/data/`. A default uninstall deliberately leaves
`~/.synveda/data/kms.key`. `--dry-run` lists every path and container it would
touch, states when the key would be retained, and changes nothing.

**Your data and its key survive by default.** The three named volumes stay,
and so does the local KEK at `~/.synveda/data/kms.key`; the output names both
and the command that would remove them. Preserve that key for any later
explicitly validated deployment using the retained database. A governed
Knowledge `forget` removes one authorised item's
plaintext, sources and index state while retaining content-free audit
evidence; it does not delete a tenant. A tenant row still cannot be deleted
(TEN-5), so a volume purge remains the only whole-tenant wipe. That is a
deployment-level wipe, not a GDPR erasure certificate.

`--purge` is the irreversible coupled path: it runs `docker compose down -v`
and removes `~/.synveda/data/kms.key` with the rest of the install state.
`--purge --dry-run` reports both destructions and performs neither. Losing the
key while retaining a database would make console sessions, tenant secrets
and sealed tenant exports unrecoverable, so a warning is not treated as
consent to delete it. If Compose cannot confirm that the volumes were removed,
purge exits non-zero and keeps the key.

**It touches no editor or AI client config**, mirroring the promise the
installer makes. Undo those explicitly, before removing the CLI:

```sh
synveda mcp uninstall --client cursor   # removes our entry, and only ours
synveda plugin uninstall                # removes the Claude Code plugin
```

`mcp uninstall` is the exact mirror of `mcp install`: your other MCP servers
survive, and a hand-maintained JSONC config keeps its comments and layout
byte-for-byte. The uninstaller lists any client configs it finds mentioning
us so you know which ones to run it for.

Everything is idempotent — a second run finds nothing, says so, and exits 0.

## What the artifact installer places

| | |
|---|---|
| `synveda` | the CLI, on your `PATH` |
| `~/.synveda/bin/synveda-gateway` | the gateway, run as a host process |
| `~/.synveda/bin/synveda-worker` | the private core-worker direct-binary artefact; Compose runs its image-contained copy |
| `~/.synveda/console/` | the admin console bundle |
| `~/.synveda/profile/` | the transitional Compose file, Rauthy config and version; not an accepted reference deployment |
| `~/.synveda/plugin/` | the Claude Code marketplace, installed into no client |
| `~/.synveda/data/` | the transitional gateway pidfile/log and rendered configuration |
| `~/.synveda/data/kms.key` | the deployment's key-encryption key, `0600` — **back this up** |

`SYNVEDA_HOME` moves all of it; `SYNVEDA_BIN` moves the CLI.

The CLI goes to `/usr/local/bin` by default, which is root-owned on macOS and
on most Linux. The installer asks `sudo` for that one file and, **if sudo is
unavailable or refused — a managed machine where you are not an admin, a pipe
with no terminal to prompt on, or you declining — it puts the CLI in
`~/.synveda/bin` instead and tells you**, rather than failing an install whose
other four parts are already in place. Nothing else here needs a privilege.
If the directory it lands in is not on your `PATH`, the installer prints the
`export` line to add. To choose up front and skip sudo entirely:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh \
  | SYNVEDA_BIN="$HOME/.local/bin" sh
```

**The installer touches nothing belonging to an editor or an AI client.** No
`~/.claude`, no Claude Desktop config, no `~/.cursor`. Hooking one up is the
separate, explicit step above, and the OPS-8 demo asserts the absence rather
than trusting it.

The host-gateway shape exists only in the unaccepted bundled-Rauthy transition.
It is not part of the deployment contract and is deleted after Keycloak
acceptance. The accepted target uses one product image with distinct gateway
and worker commands, both containerised.

The CLI and transitional profile still ship together, but `init` currently
refuses at the CPR-45 cutover gate before reading or comparing that profile.
Version comparison resumes only with the accepted reference lifecycle; it is
not current executable behavior.

## Current verification boundary

`make compose-config` and `make check-deploy` prove static Compose/Helm and
release-package contracts. `make db-test` proves exact database bootstrap,
preflight, migration, forced RLS and authority drift behavior against fresh
PostgreSQL fixtures. They do not prove a browser login, clean canonical
Compose lifecycle, backup/restore, upgrade or desktop/Linux parity.

The Docker reference may be called validated only after
`make compose-acceptance`, `make compose-backup`,
`make compose-restore-smoke` and `make compose-upgrade-smoke` exist and pass
with the Keycloak issuer path. Until then the verdict remains “Docker reference
implementation incomplete.”
