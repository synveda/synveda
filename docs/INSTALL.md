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

There are no scopes, no identities, no role bindings and no records, because
everything the product has a governed surface for is created *through* that
surface, by a person the PDP can decide about. An installer runs once, as
root-equivalent, before anybody is watching — it is the worst place in this
product to keep a shortcut past the policy engine (seed §2.2). See ADR-0055.

## Log in — this is where the organisation starts to exist

```sh
synveda login --gateway http://127.0.0.1:8120
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

Two commands, because the two kinds of client are genuinely different.

### Claude Code

```sh
synveda plugin install              # --dry-run to see it first
```

The release carries a plugin — a **marketplace**, which is the unit Claude
Code installs — and this adds it and installs the one plugin in it by running
`claude plugin` itself. That gets you more than an MCP server: four hooks, so
a session composes a watermarked context block at `SessionStart` and every
turn is observed back at `Stop`, `PreCompact` and `SessionEnd`. Start a new
session to pick it up, and check it loaded:

```sh
claude plugin list          # synveda@synveda … Status: ✔ enabled
```

`--force` reinstalls over an existing one; a second install without it leaves
what is there alone. Nothing is written outside Claude Code's own plugin
state, and the `claude` CLI has to be on your `PATH` — this drives it rather
than editing the three JSON files it keeps.

It needs a login to do anything: `synveda login` stores the bearer, and the
plugin reads it per call. There is no other configuration.

### Everything else — Claude Desktop, Cursor, Zed

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

Group claims drive placement: `synveda-admins` grants tenant-wide org-admin,
and `synveda-<department>-<team>` places by convention. `init` configures an
issuer; it does not sync a directory.

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
person in the hierarchy, move them, and seal them. It cannot name a scope, a
record, a role or a pack — those are not in the wire format.

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

## The admin console

`http://127.0.0.1:8120/console/`, served by the gateway from its own origin —
no second process and no second port. Sign in with the operator `init`
printed; the session is an `HttpOnly` cookie, so there is no token to paste.

Signing in needs a **key plane**, because a console session seals its tokens
under the deployment's encryption key (TEN-4). `init` mints one at
`~/.synveda/data/kms.key` on first run and reuses it after — **back that file
up**, since every tenant key in the database is wrapped by it. Set
`SYNVEDA_KMS_KEY` and `init` uses yours instead. It ships with the release; from a
checkout it needs `pnpm --filter @synveda/console build` first, and without a
bundle the route 404s rather than failing the boot, because a static asset
must not be a dependency of the audit log (CNSL-1, ADR-0056).

## Stopping and starting

```sh
docker compose -f ~/.synveda/profile/docker-compose.yml down     # state persists in volumes
docker compose -f ~/.synveda/profile/docker-compose.yml down -v  # wipe everything
```

From a checkout, the compose file is `deploy/compose/docker-compose.yml`
instead.

The gateway's pid and log are under `~/.synveda/data/` (`data/` in a
checkout). `synveda init` restarts it when the configuration changes.

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
in, build a hierarchy, observe a turn, recall it, and assert the chain shows
exactly one break-glass event with everything else attributed to a person.
The OPS-8 one additionally installs from packaged release artefacts with
`cargo`, `rustc` and `rustup` shadowed by shims that exit 127 — so "no Rust
toolchain" is a property of the run rather than a claim about it. It wants
ports 5432, 8100 and 8120 free, and tears its deployment down at the end
unless you set `OPS8_KEEP=1`.
