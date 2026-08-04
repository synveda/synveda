# ADR-0056: a console that adds no API and no actor — the cookie names a bearer instead of becoming one, the verdict moves to the only party that knows the rule, and parity is a corpus both renderers have to answer

- **Status**: Accepted
- **Date**: 2026-08-04
- **Feature(s)**: CNSL-1 (and the toolchain CNSL-2, CNSL-3, CNSL-4 inherit)
- **Deciders**: sujitn

## Context

CNSL-1's text is "review queue with diffs, scan reports, quality scores,
evidence; approve/reject", and its acceptance criterion is one clause: **full
review parity with CLI**.

The first thing to establish is what is actually missing, because it is much
less than the feature's size suggests. `GET /v1/proposals/{id}` already
returns `ProposalDetail`, and that struct already carries every noun in the
feature text: `members` with the bytes under review, the baseline they would
overwrite and the record's current content (FLOW-6, ADR-0035 decisions 5 and
8); `approvals`, the review log; `scan`, the SKIL-2 report recomputed on the
read rather than stored (ADR-0052 decision 6); and `quality`, the SKIL-3 pair
rendered against the pack that will decide the publication (ADR-0053 decision
11). `approve`, `reject`, `withdraw`, `checklist`, `publish`, `classify` and
`lapse` are all routes. FLOW-6 built a review flow and gave it a JSON API; the
CLI is a client of that API and not the owner of it.

So CNSL-1 adds **no governed API**. What it adds is two things this product
has never had: a browser at the `/v1` seam, and a *second renderer* of
judgements that currently exist in exactly one implementation.

Four forces.

1. **A browser cannot hold what the CLI holds, and that is on purpose.**
   ADR-0027 decision 6 made `SessionResponse` structurally incapable of
   carrying a refresh token to a browser — a separate type from the CLI's, so
   that the invariant is enforced by the compiler rather than by a reviewer
   noticing an `Option` field. `/auth/callback`'s browser branch therefore
   returns the session as JSON, which is honest for a dev affordance and
   useless for a console: it renders an access token into a browser tab, and
   nothing on the other side catches it. A review screen needs a session that
   survives a page reload, and the one credential that would make that
   possible is precisely the one a browser may not be given. Every plausible
   design here is a way of answering that, and the wrong answers are the
   comfortable ones — a token in `localStorage`, or a refresh token handed to
   JavaScript because the alternative needed server state.

2. **The CLI's review block is not a pretty-printer, and a second one is a
   second set of decisions.** `synveda proposal review` decides things.
   Severity is compared **by rank rather than by string**, so `critical` under
   a `high` threshold blocks where equality would have said it did not; an
   unknown severity — a gateway newer than the CLI — ranks above everything
   and is treated as blocking rather than as decoration; findings are ordered
   worst-first; only a blocking finding is painted as a refusal, the rest as
   metadata; quality's two numbers are rendered apart because a reviewer who
   sees them averaged cannot tell a well-formatted bundle nobody worked
   through from one somebody did; and a shortfall's *sentence* is
   reconstructed client-side, because the gateway serialises `QualityShortfall`
   as data and a CLI that printed a `kind` slug would be making its reader
   look the meaning up. Three of those are unit-tested in the CLI today.
   Reimplementing them in TypeScript produces two implementations of "does
   this block?", and two implementations that agree on the day they are
   written is not parity — it is a coincidence with a maintenance schedule.
   The AC's word is *parity*, and the only version of that word worth having
   is one a test can fail.

3. **A deployment that grows a runtime is a deployment OPS-2 has to ship
   twice.** OPS-1 already accepted two start paths — the container for a real
   issuer, the host binary for the bundled one — for a reason no configuration
   reaches (ADR-0055). Adding a Node process next to the gateway would make
   Helm ship two runtimes to configure, observe, secure and version, on top of
   a split that is already a stated cost. The console has to be something the
   existing single binary can serve.

4. **npm is a licence surface with no gate.** `deny.toml` holds the core path
   to MIT / Apache-2.0 / PostgreSQL / Unicode-3.0 with narrow per-crate
   exceptions, each annotated with the feature that introduced it. CI has run
   `pnpm -r build` and `pnpm -r test` since ADPT-1, but nothing checks a
   licence on that side, and it has not mattered: `adapters/claude-code`,
   `adapters/mcp-server` and `sdks/typescript` between them declare
   `typescript` and `@types/node` as devDependencies and no runtime
   dependency at all. A console is the first package in this repo with a real
   runtime dependency tree, so it is the first one where CLAUDE.md's licence
   rule has anything to enforce against.

