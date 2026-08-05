# ADR-0057: the surface follows the harness — hooks own the write where a harness has them and a tool owns it where it does not, the protocol era changed under us so the SDK trigger fires, and the corpus records which surface asserted the fact

- **Status**: Accepted, **amended 2026-08-05** (decisions 1, 2 and 4 — see
  the amendment below; decisions 3 and 5–11 stand unchanged)
- **Date**: 2026-08-05
- **Feature(s)**: ADPT-2 (ADPT-3, ADPT-4 inherit the standalone server's shape)
- **Deciders**: sujitn

## Amendment (2026-08-05): the TypeScript SDK cannot serve the era decision 3 requires

Decision 2 took the official MCP TypeScript SDK on the reasoning that
ADR-0042 option 8's reversal trigger had fired and "the reason to hand-write
is gone at exactly the moment the surface to hand-write got bigger". That
sentence assumed the SDK covers the bigger surface. **It was written without
checking, and checking it before writing any code found it false.**

Measured, at the versions current on the day this ADR was accepted:

| | spec `2026-07-28` | `@modelcontextprotocol/sdk@1.30.0` | `rmcp` 3.1.0 |
|---|---|---|---|
| newest version implemented | — | `LATEST_PROTOCOL_VERSION = '2025-11-25'` | `V_2026_07_28`, in `SUPPORTED` |
| `server/discover` | **MUST** (schema.ts) | *absent from `dist/esm/`* | `DiscoverRequestMethod` |
| `-32022` | `UNSUPPORTED_PROTOCOL_VERSION` | *absent* | `UNSUPPORTED_PROTOCOL_VERSION` |
| `_meta` per-request version | required | *absent* | handled in the tower layer |

1.30.0 is the latest published version — `latest` is its only dist-tag, there
is no prerelease — and it shipped **2026-07-27, one day before** the revision.
It has not fallen behind through neglect; the revision is simply newer than
it. `SUPPORTED_PROTOCOL_VERSIONS` tops out at `2025-11-25`.

The consequence is that decisions 2 and 3 do not compose. Taking the TS SDK
delivers a **legacy-era server** — which is exactly the thing decision 4
deletes `mcp.mts` for being — and the only way to keep decision 3 on top of
it is to hand-write `server/discover`, per-request `_meta` selection and
`-32022` against an SDK that does not model them, which is more
hand-written protocol than ADR-0042 wrote and rejected. Meanwhile option 1's
rejection turned substantially on "option 9's strongest argument — avoiding a
dependency-heavy SDK — evaporates once decision 2 takes an SDK anyway", and
that premise is now false in the direction that decides it: **one of the two
SDKs implements the current revision, and it is the Rust one.**

So option 1 is taken. It was already recorded here as "the live alternative",
and the trigger that fired is not the one this ADR predicted (a customer
blocked by the Node runtime) but a plainer one — the TypeScript SDK cannot do
the job decision 3 defines.

**What this changes:** decisions 1 and 2 are replaced, and decision 4's
mechanism changes with them. **What it does not change:** decision 3 is now
*deliverable as written* rather than aspirational, and decisions 5–11 — the
two tools, the `--writes` capability flag, the `remember` naming, the
`Assertion` kind, stdio-only, generated client config, and the recorded
protocol corpus — are all surface-level and survive the move intact.

**What was checked before committing to it**: `rmcp` 3.1.0 is Apache-2.0;
`cargo deny check` returns `advisories ok, bans ok, licenses ok, sources ok`
with **no new per-crate exception**; and with `default-features = false,
features = ["server", "macros", "transport-io"]` it pulls 14 direct
dependencies of which 9 are already in the workspace. The dependency-weight
objection that shaped ADR-0042 option 9 does not survive measurement either.

**The constraint that replaces the one we gave up.** Seed §7 places the
generic MCP server in the "Harness adapters (thin, stateless)" row, and that
row's defining property is the label on the arrow beneath it: *three
primitives only*. Shipping the server as a `synveda` subcommand keeps the
layer and gives up only the language, **but it puts an adapter inside a
binary that already links `synveda-store`, `synveda-identity`,
`synveda-policy` and `synveda-audit`** for its dev-bootstrap commands. So the
rule is stated rather than assumed: `synveda mcp` is a **gateway client**. It
reaches the product over `/v1` holding a bearer, exactly as `synveda login`
and the CLI's other served verbs do, and it must not call a core crate — not
for a shortcut, not in a test. This is the one property option 1 costs that
the TypeScript package would have enforced structurally, and it is now a
review obligation instead. Seed §7's diagram wants a footnote to match.

## Context

ADPT-2's text is "recall (+ policy-gated write) for any MCP client", and its
acceptance criterion is "works in Claude Desktop + one non-Anthropic client".
The phase demo goal names the second: Cursor.

CTX-5 (ADR-0042 decision 15) already shipped an MCP server — one `recall`
tool, newline-delimited JSON-RPC 2.0 over stdio, hand-written rather than
taken as an SDK dependency, living in `adapters/claude-code` and registered
through the `mcpServers` manifest slot ADR-0027 decision 1 reserved for
exactly this pair of features. It is proven frame by frame by `mcp.test.mts`
and end to end by `demos/ctx-5-recall.sh`.

Three things are missing, and none of them is the one this ADR's first draft
assumed.

### The protocol era changed, and the shipped server is on the wrong side of it

`mcp.mts` pins `PROTOCOL_VERSION = "2025-06-18"`, and `mcp.test.mts` asserts
that exact string back. The current revision is **2026-07-28**, two revisions
on (`2025-11-25` intervened), and the newer one is not an additive bump:

- **There is no negotiation handshake.** `initialize` belongs to what the
  spec now calls the *legacy* era. A modern request declares its version
  per-request in `_meta` under `io.modelcontextprotocol/protocolVersion`, and
  the server accepts or rejects **each request independently**, statelessly.
- **`server/discover` is mandatory** — "Servers **MUST** implement
  `server/discover`" — returning supported versions, capabilities and
  identity in one call.
- A version the server does not implement **MUST** be answered with
  `UnsupportedProtocolVersionError` (`-32022`) carrying the `supported` list,
  so the client can retry.
- A server wanting to serve both eras is **dual-era**: it selects behaviour
  from how the client opens — modern `_meta` served statelessly, an
  `initialize` request served under legacy semantics.

So the surface a compliant server owes is materially larger than the four
methods CTX-5 implemented, and the shipped tool is a legacy-era server
pinned two revisions back. Its test passes *because* it asserts the stale
value.

ADR-0042 option 8 rejected the MCP SDK to protect ADR-0027 decision 1's
no-bundler, no-install-step constraint, and recorded the reversal trigger in
plain words: **"protocol revisions churn, or a second transport."** The
trigger has fired, twice, and the second firing changed the initialization
model. This ADR is not overturning that decision; it is the condition that
decision wrote being met.

### A generic client has no hooks — but some clients do, and that is the rule

ADPT-1's harness calls `inject` at `SessionStart` and `observe` at `Stop`
(ADR-0027 decision 2). That is what makes them hooks: a program decides when
they run and the model has no say. CTX-5 exposed only `recall` on that
reasoning — "the only primitive an agent should *choose* to call is the deep
one."

The correct generalisation is not "generic clients have no hooks, so
everything becomes a tool". It is **the surface follows who drives the
loop**, and there are three drivers rather than two. A harness with hooks we
can tap — Claude Code today, Codex and that shape — should keep the write on
the hook, because a hook observes what actually happened, runs whether or not
the model thinks to call it, and cannot be shaped by the model for the
recorder. A *framework* that calls us through a memory interface — ADPT-4's
LangGraph shim, and the LlamaIndex or Semantic Kernel equivalents if they get
scheduled — is the same case wearing different clothes: the program writes on
its own transitions, deterministically, and the model is not consulted. Only
the third driver is new. A client that exposes no seam at all — Claude
Desktop, Cursor — has exactly one extension point, tools the model chooses to
call, and there the write is either a tool or it is absent. Absent fails the
feature's own text.

Seed §2 principle 6 is what makes this cheap: "the harness is a guest …
supporting a new harness must never require touching the core". Every one of
these drivers consumes the same three primitives over the same two endpoints,
so the only thing that varies between them is which tools this server
advertises — and that has to be expressed as a property of the host rather
than as a list of hosts.

**This is the correction that matters, because getting it wrong writes the
corpus twice.** ADR-0027 decision 1 puts the MCP server inside the same
plugin whose `Stop` hook already POSTs the turn to `/v1/observe`. A server
that advertises a write tool unconditionally would, in Claude Code, let the
model store a fact by tool call while the hook independently observes the
transcript containing it — two rows in the same home scope, different
payloads, different idempotency keys, so ADR-0020 decision 2's buffer-level
idempotency cannot see the duplication. The write tool therefore has to be
advertised by harness capability, not by vendor and not unconditionally.

### What the write actually risks, which is less than it sounds

`POST /v1/observe` takes **no scope parameter**. The write lands at the
caller's own home scope and only there — placement decides (ADR-0020 decision
4) — gated by `MemoryWrite`, the role-free own-home floor every placed
principal holds (seed §2.1). A model calling this route cannot write into a
team, a department, or another person's memory, **because the request has
nowhere to say so**. MEM-2's redaction scan still runs between validation and
the staging insert (ADR-0021), the effective pack still picks admit /
quarantine / deny per event, and nothing reaches a shared scope without the
promotion pipeline and a human verdict. The blast radius is bounded by the
route rather than by the adapter's restraint, which is the only reason this
ADR can say yes to a model-callable write at all.

