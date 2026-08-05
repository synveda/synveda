# Installing Synveda — the single-node profile

The SMB profile of tech plan §4: one gateway, Postgres, a bundled OIDC
provider, and optionally TEI for real embeddings. Laptop to working governed
memory in under ten minutes (OPS-1; the measured run is 5 seconds once the
images are present).

For the enterprise profile — HA Postgres, Temporal cluster, your own IdP,
Helm — see OPS-2, which is not built yet.

## Prerequisites

- **Docker** (Docker Desktop or OrbStack), running.
- **A Synveda checkout** and a Rust toolchain, until there is a release to
  download. `cargo build -p synveda-cli -p synveda-gateway` produces the two
  binaries this guide uses; a release would ship them.

## Install

```sh
cargo build -p synveda-cli -p synveda-gateway
./target/debug/synveda init --slug acme --name "ACME"
```

That is the whole installation. It starts Postgres, Jaeger and the bundled
Rauthy; applies the migrations; admits one tenant; registers the OIDC client
and your operator login; and starts the gateway on `http://127.0.0.1:8120`.

`init` converges — run it again as often as you like. It never drops a
database, a volume or a tenant, and if you change something it restarts the
gateway to match.

### What it deliberately does not do

It creates **no organisation**. After `init` your tenant contains one row,
and the audit chain contains one break-glass event to say so:

```
1  tenant.created  BREAK-GLASS
```

There are no scopes, no identities, no role bindings and no records, because
everything the product has a governed surface for is created *through* that
surface, by a person the PDP can decide about. An installer runs once, as
root-equivalent, before anybody is watching — it is the worst place in this
product to keep a shortcut past the policy engine (seed §2.2). See ADR-0055.

## Log in — this is where the organisation starts to exist

```sh
./target/debug/synveda login --gateway http://127.0.0.1:8120
```

`init` printed your operator's email and password. The browser opens, you
sign in, and **that login creates the organisation**: the org root is
provisioned from the tenant's own slug and name, your identity is placed
under it, and you are bound tenant-wide `org-admin` because you are in the
`synveda-admins` group. All three are chained under *your* subject, not an
installer's:

```
2  role.bound             <your subject>
3  identity.provisioned   <your subject>
```

## Build your hierarchy

```sh
root=$(synveda hierarchy root)

synveda hierarchy create --parent $root --kind department --slug eng   --name Engineering
eng=<the id it printed>
synveda hierarchy create --parent $eng  --kind team       --slug platform --name Platform

synveda hierarchy list
```

Each of those is a `HierarchyCreate` decision the PDP takes at the *parent*
scope, and each chains its own `hierarchy.node.created` carrying that
decision. There is no bulk import and no seeding shortcut; scopes are
governed objects.

Personal scopes are not created here — each person gets one when they first
log in, placed by their IdP groups. A group named `synveda-<department>-<team>`
places by convention, so `synveda-eng-platform` lands someone under
`eng/platform`.

## Check it works

```sh
synveda hierarchy list                       # your organisation
synveda recall --query "..."                 # a governed read
synveda audit tail  --tenant <id> --limit 20 # who did what
synveda audit verify --tenant <id>           # the chain
```

Traces are at <http://localhost:16686>.

## Connect an AI client

`synveda mcp` serves governed memory to any MCP client over stdio: `recall` to
search or fetch, and `remember` to store one durable fact in your own personal
scope. You do not have to write the config by hand —

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

**Claude Code needs none of this.** Its plugin already carries the entry, and it
launches the server with the write tool switched off, because its `Stop` hook is
already recording your turns — the tool would store each one a second time.

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

Group claims drive placement: `synveda-admins` grants tenant-wide org-admin,
and `synveda-<department>-<team>` places by convention. Directory
*synchronisation* — joiners, movers, leavers — is AUTH-4/5 and is not part of
this; `init` configures an issuer, it does not sync a directory.

With a real issuer the gateway runs as the compose `gateway` container. With
the bundled one it runs as a host process, because the bundled issuer's URL
is `http://localhost:8100/...` and RFC 6761 makes every resolver answer
`localhost` with the *container's own* loopback — a container cannot reach
it, by any configuration. ADR-0055 decision 8 has the measurements.

## A demo organisation to play with

```sh
synveda init --demo
```

Adds ACME's people to the bundled IdP — four users in convention-shaped
groups across `eng/platform`, `eng/payments` and `sales/emea` — so you can
log in as different people and watch what each of them can and cannot see.
The scopes themselves you create yourself, after logging in, with the
commands above: they are governed objects and there is no operator to create
them until somebody logs in.

Never use `--demo` on a deployment that will hold real memory.

## Stopping and starting

```sh
docker compose -f deploy/compose/docker-compose.yml down   # state persists in volumes
docker compose -f deploy/compose/docker-compose.yml down -v # wipe everything
```

The gateway's pid and log are under `data/gateway.pid` and `data/gateway.log`.
`synveda init` restarts it when the configuration changes.

## Verifying the whole thing

```sh
sh demos/ops-1-smb-profile.sh
```

Runs the acceptance criterion end to end on a scratch HOME: install, log in,
build a hierarchy, observe a turn, recall it, and assert the chain shows
exactly one break-glass event with everything else attributed to a person.
