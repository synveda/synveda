# ADR-0065: installing is a download, not a build — a tagged release ships binaries *and* images, because the bundled IdP forces a host process

- **Status**: Accepted, **amended seven times** — two on 2026-08-11 while
  building it, five on 2026-08-12, each by running the previous release on a
  real machine rather than by review. Every amendment below says what it
  changed and what still stands; none reverses a decision.
- **Date**: 2026-08-11
- **Feature(s)**: OPS-8
- **Deciders**: sujitn

## Amendment 7 (2026-08-12): a health check answered by a stranger

Decision 5 lets a checkout and an installed release coexist, and `Profile`
resolves explicit > checkout > bundle so a contributor's tree wins. Both
still hold. What neither anticipated is that the two share port 8120, and
that `init` could not tell whose gateway was on it.

Run from a checkout on a machine with an installed release up: `init`
spawned a gateway, that process died in milliseconds with `AddrInUse`, and
`wait_for_health` then asked `127.0.0.1:8120/healthz` — which the
**installed** gateway answered. `init` printed `pid 51544`, `healthy` and
`initialised in 6s`, wrote that pid to `gateway.pid`, and exited **0**. The
pid it named had never lived long enough to bind anything.

That is the same fault as amendment 5's upgrade and amendment 4's console,
and it is worth naming precisely rather than as "a shallow check": the
health probe asks *is something answering here*, and every claim built on it
reads as *is the thing I just started answering here*. Those are the same
sentence only when nothing else can be on the port, which is exactly the
assumption decision 5 broke.

The first fix tried was to watch the pid we spawned, and it **did not work** —
worth recording, because it looks sufficient. The gateway connects to
Postgres, reads its key and starts five workers before it binds, so a child
doomed to `AddrInUse` is still alive for the first second, while the
stranger answers immediately. Liveness narrows the race; it does not close
it. What closes it is asking for the port itself: `init` binds
`127.0.0.1:8120` and releases it before spawning, and refuses when it
cannot. The liveness check stays, because it catches the *other* startup
failures — a gateway that dies on a bad migration or an unreadable key —
and it carries the tail of the gateway's own log into the error, so
`Address already in use` reaches the person rather than staying in a file.

The restart path gained the other half. It killed the old gateway and slept
500ms, which was a guess in both directions; with a refusal now waiting
downstream, a gateway slow to shut down would have been reported as a
stranger holding the port. It waits for release, up to ten seconds.

What stands: the port is still 8120 for both profiles, and a checkout still
wins over a bundle. Coexisting was always allowed — running *at the same
time* never was, and now says so.

## Amendment 6 (2026-08-12): the default install path is the one nothing had run

Decision 7 says an installer refuses by name rather than guessing, and
decision 1 says it writes only its own directories. Both held. What did not
hold is what happens when it is allowed to write only *some* of them.

`install.sh` puts the CLI in `$SYNVEDA_BIN`, default `/usr/local/bin` — which
on macOS is root-owned. The branch for that case tested `command -v sudo` and
then ran `sudo cp` unguarded, so **a sudo that ran and refused killed the
script** under `set -e`. Refusal is the ordinary case, not the exotic one: a
managed laptop where the user is not an admin, a pipe with no terminal to
prompt on (CI, a Dockerfile, `ssh host 'curl … | sh'`), or somebody who
declines. By the time it happened the gateway, console, profile and plugin
were already on disk, so it left a **complete install with no CLI on PATH**
and sudo's own error as the only explanation. The fallback to
`$SYNVEDA_HOME/bin` existed and was unreachable — it was guarded on sudo
being *absent*, never on it failing.

Two smaller faults sat in the same block. `sudo cp` wrote **over** the target
where `install_file` deliberately does temp → `chmod` → rename; overwriting a
live Mach-O leaves a signature that no longer matches its contents and macOS
kills the next run with `Killed: 9`, which is an *upgrade's* failure and so
the one least likely to be met before a user meets it. And nothing checked
whether the directory the CLI landed in was on `PATH`, while the next-steps
block printed four `synveda …` commands regardless.

Why no test caught it is the same shape this ADR has now recorded four times:
**the check sat one layer shallower than the claim.** The OPS-8 demo hands
`SYNVEDA_BIN` a directory it created itself, so it only ever took the
writable branch. The default — the path every real `curl … | sh` takes on a
Mac — was the one nothing had ever run. The demo now installs a `sudo` shim
that refuses and asserts the install still completes, that the CLI is in the
fallback, that it is *not* in the unwritable directory, and that the PATH
hint was printed.

What stands: the default is still `/usr/local/bin`, and sudo is still tried.
An install that can use the conventional location should. It just must not
*require* a privilege it never needed — the CLI is a single user-space
binary, and the rest of the product already installs under `$HOME`.

## Amendment 5 (2026-08-12): an upgrade changes neither configuration nor liveness

ADR-0055 decision 10 established that convergence compares *configuration*,
not liveness — a re-run that finds a gateway already up must check what it is
running with. That was right, and it was not enough for the thing this
feature added.