## Decision

1. **The console is a static bundle the gateway serves from its own origin.**
   `console/` joins the pnpm workspace beside `adapters/*` and
   `sdks/typescript`; `vite build` produces `console/dist`; the gateway serves
   it with `tower-http`'s `ServeDir` (which needs the `fs` feature — `trace`
   is all that is enabled today). One runtime in the deployment, nothing new
   for OPS-2's chart to ship, no CORS layer and no cross-origin story to get
   wrong. Same-origin is also what makes decision 2 available at all: a
   `SameSite=Strict` cookie is only useful to a page served from the origin it
   is scoped to.

2. **The cookie names a bearer; it does not become one.** A new
   `console_sessions` table (migration 0034) holds the IdP access and refresh
   tokens server-side against an opaque, high-entropy session id; the browser
   receives that id as `HttpOnly; Secure; SameSite=Strict; Path=/`. When a
   `/v1` request arrives with no `Authorization` header, `tenant::resolve`
   reads the cookie, loads the stored **access token**, and hands it to
   `state.verifier.verify()` — the same call, the same JWKS, the same
   `active_tenant`, the same uniform 401 (ADR-0008). No claims are ever minted
   from a session row.

   This is the load-bearing sentence of the ADR, so it is worth stating as an
   invariant rather than as an implementation note: **the session's authority
   is the token's authority, re-checked on every request.** A token the IdP
   has expired or revoked cannot be laundered into a longer-lived console
   session, because there is no code path in which a `console_sessions` row
   answers the question "who is this" — it only answers "which bearer is
   this", and the bearer is then verified exactly where every bearer is.
   ADR-0010's "one auth mode, never two" survives intact: this is one auth
   mode with a second transport, and the transport reaches the same verifier
   or the request is a 401.

3. **The refresh token moves to the server and stops travelling.** When the
   stored access token is within skew of expiry, the gateway renews it through
   the machinery `POST /auth/refresh` already uses and verifies the result.
   ADR-0027 decision 6 is preserved in a stronger form than the CLI gets: the
   refresh token goes from a laptop's `~/.config/synveda/credentials` to a
   row the browser cannot read, and never enters JavaScript at any point. The
   stored session id is held as a **hash**, on the AUD-1 threat model — a
   database-credentialed attacker who dumps the table must not be able to mint
   a cookie from it.

4. **A cookie is ambient authority, so mutation requires proof of intent —
   and only on the cookie path.** Every non-GET `/v1` request authenticated by
   cookie must carry an `Origin` matching the gateway's own; a missing or
   mismatched `Origin` is refused before the PDP is consulted. `SameSite=Strict`
   is the first line, but it is a promise made by the browser, and approve,
   reject, publish and lapse are exactly the actions worth forging.
   Bearer-authenticated requests are untouched: a header is not ambient
   authority and cannot be attached by a cross-site form, so the CLI, both
   adapters and every service identity keep precisely the path they have
   today. The check keys off *how the request authenticated*, not off the
   route.

5. **The verdict moves to the only party that knows the rule.** The gateway
   serialises `blocking` per finding and a report-level "this blocks
   publication", because it is the only participant that holds both the rule
   table and the pack in force. The console renders that field and computes
   nothing. The CLI prefers the served field and keeps its rank comparison as
   the fallback for a gateway *older* than itself, which is the only skew
   direction that survives — **the console is the one client that can never be
   out of step with its gateway, because the gateway ships it.** Force 2's
   sharpest case dissolves rather than getting a second implementation: the
   unknown-severity rule exists because the CLI has to guess at a vocabulary
   the gateway owns, and the console never has to guess.

6. **`QualityShortfall` starts carrying its sentence, amending ADR-0053.**
   That ADR had the gateway serialise a shortfall's data and not its prose,
   and the CLI reconstruct the sentence, on the reasoning that a `kind` slug
   makes a reader look the meaning up. The reasoning was right and the
   conclusion was right *for one client*. With two renderers it is a drift
   source: the same shortfall would be explained in two languages by two
   authors, and nothing would ever fail when they diverged. The gateway now
   serves the data **and** the rendered sentence; both surfaces display the
   served one; the CLI's reconstruction is deleted and its tests move to the
   gateway. This is an amendment to a decision, not a discovery of a bug —
   ADR-0053 decided correctly against the forces it had.

