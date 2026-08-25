# Synveda beta — the tour, and what does not work yet

Both halves are in one file on purpose. A demo is driven by whoever built it;
a beta is driven by somebody who did not, so the limits stop being things we
remember to mention and become something you can read before you hit them.

**What this is.** Governed Knowledge and context for AI agents. Sessions append
events and freeze them into reviewable capture candidates; accepting a
candidate passes through VedaFlow before it can become an immutable Knowledge
revision. Policy decides which candidates, revisions and provenance may be
read, and every governed act is on an audit chain you can verify yourself.

> **Phase 5 branch note (2026-08-24):** install, login and session delivery
> remain current, but the organisation-seeding walkthrough in sections 4–7 is
> part of CPR-13's explicitly open demo-corpus debt and must not be represented
> as runnable. The supported product noun is Knowledge, browsed at
> `/console/knowledge`; the later one-command personal/team walkthrough is
> rebuilt entirely through public APIs.

**What you need.** Docker. That is the whole list.

**How long.** About fifteen minutes to the interesting part.

---

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
```

Binaries are **unsigned and un-notarized** — the installer strips macOS's
quarantine attribute and tells you it did. macOS arm64 and Linux x86_64 only;
anything else is refused by name rather than guessed at. There is no Windows
build and no package manager.

## 2. Start it, with the demo people

```sh
synveda init --demo --embedder tei
```

`--demo` adds four people to the bundled identity provider — Alice, Bob, Carol
and Dan, in convention-shaped groups across `eng/platform`, `eng/payments` and
`sales/emea`. It creates no memory and no scopes, which is deliberate and
explained in step 4.

**On `--embedder tei`:** it pulls a 2.3GB embedding model once and gives you
real semantic Knowledge search. Without it you get `--embedder deterministic`,
a hash whose geometry is deliberately never queried or labelled semantic;
Knowledge search stays lexical and says why. Immutable revisions are indexed
asynchronously for the configured real model, so a model change converges a
new model-labelled sidecar rather than reinterpreting old vectors. If you only
want to see the product shape, drop the flag and save the download.

## 3. Log in

```sh
synveda login
```

A browser opens against the bundled IdP. Use the operator credentials `init`
printed.

This login is where your tenant starts to exist: it mints the tenant root
scope from your tenant's own slug and name, gives your identity its own
scope under it, and grants you `administrator` at the root because you are
in the `synveda-admins` group — all of it audited under **your** subject
rather than an installer's.

## 4. Build the demo organisation

```sh
~/.synveda/profile/demo/seed.sh
```

(From a checkout instead: `deploy/release/demo/seed.sh`. `init` prints the
right path for you.)

**Why this is a separate step, and not something `init` did.** Creating an
org unit is a governed act. It needs a bearer, and it gets audited under
whoever holds it — so an installer that seeded your organisation would stand
the whole tenant under a break-glass actor, and the audit trail would say a
robot built your company. The product refuses to do that. Everything the
seeder makes is made by *you*, after you log in, through the same routes the
CLI and any harness use, which is why step 7's chain verifies.

It builds two org units and three below them, assigns contrasting policy packs,
observes six turns through the session capture pipeline, and opens one
proposal. It is safe to re-run — it asks what exists rather than keeping a
list — and it refuses a tenant that already holds an organisation it did not
build.

`--dry-run` prints what it would do and changes nothing.

## 5. Look at what you have

```sh
synveda scope tree
synveda recall --query "how do we roll out payments"
```

Open `/console/knowledge` for the authoritative current content, immutable
revision history and provenance browser. The terminal `recall` command opens
an ephemeral public session and asks for one budgeted context composition; it
is not a global Knowledge enumeration or a direct database query. Both paths
go through the PDP under the caller's identity.

Open `/console/learnings` after a session ends to review what extraction
proposed. Each candidate remains outside active Knowledge until you decide it.
The page groups the run's batch, previews its exact source events and visible
existing-Knowledge comparisons, and offers Accept, Edit and accept, Merge,
Replace, Change scope and Dismiss. Private, project and workspace destinations
are offered only where `/v1/me` forecasts `knowledge.write`; the gateway still
decides the act. A pending VedaFlow outcome links to Advanced Reviews rather
than pretending it was published.

## 6. Meet the thing the product is actually for

Open **http://127.0.0.1:8120/console/** and sign in with the same operator.

There is a proposal in the inbox: Knowledge climbing from your personal scope
to the whole organisation. **Try to approve it.**

You cannot. Publishing to the tenant root under `regulated-strict` takes a
`curator` *and* an `administrator`, two distinct people, and you are one
person holding every key. That refusal is the product working correctly, and it is the thing
worth showing somebody: the governance is not advisory, and it does not have
an override for the person who installed it.

While you are in the console, the explorer shows `eng` carrying `standard`
where `sales` inherits the tenant default — that difference is what decides
how expensive a publication is.

## 7. Verify the whole thing

The audit verbs use the same authenticated profile as every ordinary product
command. The gateway resolves the tenant from that bearer and applies
`AuditRead`; no tenant identifier or database connection is accepted:

```sh
synveda whoami
synveda audit verify
synveda audit tail
```

The chain covers every act since installation and holds exactly **one**
break-glass event: admitting the tenant, before any person existed to attribute
it to. Everything after it — the tenant root, your `administrator` grant, each
scope, each observed turn, the proposal — is attributed to you and hash-chained
to what came before.

## 8. Connect an agent

**Claude Code:**

```sh
synveda plugin install
```

Then start an interactive or headless session. Your session gets a watermarked
block of memory at start; each completed turn crosses the private local-spool
boundary before the hook returns and is delivered at SessionEnd or the next
SessionStart.

**Anything else that speaks MCP:**

```sh
synveda mcp install --client claude-desktop
synveda mcp install --client cursor
synveda mcp install --client vscode        # also: windsurf, continue, zed
synveda mcp install --client cursor --dry-run   # see it first
```

It edits one key in that client's own config and writes every other byte back
as it found it. An existing `synveda` entry that differs is reported rather
than replaced.

**A client we have never heard of** does not need a release. Drop it into
`~/.config/synveda/mcp-clients.jsonc`, which is read through the identical
loader as the built-in table:

```jsonc
{
  "my-editor": {
    "key": "mcpServers",                       // the key IT looks under
    "syntax": "jsonc",                         // "json" if the app rewrites the file
    "restart": "restart My Editor to pick this up",
    "path": { "any": "~/.my-editor/mcp.json" }
  }
}
```

Then `synveda mcp install --client my-editor`. If you would rather place the
entry yourself, `--print` gives you it and writes nothing.

That command installs **Synveda's** MCP adapter. Trusting some other MCP server
for a project is separately governed: the catalogue records an immutable
discovery digest and an exact approved project binding. A changed server is
quarantined and does not replace the bound version. Credentials remain secret
references, and the gateway never executes an imported stdio command or
proxies a tool call. The catalogue backend and CPR-26's Tools console use the
generated public API. Select a project, open **Tools**,
and import a credential-free manifest or supported client entry. A changed
version is visibly quarantined and links to **Advanced → Reviews**; even after
approval, the project's existing exact binding does not move until it is
explicitly repinned. The generated configuration masks opaque secret-reference
identifiers in the browser, and read-only health rows are reports from a named
trusted adapter rather than a gateway-side execution claim.

Project Knowledge can also cross the boundary as canonical **OKF v0.2**. The
generated public API creates an immutable dry-run from inert enumerated files
or bounded archive bytes, then materialises additions, updates and conflicts as
the same New Learnings candidates session capture uses. An import cannot publish
directly; accepting one still creates and evaluates a VedaFlow Knowledge
change. Deterministic export includes only freshly authorised current project
Knowledge and independently authorised provenance. The gateway does not read a
submitted path, run Git, fetch a source URL, follow links or execute bundle
content. The current adapter is pinned to upstream `ad30107`; v0.1 fallback
fields are intentionally not translated.

From a checkout, `synveda okf validate <path>` and `synveda okf inspect
<path>` apply that exact adapter locally. `synveda okf import <path> --project
<id> --dry-run` records only the immutable plan; rerun without `--dry-run` to
create New Learnings. `synveda okf export --project <id> --output <new-path>`
verifies and atomically writes the deterministic bundle without overwriting an
existing path. The selected project's **Import / Export** console page exposes
the same source revision, validation, classifications, candidates, history and
export evidence through generated operations.

## 9. Removing it

Fetched the same way as the installer — it is not placed on disk by the
install, because a script that lives inside the directory it deletes is a
script that can vanish mid-run:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/uninstall.sh | sh
```