What it does risk is **epistemic**: a hook records what happened, a tool call
is the model asserting a fact it composed. `ObserveKind` today is
`transcript_delta | tool_result | decision`, and `decision` is documented as
"a decision the agent or user made" — which describes the content well and
says nothing about who put it on the wire. That distinction has never needed
to exist because there was only ever one answer.

### Where the server belongs

Seed §7's architecture diagram lists `generic MCP server` in the **Harness
adapters (thin, stateless)** row, beside `claude-code adapter (TS: hooks +
MCP)`. FND-1 scaffolded `adapters/mcp-server/` — a TS package whose
description is ADPT-2's text verbatim. ADR-0027 decision 1 states the
expectation directly: "the MCP server that joins this package next is a TS
ecosystem, and the credential logic that would have justified Rust is
delegated in decision 4."

ADR-0042 option 9 pointed the other way — `synveda mcp` in the Rust CLI,
which "would serve ADPT-2's standalone case too". Its stated reasons were the
CLI's ownership of the credential and the cost of a second protocol
implementation. Both are real. But option 9's force came substantially from
the constraint that an SDK was off the table; once the SDK trigger fires that
constraint is gone, and three recorded positions point at TypeScript against
one pointing at Rust.

> **Superseded by the amendment above.** Counting recorded positions is not
> the same as checking whether either candidate can implement the protocol,
> and the count was doing the work here. The three positions were about where
> a TS package *belongs*; none of them was a claim that the TS SDK serves
> `2026-07-28`, and it does not.