7. **Parity is a corpus, not a claim.** A set of recorded `ProposalDetail`
   payloads under `console/fixtures/` — the ADPT-1 pattern, where a driver
   replays recorded payloads against a mock and against the live gateway — is
   consumed by **both** the CLI's renderer tests and the console's. The AC
   test asserts that for every fixture the two surfaces name the same set of
   review-relevant facts: the same findings in the same order with the same
   blocking verdicts, the same two quality numbers unaveraged, the same
   shortfall sentences, the same member set with the same three contents, the
   same approvals, and the same set of actions offered. The corpus has to
   include the cases the CLI's own tests already earn — a blocking scan, a
   severity the client has never heard of, a checklist answered against an
   earlier draft and therefore not found — because a parity suite that only
   covers the happy path proves the two renderers agree about nothing
   difficult.

8. **npm gets the gate cargo-deny gives crates.** `scripts/check-npm-licences.mjs`
   runs in `make ci` beside `check-crate-deps.mjs`, over `pnpm licenses list
   --json`, against the same allowlist and the same discipline of narrow,
   annotated, per-package exceptions rather than a widened default. Runtime
   dependencies stay minimal and everything ships **in** the bundle: no CDN,
   no external font, no runtime fetch to a third party. An air-gapped install
   is a normal install, and a console that phones home for a stylesheet is a
   console that fails in the deployments this product is sold into.

9. **The console gets no endpoint the CLI does not have, and is not a
   different actor.** There is no console-only route; if the screen needs
   something the API cannot answer, the API gains it and the CLI gains it too.
   The audit vocabulary grows by nothing — a console approval chains
   `vedaflow.proposal.approved` under the subject's own identity,
   indistinguishable from the CLI's, which is the point: the reviewer is a
   person, not a surface. No new `AuditAction`, no envelope change, and
   CNSL-4's rule — *no direct-mutation path exists; everything is a proposal*
   — is adopted here rather than waited for.

## Options considered

**Session transport.**

1. **BFF session cookie (chosen)** — server-side token custody, opaque
   `HttpOnly` cookie, same verifier at the same seam. Pros: preserves ADR-0027
   decision 6 in its strongest form, survives a reload, the browser holds
   nothing worth stealing. Cons: introduces server-side session state and the
   first credential this product stores at rest in Postgres; makes a CSRF
   defence mandatory (decision 4).
2. **In-memory bearer in JS, no refresh** — the callback completes into the
   app and the SPA holds the access token in a variable. Pros: CSRF-immune,
   no new state, nothing at rest. Cons: an XSS reads the credential straight
   out of memory; the session dies on every reload, because the token that
   would fix that is the one a browser may not hold; and the access token's
   lifetime becomes the review session's lifetime, which is a hostile
   constraint on the one screen a reviewer sits in front of for an hour.
   Rejected on the reload, not the XSS: a hero screen that logs you out when
   you refresh it is not a hero screen.
3. **`localStorage`** — not considered beyond naming it. It is option 2 with
   the XSS window widened from a page load to forever.
4. **Reuse the CLI handoff** — the console reads the credential
   `synveda login` stored. Pros: zero new auth surface. Cons: the screen that
   exists because a terminal undersells the product would require a terminal
   to log in, and it only works for a console running on the operator's own
   machine, which is not where a console runs.

**Toolchain.**

1. **Vite + React SPA, served by the gateway (chosen)** — one runtime in the
   deployment, an ecosystem for the diff, tree and virtualised-list work
   CNSL-1 and CNSL-2 both need, and it reuses ADPT-1's recorded-fixture test
   pattern directly. Cost: opens the npm licence surface, closed by decision 8.
2. **Rust-rendered templates + htmx** — genuinely tempting on this codebase's
   own terms: single binary, single toolchain, cargo-deny already covering
   100% of it, no bundle and no licence gap to close. Rejected on CNSL-2 and
   CNSL-4 rather than on CNSL-1, which it would serve adequately: the
   hierarchy and policy explorer is an interactive tree over scopes, packs,
   roles and active lapses, and the memory browser is a virtualised list with
   provenance detail. Choosing a toolchain for the first screen that the third
   screen has to abandon is a worse outcome than paying decision 8's cost now.
3. **SvelteKit (SSR)** — fastest first paint and the smallest client bundle,
   rejected on force 3. It puts a second runtime beside the gateway in every
   deployment, which OPS-2 then ships, configures, observes and secures, on
   top of a split OPS-1 already recorded as a real cost.

**Parity.**

1. **Shared fixture corpus, both renderers asserted against it (chosen).**
2. **Each surface tested independently** — what a normal project does, and it
   is exactly the arrangement in which two renderers drift while both test
   suites stay green. It cannot fail on the thing the AC is about.
