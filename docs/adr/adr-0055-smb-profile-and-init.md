# ADR-0055: an installer that seeds nothing the product has an API for — the org root arrives by logging in, the hierarchy by a governed verb, and the embedder is chosen once because a corpus remembers which one wrote it

- **Status**: Accepted
- **Date**: 2026-08-04
- **Feature(s)**: OPS-1
- **Deciders**: sujitn

## Context

OPS-1's text is "single gateway binary + Postgres + Rauthy + TEI compose;
`synveda init` seeds org", and its acceptance criterion is "laptop → working
governed memory in <10 minutes, documented."

Fifty-three features exist and none of them can be *installed*. The compose
file FND-2 wrote serves five dependencies and **not the gateway**; the CLI's
`db`, `tenant`, `token`, `role` and `service` verbs are each documented in
their own help text as "dev bootstrap" or "dev plumbing"; and there is no
`hierarchy` verb at all. The result is that every one of the fifty-two scripts
under `demos/` opens by building its own estate, and the largest of them —
`demos/adpt-1-claude-code.sh`, whose acceptance criterion is itself a
stopwatch — spends roughly two hundred lines doing it. That script is the
honest statement of what installation costs today, so it is worth reading as
a specification of the problem:

- It `curl`s Rauthy's admin API four times to converge a client, a group and
  a user, with a comment explaining that Rauthy refuses a password it has
  seen in the last three and that a re-run is exactly that case.
- It creates a **scratch database per run** and drops it on exit, because the
  long-lived dev database "carries thousands of leftover test tenants".
- It starts a **second gateway** on port 8131 with `SYNVEDA_DEV_JWT_SECRET`
  set, purely so that a dev-issued bearer can `POST /v1/hierarchy/nodes`
  three times, then kills it, unsets the secret, and starts the real gateway
  on 8120 under OIDC — because ADR-0010 forbids the two auth modes coexisting.
- It seeds two records with **raw SQL**, `insert into records … insert into
  record_embeddings`, with a comment noting that MEM-4's schema backstop
  refuses an embedding-less record so the vector has to ride the same
  statement.

Every one of those is defensible in a demo that tears its state down. None of
them is defensible in an installer, and two of them are the thing seed §2.2
and CLAUDE.md forbid outright: *never create a code path that bypasses the
PDP, even in tests.* A `psql` insert into `records` is that path. So is a
hierarchy built by a gateway running with a dev secret, one step removed.

Four forces.

1. **An installer is the most dangerous place in the product to put a
   shortcut.** It runs once, as root-equivalent, before anybody is watching,
   and whatever it writes becomes the tenant's history. The PDP rule is not
   a testing convention that an installer can be excused from; the installer
   is precisely where a bypass would be invisible and permanent. Everything
   `init` creates must be creatable by a person afterwards, through the same
   surface, or it must not be created by `init` at all.

2. **The zero-config promise already solved the chicken-and-egg, and nobody
   noticed.** The obvious objection to force 1 is that governed creation
   needs an authenticated operator, an operator needs an identity, an
   identity needs a placement, and a placement needs a hierarchy — so
   *something* has to write the first node outside the PDP. That is false
   here. AUTH-2's `ensure_root` (provision.rs, "created from the tenant's own
   slug and name on first use (seed §2.1 zero-config: a fresh tenant needs no
   admin before first login)") already creates the org root inside the
   provisioning transaction, and ADR-0015 decision 6 already places an
   admin-group subject with no team mapping under that root rather than in
   quarantine, "because quarantine's base forbid would nullify the very
   binding that makes the tenant governable". The first admin login
   *manufactures the org*. The installer does not need to.

3. **The ten minutes are a wall clock on somebody's laptop, and the two
   largest costs in it are not ours.** TEI's first start downloads BGE-M3 —
   ~2.3 GB, with a cold load measured at 2m01s *after* the download — and a
   from-source build of this workspace is minutes more. Neither is
   representative of what installing a released product costs, and neither is
   something the code can make faster.

4. **A corpus remembers which embedder wrote it.** `record_embeddings` stores
   a model and a dim (deploy/README.md says why: a corpus embedded on one
   architecture must stay comparable to one embedded on the other), and
   MEM-4's embed-or-fail is unconditional — there is deliberately no `off`.
   So the embedder is not a runtime preference an installer can leave for
   later. It is a property of the data, chosen before the first record, and
   nothing in the product re-embeds a corpus that changed its mind.

