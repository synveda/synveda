# Installing Synveda — the single-node profile

The SMB profile of tech plan §4: one gateway, Postgres, a bundled OIDC
provider, and optionally TEI for real embeddings. Laptop to working governed
memory in under ten minutes (OPS-1; the measured run is 5 seconds once the
images are present).

For the enterprise profile — HA Postgres, CloudNativePG, your own IdP, Helm —
see [the chart](../deploy/helm/synveda) (OPS-2).

## Prerequisites

**Docker** (Docker Desktop or OrbStack), running. That is the whole list.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
synveda init --slug acme --name "ACME"
```

The installer downloads one release — the `synveda` CLI, the gateway binary,
the admin console and a self-contained compose profile — and puts the CLI on
your `PATH` and the rest under `~/.synveda`. macOS arm64 and Linux x86_64;
the binaries are unsigned, and the checksums prove a download arrived intact
rather than who built it (OPS-8, ADR-0065).

`init` starts Postgres, Jaeger and the bundled Rauthy; applies the
migrations; admits one tenant; registers the OIDC client and your operator
login; and starts the gateway on `http://127.0.0.1:8120`.

<details>
<summary>From a checkout instead</summary>

Contributors, and anyone on a platform the release does not ship:

```sh
cargo build -p synveda-cli -p synveda-gateway
pnpm --filter @synveda/console build     # optional; without it /console/ 404s
./target/debug/synveda init --slug acme --name "ACME"
```

`init` prefers a checkout in the working directory over an installed
release, so having both is fine and the tree you are editing wins.
`SYNVEDA_COMPOSE_FILE` overrides both.
</details>

`init` converges — run it again as often as you like. It never drops a
database, a volume or a tenant, and if you change something it restarts the
gateway to match.

### What it deliberately does not do

It creates **no organisation**. After `init` your tenant contains one row,
and the audit chain contains one break-glass event to say so:

```
1  tenant.created  BREAK-GLASS
```

There are no scopes, no identities, no grants and no records, because
everything the product has a governed surface for is created *through* that
surface, by a person the PDP can decide about. An installer runs once, as
root-equivalent, before anybody is watching — it is the worst place in this
product to keep a shortcut past the policy engine (seed §2.2). See ADR-0055.

## Log in — this is where the organisation starts to exist

```sh
synveda login --gateway http://127.0.0.1:8120
```

`init` printed your operator's email and password. The browser opens, you
sign in, and **that login is where the tenant starts to exist**: the tenant
root scope is minted from the tenant's own slug and name, your identity gets
its own `principal`-shaped scope under it, and you are granted
`administrator` **at the tenant root** because you are in the
`synveda-admins` group (CPR-7, ADR-0074 decision 4). All three are chained
under *your* subject, not an installer's:

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

Personal scopes are not created here — each person gets their own when
they first log in, and a member of the IdP's `synveda-admins` group gets
an `administrator` grant at the tenant root on that same first login.

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
that scope's own profile — a forecast of what an act would answer, never a
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

A member of the IdP's `synveda-admins` group gets an `administrator` grant
at the tenant root on their first login — that is the operator door, and
for a login-driven deployment it is the whole story. A fresh tenant
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

## Check it works

```sh
synveda scope tree                          # your organisation
synveda recall --query "..."                 # a governed read
synveda audit tail  --tenant <id> --limit 20 # who did what
synveda audit verify --tenant <id>           # the chain
```

Traces are at <http://localhost:16686>.

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

Nothing is lost, and you do not have to do anything about it.

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

> **The one thing that is lost.** If the host client is killed outright —
> SIGKILL, a kernel panic, a battery dying — before any lifecycle hook can
> run, the events since the last `Stop` go with it. No hook fires, so nothing
> writes.
>
> Claude Code fires `Stop` at the end of every turn, so the window is one
> turn, not one session: usually seconds. Closing it entirely would mean
> writing to disk on every token, which costs more than it saves. What is
> guaranteed is the other half — **nothing that reached the spool is ever
> lost.**

### Everything else — Claude Desktop, Cursor, Zed

`synveda mcp` serves governed memory to any MCP client over stdio: `recall` to
search, and `remember` to store one durable fact in your own personal scope.
(`recall` used to fetch by handle and read the corpus at a past instant. It
composes a context run now, and both went with `/v1/recall`; Prompt 18 of the
context-platform programme is where they return.) You do not have to write the
config by hand —

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

## Choosing an embedder — do this before writing records

```sh
synveda init --embedder tei      # BGE-M3; downloads ~2.3 GB once
synveda init --embedder deterministic   # the default; no download
```

`record_embeddings` stores the model that wrote each vector, embed-or-fail is
unconditional, and **nothing in the product re-embeds a corpus**.

