# ADR-0066: a demo organisation is seeded by the operator after login, never by the installer — so the invariant that made the product look empty is the one that keeps the seeding honest

- **Status**: Proposed, **amended once** on 2026-08-13 while running it — see
  the amendment below. It changes no decision; it records the assertion the
  acceptance demo was missing and the failure that found it.
- **Date**: 2026-08-13
- **Feature(s)**: OPS-9
- **Deciders**: sujitn

## Amendment 1 (2026-08-13): the demo claimed a console sign-in it never made

This ADR's acceptance demo was written citing OPS-8's central finding — that
`GET /console/ → 200` passes against a console nobody can sign into, because
the bundle is static and holds no data — and then repeated it. The demo's own
header said "a console signed in to"; the demo never touched the console at
all. It asserted the route, the KEK file's existence on disk, and nothing in
between.

**It was not hypothetical, and the gap was found by a person opening the
console rather than by the demo.** The deployment this feature had just seeded
could not be signed into. The gateway said why:

```
crypto.kms.unwrap{kms.method="local" key.scope="deployment"}:
  invalid: sealed payload for kms.data_key did not open under this key
```

Not OPS-8's failure — a KEK was present and in use. It was the **wrong** one.
The database's `deployment_keys` row had been sealed at 13:26 UTC under a KEK
that was overwritten at 19:23 the same day, and both were named
`local:default`. That is precisely the hazard **ADR-0064 wrote down**: "`kek_ref`
is a name an operator chooses, so two different KEKs sharing a name is a
hazard the schema cannot catch". The name matched, so nothing warned; the
bytes differed, so nothing opened. A shared dev database had been unable to
seal a console session for a day and nothing noticed, because the only thing
watching was a 200.

Three things follow, and the third is the one worth keeping.

1. **The demo signs in now.** It drives `/auth/login?console=true` through the
   IdP, asserts the callback lands on `/console/` rather than
   `?error=…`, asserts a `__Host-` cookie comes back, and then asserts the
   cookie *works* — `/v1/whoami` resolving the right tenant, and the inbox
   holding the seeded proposal. A sealed session the gateway cannot reopen
   would still set a cookie, so the cookie is not the assertion either.
2. **The refusal names its cause.** On failure the step greps the gateway's
   own log for the sealing error and prints it, because "sign-in failed" and
   "the deployment key does not open" are different bugs and only one of them
   is actionable at 2am.
3. **The assertion was mutation-tested before it was believed.** Swapping the
   KEK under a live `deployment_keys` row reproduced the original failure
   exactly: the demo failed at the console step, exit 1, with the unwrap error
   surfaced — and **every step before it still passed**. Recall, the pack
   contrast, dual control: all green, against a product a person could not log
   in to. That is the whole argument for the step, and it is why a demo that
   stops one layer above its claim is worse than no demo, because it reports
   success.

**And the demo was destructive to the machine it ran on**, found the same
way — by a person using the deployment it had just broken. The bundled
Rauthy binds every login from its issuer to one configured tenant
(`TenantBinding::Static`, ADR-0010 §4), and the demo used the checkout's own
profile, so each run pointed the shared gateway at its throwaway tenant and
left it there. The developer's deployment then answered every command with a
policy denial naming a tenant they had never created — which reads as a
governance bug rather than as a demo that moved the furniture.

The fix is isolation of everything that can be isolated: its own compose
project, its own HOME, and — the load-bearing one — its own **state**, via a
scratch *bundle* profile pointed at by `SYNVEDA_COMPOSE_FILE`, since
`Profile::from_explicit_compose_file` returns `Bundle` when a `version` file
sits beside the compose file, and a `Bundle` keeps `data/` under its own
home. That directory holds the rendered gateway environment that carries the
tenant binding, which is why moving it is what actually fixes this.

**The ports cannot be isolated**, and saying so is part of the decision:
`GATEWAY_URL` and `RAUTHY_ISSUER` are constants, and ADR-0010 compares the
issuer byte-for-byte against the discovery document and the `iss` claim, so
moving Rauthy off 8100 means reissuing it everywhere it is checked. A demo is
not a good reason for that change. So the demo refuses to start on a held
port and names what to stop — protection rather than coexistence, which is
the honest available answer.

One payoff worth recording: building the bundle made the previously-unproven
thing free. The seeder is copied into it and executed from
`$SYNVEDA_HOME/profile/demo/`, so the installed path is now the path under
test rather than a gap in the criteria.