## Decision

1. **[Amended 2026-08-05]** **The server ships as `synveda mcp`, a subcommand
   of the existing Rust CLI** — option 1 below, taken for the reason the
   amendment sets out. It stays in seed §7's adapters row by *behaviour*: a
   gateway client over `/v1`, three primitives only, no core-crate call. The
   config line both AC clients get is the absolute path to a binary they
   already have after `synveda init`, which is one fewer runtime than `npx`
   and removes the install step rather than relocating it.

   *This replaces:* shipping as `@synveda/mcp-server`, the TypeScript package
   FND-1 scaffolded, launched via `npx -y`. That scaffold is now an empty
   package with no feature behind it; it goes, and seed §7's file tree
   (`adapters/mcp-server/`) is stale until someone updates it.

2. **[Amended 2026-08-05]** **It takes `rmcp`, the official MCP Rust SDK.**
   ADR-0042 option 8's trigger has fired and an SDK is still the answer —
   only not the one this ADR first named, because the TypeScript one does
   not implement the revision decision 3 requires and the Rust one does.

   *This replaces:* taking the official MCP TypeScript SDK.

3. **Dual-era, modern-preferred.** The server implements `2026-07-28` —
   `server/discover`, per-request `_meta` version, `UnsupportedProtocol-
   VersionError` — and answers `initialize` under legacy semantics for
   clients that open that way. The AC's two clients decide which era is
   exercised, and the corpus records both rather than assuming.

