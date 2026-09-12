# ADR-0106: Explicit task identity and public client interoperability

- **Status**: Accepted
- **Date**: 2026-09-12
- **Feature(s)**: ADPT-1, ADPT-2, ADPT-4, CPR-12, CPR-23, CPR-39
- **Deciders**: sujitn (implementation plan authorised 2026-09-12)

## Context

The interoperability audit at `7fa7d56` found a missing Skill-sync scope,
connection-dependent MCP Session identity, an incorrect observation
acknowledgement and no public Python/TypeScript SDKs. The existing public
Session, Context, Skill, Knowledge/proposal and audit APIs already implement
the product operations. ADR-0098 requires authentic evidence for new harnesses.

## Decision

Clients name the existing application Session independently of transport.
`synveda mcp --session <id>` binds a dedicated process; tool arguments may carry
`session_id` when a host multiplexes tasks. A bound process rejects a different
per-call Session. `--task <key>` instead opens a Session idempotently using a
bounded caller-supplied external identity, scoped by authenticated principal
and selected target. No task key is derived from a connection or process ID.
Calls without identity fail with guidance; protocol discovery remains usable.
Every existing Session is read through its authorised public endpoint before
use and checked against explicit workspace/project constraints. Disconnect
never ends an application Session. These rules amend ADR-0057's implicit
launch identity, not its stdio transport or host-versus-tool write ownership.

Claude's hook includes its resolved Session ID in context for MCP calls and
the launcher forwards project/workspace/profile settings. Skill sync resolves
the explicit project's scope, or the caller's principal scope when there is no
project, then calls the existing CLI. An unavailable explicit target never
falls back to another target. Observation acknowledgements promise only the
returned append disposition; Capture and Knowledge publication stay separate.

Python and TypeScript base clients derive wire types/operation metadata from
checked OpenAPI and use maintained HTTP tooling. Bearer acquisition is a
caller-provided boundary; it may use the existing CLI login or a maintained
OIDC library. Clients expose explicit pagination and request correlation,
bound response reads and retries, refuse redirects with credentials, and only
replay requests with the existing public idempotency contract. Gateway policy,
retrieval and orchestration never move into an SDK.

The public Compose edge deliberately removes caller trace context. The gateway
therefore returns its actual OpenTelemetry trace ID in `X-Synveda-Trace-Id` on
responses when tracing is available. SDK correlation uses that bounded opaque
ID, with the sent trace ID as a fallback for servers without the header. It is
diagnostic plumbing, never task identity, authority or proof of an audit event.
The existing audit API supplies the governed Session/artifact-to-trace link.
The proxy's request-header sanitisation remains unchanged.

Codex qualification starts with authentic versioned frames and only then adds
necessary host translation. A filesystem target or successful MCP handshake
does not establish lifecycle support. Registry promotion still requires every
applicable ADR-0098 criterion; absent credentials remain a blocker.
The captured 0.152.0 lifecycle emits `SessionEnd` at runtime exit and subsequently
resumes the same native Session ID. Its adapter therefore flushes on that hook;
only an explicit task-owner API call freezes Capture or ends the Synveda Session.
`Stop` records the bounded native transcript before network delivery. Only
captured message/tool shapes are translated; injected instructions and reasoning
are excluded. Oversized or mismatched transcripts are held with diagnostics.
Reuse the existing adapter's credential, Session and durable delivery functions
through a narrow workspace export and a closed Claude Code/Codex client identity.
Codex external IDs are namespaced so two harnesses cannot share a local spool.
The installed client caps `SessionEnd` at three seconds. After persisting local
events, Codex uses one two-second deadline for credential resolution and all
delivery requests, leaving time to save acknowledgements before host shutdown.
Each in-flight request consumes the remaining delivery budget; pending events
remain in the existing spool for resume or explicit flush.
Retry delivery must match the saved gateway and client, including background
backlogs. This adds no plugin registry, event store or orchestration layer.
Captured clients may have no Synveda configuration writer. Such entries remain
visible in the support matrix but are excluded from generated installation
choices; protocol evidence never justifies writing a host's unsupported format.

## Options considered

1. Explicit application Session/task references over existing APIs — selected;
   bounded state and a common identity across MCP and language applications.
2. Infer identity from MCP connections, cwd or the last active hook — rejected;
   restart changes identity and concurrent conversations can be mixed.
3. Add a task orchestrator or external MCP proxy — rejected; neither repairs
   the demonstrated client boundary defects.
4. Leave the current callers unchanged — rejected; Skill sync demonstrably
   fails and the observation acknowledgement overstates the effect.

## Consequences

- Positive: client transports share one governed application identity and
  immutable artifact contract; existing enforcement remains authoritative.
- Accepted trade-off: a tool-only host must supply a task/Session reference;
  a host context instruction helps the model select it but grants no access.
- Reversal trigger: authentic client evidence that cannot carry per-call
  identity requires a separately justified integration decision, not heuristic
  selection of the most recent conversation.

## Compliance notes

No SQL/schema, Cedar, RLS, audit-chain or VedaFlow bypass is introduced.
Tenant/principal and operation authority come from the gateway's bearer and
PDP. Local scope/task selectors carry identity only. Existing `rmcp` owns the
wire protocol. New Rust code uses explicit, bounded control flow and checked
errors under repository lints; no new unsafe code or panic-based boundaries.
