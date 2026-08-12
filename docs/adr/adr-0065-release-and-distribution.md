# ADR-0065: installing is a download, not a build — a tagged release ships binaries *and* images, because the bundled IdP forces a host process

- **Status**: Accepted, **amended twice on 2026-08-11** while building it
  (amendment 1: decision 4 gains a printed path, decision 6 gains two more
  defects of the same shape and a third about the messages themselves;
  amendment 2: the release gains a fifth artefact — the Claude Code plugin —
  and decision 2's "two artefact kinds" becomes three; everything else stands)
- **Date**: 2026-08-11
- **Feature(s)**: OPS-8
- **Deciders**: sujitn

## Amendment 2 (2026-08-11): the release shipped no way into the harness it is for

Decision 2 said "two artefact kinds": a **binary** for the default path and an
**image** for the `--issuer` path. That was the right split for *serving*, and
it left out the artefact that makes the product worth installing.

This release drops the prerequisite list to Docker and gets a tester to a
governed round trip in thirteen seconds — through `curl`, the CLI and the
console. What it could not do is put governed memory inside **Claude Code**,
which is the integration the phase demo goal names first and the one ADPT-1
built. The plugin lives in `adapters/claude-code`, its `dist/` is gitignored,
and nothing in the release built or carried it. A tester's only route was to
clone the repository and run `npx tsc` — which is exactly the barrier this
feature exists to remove, surviving in the one place it mattered most.

So the release gains a **fifth asset**, `synveda-plugin-<version>.tar.gz`, and
the CLI gains `synveda plugin install`.

1. **The unit is a marketplace, not a plugin directory.** This is the finding
   rather than a preference. Claude Code installs from a directory carrying
   `.claude-plugin/marketplace.json` into a cache it owns, tracked in
   `known_marketplaces.json` and `installed_plugins.json`. The path this
   repository has documented and demoed for a year —
   `~/.claude/plugins/synveda/` — is read by nothing. `package-plugin.sh`
   builds the wrapper; `adapters/claude-code/marketplace.json` is its
   manifest.

2. **It drives `claude plugin`; it does not write Claude Code's files.**
   ADR-0057 decision 10 justified writing another application's config by the
   absence of an alternative — Claude Desktop ships no CLI. Claude Code ships
   one, so the justification does not transfer. Hand-reproducing three
   cross-referencing JSON files would be a second implementation of somebody
   else's installer, wrong the first time the format moves. Seed §2 principle
   6 is the same argument from the other end: the harness is a guest, and a
   guest is asked rather than edited. The cost is a hard dependency on the
   `claude` binary, which the command names when it is absent and prints the
   two commands to run by hand.

3. **The demo asserts the vendor's view, not the presence of files.**
   `✔ enabled`, four hooks, one MCP server, read back from `claude plugin
   list` and `claude plugin details`. Installing and loading are different
   events — the manifest defect in ADR-0027's amendment installs perfectly and
   then fails to load — so a check that stopped at "the files are there", or
   even at "the install succeeded", would have passed against a plugin that
   has never worked. Where the `claude` CLI is absent, the demo says loudly
   that the assertion did not run rather than skipping quietly.

4. **The installer still touches nothing a client owns.** The bundle is
   unpacked to `$SYNVEDA_HOME/plugin` and installed into nothing. Hooking up a
   client stays a separate, explicit command, and the demo now asserts the
   absence of `~/.claude`, `~/.cursor` and Claude Desktop's config directory
   after `install.sh` runs — because "it writes nothing outside its own
   directories" is a promise the next edit could quietly break.

**What this does not do.** It installs the plugin and proves Claude Code loads
it. It does not prove a *live session* injects and observes — the assertion is
still the hook contract under ADPT-1's own driver, and the missing test is a
real Claude Code session against a real gateway. That is the same gap ADR-0057
decision 11 records for Cursor, in a different place: a fixture is not a
client, and now neither is an inventory.

## Amendment 1 (2026-08-11): the console 404 was not the only one of its kind

Decision 6 treated `SYNVEDA_CONSOLE_DIR` as a single oversight — the image
set it, the host process did not, and CNSL-1's "a missing bundle is a 404,
not a boot failure" kept it quiet. Building the release found **two more with
the same shape**, and the shape is the finding: *code on the install path
that only a contributor had ever executed, failing in a way that reads as
something else.*

- **`synveda init --embedder tei` pulled the amd64 image on Apple Silicon.**
  Upstream publishes two TEI builds and versions only one, so `make dev-up`
  has selected by `uname -m` since FND-2. `init` never did, and inherited the
  compose default. Nobody noticed because every install so far was a
  contributor's, and a contributor runs `make dev-up`. Half of what this
  release ships binaries for is the architecture that was wrong. `init` now
  carries the same table, passed on the compose invocation rather than
  exported, and `scripts/check-chart-images.mjs` learned to read the
  Makefile's pins — which is how the arm64 build, pinned by *commit* because
  there is no versioned tag, reached the licence inventory for the first
  time.

- **`connect()` named a file the operator does not have.** It required
  `DATABASE_URL` and said "dev default is in the Makefile". `synveda audit
  tail` and `synveda audit verify` are the two commands INSTALL.md tells a
  new operator to run after logging in, and on an installed machine there is
  no Makefile to look in. It now falls back to the same URL `init` installs
  against — one `database_url()`, shared — so the documented next step works
  on the deployment the CLI arrived with. Erroring on a missing variable was
  right for as long as a checkout was the only way to reach the code.

- **Three error messages pointed at a domain that does not resolve.** They
  suggested `curl -fsSL https://synveda.dev/install.sh | sh`; `synveda.dev`
  has no DNS record. An error that names a dead URL is worse than one that
  names none, and these are the messages a person reads at exactly the
  moment they have nothing working. There is now one `INSTALL_COMMAND`
  constant, naming a raw GitHub URL, because a vanity domain is a purchase
  and this file has already been wrong about assuming one.

They were found differently, and the difference is worth recording. The TEI
one came from **reading** — comparing the Makefile's table against `init`
while deciding what the bundle had to contain. The other two came from
**running**: `connect()` failed the acceptance demo outright, and the dead
URL surfaced only because a message printed during a manual check. That is
the argument for decision 3's choice of a test over a lint, applied to a
wider surface than the compose file it was written about — and a reminder
that the test catches what it executes, so the reading is not optional.

**Decision 4 gains a sentence: `init` prints the profile it resolved and the
console directory it found.** The precedence is only safe if it is visible —
a contributor whose `target/` is empty falls through to an installed
binary, and a fallback nobody can see is the "debugging session that lies"
the decision was written to prevent, arriving by a different route.

## Context

OPS-1 built an installer that works. `synveda init` goes from nothing to a
governed round trip in 5 seconds of its 600-second budget, and
`demos/ops-1-smb-profile.sh` proves it on a scratch HOME. What it cannot do is
run on a machine that is not this one.

Three dependencies on the developer's checkout, all of them deliberate at the
time and all of them recorded as temporary:

- `init` resolves its compose file relative to the working directory
  (`init.rs:95`), and errors with "run `synveda init` from a Synveda checkout"
  when it is absent. `repo_root()`'s own comment says "a released binary would
  carry its own profile — see ADR-0055 decision 6's trigger."
- `gateway_binary()` looks in `target/release` and `target/debug` only
  (`init.rs:701`), under a comment saying "a release ships this binary."
- The images are built locally and pushed nowhere. `synveda/gateway:dev` builds
  from source at install time (`--build`, `init.rs:202`); `synveda/dev-postgres:17`
  builds from `deploy/compose/postgres`. `.github/workflows/` holds `ci.yml` and
  `eval.yml`, there is no release job, and the repository has no tags.

So the prerequisites INSTALL.md states — "Docker" and "a Synveda checkout and a
Rust toolchain, until there is a release to download" — are honest, and the
second half is the whole of this ADR. A cold release build of this workspace is
minutes of Cedar, Tantivy and sqlx before anything the tester came to see. The
Helm chart has the same problem one layer up: it names `synveda/gateway:<appVersion>`
and `synveda/enterprise-postgres:17`, and nobody outside this laptop can pull
either.

The constraint that shapes everything below is **ADR-0055 decision 8**, which
was found by building the container first and watching it fail. An OIDC issuer
identifier is one URL that both the browser and the gateway must reach.
`IssuerConfig` carries no separate discovery URL, because ADR-0010 compares the
issuer byte-for-byte against the discovery document and the `iss` claim. The
bundled Rauthy's is `http://localhost:8100/auth/v1/`, and RFC 6761 requires
every resolver to answer `localhost` with the *caller's own* loopback ahead of
DNS and ahead of `/etc/hosts`. Inside a container that URL is the container. The
ADR measured the three escapes — `extra_hosts` with `host-gateway`, a network
alias, `network_mode: host` — and none of them works on the two platforms this
release targets.

## Decision

Ship a **tagged GitHub Release with public GHCR images**: prebuilt `synveda` and
`synveda-gateway` binaries, the console bundle, and a self-contained profile
bundle, installed by one `curl | sh`. `synveda init` learns to find all three
outside a checkout. Docker becomes the only prerequisite a tester needs.

1. **The unit of distribution is a tagged release, not a package manager.** A
   Homebrew tap, an apt repository and a `winget` manifest are all wrappers
   around a release's assets; none of them can exist before the assets do, and
   each is a separate maintenance surface with its own review latency. This is
   the artefact they would all point at. Reversal trigger: the first tester who
   asks how to *upgrade* rather than how to install — an installer that cannot
   upgrade is the thing a tap fixes.

2. **Two artefact kinds, because one cannot serve the friendly path.** The
   tempting shape is Docker-only: no binary to install, no architecture matrix,
   no code signing. ADR-0055 decision 8 forecloses it. The default install —
   the one with no IdP to configure, which is the only one a tester can run in
   under a minute — uses the bundled Rauthy, and the bundled Rauthy's issuer
   makes the gateway a host process by RFC 6761 rather than by preference. So
   the release ships a **binary** for the default path and an **image** for the
   `--issuer` path, and the two are built from one source tree at one tag.

   This is not a workaround being preserved out of laziness. The alternative
   ADR-0055 named — moving the bundled IdP off a loopback URL — is still
   available and still costs the same: `pub_url`, `rp_id` and `rp_origin` in
   the shared dev config, and five demos that hard-code `http://localhost:8100`.
   Reversal trigger: a second reason to want it. One is not enough.

3. **The profile bundle is a shipped artefact of its own, and the dev compose
   is not it.** `deploy/release/` holds a compose file that names published
   image tags and has no `build:` stanza anywhere, plus the Rauthy config and
   the Postgres initdb SQL it mounts. `deploy/compose/docker-compose.yml` stays
   what its own header already calls it — the contributor's loop — and keeps
   Temporal, AGE and the build contexts that only make sense next to a source
   tree.

   Splitting them creates a drift risk, and the answer to it is a test rather
   than a lint: `demos/ops-8-release-install.sh` installs from the **packaged**
   bundle, not from `deploy/release/` in place, so a bundle that has drifted
   from the product fails the demo the same way a broken install would fail a
   tester. A checker that compared the two files would only prove they are
   similar, which is not the property anybody wants.

4. **Discovery precedence in `init` is explicit > checkout > installed
   bundle.** `SYNVEDA_COMPOSE_FILE` still wins, unchanged. A checkout in the
   working directory comes next, so a contributor who has also installed a
   release gets the tree they are editing rather than the tag they downloaded —
   the reverse would be a debugging session that lies. The installed bundle at
   `$SYNVEDA_HOME/profile` (default `~/.synveda/profile`) is last, and is what
   a tester has. The same order governs `gateway_binary()`: `target/`, then
   `$SYNVEDA_HOME/bin`, then the directory holding the running CLI.

5. **The bundle's version must equal the CLI's, and `init` refuses otherwise.**
   The CLI, the gateway binary, the image tag and the profile bundle all carry
   one tag, and the bundle records it in a `version` file the installer writes.
   A newer CLI against a stale `~/.synveda/profile` is the failure this
   prevents: it presents as a compose service that will not start or an
   environment variable the gateway does not read, both of which look like
   product bugs and neither of which is. This is ADR-0055 decision 10's
   argument — convergence compares configuration, not liveness — applied to
   the artefact instead of the process.

6. **The console bundle ships, and the host-gateway path finally sets
   `SYNVEDA_CONSOLE_DIR`.** The image has baked the console in and set that
   variable since CNSL-1. The host process never set it, so it falls back to
   `DEFAULT_DIR = "console/dist"` relative to the working directory
   (`console.rs:43`) — which resolves only inside a checkout where somebody has
   run `pnpm --filter @synveda/console build`. The default install path has
   therefore been serving a 404 at `/console/` to everyone who did not build the
   frontend by hand. CNSL-1's rule that a missing bundle 404s rather than
   failing the boot is exactly right and is why this went unnoticed; it is not a
   reason to keep shipping the 404.

7. **macOS arm64 and Linux x86_64, and the installer refuses everything else by
   name.** Two targets cover Apple Silicon laptops and cloud boxes, which is
   the whole of the intended audience. An installer that guesses — downloads
   x86_64 for an Intel Mac it did not recognise, or a glibc build for Alpine —
   fails later, further from the cause, and in the product rather than in the
   installer. So the unsupported case exits with the platform it detected and
   the `cargo build` path that still works. Reversal trigger: a tester on a
   platform we refused. Adding a target is a matrix row.

8. **Unsigned and un-notarized, said out loud.** These binaries carry no Apple
   Developer ID. macOS quarantines anything downloaded by a browser and will
   refuse to run it; `curl | sh` does not set the quarantine attribute, but a
   tester who downloads an asset by hand will hit it, so the installer strips
   `com.apple.quarantine` from what it wrote and prints one line saying the
   binaries are unsigned. Notarization is a purchase and an identity, not an
   engineering decision, and pretending otherwise in a release workflow would
   mean a secret nobody can rotate. Reversal trigger: the first install this
   costs, or the first customer who asks — at which point it is a
   `codesign`/`notarytool` step and two repository secrets, not a redesign.

9. **The release profile's images join the licence inventory.** ADR-0062
   decision 11 established that container images are a fourth artefact class
   that `cargo-deny`, `check-npm-licences` and `check-corpus-licences` do not
   look at, and built `deploy/helm/IMAGES.md` plus a checker for the chart's.
   A release profile is images a *customer installs*, which is a stronger
   reason, not a weaker one — so `scripts/check-chart-images.mjs` gains
   `deploy/release/docker-compose.yml` as a surface and the inventory gains a
   section. The file and the script keep their names, which are now one
   profile too narrow; renaming both plus every reference is churn against
   OPS-2's artefacts for no reading, and the headers say what they cover.

10. **The installer creates nothing the PDP does not see.** It writes files and
    exits. Everything about what `init` may and may not do — no organisation,
    no scopes, exactly one break-glass `tenant.created` event, everything else
    authored by a person who logged in — is ADR-0055's and is untouched here.
    `demos/ops-8-release-install.sh` re-asserts that invariant from the
    installed path rather than trusting that it survived, because the point of
    this feature is a code path nobody has run before.

## Options considered

1. **Docker-only, with the CLI as a `docker run` wrapper.** No binaries, no
   matrix, no signing — the friendliest-sounding shape. Refused on decision 2:
   it cannot serve the bundled IdP without rewriting the dev config, and it is
   worse where this product is most interesting. `synveda mcp install` writes a
   command path into Claude Desktop's and Cursor's config, and those clients
   launch an MCP server over **stdio**; a container wrapper there means the
   client's config points at a `docker run` line whose lifecycle nobody owns.
   The AI-client demo is the demo.

2. **Publish images only, and keep "build the CLI from source" in the
   instructions.** Half the work for none of the benefit: the tester still
   needs a Rust toolchain, which is the actual barrier. The compose stack was
   never the slow part — everything in it except our two images already pulls
   from a public registry today.

3. **A hosted always-on demo instance.** Removes installation entirely for
   people who only want to look. Rejected for the MVP rather than refused: it
   needs TLS, a real issuer, and a story for a shared tenant, and it inherits
   OPS-7's single-replica pin. It also answers a different question — this
   product's claim is about memory that stays inside a customer's boundary, and
   a shared sandbox demonstrates the opposite. Worth revisiting once there is
   something to point people at between meetings.

4. **A package manager first (Homebrew tap).** `brew install synveda` is the
   nicest single line in this document. It cannot be built before the release
   assets exist — a formula downloads a tarball and checks a SHA — so it is
   strictly later, not instead. Decision 1's reversal trigger is the moment.

5. **Do nothing; hand testers the repository.** What happens today. It costs
   every tester a Rust toolchain, a cold release build and a `pnpm` install
   before the product does anything, and it means the first thing a prospective
   customer learns about a product that sells trustworthiness is that it did
   not compile on their laptop.

## Consequences

- **Positive**: the prerequisite list drops to Docker. The install is three
  lines and the fastest of them is the product. The Helm chart's image
  references become pullable for the first time, which OPS-2 could not do for
  itself. The console reaches the default install path. And the release
  workflow is the first thing that builds this workspace for a target nobody
  has built it for, which is worth knowing before a customer finds out.
- **Negative / accepted trade-offs**: a second compose file to keep true
  (decision 3 answers with a test); unsigned binaries (decision 8); no Windows
  and no upgrade path in this feature; and a release process that is now a
  thing that can break independently of `make ci`.
- **What this does not change**: nothing has still replayed a live Entra or
  Okta frame — the release makes the bundled-issuer path installable and says
  nothing about the other one. The chart still pins one gateway replica
  (OPS-7). `records`, `record_embeddings` and the Tantivy sidecars are still
  unsealed (ADR-0064). A release does not make any of those less true, and the
  install docs should not read as though it did.
- **Reversal trigger**: an install that fails on a tester's machine for a
  reason the installer could have detected. That is the measurement this
  feature exists to produce, and the demo is only evidence that it works here.

## Compliance notes

No effect on the audit chain, tenancy isolation or policy enforcement. The
installer performs no store writes; `init` performs exactly the two ADR-0055
allows (migrations, and the tenant admitted as break-glass), and the OPS-8 demo
asserts the resulting chain from the installed path. The bundled Rauthy's
credentials remain dev-only, `--demo`'s people remain a flag a customer must
ask for, and `SYNVEDA_DEV_JWT_SECRET` is still `env_remove`d on the install path
(ADR-0055 decision 3's test still covers it). The release publishes no secret:
GHCR authentication is the workflow's `GITHUB_TOKEN`, and the absence of a
signing identity is decision 8's stated position rather than a missing
credential.
