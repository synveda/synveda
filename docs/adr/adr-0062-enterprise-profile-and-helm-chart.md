# ADR-0062: an enterprise profile that installs only what has a consumer — the chart refuses a second gateway replica and names the two things that break, and the deployed gateway stops being a superuser

- **Status**: Accepted, **amended 2026-08-10** while writing the chart
  (decision 3's mechanism; decision 9 gains the product image's second
  binary; everything else stands)
- **Date**: 2026-08-10
- **Feature(s)**: OPS-2
- **Deciders**: sujitn

## Amendment (2026-08-10): a migration cannot create the extension a migration needs

Decision 3 said `create extension if not exists pgmq` would "join the other
two in a migration", on the observation that migration 0015 creates `vector`
and 0028 creates `btree_gin` while `pgmq` and `age` are created by an initdb
script in the dev image — a split it called accidental. **It is not
accidental, and the tidier arrangement is unavailable.**

Migration **0012** calls `select pgmq.create('observe')`. So the extension has
to exist before the twelfth migration runs, and neither way of putting it in a
migration works:

- Appended as a new migration it runs twenty-five migrations too late, and a
  fresh database — which is every enterprise install — fails at 0012.
- Added to 0012 itself it changes an applied migration's checksum, and sqlx
  validates those on every run. Every existing database, dev and CI included,
  would refuse to migrate.

So **creating an extension is a bootstrap act, not a schema act**, which is
what the dev image was saying all along. The chart's CNPG `Cluster` carries
`postInitApplicationSQL: [create extension if not exists pgmq]`, running once
as superuser in the application database at bootstrap, before the migrator
first connects. `vector` and `btree_gin` still create themselves in 0015 and
0028 and are not repeated. The rest of decision 3 stands unchanged: the image
supplies binaries, and AGE is not among them.

The general form is worth keeping, because it will come up again in OPS-6: a
migration series that is already applied somewhere is append-only in practice,
so anything that must run *before* an existing migration is not a migration.

**Decision 9 gains a sentence: the product image carries two binaries.** The
runtime stage copied only `synveda-gateway`, which is right in the SMB profile
— the CLI is on the operator's laptop and only the gateway is a container. In
a cluster there is no laptop with a route to the database, and the two things
an enterprise install must run are CLI verbs: `synveda db migrate` under the
admin identity, and `synveda scim token issue` to wire Entra or Okta
(ADR-0059). A second image was the alternative; it doubles a build to carry
one binary that shares the first one's entire dependency compilation.