Two smaller things the same session found, recorded so they are not
rediscovered. `synveda init` **cannot repair this**: it converges on
configuration, an empty or unopenable key plane is not part of the
fingerprint, so it reports a healthy gateway and changes nothing — the fix is
to stop the gateway, since the deployment key is provisioned at boot
(`main.rs`, deliberately: "a login is a bad moment to discover the key plane
is empty"). And a restored KEK **does not take effect until the gateway
restarts**, because the running process holds it in memory; restoring the file
and re-testing immediately reads as the fix having failed.

## Context

OPS-8 removed the prerequisite that stopped anybody outside this laptop from
installing the product. It did not remove the one that stops them from seeing
it.

`synveda init --demo` seeds four people into the bundled IdP and then *prints*
the commands that would build ACME's scopes (`init.rs:277-287`; the
`--dry-run` block at `init.rs:339-356` prints them in full). Nothing creates
them. A tester who follows INSTALL.md lands, correctly authenticated, in an
organisation with no scopes, no memory, no proposals and an empty console.

The printing is deliberate and correct. **ADR-0055 decision 1**: these are
governed creates that need the operator's own bearer, and the operator has not
logged in when `init` runs. **Decision 2** is the invariant it protects, and
`demos/ops-1-smb-profile.sh` asserts it — 0 scopes, 0 identities, 0 role
bindings, 0 records the moment the installer finishes, and exactly one
break-glass event in the chain. An installer that seeded an org would stand
ACME's hierarchy under a break-glass actor, which is the opposite of what this
product sells.

So the constraint is real and the emptiness is its consequence. The question
this ADR answers is not "how do we relax it" but "where does seeding go
instead".

Two forces beyond the invariant. Seed **§2 principle 6** — the harness is a
guest, supporting a new one must never require touching the core — bears on
the harness half of this feature. And the audience is specifically somebody
who ran `curl … install.sh | sh`: they have a binary, a compose profile and no
checkout, which forecloses several otherwise obvious answers.

## Decision

Seeding moves to **`demo/seed.sh` inside the release profile bundle** — run
*after* `synveda login`, under the operator's own bearer, driving the same
CLI verbs and `/v1` routes a person or a harness already drives. No new
product surface, no new endpoint, no direct SQL, no break-glass, no PDP
bypass. **One principal seeds everything**, because the approval matrices
already produce every state the demo needs from a single one (decision 2).

OPS-1's invariant is untouched, because it is a statement about what an
*installer* may do and this is not one.

## Options considered

1. **Seed from `init --demo`** — one command, no login step. Rejected: it is
   precisely what ADR-0055 decisions 1 and 2 refuse, and it would break the
   assertion `demos/ops-1-smb-profile.sh` makes. It also cannot work as
   described: the creates need a bearer that does not exist yet.

2. **A script under `demos/`** — matches the convention of the other 60 and
   costs no product surface. Rejected on audience alone: the tester has no
   checkout, so the file is unreachable to exactly the person it is for. The
   convention is also a poor fit — every script in `demos/` is a per-feature
   acceptance proof that seeds scratch state and tears it down, and none is a
   tour.

3. **A `synveda demo seed` CLI verb.** Reachable, and unit-testable in
   `make ci`. **Written first and then withdrawn**, which is worth recording
   because the reasoning that justified it was three claims and two of them
   were wrong. It was chosen over option 4 on the grounds that a shipped
   script would be the only release asset that is a program rather than data
   (false — `install.sh` is itself a shipped shell program, and the release
   already extracts a `plugin/` tree), that it would inherit shell-portability
   surface for no gain (weak — the product is macOS arm64 and Linux x86_64
   only, both POSIX, and the 60 scripts in `demos/` are already
   `#!/usr/bin/env sh`), and that it would put the governed call sequence
   where `cargo test` cannot reach it (**true**, and the only one that
   survived). Set against that one real cost is the thing that decides it:
   **the demo is not the product.** A `demo seed` verb is permanent surface in
   a binary sold on trustworthiness — discoverable by a customer, runnable
   against a live deployment, and something every future security review has
   to reason about. The reachability argument that motivated it also
   dissolves: `synveda auth token` already prints a refreshed bearer, so a
   script authenticates with no new machinery.