## Decision

1. **`init` writes through the store only what has no governed surface and
   no operator yet — the migrations and the tenant row — and nothing else.**
   Both already exist as audited break-glass paths: `db migrate`, and
   `tenant create`, which opens an RLS-scoped transaction and chains
   `TenantCreated` through `record_break_glass` under the break-glass actor.
   `init` composes those two; it introduces no new store-level write and no
   new SQL. In particular it does **not** insert hierarchy nodes, identities,
   role bindings or records.

2. **The org root is provisioned, not seeded.** `init` finishes at "a tenant
   exists, and an issuer resolves to it"; the hierarchy begins to exist when
   a human runs `synveda login`. AUTH-2 creates the root from the tenant's
   slug and name, binds tenant-wide `org-admin` from the `synveda-admins`
   group, and chains `identity.provisioned` and `role.bound` for both — so
   the org's first two facts are on the audit chain under the operator's own
   subject, which is not true of anything an installer writes. The immediate
   consequence is that **the dev-JWT gateway disappears from the install
   path**: the gateway starts once, in OIDC mode, and never runs with
   `SYNVEDA_DEV_JWT_SECRET` set. ADR-0010's "one auth mode only" stops being
   a constraint the bootstrap has to dance around and becomes a description
   of what installation does.

3. **`synveda hierarchy` becomes a product verb.** `create`, `list` and
   `show`, over `/v1/hierarchy/nodes` under the bearer `synveda login`
   stored — the `proposal`/`channel`/`prompt` shape, no database connection,
   the PDP deciding `HierarchyWrite`/`HierarchyRead` per call and the gateway
   chaining under the caller's own identity. This is not a convenience. The
   AC's word is **documented**, and the only way to build a department today
   is a `curl` with a hand-assembled JSON body; a documented `curl` is an
   undocumented product. It also removes the last reason the install path had
   to issue a dev token.

4. **The bundled IdP is the installer's to configure; a customer's IdP is
   not.** With no `--issuer`, `init` converges the `synveda` client, the
   `synveda-admins` group and the first operator in the **bundled Rauthy**,
   because in the SMB profile Rauthy is a part of this product that we ship
   and version. Given an `--issuer` naming anything else, `init` writes the
   gateway's issuer configuration, creates nothing in the directory, and
   prints the client registration the operator must perform there. A customer
   directory is AUTH-4/5's subject and an installer must not hold admin
   credentials to one.

5. **The embedder is chosen at `init` time, defaults to `deterministic`, and
   the choice is stated as permanent.** Force 4 makes this a data decision,
   not a convenience: `--embedder tei` starts TEI, waits for `/health`, and
   documents the first-run download; the default starts no TEI at all and the
   AC's ten minutes never contain a model download. `init` prints, in both
   cases, which embedder the corpus will be written with and that changing it
   later requires re-embedding every record — because a corpus half-written
   at `hash@1` and half at `bge-m3` is a retrieval failure that presents as
   bad relevance and nothing else. There is deliberately **no re-embed
   command**: see the deferral.