**And one finding that changed the chart's shape rather than a decision.**
`/readyz` is `synveda_store::ping`, which is `select 1`. It answers "the pool
reached Postgres" — true of an unmigrated database and of a role holding no
grants alike — so it cannot order an install behind the migration job, and a
gateway pod would join its Service while every request 500s. That is the right
probe for the failover assertion in decision 7 and the wrong one for start-up,
so the chart waits in an initContainer instead, on the exact read the gateway
needs: `select 1 from tenants limit 1` under the gateway's own credential,
which is false until both the schema exists and the `synveda_app` grant has
landed. Making `/readyz` itself mean "this process can serve" is a better
answer and a product change with its own blast radius (every existing
deployment's readiness semantics), so it is recorded here rather than taken
on the way past.

## Context

OPS-2's text is "HA Postgres (CloudNativePG), Temporal cluster, optional
Qdrant, customer IdP wiring", and its acceptance criterion is "kind-cluster
CI install test."

That sentence was written in the tech plan before three things became true, and
reading it against the workspace as it stands is most of this ADR.

**Force 1 — two of the four components named have no consumer.** Nothing in
this workspace links a Temporal client: `rg temporal` over every `Cargo.toml`
returns nothing, VedaFlow was built in Postgres (ADR-0003), and MEM-3's stages
are only *Temporal-shaped* — serializable activity I/O, orchestration split
from the polling transport — against a day the SDK can be admitted at all
(it is git-distributed and its licence graph pulls ring and aws-lc, both of
which deny.toml refuses). The compose file has run a Temporal cluster since
FND-2 and no line of Rust has ever dialled it. Qdrant is worse placed: OPS-4
owns the `VectorIndex` trait and `rg 'trait VectorIndex'` returns nothing, so
there is not yet a seam for a Qdrant to sit behind. A chart that ships both
hands an operator two systems to patch, back up, monitor and pay for, with
nothing connected to either.

**Force 2 — "HA" is a claim about a process, and this process is also seven
background loops.** `main.rs` starts a pool-saturation ticker, the policy pack
refresher, the extraction worker, the promotion engine, the retention sweep,
the lapse expiry sweep and the search indexer, plus a directory pull for any
issuer configured with one. Five of those write. The surprise on reading them
is which way the evidence falls: **most of the concurrency work is already
done, in the database, on purpose.** The audit chain takes
`select seq, head_hash from audit_chain_heads where tenant_id = $1 for update`
inside the caller's tenant transaction (`chain.rs`), so two processes appending
for one tenant serialize rather than fork the chain. The promotion sweep takes
`watermark_for_update` with the comment "two sweepers that both acted on the
same watermark would fold the same events twice". The lapse sweep uses its
expiry stamp as an idempotency key, "so two overlapping sweeps cannot chain one
expiry twice". PGMQ's archive-lock runs inside the tenant write transaction, so
racing extraction consumers cannot duplicate a record (ADR-0022). Console
sessions are a table (migration `0034_console_sessions.sql`), not a map.

What is *not* safe is much smaller, and both halves are load-bearing:

- `LoginFlow` parks pending logins and CLI handoff codes in a bounded
  in-memory store, and says so in the module doc: "single-replica only until
  OPS-2 (ADR-0010)." A callback that lands on a different pod than the
  `/auth/login` that minted the `state` is a 401 for a login the IdP
  completed.
- `ScopeChainCache` is invalidated **in-process**, tenant-wide, after a
  committed hierarchy mutation, with no TTL and no eviction anywhere in
  `scope_chain.rs`. A hierarchy move handled by one replica leaves every other
  replica composing against the ancestry the mover has left — indefinitely,
  silently, and in the one direction that matters.

**Force 3 — the isolation backstop this product advertises is inert in every
deployment that exists.** ADR-0009 forces RLS on every tenant-scoped table
(owners are not exempt) and creates `synveda_app` NOLOGIN, non-superuser, no
BYPASSRLS, granted least privilege by every migration since. Its decision 5
ends "LOGIN and credentials are provisioned per deployment profile
(OPS-1/OPS-2), never by a migration", and its consequences name the cost
plainly: only paths exercised under `synveda_app` — the AC suite, the demo,
"and eventually the deployed gateway" — observe the backstop. `begin_tenant_tx`
sets the GUC and nothing else; it does not `SET LOCAL ROLE`, which is the test
harness's device. The compose gateway's DSN is
`postgres://synveda:synveda-dev@postgres:5432/synveda` and `synveda` is
`POSTGRES_USER` — the bootstrap superuser. **A superuser bypasses forced RLS.**
ADR-0009 deferred this twice in one sitting — decision 5, and option 3's
"deferred to the deployment profiles; `SET LOCAL ROLE` proves identical
enforcement now" — and STATUS.md has recorded it as still standing since
2026-07-19. This is the deployment profile.

**Force 4 — nothing has ever asked the gateway image to serve.** ADR-0055
decision 8 built the container first, found that RFC 6761 makes the bundled
IdP's `http://localhost:8100/auth/v1/` resolve to the container itself, and
split the profiles: a real issuer runs the image, the bundled one runs the
binary on the host. Its consequences record the residue exactly — "the
container path is built on every demo run, so it cannot silently stop
compiling, but nothing yet proves it *serves* — OPS-2's kind-cluster install
test is where that becomes true." A cluster changes the premise that forced
the split: a Service DNS name is a real name, so
`http://rauthy.<ns>.svc.cluster.local:8080/auth/v1/` is one URL that resolves
identically for the gateway pod and for anything else in the cluster.

**Force 5 — `synveda init` cannot install this.** `init.rs` shells
`docker compose -f <file> …`; it is a compose orchestrator with a pidfile and
a rendered-config comparison. But ADR-0055 decision 1 is what makes a second
installer cheap rather than a rewrite: the only store-level writes an installer
performs are the migrations and one tenant row, because the org root arrives
by logging in and everything else is a governed verb.

## Decision

1. **The chart installs the product and its database, and nothing that has no
   consumer.** CloudNativePG is a real dependency and ships: the chart carries
   a `Cluster` resource, three instances, and expects the CNPG **operator** to
   be installed separately — an operator is cluster-scoped infrastructure, and
   a product chart that installs one fights every other chart that does. The
   kind job installs it as a test step. **Temporal is not in the chart** and
   **Qdrant is not in the chart**; neither is a value that exists and is
   defaulted off, because a values key is a promise. Triggers, both named
   below.

2. **Two database identities, and neither of them is a superuser at request
   time.** The gateway connects as a non-superuser LOGIN role that is a member
   of `synveda_app` — which is exactly the privilege set every migration has
   been granting since 0003, so nothing new is granted and nothing has to stay
   in sync. Migrations run in a separate Job under an admin identity, because
   they `create extension` and `create role`. `values.yaml` has no key that can
   point the gateway at a superuser DSN. This discharges ADR-0009 decision 5
   and the TEN-2 obligation standing behind it, and it means the forced-RLS
   backstop is observed by a deployment for the first time.

3. **Extensions are the schema's business; the image supplies binaries.** The
   split today is accidental: migration 0015 runs `create extension vector`,
   0028 runs `btree_gin`, and `pgmq` and `age` are created by an initdb script
   baked into the dev image — so a migration (ADR-0020) depends on an extension
   that no migration creates. `create extension if not exists pgmq` joins the
   other two in a migration, and the enterprise Postgres image is CNPG's base
   plus pgvector and PGMQ binaries. **AGE is not in the enterprise image.**
   GRPH-1 named the condition and it holds — `crates/*/src` contains zero
   `cypher(` call sites, which is the count `demos/grph-1-graph-schema.sh`
   prints — and AGE is the one extension here that needs
   `shared_preload_libraries`, has no PG17 release (the dev image builds
   `PG17/v1.7.0-rc0` from source), and would have to be recompiled against the
   operator's base image on every minor upgrade. The dev compose image keeps
   it, for reasons that are not the enterprise profile's: the only Rust caller
   is `crates/synveda-store/tests/graph_spike.rs`, which is GRPH-4's evidence,
   and `scripts/smoke.sh` and the two GRPH demos run `cypher()` directly.
   GRPH-1's condition was about what we ship, not about what we keep to think
   with.

4. **The chart ships one gateway replica and refuses to be told otherwise.**
   There is no `replicaCount`; the Deployment is `replicas: 1` with
   `strategy: Recreate`, and the template fails with the two reasons if a
   values file tries to override it. This is the opposite of the usual default,
   and it is deliberate: both failures under N replicas are **silent**. A login
   whose callback lands on the wrong pod is a 401 that looks like an IdP
   problem, and a replica composing against a stale scope chain looks exactly
   like a policy decision — the material it returns is material a real ancestry
   once permitted. A comment in `values.yaml` is not an enforcement.

   So the HA in this profile is the HA the feature text actually names: **the
   data plane survives losing a node, and the gateway restarts onto it.** Force
   2's inventory is why that is a defensible sentence rather than a retreat —
   the chain, the sweeps and the queue are already multi-process safe, so what
   remains is two specific pieces of process-local state rather than a
   pervasive assumption. The retention sweep is the one writing loop where
   nothing in it names which device it relies on; it is recorded as unverified
   rather than claimed safe, and one replica makes the question moot for now.

5. **Gateway horizontal scale is filed as its own feature, OPS-7, in Phase 4.**
   It needs three things and each is a decision, not a chore: pending logins
   and handoff codes moved to Postgres beside the console sessions already
   there (a schema and a TTL sweep); cross-process scope-chain invalidation
   (LISTEN/NOTIFY, or a generation column polled beside the pack refresher that
   already polls); and a ruling on whether the writing loops keep running on
   every replica — they are safe, but N replicas is N times the sweep load on
   one database. Phase 4 rather than Phase 3 because the demo goal asks for a
   Helm install and this chart is one, and because the alternative to filing it
   is smuggling three ADRs' worth of design into an install test. It moves
   forward the moment a deployment cannot serve its request rate from one
   gateway, or cannot accept a restart-shaped upgrade.

6. **The chart wires a customer's IdP and ships none.** `SYNVEDA_OIDC_ISSUERS`
   comes from a Secret the operator provides; the chart has no key that can set
   `SYNVEDA_DEV_JWT_SECRET`, so ADR-0010's "one auth mode, never two" is again a
   description of the profile rather than a rule it obeys. `SYNVEDA_PUBLIC_URL`
   is the **externally reachable origin** — the ingress host, never the Service
   — because `main.rs` derives both the `/auth/callback` redirect URI and
   CNSL-1's console `Origin` check from it, and a Service URL there is a console
   that refuses every session for a reason that reads like a bug. The chart
   templates it from the ingress and refuses to render if the ingress is enabled
   and the two disagree. `SYNVEDA_DB_MAX_CONNECTIONS` becomes a value with its
   sizing rule stated against the CNPG cluster's `max_connections`: the pool is
   shared by the request handlers and the background loops, and a chart is
   where those two numbers are chosen together or not at all. Not because the
   pool is known to wedge — commit 29ae21f withdrew that diagnosis and recorded
   that there is no evidence in this repository that it does — but for the gap
   the same commit let stand: a deployment-shaped setting an operator cannot
   reach is one that will be wrong in the place nobody can look.

7. **The kind test asserts a governed round trip, a failover, and a live
   backstop — never readiness.** "Every pod is Ready" is precisely the shape
   EVAL-3's harness lesson warns about: a validity guard that passes when there
   is nothing to validate. The test installs the chart, runs the migration Job,
   creates the tenant, then from inside the cluster performs a **real
   authorization-code + PKCE login** against the test issuer — the technique
   `demos/adpt-1-claude-code.sh` already carries, proof-of-work challenge and
   all — and from there: AUTH-2 provisions the org root at first login,
   `synveda hierarchy create` builds a scope, an observe signal is extracted and
   embedded, an inject returns that material, and the audit chain verifies. Then
   two assertions nothing else in the repository can make:

   - **Failover.** Delete the CNPG primary pod; the same inject succeeds after
     the new primary is elected. The gateway's pool is `connect_lazy`
     specifically so a database outage is a `/readyz` report rather than a
     crash-loop, and this is the first thing that tests that claim.
   - **The backstop is live.** Assert from the gateway's own connection that
     its role is not a superuser and holds no BYPASSRLS. Decision 2 is worth
     nothing if the chart can be misconfigured back to a superuser DSN without
     anything noticing.

8. **The test's IdP is Rauthy at a cluster-DNS issuer, deployed by the test and
   not by the chart.** ADR-0055 decision 4's rule holds one profile up — a
   customer directory is not the installer's to create — so the IdP is test
   scaffolding. What makes the bundled one usable here when a laptop could not
   use it is force 4: the issuer identifier is a Service DNS name, so the URL
   the gateway resolves and the URL the test client resolves are the same URL,
   and the byte-for-byte comparison ADR-0010 performs against the discovery
   document and the `iss` claim holds. This is the run that finally exercises
   the container path. Its honesty limit is stated in the test's own output: no
   browser is involved, so what it proves is the protocol path, not a person's.

9. **The image under test is the released image, and the sidecar volume is a
   cache.** The kind job builds `deploy/compose/gateway/Dockerfile` as it
   stands — not a thin wrapper around a CI-built binary — because the whole
   point of the job is that *this* artefact serves; layer caching pays for it.
   The Tantivy sidecar gets a PVC rather than an `emptyDir`, and the chart
   documents what losing it costs: `index.rs` keeps a state file and a
   watermark beside each tenant's index and heals from Postgres, so a lost
   volume is a rebuild sweep, not a data loss — "deleting a tenant's index
   directory *is* the operator's rebuild procedure". An RWO PVC and `Recreate`
   compose exactly with decision 4; when OPS-7 lands, per-replica volumes are
   the shape, because the index is derived and each replica may hold its own.

10. **The chart refuses to choose the embedder.** There is no default:
    rendering fails until `embedder` is set, and if it is set to `tei`, until
    TEI is given either an in-cluster Deployment with a model-cache PVC —
    BGE-M3 is ~2.3 GB, and a pod that re-downloads it on every restart is a
    pod that is never ready — or an external URL. ADR-0055 decision 5 defaulted
    the SMB profile to `deterministic` to keep a ten-minute clock free of a
    model download; there is no clock here, and the same reasoning inverts.
    `record_embeddings` stores a model and a dim, the dense leg filters on both
    (`search.rs`), and a corpus written under the wrong one is not mis-ranked —
    it drops out of the dense leg entirely and survives on BM25, with no error
    and no degraded header. There is still no re-embed command (ADR-0055's
    first deferral), so the choice is permanent, and **a default that silently
    becomes permanent is not a default**. The extractor is the deliberate
    contrast: `SYNVEDA_EXTRACTOR` is an ordinary value with its API key from a
    Secret, because changing it changes what future extraction produces and
    invalidates nothing already written.

11. **The chart carries an image inventory, and a check asserts it is
    complete.** CLAUDE.md's licence rule is enforced by cargo-deny over crates,
    `check-npm-licences` over packages and — since ADR-0061 — `check-corpus-
    licences` over corpora. A Helm chart references a fourth kind of thing:
    **container images**, and optionally the model weights inside one. Nothing
    in this repository looks at those, which is the same gap in the same shape
    as the one that let a CC BY-NC corpus reach a published phase demo goal
    untouched by any check. So: every image the chart can reference is listed
    in `deploy/helm/IMAGES.md` with its licence and the reason it is there, and
    `scripts/check-chart-images.mjs` fails the build when a template names an
    image the inventory does not. The check is the modest half — it proves the
    list is complete, not that the licences are admissible; a human reads those
    once and records what they read. TEI is the one to read first: an inference
    server's licence is exactly the kind that changes between releases, and the
    optional TEI deployment carries both a binary and a model.

## Options considered

1. **Ship `gateway.replicaCount` with a warning comment.** The conventional
   chart, and what an operator expects. Refused: both failure modes are silent
   and one of them is a governance failure that reads as a policy decision. A
   chart that can be configured into serving stale ancestry is worse than a
   chart that cannot scale, because only one of those two is visible.

2. **Fix the two blockers inside OPS-2 and ship real gateway HA.** The
   tempting scope, and the honest reason to refuse it is that it is three
   designs: a durable login store with a TTL, an invalidation transport, and a
   loop-ownership model. Each wants its own reversal trigger. OPS-2's AC is an
   install test; decision 5 files the work where it can be argued rather than
   inferred from a chart.

3. **Assert readiness only, and call the AC met.** The cheapest passing job.
   Refused by name: EVAL-3's first complete run reported a passing
   `retrieval_recall` over blocks the pipeline had not filled, and
   `bound_instances` read 1.0 for six empty blocks. An install test that never
   asks the installation to do anything is the same instrument.

4. **Teach `synveda init` a Kubernetes backend.** Keeps one installer verb.
   Refused: `init.rs` is a compose orchestrator down to the pidfile, and a
   second orchestration backend inside it makes both worse. Helm *is* the
   enterprise installer, and ADR-0055 decision 1 is what makes that a small
   claim — the install writes migrations and a tenant row, and the org root
   still arrives by logging in.

5. **Use a static JWKS fixture as the test issuer.** Faster and less
   scaffolding. Refused: the deployed image's OIDC path — discovery, JWKS
   fetch, PKCE exchange, `iss`/`nonce` verification — is a substantial part of
   what "it serves" means, and force 4 is a debt about that exact path.

6. **Ship the Temporal cluster because the feature text says so.** Refused;
   see the trigger. The same reasoning would have shipped Qdrant against a
   trait that does not exist.

7. **Defer the chart to Phase 4.** Fails the phase demo goal: it names five
   features and the other four — AUTH-4, AUTH-5, ADPT-2, EVAL-3 — are
   delivered. "Helm install" is the part left.

## Consequences

- Positive: the RLS backstop is enforced in a running deployment for the first
  time since TEN-2 landed, and the kind test asserts the property rather than
  the configuration. Expect the first honest cost of this to be a missing grant
  — the AC suite exercises `synveda_app` under `SET LOCAL ROLE`, but the whole
  product's surface has never run a request through a non-superuser connection,
  and a grant nobody needed in a test is exactly the kind that is missing.
- Positive: the container path stops being unproven, which closes the residue
  ADR-0055 recorded against its own decision 8.
- Negative / accepted: **two Postgres images**, dev with AGE and enterprise
  without. The trigger for collapsing them is the retirement of
  `graph_spike.rs`'s evidence and the `cypher()` line in `scripts/smoke.sh`;
  no product source calls it, so that is a documentation decision rather than
  a code one whenever somebody wants it.
- Negative / accepted: **a restart is a brief outage**, and the chart's own
  upgrade path is a `Recreate`. Stated in the chart's README rather than
  discovered; OPS-7 and OPS-6 are the two features that change it, from
  different directions.
- Negative / accepted: **CI gets slower.** A kind job builds two images, starts
  an operator, a Postgres cluster and an IdP, and runs a round trip. Budget:
  **20 minutes**, matching the existing `eval` job's shape. If it exceeds that
  once caching settles, the split is a PR-time job that lints and renders the
  chart plus a nightly that installs — the same division `eval` already makes
  between its deterministic half and its retrieval half, for the same reason.
- Negative / accepted: the chart requires an operator to install CNPG first.
  One more step in a runbook, against a real problem — cluster-scoped CRDs
  owned by a product chart break the second product that wants them.
- Reversal triggers:
  - **Temporal enters the chart** when a crate in this workspace links a
    Temporal client, which needs the licence graph to change (deny.toml refuses
    ring and aws-lc; the SDK is git-distributed). MEM-3's stages are shaped for
    that day and are not waiting for it.
  - **Qdrant enters the chart** when OPS-4's `VectorIndex` trait exists and its
    benchmark gate picks a per-deployment default. Not before: there is nothing
    for it to be behind.
  - **The single-replica rule is lifted by OPS-7 and by nothing else** — not by
    a customer asking, and not by a values override.
  - **The install test moves to nightly** if it cannot hold 20 minutes on a
    standard runner with layer caching warm.

## Compliance notes

- **Seed §2.2 / CLAUDE.md (no path around the PDP):** the chart creates nothing
  through a bypass. Its install Job runs the two audited break-glass paths
  ADR-0055 decision 1 already named — `db migrate` and `tenant create` — and
  every scope, identity, role binding and record the kind test produces is
  created through the gateway under a real bearer and a real PDP decision,
  which is ADR-0055 decision 9's rule applied to a test rather than a demo.
- **TEN-2 / ADR-0009 decision 5:** discharged by decision 2. The deferral
  standing in STATUS.md since 2026-07-19 — "deployment profiles (OPS-1/OPS-2)
  must connect as a non-superuser `synveda_app` login" — closes here, and
  decision 7's second assertion is what keeps it closed.
- **ADR-0010 decision 3 (one auth mode):** decision 6; the chart cannot express
  the dev mode at all.
- **ADR-0016 (scope chain):** decision 4 is the first place that ADR's
  process-local invalidation contract becomes a *deployment* constraint. The
  contract is unchanged; what changes is that a chart now knows it exists.
- **ADR-0019 (audit chain):** unchanged and, as force 2 records, already safe
  across processes by the head lock — which is why the constraint in decision 4
  is two named pieces of state rather than "the gateway is not HA".
- **AUD-1 (DoD item 4):** no new action type. A chart is not an actor, and
  nothing in this feature performs an action the audit vocabulary does not
  already carry.
- **CLAUDE.md's licence rule:** decision 11 extends it to the artefact class a
  chart introduces. ADR-0061 is the precedent and the warning — the rule was
  enforced over crates and packages while a corpus walked past it, and a
  container image is the next thing in the queue.
- **DoD item 3 (spans and metrics):** OPS-2 adds no request-path code. What it
  does add — `create extension pgmq` in a migration, a chart, a CI job, an
  image-inventory check, and whatever decision 7's assertions need — runs
  through existing instrumentation or outside the process entirely. This note
  exists so that "no new spans" is a recorded finding rather than a skipped
  step.