4. **In the release profile bundle — as chosen.** Reachable (the bundle is
   already installed to `$SYNVEDA_HOME/profile/`), exercised by the same CI
   job that exercises the binaries, and invisible to a customer. Chosen over a
   *sixth* asset because the profile bundle already carries a `version` that
   `synveda init` compares against the CLI's own before it starts anything
   (ADR-0065 decision 5) — so a seeder that has drifted from the product it
   seeds is caught by machinery that already exists, rather than needing its
   own. The cost is option 3's surviving objection, mitigated in decision 5.

5. **Restore the database from a fixture dump.** Fast and reproducible.
   Rejected outright, and it is worth saying why in an ADR rather than leaving
   it as taste: a dump writes rows the PDP never saw and the audit chain never
   recorded, so the demo organisation would be the one artefact in the product
   whose existence nothing can account for — a governance demo assembled by
   bypassing governance. It would also make the chain unverifiable, which is
   the first thing the tour asks a tester to check.

## Decisions in detail

### 1. The seeder ships beside the product, not inside it

`init --demo` keeps its current job — the IdP's people, which is
infrastructure and correctly the installer's. `demo/seed.sh` does the governed
half, and it is a separate artefact rather than a second flag or a new verb.
The split is the ADR-0055 boundary made physical: what needs no bearer is the
installer's, what needs one is the operator's, and the second thing is not
part of the product a customer buys.

`init --demo` prints the seeder's resolved path on completion, through the
same `Profile` that already distinguishes a checkout from an installed
bundle — so a contributor is pointed at `deploy/release/demo/seed.sh` and a
tester at `~/.synveda/profile/demo/seed.sh`, and neither has to know that the
distinction exists.

### 2. One principal seeds everything, because the matrices already do the work

This decision was first written the other way round — service identities
(AUTH-3) would author what the operator could not review alone — and reading
the approval matrices before implementing it showed that it was solving a
problem the product does not have, and would have obscured a better demo.

`crates/synveda-policy/src/approvals.rs` and the invariant floor at
`crates/synveda-types/src/approval.rs:200` produce all three states this demo
needs **from one principal**, purely by where the content lands:

| Seeded act | Requirement resolves to | Demo value |
|---|---|---|
| Climb a memory from the operator's personal scope to the **org root** under `regulated-strict` | curator + steward, **2 distinct** | sits pending; a real FLOW-5 cross-scope promotion the operator cannot finish alone |
| Author and propose a **skill** | SecurityReviewer + 2 distinct, from the **floor** | sits pending; demonstrates the one requirement no pack can lower |

Two further constraints shaped this, both found by reading the code rather
than by reasoning about it, and both worth recording because they bound what
*any* single-operator seeding can produce:

- **Extracted records land in the observer's own personal scope** —
  `let home = identity.scope_id` (`crates/synveda-gateway/src/observe.rs:203`).
  The operator is placed under the org root rather than in a team, so their
  observed turns are not `eng/platform`'s records and cannot be published
  there. What *is* reachable is a climb to an ancestor, which is the org root
  — and FLOW-5's rule that a climb goes up the chain composition walks down is
  what makes the org root the only valid target.
- **Roles are literal.** `effective_roles_at` (`pdp.rs:1253`) collects bound
  roles and implies nothing, so the tenant-wide `OrgAdmin` the operator gets
  at first login satisfies neither `Curator` nor `Steward`. An operator can
  bind themselves those roles — it is their tenant and the act is audited —
  but they remain **one person**, so `regulated-strict`'s two-distinct rule
  still holds. The dual-control demo survives the operator having every role.

So a single operator can seed a governed organisation and two honestly stuck
proposals, and **cannot** seed published channel history — that needs a second
person, which is the product working correctly. The tour therefore makes
publication something the tester *attempts* and is refused at, rather than
something they find already done. Meeting dual control by hitting it is a
better demonstration than reading that it exists.

### 3. A named human cannot be pre-authorised on this path — a product fact, not a demo limitation

The original plan had the seeder pre-binding roles for Alice and the others,
so their first login would land somewhere with authority. It cannot, and the
reason generalises past the demo.

A role binding is keyed on the OIDC **`sub`** (`role_bindings.subject`;
`crates/synveda-store/src/role_bindings.rs:54`), which is the IdP's to assign
and which the product learns when the person first presents a token. There is
no `/v1/identities` route to resolve an email to a subject — `identities` is
reachable over HTTP only for *service* identities — so nothing a CLI verb can
call knows who Alice is until Alice arrives.

The product's answer to pre-provisioning a named person is **SCIM** (AUTH-4),
which creates the identity ahead of login and lets the first login adopt it
(`provision.rs:171`, `adopt_directory_identity`). The bundled IdP is not a
SCIM source, so the demo path structurally cannot do it.