4. **[Amended 2026-08-05]** **`adapters/claude-code`'s protocol loop is
   deleted; the plugin's `mcpServers` entry execs `synveda mcp`.** The
   reasoning is untouched — two implementations of one protocol is two
   places for the `ids` xor `query` rule to drift, which is CTX-5's own
   argument for one tool instead of three applied one level up, and the
   stale `2025-06-18` pin plus the test asserting it are a live defect
   rather than a cosmetic one. Only the exec target changes: the `synveda`
   binary rather than a TypeScript package.

   This makes the plugin depend on the CLI being installed, which
   ADR-0027 decision 4 already established — the plugin shells out to
   `synveda` for credentials, so the binary is a prerequisite the plugin
   has today. The `mcpServers` entry resolves it the same way that code
   does; **a plugin that cannot find the binary must fail with the message
   ADR-0027 decision 4 already writes for that case, not silently serve no
   tools.**

5. **Two tools, `recall` and `remember`.** `recall`'s schema is CTX-5's
   unchanged — `{query?, ids?, as_of?, valid_at?, limit?}`, one tool rather
   than one per shape — so a client that has read one has read both.

6. **`remember` is advertised by who owns the write, and the flag says that
   rather than naming harnesses.** `--writes tool` advertises both tools;
   `--writes host` advertises `recall` only, because something in the host
   already writes observations. Three kinds of host qualify and they are the
   same fact from the server's side:

   - **hook-driven** — the harness calls us on its own schedule (Claude
     Code's `Stop` today, Codex and anything with that shape);
   - **framework-driven** — the program calls us through a memory interface
     (ADPT-4's LangGraph shim, and the LlamaIndex / Semantic Kernel
     equivalents if they are scheduled), writing on graph or chain
     transitions;
   - **model-driven** — nothing else writes, so the tool must.

   The flag is capability-shaped on purpose. Seed §2 principle 6 is law:
   "supporting a new harness must never require touching the core", and a
   mode enum carrying vendor names is a vocabulary every new guest has to be
   added to. `--writes` is a statement the launcher makes about itself, and a
   harness nobody has heard of configures correctly without this ADR being
   reopened. Because `tools/list` is per-process the flag is a launch
   argument, which keeps ADR-0027 decision 1's property that this arrives as
   configuration rather than restructuring.

7. **The write tool is named `remember`, not `observe`.** `observe` names the
   primitive from the platform's side — a batch of session events into a
   staging buffer. The audience for a tool description is a model deciding
   whether to call it, and `observe` invites it to narrate the session, which
   is the hook's job and would flood a personal scope with material the
   pipeline must score and discard. The route, the ADR-0020 batch caps, the
   idempotency key and the per-event dispositions are untouched: this is the
   tool's name and description, not a second write path.

8. **A new `ObserveKind::Assertion`, carried through to recall time.** A fact
   the model composed and chose to store, distinct from `decision`, a
   decision the harness observed being made. Chosen over a batch-level
   `source` field because `kind` is already per-event, already on the wire,
   already what extraction switches on, and already stable across MEM-1/2/3;
   a parallel field on the same axis is a second thing to keep in sync and a
   second thing a caller can leave unset. If the distinction stops at the
   staging buffer it is telemetry, not provenance.

9. **stdio only.** ADR-0042 option 8's trigger names a second transport as an
   SDK reason, and taking the SDK does not oblige us to serve one: both AC
   clients launch a subprocess, an HTTP listener holding a live credential on
   a developer laptop is a new exposure, and the hosted story is ADPT-3's —
   versioned API, API keys for service identities — rather than something to
   improvise here.

10. **Client configuration is generated, not documented.** `synveda mcp
    install --client claude-desktop|cursor` writes the client's own config
    and says what it wrote, with a dry-run and a refusal to clobber. The AC
    is "works in", and "paste this JSON into that file" is exactly where two
    clients diverge into a support burden nobody can test.

11. **The AC is a recorded protocol corpus, per client and per era.** CNSL-1
    established the pattern and the reason: a criterion phrased "works in X"
    is unfalsifiable until what X actually exchanges is on disk and replayed.
    Each client's real frames — `server/discover` or `initialize`,
    `tools/list`, `tools/call` — are recorded and replayed against the
    server, so a protocol regression fails a test rather than a demo.

## Options considered

1. **`synveda mcp` in the Rust CLI (ADR-0042 option 9)** — **ACCEPTED on
   2026-08-05; see the amendment.** The original entry is kept verbatim
   below, because the shape of the mistake is the useful part: every clause
   of the rejection was about fit and cost, none of it about whether the
   chosen SDK could do the job, and *that* was the question that decided it.

   > the CLI already holds the credential, is already a shipped binary,
   > needs no Node, and the official Rust SDK (`rmcp`) implements 2026-07-28
   > with compatibility back through 2025-11-25, so dual-era would come
   > free. Rejected on architecture fit rather than capability: seed §7
   > places the generic MCP server in the adapters row, FND-1 scaffolded the
   > package, and ADR-0027 decision 1 says in terms that this server is a TS
   > ecosystem. Option 9's strongest argument — avoiding a dependency-heavy
   > SDK — evaporates once decision 2 takes an SDK anyway, and `npx` is the
   > ecosystem's install norm for the two clients the AC names. **Recorded
   > as the live alternative**: if the Node runtime becomes a real obstacle
   > for a customer, this is the move, and `rmcp` makes it cheaper than it
   > was when ADR-0042 wrote it down.

   Two clauses were checked when it was taken and both held: `rmcp` 3.1.0
   does carry `V_2026_07_28` in `SUPPORTED` alongside `V_2025_11_25` (its
   `LATEST` const still names the older one, so dual-era is a configuration
   rather than literally free), and the dependency weight is 5 crates the
   workspace did not already have.
2. **Keep the hand-written loop and just add the new methods** — no
   dependency, no bundler, and the existing code is proven. Rejected:
   `server/discover`, per-request `_meta` validation, `-32022` with a
   supported list, stateless request handling *and* a legacy compatibility
   path is a protocol implementation, not four methods, and it is the exact
   condition ADR-0042 option 8 said would reverse it.
3. **Ship both a TS and a Rust server** — Rejected on decision 4's reasoning.
   The cost is not the second build; it is the second place the tool schemas
   and the xor rule live.
4. **Advertise `remember` unconditionally and dedupe server-side** — simpler
   to configure, no launch mode. Rejected: the duplicate is semantic, not
   byte-identical — a model's composed assertion and the transcript slice
   containing its reasoning are different payloads — so there is nothing for
   ADR-0020 decision 2's idempotency to key on, and a fuzzy dedupe at the
   admission point is a new judgement in the one place the product promises
   determinism.
5. **Expose no write; ship `recall` only, standalone** — smaller, safer,
   defensible on CTX-5's stated argument. Rejected: it fails the feature text
   in the way that matters, leaving a Cursor user able to read a corpus and
   never contribute to it.
6. **Make the write `propose` into a shared scope** — a model proposes, a
   human approves in CNSL-1's inbox. Rejected: the promotion pipeline already
   does this from observed material, with quality scoring and evidence the
   model does not have; letting a model mint proposals directly puts unscored
   material in a human queue and makes the queue worse.
7. **A `forget` tool** — Rejected here, not on principle: retirement is a
   governed act with its own surface (CNSL-4's "manual pin/retire, as
   proposals"), and a second path to it from a model's tool call, invented
   before that surface exists, is how the direct-mutation path this product
   promises not to have gets built by accident.
8. **Reuse `ObserveKind::Decision` for model assertions** — no migration, no
   extraction change, ships sooner. Rejected: it is the one option that
   destroys information silently. Once assertions and observations share a
   kind, no later feature can separate them, and the corpus can never answer
   "did a person say this or did a model decide it" about anything written
   before someone noticed.
9. **Do nothing — point Cursor users at ADPT-3's REST API** — Rejected: it is
   a phase demo goal, MCP clients are the named audience, and ADPT-3 is six
   slots away.

## Consequences

- **Positive:** one protocol implementation instead of two, and it is
  current rather than two revisions stale; a server installed the way both AC
  clients already install servers; the write primitive reaches model-driven
  clients with its blast radius fixed by the route; hosts that already write
  keep the stronger path and cannot double-write; and the corpus gains a
  model-asserted / host-observed distinction it cannot recover later.
- **Reach this feature gets for free, and the obligation that comes with
  it.** LangGraph, LlamaIndex and Semantic Kernel can all consume an MCP
  server, so shipping this one gives them a working governed path with no
  adapter written for any of them — which is seed §2 principle 6 paying out.
  What ADPT-4's shims add later is the *other* surface: the framework's own
  memory interface, program-driven. **Decision 6 is what keeps the two from
  colliding** — a LangGraph app running the shim and this server must launch
  it `--writes host`, or the checkpointer and the model both write the same
  turn. That is a forward obligation on ADPT-4, recorded here because this is
  the ADR that creates the hazard. LlamaIndex and Semantic Kernel appear in
  no feature, epic or roadmap line today; scheduling them is a
  SYNVEDA_FEATURES.md decision, not this ADR's.
- **Negative / accepted trade-offs** *(revised 2026-08-05)*: the CLI binary
  grows an SDK and 5 crates not already in the workspace, and every `synveda`
  invocation — `init`, `login`, `policy apply` — carries them whether or not
  it serves MCP; **an adapter now lives in a binary that links the core
  crates, so "three primitives only" is a review obligation rather than a
  structural one** (decision 1); a new `ObserveKind` variant is a migration
  and touches MEM-3's extraction, which is not this feature's code; `synveda
  mcp install` writes another application's config file; deleting `mcp.mts`'s
  loop churns a CTX-5 AC test that currently asserts the stale protocol
  version; and stdio-only means the first hosted-agent user waits for ADPT-3.

  *No longer applicable, having been the TypeScript path's costs:* the
  bundler question, and `scripts/check-npm-licences.mjs` covering a real
  dependency tree. The npm surface stays three first-party packages.
- **Reversal triggers** *(revised 2026-08-05 — the first one has fired and is
  now the decision)*: the TypeScript SDK ships `2026-07-28` **and** a customer
  is blocked by requiring the `synveda` binary → the TS package becomes the
  thin alias in the other direction, on this ADR's original reasoning, which
  is preserved below rather than deleted; ADPT-3 lands API-key service
  identities → decision 9's HTTP
  transport becomes the hosted story and stdio stays local; a third client
  arrives with another config format → `install` grows a generic
  print-the-JSON mode rather than a branch per vendor; a harness we tap hooks
  for turns out to want the tool as well (a hook that cannot see what the
  model chose to keep) → decision 6's two modes become three rather than
  collapsing; model-asserted writes measurably degrade a personal corpus
  (extraction precision on `assertion` events below the `decision` baseline,
  EVAL-6) → a pack-level off switch on SKIL-4's precedent, not removal.

## Compliance notes

- **The PDP is not bypassed and gains no new path.** `remember` calls
  `/v1/observe` and takes its `MemoryWrite` decision exactly as ADPT-1's hook
  does; `recall` calls `/v1/recall`, whose per-`(scope, tier)` decisions
  ADR-0042 fixed. The server adds no route, no scope producer and no third
  way to reach the store: it is a client of the same two endpoints, holding
  the same bearer.
- **The audit actor is the person, not the surface.** A `remember` call
  chains `memory.observed` under the logged-in subject, one event per
  admitted batch in the ingest transaction (ADR-0019 decision 4), like every
  other observe. This is deliberate against decision 8: the *actor* is the
  person whose credential authorised the write, the *kind* records that a
  model composed it. Conflating them would answer "who is accountable" with
  "a tool call", which is not a party to anything.
- **Tenant isolation is untouched** — the bearer carries `tid`, placement
  puts the write at the caller's home scope, and no request field names a
  scope.
- **Redaction still runs before persistence**: a model that composes a secret
  into a `remember` call gets the same scan, the same quarantine or deny
  disposition, and the same guarantee that raw finding text survives in no
  table, response, metric or audit payload (ADR-0021).
- **Licence path** *(revised 2026-08-05)*: the SDK enters the **Rust** core
  path, so `cargo deny` gates it on the MIT/Apache-2.0/PostgreSQL rule
  directly. Measured before the decision was taken: `rmcp` 3.1.0 is
  Apache-2.0 and `cargo deny check` returns `advisories ok, bans ok,
  licenses ok, sources ok` with **no new per-crate exception added to
  `deny.toml`** — which matters, because every existing exception in that
  file is a reviewed diff with a written justification, and a new SDK that
  needed one would be a worse trade than it looks.
- **No test bypasses the PDP** (seed §2.2): the AC corpus drives the real
  server against the real gateway under a test policy pack, on ADR-0042's
  precedent for `demos/ctx-5-recall.sh`.
