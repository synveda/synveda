---
title: "OPS-9: Beta demo profile"
labels:
  - epic:OPS
  - phase:3
size: L
---

# OPS-9: Beta demo profile

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** L

## Description

A locally-runnable build somebody else can be *shown*, and then left alone
with. `synveda demo seed` builds a living ACME organisation through the
product's own governed surfaces; `docs/BETA.md` is the tour and the honest
limits in one file; the MCP client registry gains breadth and a demonstrated
extension point, so the harness story is not Claude Code alone.

## Why this exists

Filed 2026-08-13. OPS-8 removed the prerequisite that stopped anybody outside
this laptop from installing the product, and stopped one step short of the
prerequisite that stops them from seeing it.

`synveda init --demo` seeds four people into the bundled IdP and then prints
the commands that would build ACME's scopes (`init.rs:277-287`, and the
`--dry-run` block at `init.rs:339-356` prints them in full). Nothing creates
them. A tester who follows INSTALL.md lands, correctly authenticated, in an
organisation with no scopes, no memory, no proposals and an empty console —
and the governance machinery that is this product's entire differentiator is
invisible until they hand-run a dozen verbs nobody handed them.

**The printing is not the defect.** ADR-0055 decision 1 is explicit: these are
governed creates that need the operator's own bearer, and the operator has not
logged in when `init` runs. Decision 2 is the invariant it protects, and
`demos/ops-1-smb-profile.sh` asserts it — 0 scopes, 0 identities, 0 role
bindings, 0 records the moment the installer finishes, and exactly one
break-glass event in the chain. An installer that seeded an org would show
ACME's hierarchy standing under a break-glass actor, which is the opposite of
what this product sells.

So the seeding moves rather than being abolished. `demo seed` runs *after*
login, under the operator's own bearer, through the same routes the CLI
already drives — the one shape that produces a living organisation while
leaving OPS-1's invariant exactly as it was, because the invariant is about
what an *installer* may do and this is not one.

## Why a CLI verb rather than a `demos/` script

The audience is precisely the person who ran `curl … install.sh | sh` and has
no checkout, so a script under `demos/` is unreachable to them. The 60 scripts
there also could not serve this purpose if they were reachable: each is a
per-feature acceptance proof that seeds its own scratch state and tears it
down, and none of them is a tour. `demo seed` ships in the binary the
installer places.

## What it seeds

A *living* organisation, not a scaffold:

- The two departments and three teams `init.rs` already describes, created
  through `POST /v1/hierarchy`.
- Policy packs in deliberate contrast — and the contrast is load-bearing
  rather than decorative, because it is what produces the next three items
  from one principal (ADR-0066 decision 2).
- A memory corpus per team through `POST /v1/observe` — the real
  observe → extract → embed path, waited on rather than assumed.
- A **published** memory at a `standard` team scope, where the matrix
  resolves to no approvers at all, so the channel has real history.
- A memory proposal at a `regulated-strict` department, where it takes
  curator + steward and two distinct people, so it sits **pending** and the
  operator cannot approve their own.
- A **skill** proposal, which the invariant floor holds at SecurityReviewer +
  two distinct whatever the pack says — a second inbox item that demonstrates
  the one requirement no pack can lower.
- An active lapse, so "strict by default, relaxable by design" has an instance
  a tester can look at.

The audit chain is not seeded. It falls out of the seeding, which is the
point, and the acceptance criterion reads it back.

## Who authors what — one principal, and why that is not a compromise

Everything is authored by the operator. This was planned the other way round
(service identities writing what the operator could not review alone) and
reading the approval matrices first showed it was unnecessary: under
`standard` a memory at a **team** matches no rule and the floor covers only
`Restricted` and `Skill`, so it publishes with zero approvers; the same
publication at a `regulated-strict` **department** needs two distinct people;
and a skill needs a SecurityReviewer and two distinct approvers under every
pack, from the floor. One operator, three outcomes, no impersonation.

Separately — and this is a product fact rather than a demo constraint — a
named human **cannot** be pre-authorised on this path. A role binding is keyed
on the OIDC `sub`, which the product learns when somebody first presents a
token, and no route resolves an email to a subject. Pre-provisioning a named
person is SCIM's job (AUTH-4) and the bundled IdP is not a SCIM source. What
works without knowing anyone in advance is JIT placement by convention group,
which needs the scopes to exist and nothing else — so `demo seed` creating
them is the whole of what Alice's first login requires.

## Beyond Claude Code