What the demo relies on instead is the mechanism that *does* work without
knowing anyone in advance: **JIT placement by convention group** (ADR-0013
decision 3). Alice's `synveda-eng-platform` group resolves to the
`eng/platform` team the seeder created, so her first login places her under
it with a personal scope, and what she can and cannot see is decided by her
chain and the pack above it. That needs the scopes to exist and nothing else,
which is exactly what `seed.sh` provides.

`role_bindings::bind` is an upsert with no foreign key to `identities`, so
binding a subject that has never logged in *would* succeed at the database —
which is why this is written down. It would produce a row that matches nobody,
and the demo would look correct while granting authority to a string.

### 4. Guard rails: refuse a tenant that holds an organisation, not one that lacks a marker

The standing warning — "never use `--demo` on a deployment that will hold real
memory" — is prose, and prose is not a guard. A verb that writes a fabricated
organisation must not be able to run against a real one.

This was first designed as a marker: migration 0039 adding
`tenants.demo boolean`, set by `init --demo` at admission, checked here. It is
the wrong guard, and noticing why is worth more than the column.

**The marker records provenance; the hazard is content.** A tenant admitted
with `--demo` that a tester then filled with real memory passes a provenance
check and is exactly the deployment that must be refused. Meanwhile a tenant
admitted without `--demo` and still empty has nothing to lose, and refusing it
only teaches the tester to reach for the override.

So the guard reads the hierarchy: **`seed.sh` refuses if any scope exists
that it did not create.** A foreign scope is an organisation somebody built,
which is the signal that actually matters, and it needs no schema change, no
change to `Tenant`, and no sqlx cache regeneration — the seeder already lists
descendants to be idempotent, so the check is the same read.

Consequences:

- The check composes with decision 5 rather than duplicating it: "everything
  present is mine" is both the idempotency condition and the safety condition,
  so there is one rule and not two that can disagree.
- A **genuinely empty** production tenant will be seeded. Accepted: it is
  empty, and seeding is not destructive.
- The refusal names `--i-know-this-is-not-a-demo-tenant` as the override,
  spelled that way so it cannot be typed by accident or pasted from a blog
  post without the reader noticing what they are asserting.
- It is a guard against *mistakes*, not against an operator who means it —
  the same boundary AUD-1 draws, and this ADR claims no more.

### 5. Idempotency is by lookup, and the org's shape is data so a test can still reach it

`seed.sh` re-run changes nothing. It achieves that by asking the product what
already exists — a scope with this slug under this parent, a proposal with
this title still open — rather than by keeping a manifest of what it wrote. A
manifest is a second source of truth that drifts from the first, which is the
failure ADR-0065 amendment 5 recorded in a different place: *compare the
artefact, not its identity*.

**And this is where option 3's surviving objection is paid.** Moving the
seeder out of the binary moves it out of `cargo test`, so the highest-value
check has to be kept reachable deliberately rather than by accident. The
organisation's shape — departments, teams, the pack assigned to each, the
corpus — lives in `demo/organisation.json` as **data**, and a unit test in the
CLI crate asserts that every group `init --demo` creates in the IdP resolves
to a team that file defines.

That check earns the arrangement on its own. `init --demo` puts people in
`synveda-<department>-<team>` groups and AUTH-2 places them by that
convention, so a team the seeder does not create is a group whose members land
in **quarantine** on first login — a failure that surfaces one person later,
in a different command, looking like an authorisation bug. The shell can be
covered by the integration run; this one cannot wait that long.

What is genuinely lost, and accepted: the guard's branch logic and the
idempotency lookups are now covered only by `demos/ops-9-beta-demo.sh` in CI,
where they were unit tests in the withdrawn design.

### 6. Embedder: the tour leads with TEI and names the cost

`--embedder deterministic` is the default and is a hash: retrieval works and
is exact on the lexical leg, and semantic similarity is not meaningful.
`--embedder tei` serves BGE-M3 at a 2.3GB one-time pull.

This is chosen **before the first record is written** — `record_embeddings`
stores the model and nothing in the product re-embeds a corpus — so a tester
who takes the fast path and later wants honest semantics is rebuilding, not
reconfiguring. The tour therefore leads with TEI and states the download;
`--fast` documents the deterministic path for somebody who wants to see the
shape in two minutes.