From a checkout, it is `scripts/uninstall.sh`.

**Your data survives by default.** It stops the containers and removes what
the installer wrote — the CLI, the gateway, the console, the profile, the
plugin marketplace and `~/.synveda/data` — and leaves the four Docker volumes
in place, naming them. `--dry-run` lists everything it would touch and changes
nothing. `--purge` also destroys the volumes, and says in the same breath that
a tenant's memory is what it just removed.

Two things it deliberately does **not** do:

- **It does not touch your editor or AI client configs**, exactly as the
  installer promised not to write them. Those were separate explicit steps and
  so is their removal: `synveda mcp uninstall --client cursor` takes out our
  entry and only ours, leaving your other MCP servers, your comments and your
  layout byte-identical. `synveda plugin uninstall` removes the Claude Code
  plugin. Run both *before* removing the CLI. The uninstaller lists any client
  configs it finds mentioning us, so you know what to clean.
- **It cannot delete a tenant**, because nothing can — see TEN-5 below.
  Governed Knowledge `forget` can erase one authorised Knowledge aggregate;
  the volume remains the only whole-tenant wipe, so `--purge` is still
  all-or-nothing and is not a GDPR erasure certificate.

One thing to know before you keep data: `~/.synveda/data/kms.key` goes with a
default uninstall. Ordinary Knowledge content stays readable — it is not
sealed under that deployment key —
but console sessions, tenant secrets and any `synveda tenant export` archive
can never be opened again without that file. Copy it first if you might come
back to the same volumes.