3. **The gateway renders the review to a display model both clients consume
   verbatim** — the maximal version of decisions 5 and 6, and rejected as
   overreach: a terminal and a browser have genuinely different affordances
   (colour, width, a diff a mouse can scroll), and a gateway serialising
   layout would either constrain the console to what a terminal can do or ship
   two display models and call it one. The line drawn here is that the gateway
   owns **verdicts and sentences** — the things with one right answer — and
   each client owns its own layout.

## Consequences

- **Positive.** CNSL-1 ships against an API that already exists, so the
  feature is a shell, an auth transport and a renderer rather than a new
  governed surface. The PDP, the audit chain and the tenant seam are untouched
  by decision 9 and re-used by decision 2. Decisions 5 and 6 reduce the number
  of implementations of a review judgement from a prospective two back to one,
  which leaves the CLI *better* than CNSL-1 found it. CNSL-2, 3 and 4 inherit a
  toolchain, a session and a test pattern, and their cost drops accordingly.
  The air-gap rule in decision 8 is the kind of constraint that is nearly free
  when adopted at the start and expensive at the end.

- **Negative / accepted.** `console_sessions` is the first table in this
  product to store a live credential at rest, and it is stored recoverable
  because the gateway has to *use* the refresh token — a hash would defeat the
  purpose. That is an accepted exposure with a named successor: **TEN-4
  (per-tenant encryption keys) is where those columns get a key**, and until
  then the compensating controls are the hashed session id, the row's
  usefulness expiring with the IdP's own token lifetime, and the same database
  trust boundary AUD-1 already documents. The gateway gains session state,
  which it did not have; it lives in Postgres rather than in memory precisely
  so that OPS-2's multi-replica chart does not turn a rolling restart into a
  mass logout. And npm's transitive tree is now a supply-chain surface in a
  product that sells trustworthiness — decision 8 gates the licences, and it
  does not pretend to gate anything else.

- **Reversal triggers.** If a second client of `blocking` or of the shortfall
  sentence ever needs a *different* answer from the gateway's, decisions 5 and
  6 are wrong and the verdict belongs back in the clients. If the parity
  corpus starts being edited to match a renderer instead of a renderer being
  fixed to match the corpus, decision 7 has become ceremony and should be
  replaced with a generated display model (option 3). If the console's runtime
  dependency count passes the point where decision 8's exception list stops
  being readable in one sitting, the htmx option is worth re-costing against
  whatever CNSL-2 and CNSL-4 actually turned out to need. And if `Origin`
  checking proves insufficient — a browser or a proxy that omits it on a
  request we must accept — decision 4 escalates to a double-submit token
  rather than being relaxed.

## Compliance notes

- **PDP.** No bypass and no new decision point. A cookie-authenticated request
  reaches `state.verifier.verify()` and `active_tenant` by the same call
  sequence a bearer does, and every governed action behind it takes the same
  PDP decision under the same effective pack. Seed §2.2 is satisfied
  structurally rather than by convention: there is no code path from a
  `console_sessions` row to a `Claims` value that does not pass through token
  verification.

- **Multi-tenancy.** `console_sessions` is read *before* the tenant scope
  exists — it is one of the inputs that establishes it — so it sits with
  `tenants` on the pre-scope side of RLS rather than under a tenant predicate
  that would return zero rows at the moment it is needed. Lookup is only ever
  by the hash of a full session secret, never by tenant or subject, so the
  table is not a listing surface; the row carries `tenant_id` and the tenant
  it resolves to is then subject to TEN-1's active-tenant rule and the uniform
  401 exactly as a bearer's is. The trade-off is the one `tenants` already
  makes and is recorded here rather than rediscovered.

- **Audit.** Unchanged vocabulary and unchanged envelope (decision 9).
  Successful logins remain non-events on ADR-0019 decision 6's reasoning —
  every subsequent chained event proves resolution — and that reasoning holds
  identically for a session cookie, since the session grants nothing a token
  did not. Governed acts performed from the console chain under the subject's
  own identity and are indistinguishable from the CLI's, which is deliberate:
  the audit answers *who approved this*, and the answer is a person.

- **Credential handling.** The refresh token's blast radius shrinks relative
  to today's CLI, which writes it to disk on every developer laptop that logs
  in; the console's never leaves the server. Decision 4's `Origin` requirement
  is a security control on a governed mutation path and is tested as one, not
  assumed from `SameSite`.