The seeded corpus is written so the **lexical leg alone retrieves it**. A
corpus that only demoed well under TEI would make the default path look
broken, which is a worse first impression than a slow one.

Extraction is unaffected: `SYNVEDA_EXTRACTOR=deterministic` is the default,
runs offline, and scored 0.958 macro precision on MEM-3's labelled fixtures.

### 7. Harnesses beyond Claude Code: breadth and proof, no new mechanism

The extension point already satisfies seed §2 principle 6.
`adapters/registry.json` is data rather than a `match`,
`~/.config/synveda/mcp-clients.jsonc` is read through the identical loader
(the built-ins use the extension path, which is what keeps it first-class),
`--print` covers a client we have never heard of and `--config` a layout we
have not.

So this feature adds **no mechanism**. It adds three built-in entries, a
`synveda mcp clients` verb so the registry is discoverable without reading a
doc paragraph, and a tour section that configures a client the release has
never heard of — turning the extensibility claim from an assertion into
something a tester has watched work.

New entries carry the same honesty as the existing Cursor one, which says in
its own comment that nothing has been replayed against a real Cursor.

### 8. What this feature does not do

It adds no capability. Every surface it seeds already exists and every call it
makes is one the CLI or a harness already makes. What it removes is the
requirement that the person watching already knows the verbs.

It proves nothing new about Entra or Okta. It does not close **ADPT-8** — a
headless Claude Code session still injects and never observes — and
`docs/BETA.md` states that rather than routing around it, because the gap is
silent, returns exit 0, and reads exactly like a session that was observed.

## Consequences

- **Positive.** A tester sees a governed organisation two commands after
  install. Every object in it was created through the PDP and is accounted for
  in a chain they can verify — which makes the demo itself an argument for the
  product, in a way a restored dump could not be. `docs/BETA.md` puts the tour
  and the standing limits in one file, so what we invite somebody to try and
  what we admit does not work cannot drift apart.
- **Negative / accepted.** The seeder's own logic is covered by an integration
  run rather than by unit tests — option 3's surviving objection, paid down
  only partly by decision 5's data file. Seeding takes real time because it
  waits on real extraction and embedding. Every seeded object is authored by
  the operator, so the inbox shows one name where a real deployment would show
  several — accepted, because decision 3 establishes that the alternative is
  not available on this path at all. The guard reads the hierarchy rather than
  a marker, so a tenant whose scopes were removed out from under it would be
  seeded again; that is the same latitude every idempotent verb has.
- **Reversal trigger.** If a beta tester runs `seed.sh` against a deployment
  holding real memory despite the guard, the guard is insufficient and the
  seeder stops shipping in the profile bundle at all — becoming something a
  tester fetches deliberately. If the shell ever grows logic that an
  integration run cannot honestly cover — branching on policy decisions, say,
  rather than on what exists — that is the signal option 3 was right after
  all, and the seeder moves back into a binary behind a build feature
  excluded from release artefacts.

## Compliance notes

- **Product surface**: none added. This feature ships no new route, no new
  verb and no new flag — the seeder drives verbs that already exist, and a
  customer's `synveda --help` is byte-identical before and after.
- **PDP**: no path added that bypasses it. `seed.sh` holds no credential of
  its own; it asks `synveda auth token` for the operator's stored bearer, the
  same one every other verb uses, and every create it makes decides at its own
  seam. A seed step the operator lacks authority for fails as a denial,
  audited, rather than being escalated — and that denial is itself a truthful
  demonstration.
- **Audit**: nothing new is emitted. The seeded organisation's chain is
  ordinary product output, which is why the acceptance criterion reads it back
  and asserts exactly one break-glass event — OPS-1's invariant, re-asserted
  by the feature most likely to break it.
- **Multi-tenancy**: the seeder writes only within the operator's tenant,
  reached through the same `begin_tenant_tx` path as everything else, with the
  RLS backstop underneath unchanged. No schema change: decision 4 replaced
  the marker column with a read the seeder was already making.
- **Secrets**: the seeder registers no credential and mints none — decision 2
  removed the only reason it would have. The demo people's shared IdP password
  is `init --demo`'s and predates this feature; BETA.md lists it among the
  things a tester must not carry into a real deployment.
- **Roles**: no role binding is created for a subject that has not
  authenticated. `role_bindings::bind` would accept one — it is an upsert with
  no foreign key to `identities` — and decision 3 records why the seeder must
  not use that latitude: the row would grant authority to a string that
  matches nobody, and the demo would look correct while being wrong.