The default is `deterministic`: BLAKE3 of the content expanded to a 16-dim
unit vector, recorded as `hash@1`. It needs no network and no model, and the
same text always gives the same vector — which is what makes tests and demos
reproduce exactly. Its geometry carries no meaning, though: equal texts
collide and similar texts do not attract, so the dense leg contributes
nothing and BM25 does all the real work. Right for a functional demo, wrong
for a quality one.

If you switch after writing records, the older half does not rank badly — it
**disappears from the dense leg**. That leg filters on `model` and `dim`, so
records written under the previous embedder are excluded from it entirely and
survive on BM25 alone, with no error and no warning. Choose before you write,
or start a fresh tenant. Supported dimensions are 16 and 1024 (ADR-0024
decision 5), so a third model is not simply a flag.

## Using your own IdP

```sh
synveda init --issuer https://login.microsoftonline.com/<tenant>/v2.0
```

Nothing is created in your directory. `init` writes the gateway's issuer
configuration and prints the client registration to perform there — a public
client, PKCE S256, redirect `http://127.0.0.1:8120/auth/callback`, scopes
`openid profile email groups`.

One group claim is read: `synveda-admins` upserts an `administrator` grant at
the tenant root on every login. There is no placement convention — everybody
arrives at their own scope and reaches anything else through a grant
(ADR-0074 decision 3). `init` configures an issuer; it does not sync a
directory.

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

What synchronisation then does is placement and lifecycle only: it can put a
person, move them, and seal them. It cannot name a scope, a
record, a role or a pack — those are not in the wire format.

With a real issuer the gateway runs as the compose `gateway` container. With
the bundled one it runs as a host process, because the bundled issuer's URL
is `http://localhost:8100/...` and RFC 6761 makes every resolver answer
`localhost` with the *container's own* loopback — a container cannot reach
it, by any configuration. ADR-0055 decision 8 has the measurements.

## A demo organisation to play with

```sh
synveda init --demo          # adds ACME's people to the bundled IdP
synveda login                # …then become somebody who can build the org
~/.synveda/profile/demo/seed.sh
```

`--demo` adds four users in convention-shaped groups across `eng/platform`,
`eng/payments` and `sales/emea`, so you can log in as different people and
watch what each of them can and cannot see.

**It creates no scopes and no memory**, and the seeder is a separate step run
after you log in, because these are governed objects: creating an org unit is
an act the PDP decides and the audit chain attributes, and there is no
operator to attribute it to until somebody has logged in. An installer that
seeded your organisation would stand the whole tenant under a break-glass
actor (ADR-0055 decisions 1 and 2).

`seed.sh` builds the org units those groups are named for, assigns
contrasting policy packs, observes a small corpus through the real extraction
pipeline, and opens one proposal that needs two people — so the console has
something in it and the governance has something to refuse. It is safe to
re-run, takes `--dry-run`, and refuses a tenant that already holds an
organisation it did not build. From a checkout it lives at
`deploy/release/demo/seed.sh`, and `init` prints whichever path applies to you.

Never use `--demo`, or the seeder, on a deployment that will hold real memory.

**`docs/BETA.md` is the guided tour** — the same steps with what to look at and
why, plus the standing list of what does not work yet.

## The admin console

`http://127.0.0.1:8120/console/`, served by the gateway from its own origin —
no second process and no second port. Sign in with the operator `init`
printed; the session is an `HttpOnly` cookie, so there is no token to paste.

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

Governance lives under **Advanced** — Reviews (the proposals inbox), Scopes
(the scope tree, the pack in force, standing relaxations), Policies, Audit and
Service identities. Those five appear only if the policy decision point says
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

The Knowledge aggregate and governed lifecycle now exist behind the application
service, but the public browser/search contract and New Learnings workflow land
in the following context-platform packages. Until then the navigation remains
an honest empty state rather than exposing a second hand-written record API.
Tools likewise names its later registry package.

Signing in needs a **key plane**, because a console session seals its tokens
under the deployment's encryption key (TEN-4). `init` mints one at
`~/.synveda/data/kms.key` on first run and reuses it after — **back that file
up**, since every tenant key in the database is wrapped by it. Set
`SYNVEDA_KMS_KEY` and `init` uses yours instead. It ships with the release; from a
checkout it needs `pnpm --filter @synveda/console build` first, and without a
bundle the route 404s rather than failing the boot, because a static asset
must not be a dependency of the audit log (CNSL-1, ADR-0056).

## Upgrading