---

# What does not work yet

Written plainly because you will meet some of these, and finding out from a
document beats finding out from behaviour.

### A killed Claude Code process can lose its in-flight tail

ADPT-8 is closed: CPR-14 passed an installed authenticated `claude -p` run on
Claude Code 2.1.241. Stop and PreCompact now synchronously write only the local
durable spool and make no gateway request; SessionEnd, the next SessionStart or
an explicit flush delivers. The live run composed one context run, persisted
four authentic user/tool/assistant events and ended normally.

The guarantee cannot begin before a hook runs. SIGKILL, a machine failure or a
harness crash before the in-flight turn reaches Stop can lose that tail.
Nothing which reached the spool is lost, including during a gateway outage.

### A tenant cannot be deleted

`tenants` is referenced by 32 foreign keys, all `ON DELETE NO ACTION`, so a
tenant anybody has logged into cannot be removed. To start over: `synveda
tenant export` for anything you want to keep, then `docker compose down -v`.
Tracked as TEN-5, which will make erasure a deliberate ordered traversal with
a destruction certificate.

### Nothing has spoken to a live Entra or Okta tenant

SCIM and directory sync are built and tested against mocks transcribed from
the vendors' published documentation. The bundled IdP is what has actually
been exercised. If you point this at a real corporate directory, you are the
first — please tell us what happens.

### Retrieval recall varies run to run

The dense retrieval leg is a prepared statement on a pooled connection, and
PostgreSQL switches it to a generic plan after five executions — which stops
using the vector index. Two identical queries can return different recall
depending on how old the connection they landed on is. Measured between 0.355
and 1.000 on the same corpus. Tracked as CTX-7. Keyword retrieval is
unaffected.

### One gateway, and restarts are visible

The Helm chart refuses a second gateway replica, because login state and the
scope-chain cache are in-process. Upgrades are a restart. Tracked as OPS-7.

### Cursor is configured but unverified

The Cursor entry is transcribed from Cursor's documentation and no real Cursor
frame has been replayed against it. The same is true of the VS Code, Windsurf
and Continue entries added for this beta. Claude Desktop and Zed are the two
with recorded frames. A wrong path fails loudly at install; a wrong config
*key* fails silently, with the client reporting no server — that is the field
to doubt first.

### Reinstalling is how you upgrade

No upgrade path, no package manager. Re-run the installer and `synveda init`.
Then run `synveda plugin install` again — Claude Code caches its own copy of
the plugin and will keep running the old one otherwise.

### One upgrade will refuse to start, and it is meant to

This product is pre-1.0, and the context-platform redesign is a hard cut: the
model underneath every table changes, and **nothing translates the old rows
into the new one**. Since that cut the database carries a *schema epoch* and a
build serves exactly one of them, so a gateway pointed at a database from
before it exits at startup rather than serving rows in a model it does not
implement. It prints why, and the one command that fixes it:

```sh
synveda reset --database --force
```

That destroys the database — every tenant, session, Knowledge item and audit
event in it — and
builds an empty one at the current epoch. It keeps your `kms.key`, the compose
profile, your stored logins and every other database on the server. Nothing
about it is recoverable, so if there is anything in there you want, take it
out first: `synveda db migrate` refuses the same database and **writes
nothing**, so a refused deployment is not a deployment on a clock.

Refusing rather than migrating is the decision, not the fallout of one; the
argument is in `docs/adr/adr-0068-context-platform-domain-and-epoch.md`.

### The demo people share one password

`init --demo` gives Alice, Bob, Carol and Dan the same password and prints it.
Fine for a laptop. Do not carry any of it into a deployment holding anything
real, and do not run `seed.sh` against one — it refuses tenants that already
hold an organisation, but that guard is against mistakes, not against somebody
who means it.

### Benchmarks are one thin data point

Published LongMemEval scores are 10 instances of 500 — QA 0.300, retrieval
recall 0.357. Real numbers, honestly reported, and far too few to draw a
conclusion from. See `docs/BENCHMARKS.md`; the full run exists as
`make eval-longmemeval-full` and nobody has scheduled it.

---

## Telling us things

What is most useful, roughly in order: something that behaved differently from
this document; a refusal you could not explain from the console; a harness you
tried to connect and could not. Include `synveda audit tail`
output where it is relevant — the chain usually says what happened.