6. **The AC clock starts at the first `synveda init` keystroke, with image
   acquisition untimed — the ADPT-1 split, for the ADPT-1 reason.** That
   demo's criterion is also a stopwatch, and it divides at the person: the
   estate ("what an operator did once, and what any organisation already
   has") is untimed, and the fresh machine is timed. Here the untimed part is
   `docker compose pull` and the one local build of the gateway image, which
   a release pipeline performs once and ships; the timed part is everything a
   person does. This is recorded as a split rather than hidden: the demo
   prints both numbers, and the trigger for collapsing them is below.

7. **`init` is desired state and converges.** A second run against an
   initialised deployment reports what is already there and changes nothing;
   it never drops a database, a volume or a tenant. The ADPT-1 bootstrap
   learned this in the one place it could not avoid it — Rauthy's refusal to
   re-set a recent password, which it handles by attempting the state with a
   password and retrying without — and that handling moves into `init` rather
   than being rediscovered by the next caller.

8. **Where the gateway runs is decided by which IdP it trusts, and the
   bundled one forces a host process.** `--issuer` (a real directory) runs
   the compose `gateway` service; the bundled Rauthy runs the binary on the
   host. This is not a preference, and it was found by building the
   container first and watching it fail:

   An OIDC issuer identifier is **one URL that both the browser and the
   gateway must reach**. `IssuerConfig` carries no separate discovery URL —
   deliberately, since ADR-0010 compares the issuer byte-for-byte against
   the discovery document and the `iss` claim — and the bundled Rauthy's is
   `http://localhost:8100/auth/v1/`. RFC 6761 requires resolvers to answer
   `localhost`, and every `*.localhost` name, with the *caller's own*
   loopback, ahead of DNS and ahead of `/etc/hosts`. So inside a container
   that URL is the container itself, and no amount of Docker configuration
   changes it: `extra_hosts` with `host-gateway` was measured resolving to
   the right host address and the name still answering 127.0.0.1; a network
   alias loses to the same rule; `network_mode: host` is a Linux-only
   escape. The gateway container answered `502 {"service":"oidc-jwks"}`,
   correctly.

   The alternative was to move the bundled IdP off a loopback URL, which
   would rewrite `pub_url`, `rp_id` and `rp_origin` in the shared dev
   config and churn the five demos that hard-code `http://localhost:8100` —
   a large, unrelated blast radius to make one deployment shape uniform.
   A real issuer has no such problem (an Entra or Okta URL resolves the
   same everywhere), which is why the split falls exactly where it does.
   The image is built and tested either way, so the container path does not
   rot.

9. **The playground org is a separate, optional, and equally governed step.**
   `init --demo` (and `demos/ops-1-smb-profile.sh`, which runs it) builds
   ACME — two departments, three teams, four users in mapped IdP groups — by
   the same two routes a customer has: `synveda hierarchy create` under the
   operator's bearer for the scopes, and the **observe → extraction →
   embed pipeline** for the material. Not `insert into records`. It is slower
   than the raw-SQL seed by exactly the amount of work the product does, and
   that is the point: what a demo shows must be what a customer gets. The
   flag is separate from `init` because a customer installing the product
   must not be given a fictional company inside their tenant.

10. **Convergence compares configuration, not liveness.** An `init` that
    finds a gateway already running must check *what it is running with*
    before leaving it alone. Skipping on liveness alone was written first
    and was wrong in the way that matters: a second `init` admitting a
    different tenant left the previous process up, so the login that
    followed authenticated against the previous tenant's issuer
    configuration and provisioned an org root in the wrong organisation —
    silently, with every surface downstream looking healthy. The rendered
    configuration is written beside the pidfile and compared on every run;
    a difference restarts.

## Options considered

1. **Let `init` write the hierarchy directly to the store, with a comment
   saying it is trusted.** The shortest path and the one the existing
   bootstrap effectively takes. Refused on seed §2.2 — and refusing it cost
   nothing once force 2 was noticed, which is the argument for reading the
   provisioning path before writing an installer rather than after.

2. **Keep the dev-JWT seeding gateway, but hide it inside `init`.** Would
   have preserved the current sequence and let `init` create the hierarchy
   without a human. Refused: it needs `SYNVEDA_DEV_JWT_SECRET` on a
   production-shaped deployment, which ADR-0010 decision 3 forbids for a
   reason that gets stronger, not weaker, when the caller is an installer;
   and it makes the tenant's first hierarchy events actorless where decision
   2 makes them the operator's.

3. **`init` performs the login itself with a client-credentials service
   identity, so installation needs no browser.** Attractive for CI. Refused
   for now: the operator provisioned this way would be a service identity,
   and AUTH-3's base-layer confinement forbid means a service identity cannot
   hold the tenant-wide `org-admin` an install needs — the demo would then
   want the confinement widened, which is exactly the carve-out AUTHZ-1's
   golden matrix exists to prevent. The headless install path is a real gap;
   it is recorded as a deferral rather than solved by weakening the forbid.

4. **Default `init` to TEI, and rely on the read path's degradation while the
   model downloads.** CTX-3 does degrade an embedder-less inject to
   sparse-only with a warning header, so reads would work. Refused because
   the *write* path does not degrade and must not: embed-or-fail is
   unconditional, so extraction would stall and redeliver until BGE-M3
   answered (MEM-4's chaos test is precisely this scenario, and proves
   nothing is lost). "Working governed memory in <10 minutes" cannot mean a
   product whose memory arrives when a 2.3 GB download finishes.

5. **Ship the gateway as a published image and make the AC a true cold
   start.** The right answer, and unavailable: there is no release pipeline
   and no registry. Recorded as the trigger against decision 6.

6. **Fold `init` into `make dev-up`.** Refused: `make` is the contributor's
   surface and every target in it is about this repository. An installed
   product is reached by a binary a customer has, not by a checkout.

## Consequences

- The gateway joins compose as a service built from a multi-stage Dockerfile
  (`profiles: ["deployed"]`, so `make dev-up` is unchanged);
  `deploy/README.md`'s note that `docker-compose/` is "the SMB single-node
  profile … Lands with FND-2" becomes true of the whole product rather than
  of its dependencies. Decision 8 means the bundled-IdP profile runs the
  binary on the host instead, so **two start paths exist** and only one of
  them is exercised by the acceptance demo. That is a real cost, accepted
  because the alternative was rewriting the shared dev IdP's URL; the
  container path is built on every demo run, so it cannot silently stop
  compiling, but nothing yet proves it *serves* — OPS-2's kind-cluster
  install test is where that becomes true.
- The measured install (2026-08-04, `demos/ops-1-smb-profile.sh`): **5s**
  against the 600s budget, with images present. The cold path — one
  `docker compose pull` plus the gateway image build — measured 227s on the
  same laptop, so even a genuinely cold start is inside the criterion; the
  split in decision 6 buys honesty rather than a passing number.
- `synveda hierarchy` is the first governed verb added for an operational
  reason rather than a feature's, and it makes three existing demos'
  `curl`-with-a-dev-token blocks replaceable. They are not rewritten here —
  a demo that proves its own feature is not OPS-1's to churn — but the
  duplication is now removable.
- The install path no longer contains `SYNVEDA_DEV_JWT_SECRET`. `token issue`
  keeps its existing dev-plumbing role and its warning; nothing in the
  documented path uses it.
- A deployment initialised with `--embedder deterministic` and later switched
  to TEI has a corpus it cannot compare across the switch. `init` says so;
  the product cannot yet fix it.
- The ten-minute number is now a thing the repository measures on every run
  of the demo rather than a claim in a backlog file, and it will regress
  visibly when something slow is added to startup.

Deferred, with triggers:

- **A re-embed command** (`synveda db reembed`, or a maintenance workflow):
  triggered when a deployment needs to change embedder or model after
  writing records — which is also what OPS-4's Qdrant benchmark and any
  BGE-M3 version bump will need. Not built here because doing it correctly
  is a bitemporal rewrite of `record_embeddings` under RLS with the sidecar
  index rebuilt, and inventing that inside an installer is how it gets done
  badly. Until it exists, the documented answer is to re-initialise.
- **A headless `init`** for CI and for OPS-2's kind-cluster install test:
  triggered by option 3's gap. The likely shape is an install-time operator
  bootstrap that is a *user* identity rather than a service identity, which
  needs AUTH-4's joiner path to exist first — another reason the reorder put
  AUTH-4 in the front block.
- **`init` for a customer IdP beyond writing configuration** (decision 4's
  other half): triggered by AUTH-4/5 landing SCIM and directory sync, at
  which point the directory is synchronised rather than configured and the
  question changes shape.

## Compliance notes

- **Seed §2.2 / CLAUDE.md (no path around the PDP):** decisions 1, 2, 3 and
  8. The installer's only store-level writes are migrations and the tenant
  row, both pre-existing audited break-glass paths; every scope, identity,
  role binding and record in an initialised deployment — including the demo
  org's — is created through the gateway under a real bearer and a PDP
  decision.
- **Seed §2.1 (zero-config):** decision 2 uses `ensure_root` rather than
  duplicating it; decision 5 keeps the default path free of external model
  downloads.
- **ADR-0010 decision 3 (one auth mode):** decision 2 removes the only place
  the product ran two.
- **ADR-0015 decision 6 (admin-group placement):** relied on by decision 2 —
  the first operator lands under the org root and not in quarantine.
- **ADR-0023 decision 6 (embed-or-fail):** decision 5 and option 4; the
  installer chooses an embedder rather than tolerating the absence of one.
- **ADR-0027 (adapter):** the ADPT-1 timing split is reused verbatim in
  decision 6 rather than a second convention being invented.
- **Audit (DoD item 4):** `init` emits no new action type. Everything it
  causes is chained by the code that already chains it —
  `tenant.created` through break-glass, `identity.provisioned` and
  `role.bound` at first login, `hierarchy.node.created` per governed create.
  A new installer action would have been an actorless event, which is what
  decision 2 exists to avoid.