Re-run the installer and then `synveda init`:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
synveda init --slug <your slug> --name "<your name>"
```

The installer carries your `.env` across, `init` reuses the key it minted
rather than replacing it, and the gateway restarts onto the new binary. There
is no in-place upgrade and no migration story beyond that — reinstalling is
how you upgrade (ADR-0065 decision 1's reversal trigger is somebody wanting
more).

### If the upgrade refuses to start: the schema epoch

Synveda is pre-1.0, and one upgrade in this product's life is a **hard cut**
rather than a migration. Since the context-platform redesign the database
carries a **schema epoch**, and a build serves exactly one of them. If your
database was written before the cut, the gateway will not start — it exits
with a message rather than serving rows in a model it does not implement:

```
this database carries no Synveda schema epoch marker, so it was written
before the context platform (epoch 1).

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
synveda init                       # brings the deployment back up
synveda login                      # provisions your identity and admin grant again
```

`reset` drops and recreates **the application database** — not the volumes,
not the installation. Your `kms.key`, the compose profile, the console bundle,
your stored logins and every other database on the same server (Temporal's
two live in the same volume) all survive. It stops the gateway first, installs
the extensions, migrates to the current epoch, removes the derived search
index, and is idempotent: running it twice leaves the same thing.

It requires both flags. `synveda reset --database` on its own tells you what
it would destroy and destroys nothing. It also refuses a `DATABASE_URL` that
points at another machine, and prints the two statements to run there by hand
instead — `--force` says "yes, destroy it", not "and I checked which server I
am pointed at".

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

```sh
docker compose -f ~/.synveda/profile/docker-compose.yml down     # state persists in volumes
docker compose -f ~/.synveda/profile/docker-compose.yml down -v  # wipe everything
```

To remove the product rather than stop it, see **Uninstalling** below.

From a checkout, the compose file is `deploy/compose/docker-compose.yml`
instead.

The gateway's pid and log are under `~/.synveda/data/` (`data/` in a
checkout). `synveda init` restarts it when anything it was started with
changes — the issuer, the tenant, the embedder, the key, **or the gateway
binary itself**, so re-running `init` after an upgrade actually puts the
new release on the port rather than reporting the old one as healthy.

## Uninstalling

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/uninstall.sh | sh
```

Fetched rather than installed on disk, for the reason a self-deleting script
is a bad idea: it removes the directory it would have lived in. From a
checkout it is `scripts/uninstall.sh`.

It stops the gateway and the containers, and removes exactly what the
installer wrote — the CLI from wherever the sudo fallback put it, plus
`~/.synveda/{bin,console,profile,plugin,data}`. `--dry-run` lists every path
and container it would touch and changes nothing.

**Your data survives by default.** The four named volumes stay, and the
output names them and the command that would remove them. `--purge` destroys
them. A governed Knowledge `forget` removes one authorised item's plaintext,
sources and index state while retaining content-free audit evidence; it does
not delete a tenant. A tenant row still cannot be deleted (TEN-5), so a volume
purge remains the only whole-tenant wipe. That is a deployment-level wipe, not
a GDPR erasure certificate.

`~/.synveda/data/kms.key` goes with a default uninstall even though the data
stays. Records are not sealed under it and remain readable, but console
sessions, tenant secrets and any `synveda tenant export` archive cannot be
opened again without it — copy it first if you intend to come back to the
same volumes.

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

## What an install is made of

| | |
|---|---|
| `synveda` | the CLI, on your `PATH` |
| `~/.synveda/bin/synveda-gateway` | the gateway, run as a host process |
| `~/.synveda/console/` | the admin console bundle |
| `~/.synveda/profile/` | the compose file, the Rauthy config, the version |
| `~/.synveda/plugin/` | the Claude Code marketplace, installed into no client |
| `~/.synveda/data/` | the gateway's pidfile, log and rendered configuration |
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

The gateway runs on the host rather than in a container **only** on the
bundled-issuer path, and that is a measurement rather than a preference —
see the note under "Using your own IdP" above. With `--issuer` it runs as the
`gateway` container from `ghcr.io/synveda/gateway`.

The CLI and the profile ship together and `init` refuses to mix them: a
profile from another release stops the install with the two versions named,
because the alternative presents as a service that will not start.

## Verifying the whole thing

```sh
sh demos/ops-1-smb-profile.sh       # from a checkout
sh demos/ops-8-release-install.sh   # from a downloaded release
```

Both run the acceptance criterion end to end on a scratch HOME: install, log
in, build a scope tree, observe a turn, recall it, and assert the chain shows
exactly one break-glass event with everything else attributed to a person.
The OPS-8 one additionally installs from packaged release artefacts with
`cargo`, `rustc` and `rustup` shadowed by shims that exit 127 — so "no Rust
toolchain" is a property of the run rather than a claim about it. It wants
ports 5432, 8100 and 8120 free, and tears its deployment down at the end
unless you set `OPS8_KEEP=1`.
