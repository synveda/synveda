# ADR-0027: Claude Code adapter — hook seams, the CLI as credential authority, cursor-and-idempotency observe

- **Status**: Accepted
- **Date**: 2026-07-24
- **Feature(s)**: ADPT-1
- **Deciders**: sujitn

## Context

ADPT-1 is the harness half of the Phase-1 spine: "TS plugin:
SessionStart/PreCompact/Stop hooks → inject/observe; MCP recall tool;
skills materialisation (SKIL-4); zero-config after `synveda login`. AC:
fresh machine to personalised session in <2 minutes; demo script." The
Phase-1 sequencing qualifies it as **ADPT-1 (minimal)**, and the
qualification is load-bearing: `recall` is a distinct primitive whose
API and MCP tool are CTX-5 (Phase 2), and skills materialisation is
SKIL-4 (Phase 3). What remains, and what this ADR decides, is the
adapter over the two primitives that exist — `/v1/inject` (CTX-3,
ADR-0026) and `/v1/observe` (MEM-1, ADR-0020) — plus the login that
makes them zero-config.

Forces at play:

- **The server side is finished and opinionated.** Inject returns a
  watermarked block and degrades rather than fails (ADR-0026 decisions
  1 and 4); observe acks in <20ms, is idempotent per client-minted key,
  and redacts at admission (ADR-0020 decision 2, ADR-0021). The adapter
  adds no policy, no retrieval, and no redaction — it is transport
  (seed §2.4, §8: adapters depend only on the public API).
- **The hook contract does not match the feature's shorthand.** Of the
  three events ADPT-1 names, only `SessionStart` can add context:
  its `hookSpecificOutput.additionalContext` (and, for this event,
  bare stdout) is injected for the model. `PreCompact` has no
  context-injection output at all, and its only decision control is
  exit code 2 — which *blocks compaction*. `Stop`'s stdout is not
  context either. So "PreCompact → inject" is not implementable;
  `SessionStart` fires again after compaction with `source: "compact"`,
  and that is the documented re-injection seam.
- **`SessionStart` fires before any user prompt.** Cold session start
  has no task, so the first inject is the taskless, recency-ordered
  branch (ADR-0025 decision 5's else-branch; ADR-0026 decision 3 calls
  this by design, not degradation). A resumed or compacted session, by
  contrast, has a transcript — and therefore a task.