The extension framework already exists and is first-class:
`crates/synveda-cli/src/mcp/clients.jsonc` is data rather than a `match`,
`~/.config/synveda/mcp-clients.jsonc` is read through the identical loader,
`--print` covers a client we have never heard of and `--config` a layout we
have not. Seed §2 principle 6 is already honoured. What is missing is breadth
(three built-ins) and *proof* — nothing demonstrates the extension point, so
its existence is a paragraph in INSTALL.md rather than a thing a tester has
seen work.

## What it deliberately does not do

It adds no capability. Every surface it seeds already exists and every call it
makes is one the CLI or a harness already makes; what it removes is the
requirement that the person watching already knows the verbs. It proves
nothing new about Entra or Okta, ships no Windows support, and does not
close ADPT-8 — a headless Claude Code session still injects and never
observes, and BETA.md says so rather than routing around it.

## Acceptance criteria

Demonstrated by `demos/ops-9-beta-demo.sh`:

- `init --demo` → `login` → `seed.sh` → a recall that returns seeded memory,
  with OPS-1's invariant re-asserted in between: **0 scopes, 0 identities,
  0 role bindings, 0 records** the moment the installer finishes.
- The seeder **refuses before a login**. The design rests on it holding no
  authority of its own, so it must be unable to act until a person lends it
  theirs — and the refusal names `synveda login` as the fix.
- The inbox holds an open proposal, and the operator **cannot finish it**:
  approve and publish are both refused, because publishing to the org root
  takes two distinct people and there is one.
- The pack contrast is real — `eng` carries `standard` where `sales`
  inherits the tenant default.
- The console is **signed in to**, not merely served: the callback lands on
  `/console/` rather than an error, a `__Host-` cookie comes back, and that
  cookie authorizes `/v1/whoami` for the right tenant and reads the inbox.
  `GET /console/ → 200` is asserted too and is explicitly *not* the criterion
  — OPS-8's finding is that a 200 passes against a console nobody can sign
  into, and this demo shipped repeating it until a person tried the console
  by hand (ADR-0066 amendment 1).
- The chain verifies and holds exactly **one** break-glass event: the tenant
  admission, before any person existed to attribute it to.
- A second run changes nothing: scopes, records and proposals all unchanged.
- A tenant holding a scope the seeder did not create is **refused**, the
  refusal names the foreign scope, and the override seeds anyway.
- `--dry-run` prints a plan and writes nothing.

Covered elsewhere rather than here, deliberately:

- **The released path** is OPS-8's criterion and its demo already proves it
  end to end on a scratch HOME with the Rust toolchain shimmed out. What this
  feature adds to it is one file inside the profile bundle, which
  `scripts/package-release.sh` asserts is present, executable and valid
  shell. Re-running a twenty-minute release install to test a file copy buys
  nothing.
- **The IdP groups agreeing with the seeded teams** is a unit test in
  `crates/synveda-cli/src/init.rs`, because its failure mode is silent — a
  demo person landing in quarantine one login later, looking like an
  authorisation bug.

## What the acceptance demo isolates, and the one thing it cannot

Found by the demo breaking a live deployment on the machine it ran on, which
is the failure it now exists to prevent. The bundled Rauthy binds every login
from its issuer to **one configured tenant** (`TenantBinding::Static`,
ADR-0010 §4), so a demo run using the checkout's profile pointed the shared
gateway at its own throwaway tenant and left it there. Every subsequent
`synveda` command against the developer's own deployment came back as a
policy denial naming a tenant they had never created.

Isolated now: its own **compose project** (so it can never adopt or destroy
another project's containers or volumes), its own **state** — a scratch
*bundle* profile via `SYNVEDA_COMPOSE_FILE`, which moves `data/` and with it
the rendered gateway environment carrying the tenant binding — and its own
**HOME**.

Not isolated, and not isolatable without a product change: **the ports**.
`GATEWAY_URL` and `RAUTHY_ISSUER` are constants, and the issuer is compared
byte-for-byte against the discovery document and the `iss` claim (ADR-0010).
So the demo **refuses to start** when a port it needs is held, and names what
to stop — which is the half of isolation that actually protects the thing it
used to break.

## What was unproven, and now is not

The acceptance criteria recorded that nobody had run `seed.sh` from
`$SYNVEDA_HOME/profile/demo/` — only from the tree. Building a scratch bundle
for isolation made proving it free: the demo copies the seeder into the
bundle and executes it from there, so the installed path is the path under
test.

Still true, and smaller: the bundle is assembled from the dev compose file
rather than from `scripts/package-release.sh`'s output, because a release
profile pulls published image tags and the demo runs against a tree that may
be ahead of any of them. So the *seeder's* installed path is proven and the
*profile packaging* is covered separately, by `package-release.sh`'s own
assertions that the bundle carries the seeder, executable and valid shell.