Measured on the first real upgrade, `v0.1.0` → `v0.1.1`: `install.sh`
replaced `bin/synveda-gateway`, `init` reported **"already running with this
configuration"** and healthy, and the previous release kept serving. The
process had been up for two hours; the binary under it was minutes old. An
upgrade changes neither the configuration nor the liveness the comparison was
built from — it changes the artefact, which nothing looked at.

So the fingerprint carries the gateway's length and modification time.
Reinstalling moves both. A digest would be tidier and costs a read of tens of
megabytes on every `init` to answer a question that is "did this change",
not "which one is this".

The pattern is now three for three in this feature, and worth stating as a
rule rather than a coincidence: **every check here that was one layer
shallower than the claim it stood for passed against something broken.**
`GET /console/ → 200` against a console nobody could sign into; "the plugin
files are in place" against a plugin that never loaded; "already running"
against a release that was no longer installed. The fix each time was to
assert the thing itself — sign in, ask the harness, compare the artefact.

## Amendment 3 (2026-08-12): what the dry runs measured, and why images build natively

Decision 1's `workflow_dispatch` — "builds and publishes nothing" — existed
because nothing in `make ci` runs this workflow. Two dispatches justified it
before a tag was ever cut, and both failures were in the release path rather
than the product.

- **A reader that closed early failed the writer.** `bundles` built the
  console tarball perfectly and died *listing* it: `tar -tzf … | head -n 3`
  closes the pipe, `tar` takes EPIPE, and `set -euo pipefail` promotes that
  to a failed job — taking the profile and plugin bundles, later steps in the
  same job, with it. Worth keeping because it is **non-deterministic**: the
  same pattern exits 0 locally against a *larger* listing, because `tar`
  finishes writing into the pipe buffer before `head` closes. `sed -n '1,3p'`
  reads to EOF and cannot lose that race.

- **Multi-arch images under QEMU are not a slow path, they are an unusable
  one.** One `ubuntu-latest` job building `linux/amd64,linux/arm64` through
  `setup-qemu-action` hit its 90-minute timeout with the **gateway image
  alone** unfinished; Postgres never started and `publish` never ran.
  Emulating a Rust release build of Cedar, Tantivy and sqlx is the whole
  cost. The fix is one architecture per **native** runner —
  `ubuntu-24.04-arm` is free for public repositories, which this is — and a
  manifest join at publish time.

  **Dropping arm64 was the alternative and is worse than it sounds.**
  `ghcr.io/synveda/postgres` is pulled by *every* install, not only the
  `--issuer` path, so an amd64-only image would put every Apple Silicon
  tester under emulation — in the one feature whose subject is that
  installing should be easy. The binaries already cover Apple Silicon
  natively; the images have to as well.

- **`publish` was downloading four artefacts nobody asked for.**
  `docker/build-push-action` uploads a `.dockerbuild` build record per build
  — four across the images matrix — and the download step named no pattern,
  so it took everything the run produced. One of those four failed its
  download and took the job with it, on the first run that ever reached
  `publish`. The records are not produced any more
  (`DOCKER_BUILD_SUMMARY: false`) and the download names what it wants, so a
  future action that uploads an artefact cannot land in the directory this
  job checksums and publishes.

**What a dry run still cannot cover**, and this is now stated rather than
implied: the manifest join and `gh release create` are publish-only, so a
dispatch proves both architectures build and never exercises the last step.
That is the residue a first real tag carries, and it is the argument for
cutting `v0.1.0` as a deliberate act rather than a formality.

## Amendment 4 (2026-08-12): the console has never been signed in to

`init` now mints a key-encryption key when none is configured, keeps it at
`<state>/kms.key` (0600), and passes it to the gateway on both start paths.

Not a convenience. A console session seals its tokens under the
**deployment-scope** key (TEN-4, ADR-0064), and `init` never minted one — so
every install booted `Kms::Disabled`, warned that "console sessions and
per-tenant secrets are unavailable", and then failed every sign-in with
`not found: encryption key for deployment`, redirecting to
`/console/?error=server_error`. **The admin console has been unusable on
every fresh install since TEN-4 sealed console sessions.** CNSL-1 built it
before that; TEN-4 gave it a dependency; nothing connected the two.

Amendment 1's shape again, one turn further out: code on the install path
that only a contributor had run, failing as something else. But the reason it
survived *this* feature is the part worth keeping. OPS-8 fixed the console's
404 and asserted the fix as `GET /console/ → 200` — and 200 is what a
keyless deployment returns too, because the bundle is static and holds no
data. **The assertion was one layer too shallow to see that nobody could get
past the login**, exactly as `claude plugin list` had to replace "the files
are in place" in amendment 2.

So the demo signs in: `/auth/login?console=true`, the IdP round trip, the
`__Host-` cookie, and a `/v1/whoami` that resolves the tenant. A deployment
with no key plane fails it at the callback.

Minting rather than prompting follows ADR-0055 decision 5's judgement about
the embedder — an installer's job is to leave a working deployment, and a key
the product needs for its own admin surface is not a decision worth
interrupting an install for. `SYNVEDA_KMS_KEY` still wins where it is set,
the same way `DATABASE_URL` does, and the file carries the backup warning
`kms keygen` has always printed: every tenant key in that database is wrapped
by it.

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