- **A memory system must never break the session it serves.** A
  non-zero hook exit is a non-blocking error for these events, but exit
  2 blocks (compaction, in PreCompact's case), and the default command-hook
  timeout is 600 seconds — long enough that a hung gateway would look
  like a hung harness. The client edge needs the same posture ADR-0026
  gave the server: degrade, never fail.
- **`PreCompact`'s payload carries no `transcript_path`.** `Stop`,
  `SessionStart`, and `SessionEnd` do. Any design that wants to flush
  transcript content at compaction must already hold a cursor.

  *Corrected 2026-07-25 (step 3): it does carry one. Recording the
  fixtures decision 14 asks for put the payload shapes under a microscope,
  and all four events are built from one common envelope — `session_id`,
  `transcript_path`, `cwd`, and what the harness has of `prompt_id`,
  `permission_mode`, `agent_id`, `agent_type`, `effort` — verified against
  Claude Code 2.1.220. The design the false premise led to is unchanged
  and still right: the payload is another program's internal format
  (decision 9), so the adapter reads the path when it is there and falls
  back to the spooled one when it is not, and the spool has to exist for
  the cursor regardless. What the correction costs is only the argument
  that `PreCompact` forced it.*
- **The transcript is an internal format.** Session JSONL entries of
  type `user`/`assistant` carry `uuid`, `timestamp`, `sessionId`,
  `message`, `isMeta`, `isSidechain` — verified against a live
  transcript — but none of it is a published contract.
- **Zero-config is the AC.** Two minutes from a fresh machine to a
  personalised session leaves no room for issuer URLs, client ids,
  pasted tokens, or an `npm install`. And the credential path is the
  one place a thin adapter cannot stay thin by itself: OIDC access
  tokens expire in minutes, so "log in once" implies refresh.
- **AUTH-1's login is the only path that provisions.** `/auth/callback`
  runs code exchange, JWKS verification, TEN-1's active-tenant rule, and
  AUTH-2 JIT provisioning. A subject that never completed a login is
  quarantined at the PDP seam (ADR-0013). Any login design that talks to
  the IdP directly gets a valid token for an unplaced identity.

## Decision

`adapters/claude-code` becomes a **Claude Code plugin** whose hooks call
the two `/v1` primitives with a bearer resolved by the `synveda` CLI,
and the CLI gains `login` — a gateway-mediated loopback flow that reuses
AUTH-1 end to end. Nothing in the adapter decides anything; it maps hook
events to primitives and never lets a failure reach the user's session.

Decisions, specifically:

1. **Shape: a plugin, TypeScript, prebuilt, dependency-free.**
   `.claude-plugin/plugin.json` plus `hooks/hooks.json` referencing one
   entry point, `${CLAUDE_PLUGIN_ROOT}/dist/hook.mjs <mode>`, dispatched
   by the payload's own `hook_event_name` with the mode argument as the
   cross-check — so `hooks.json` reads as documentation and there is
   still exactly one artifact. Node ≥22 stdlib only —
   global `fetch`, no runtime dependencies, no install step when the
   plugin is enabled (the AC's two minutes cannot include an `npm
   install`). The manifest reserves `mcpServers` for CTX-5/ADPT-2 and
   `skills/` for SKIL-4, so both arrive as configuration rather than
   restructuring. TypeScript stays per seed §7 — the MCP server that
   joins this package next is a TS ecosystem, and the credential logic
   that would have justified Rust is delegated in decision 4.
2. **The seams, as the hook contract actually allows.**
   `SessionStart` (all sources: `startup`, `resume`, `clear`, `compact`,
   `fork`) → `POST /v1/inject`, returning the block as
   `additionalContext`. `Stop` → `POST /v1/observe` for the turn's
   transcript delta, `async: true` so the write path never sits in the
   user's turn. `PreCompact` and `SessionEnd` → flush only: they resend
   whatever a previous flush left behind (decision 7) and inject
   nothing, because they cannot. Post-compaction re-injection — the
   behaviour "PreCompact → inject" was reaching for — is
   `SessionStart` with `source: "compact"`.
3. **Every hook exits 0, always.** The adapter never exits 2 and never
   emits a `decision: "block"`. A dead gateway, an expired login, a
   malformed transcript, or a timeout produces a hook that contributes
   no context and returns success. `hooks.json` sets an explicit
   `timeout: 5` (against the 600s default), and the adapter applies its
   own 3s deadline per call — two decimal orders above the 150ms inject
   SLO, sized to absorb a cold cache rather than to hide a broken
   dependency. `systemMessage` is used only when the user must act
   ("run `synveda login`"); a *degraded* inject
   (`X-Synveda-Degraded`) still delivers context and stays silent —
   it is already recorded server-side in the audit event and metrics.
4. **The CLI is the sole credential authority; the adapter holds no
   OAuth.** Two new subcommands: `synveda login` (decision 5) and
   `synveda auth token --json`, which returns a currently-valid bearer,
   refreshing if needed. The hooks shell out to it and hold no
   OAuth code, no client configuration, and no refresh logic. One
   implementation of PKCE, expiry, and refresh — in Rust, next to the
   `synveda-identity` code that already does this — instead of a second,
   drifting one in TypeScript. Cost: one process spawn per hook,
   against a network call two orders of magnitude larger.
5. **`synveda login` is a gateway-mediated loopback flow.** The CLI
   binds a listener on `127.0.0.1:<ephemeral>` and opens the browser at
   `GET /auth/login?issuer=…&cli_redirect_uri=http://127.0.0.1:<port>/callback`.
   The gateway parks the CLI's return URI on the existing `PendingLogin`
   and runs AUTH-1 unchanged — PKCE, JWKS verification, active-tenant
   rule, AUTH-2 provisioning. Where `/auth/callback` today returns the
   session JSON, a CLI-initiated login instead 302s to the loopback with
   a **one-time, 60-second, state-bound handoff code**, which the CLI
   redeems at `POST /auth/cli/exchange` for the session material. The
   `cli_redirect_uri` allowlist is absolute: scheme `http`, host literal
   `127.0.0.1` or `[::1]` (never the `localhost` name), any port, fixed
   path. No other target is ever accepted. Tokens never travel in a URL
   or a browser history; only a single-use code does.
6. **Refresh runs through the gateway; the CLI holds no client
   credentials.** The login flow requests `offline_access` where the
   issuer advertises it; the refresh token is returned on the handoff
   exchange only, never in the browser-facing response. `POST
   /auth/refresh` mints a new access token — the gateway remains the
   OAuth client, which is what lets the CLI stay configuration-free.
   Credentials live at `$XDG_CONFIG_HOME/synveda/credentials.json`
   (mode 0600), keyed by profile: gateway URL, tenant, subject, access
   token, expiry, refresh token. An issuer that will not issue refresh
   tokens degrades to decision 3's `systemMessage`.
7. **Observe is at-least-once over a durable cursor; MEM-1's
   idempotency makes it exact.** Per session, the adapter keeps a cursor
   (the last observed transcript entry `uuid`) in a spool directory.
   Each `Stop` reads the transcript past the cursor, posts one batch,
   and advances the cursor **only on a 2xx**. A failed or interrupted
   flush leaves the cursor where it was; the next `Stop`, `PreCompact`,
   or `SessionEnd` resends, and the buffer reports the overlap as
   duplicates without re-enqueuing anything (ADR-0020 decision 2). No
   daemon, no local queue, no delivery state to get wrong — the
   server-side idempotency the write path already guarantees is exactly
   the property that makes the naive client correct.
8. **The event mapping, bounded by the observe contract.**
   `user`/`assistant` entries map to `transcript_delta`; entries whose
   content is a tool result map to `tool_result`; `decision` is unused
   in ADPT-1 (it belongs with FLOW/PRMT signals). `idempotency_key` is
   the entry's `uuid` — per-session unique and stable across retries,
   precisely the client-minted key ADR-0020 decision 2 asks for.
   `occurred_at` is the entry `timestamp`. Batches chunk at 256 events;
   a payload over the 64 KiB cap is truncated with an explicit
   `truncated: true` marker rather than dropped, because extraction
   (MEM-3) wants the gist and silence would be a lie. `isMeta` entries
   are skipped, and so are sidechain (subagent) entries in ADPT-1 —
   whose scope a subagent's work belongs to is a real question, not an
   oversight. Each payload carries a small envelope: harness name and
   version, `cwd`, git branch, model.
9. **The transcript parser is defensive by construction.** It reads
   `type`, `uuid`, `timestamp`, `message`, `isMeta`, `isSidechain` and
   treats every other field as opaque; a line it cannot parse is skipped,
   never fatal to the flush. This is an internal format being read by an
   outside process, and the adapter's job is to keep working across
   harness releases, not to validate them.
10. **Session identity is the audit correlation.** Both primitives
    receive `session_id = "claude-code:<harness session id>"` — opaque,
    content-free, inside the 200-character cap. That single string is
    what joins a session's `context.injected` event to its
    `memory.observed` events in the AUD-1 chain, and it is what makes
    Phase 1's "fully audited" demonstrable rather than asserted.
11. **Task and budget.** A cold `startup` injects tasklessly (seed §3's
    session-start contract). `resume`, `compact`, and `fork` derive the
    task from the last user prompt in the transcript, capped at inject's
    4096 characters — post-compaction is exactly where relevance is
    worth the embed round-trip. `budget_tokens` is sent only when
    configured (`budget_tokens`, and a smaller `compact_budget_tokens`),
    which honours ADR-0026 decision 7's narrowing without inventing a
    remaining-room signal the hook payload does not carry.
12. **No client-side redaction.** MEM-2 is the single, pack-governed
    redaction implementation and it runs before anything persists
    (ADR-0021). A TypeScript re-implementation would drift from the
    rules the policy pack names and would offer false assurance
    precisely where assurance is the product.
13. **Capture is disclosed and locally revocable.** `SYNVEDA_DISABLED=1`
    and a `disabled` key in an optional per-project `.synveda/config.json`
    turn the adapter off; the first successful session in a project
    prints one `systemMessage` naming what is sent and to which gateway.
    Login is consent, but silent capture is not something this product
    gets to do.
14. **The AC is a timed script, and the hooks are tested against
    recorded events.** `demos/adpt-1-claude-code.sh` runs the AC
    literally: a clean HOME, plugin install, `synveda login`, a session
    that receives its watermarked block, a turn that is observed, and
    `synveda audit tail` showing `context.injected` and `memory.observed`
    under one session id with the chain verifying — timed, with the
    two-minute budget asserted. Alongside it, a harness-free driver
    feeds recorded hook JSON to each hook entry point against mock and
    live gateways: gateway down, 401, degraded header, oversized
    payload, duplicate replay, cursor resume after a failed flush,
    unparseable transcript line. Every one of them must exit 0.

## Options considered

1. **`type: "http"` hooks pointing straight at the gateway** — no Node,
   no adapter process, nothing to install. Rejected: the gateway would
   have to answer in Claude Code's hook JSON dialect, which is exactly
   the harness knowledge seed §2.4 keeps out of the core; the bearer
   would live in a long-lived environment variable (`allowedEnvVars`)
   with no refresh; and with no cursor or spool, observe becomes
   fire-and-forget. Worth revisiting for a read-only kiosk deployment.
2. **Ship the hooks as `synveda hook <event>` in the Rust binary** —
   one artifact, no Node runtime, millisecond startup, credentials
   already in-process. Genuinely tempting, and rejected on the narrow
   grounds that seed §7 fixes TypeScript for the harness adapter and
   ADPT-2/CTX-5's MCP server lands in this same package: a Rust hook
   would split one adapter across two languages to save a process
   spawn. Reversal trigger: measured Node startup dominating session
   start.
3. **The CLI as its own OAuth public client** (RFC 8252 loopback +
   PKCE, no gateway involvement) — the spec-canonical native-app
   answer. Rejected: JIT provisioning and tenant resolution live in
   `/auth/callback`, so a directly-obtained token belongs to an
   unprovisioned subject that the PDP quarantines; and the CLI would
   need per-issuer client configuration, which is the opposite of the
   AC.
4. **Paste the token from the browser** (`synveda login --token …`
   against today's `/auth/callback` JSON) — zero new gateway surface,
   ships in an afternoon. Rejected: no refresh, fails the AC's spirit,
   and it trains users to move bearer tokens by hand.
5. **A background daemon spooling observe batches** — better tail
   latency, survives a crash mid-flush. Rejected as machinery the
   idempotent buffer already makes unnecessary (decision 7). Trigger:
   transcripts large enough that per-turn reads bind.
6. **Inject at `UserPromptSubmit` with the prompt as the task** — the
   most task-relevant context this harness can produce, and
   `additionalContext` supports it. Rejected for ADPT-1: an inject per
   prompt multiplies latency, token cost, and audit volume, and the
   designed answer to per-prompt relevance is CTX-4's tiered
   index/body split, not more full injects.
7. **Send the whole transcript every turn** and let idempotency dedupe
   — simplest possible client. Rejected: it re-uploads the session on
   every turn against the 256-event/64 KiB caps and pays MEM-2 scan cost
   repeatedly for content already admitted.
8. **Pre-redact in the adapter before sending** — rejected per decision
   12.

## Consequences

- Positive: Phase 1's demo becomes a real workflow — SSO login, then a
  live session that receives governed context and contributes governed
  memory, joined in one audit chain by session id. Every credential
  path is Rust and tested once. Observe is effectively exactly-once
  with no moving parts. The adapter cannot break a session by
  construction, and cannot see anything the caller's own bearer cannot.
- Negative / accepted trade-offs: `Stop` races the transcript writer —
  measured against a live session, the turn's assistant message is not
  yet on disk when the hook fires, so it rides the *next* flush (the
  following turn's `Stop`, or `SessionEnd`). Nothing is lost, because
  the cursor only advances on acceptance, but a session's final
  assistant message depends on the `SessionEnd` flush landing, which is
  one more reason that hook is registered. The adapter also reads an
  undocumented transcript format (defensive parsing, plus a reversal
  trigger); a Node process starts per hook and shells to the CLI for its
  bearer; session
  content crosses TLS before MEM-2 redacts it, so the guarantee is
  "never persisted unredacted", not "never transmitted"; cold session
  starts are recency-ordered because no task exists yet; subagent work
  goes unobserved in ADPT-1; and the gateway grows three small auth
  surfaces (`cli_redirect_uri`, `/auth/cli/exchange`, `/auth/refresh`)
  that exist only to keep the client configuration-free.
- Reversal triggers: the harness publishes a supported transcript or
  context API → drop the JSONL parser (decision 9); Node startup
  measured to dominate session start → option 2; CTX-4's tiering lands
  → revisit option 6; CTX-5/ADPT-2 land → the MCP recall tool joins this
  plugin's manifest as `mcpServers`, no restructuring; SKIL-4 lands →
  `skills/` in the same manifest.

## Compliance notes

- The PDP stays unbypassable: the adapter holds no privileged path and
  no service identity of its own. It calls the same two `/v1` routes any
  client calls, with the human caller's bearer, and inherits their plan
  (seed §2.2). A quarantined or unplaced user gets the empty block —
  the adapter cannot tell, and must not try (ADR-0026 decision 1).
- Tenancy: the tenant comes from the token. The adapter never names one,
  never sends a scope, and observe lands at the caller's home scope by
  placement (ADR-0020 decision 4).
- Audit: no new action types. `context.injected` and `memory.observed`
  already chain server-side; ADPT-1's contribution is the correlating
  `session_id` of decision 10 (DoD #4 is satisfied by the existing
  emissions).
- Secrets: the credentials file is 0600 and never enters
  `settings.json`, the environment, or the transcript. The adapter
  writes diagnostics to `$XDG_STATE_HOME/synveda/adapter.log` and
  **never to stdout** — for `SessionStart`, stdout is model-visible
  context, so a stray debug line would become context and, worse, a
  stray token would become context.
- Redaction: decision 12 — one implementation, server-side, at
  admission.
- Observability (DoD #3): every call carries `X-Synveda-Client:
  claude-code/<version>` and a W3C `traceparent`; ADPT-1 wires the
  extractor into the FND-5 trace layer so a slow session start is one
  trace from hook through plan, embed, search, compose. The adapter's
  own timings go to its log, not to the user's terminal.
