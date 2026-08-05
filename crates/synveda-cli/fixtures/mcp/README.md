# The MCP protocol corpus (ADPT-2, ADR-0057 decision 11)

Frames exchanged with `synveda mcp`, on disk and replayed by
`crates/synveda-cli/tests/mcp_corpus.rs`.

ADPT-2's acceptance criterion is *works in Claude Desktop + one non-Anthropic
client*, and CNSL-1 established both the pattern and the reason: a criterion
phrased "works in X" is unfalsifiable until what X exchanges is on disk and
replayed. These files are that turned into something a test can fail.

```sh
cargo test -p synveda-cli --test mcp_corpus                    # verify
SYNVEDA_RECORD_MCP=1 cargo test -p synveda-cli --test mcp_corpus  # re-record
```

## Where these frames come from

Both halves of a case are real for the two clients the AC names:

| | where it comes from |
| --- | --- |
| `expect` — the answers | **Recorded** from the shipped binary, over a real pipe, by the test |
| `send` — the questions | **Recorded** from Claude Desktop 1.25927.0 and Zed 1.13.2 on 2026-08-05, via `capture.sh` |

Every case declares this in its own `provenance` block, and the suite fails
if a case is missing one *or if either AC client stops being represented by a
real recording*. Two cases remain `authored` and say why below.

### What recording actually found

The authored cases these replaced were wrong in ways nothing else here would
have caught:

| | authored | recorded |
| --- | --- | --- |
| first request id | `1` | **`0`**, from both clients |
| `tools/list` params | `"params": {}` | **absent** — and *present* on Claude Desktop's other launch |
| Claude Desktop launches | one | **two**, different `clientInfo`, different capabilities |
| era each opens at | Desktop `2025-06-18`, Zed `2025-11-25` | **`2025-11-25` from both** |

An id of `0` is falsy in every language a client is written in and an absent
`params` is where a hand-written parser reaches for the unchecked access — so
these are not cosmetic. The server answered all of them correctly, which is
the point: the corpus stopped being a transcript of its author's assumptions.

The last row is the one that reaches beyond this directory. **No shipping
client opens in the modern era.**

## Capturing the real thing

`capture.sh` is the path. Put it in a client's config where `synveda` would
go, use the client, and both directions land in `in.jsonl` / `out.jsonl`. Fold
the inbound frames into a case's `send`, flip `provenance.kind` to `captured`
with a note naming the client and its version, re-record, and read the diff —
the answers changing is the interesting part, because it means a real client
asks for something the authored frames did not.

It is a shell script rather than a flag on the server deliberately: a
recording mode inside `synveda mcp` would be a product surface existing only
for a test, at exactly the layer this feature keeps thin.

## The cases

| case | client / era / launch | provenance | what it is in the corpus for |
| --- | --- | --- | --- |
| `claude-desktop-probe` | claude-desktop · legacy · `--writes tool` | captured | Claude Desktop's **first** launch: an enumeration probe whose `clientInfo` is `claude-ai`, asking only what tools exist. Its `tools/list` carries `"params": {}`. Nothing predicted that the app starts the server twice. |
| `claude-desktop-agent` | claude-desktop · legacy · `--writes tool` | captured | The **second** launch, the agent session — `clientInfo` is `local-agent-mode-synveda`, and it negotiates `roots.listChanged` and an `io.modelcontextprotocol/ui` extension the probe does not. Carries both `tools/call` frames, composed by the model: `recall` with a `query` and a `limit`, `remember` with prose. Both stop at the credential seam and answer `isError` with readable text, which pins the failure posture ADR-0057 inverts from the hooks — a caller who *asked* is told, not handed a protocol error the client renders opaquely. Its `tools/list` sends **no** `params` member, unlike the probe's. |
| `zed` | zed · legacy · `--writes tool` | captured | The non-Anthropic client decision 11 names as amended. Opens at `2025-11-25`, ids from `0`, `tools/list` with no `params` — and asks twice. |
| `modern-era` | spec · modern · `--writes tool` | authored | The `2026-07-28` era decision 3 requires: `server/discover`, the version carried per request in `_meta`, no handshake at all. Attributed to the **specification, not a vendor**, because neither AC client opens here — so this is the only thing exercising that path, and it says so rather than borrowing a vendor's name for frames the vendor does not send. Becomes `captured` the day a client ships that opens there. |
| `claude-code-plugin` | claude-code · legacy · `--writes host` | authored | The one launch this repository owns rather than a vendor: the plugin's entry point execs `--writes host`, so `tools/list` carries `recall` alone and `tools/call remember` is `-32602`. Decision 6 has two halves and this pins the second — a tool absent from the listing that still answered a call would not be absent. |
| `unsupported-version` | any · modern · `--writes tool` | authored | Synthetic by construction, and permanently so: no client sends a version on purpose in order to be refused. What *any* client on a revision this server does not implement must be told — `-32022` carrying `{requested, supported}`, which is what lets it retry instead of fail. |

## What is not here, and why

**No gateway.** Every case is decided by the server alone, so the suite runs
in CI with nothing running. That bounds `tools/call`: the cases that make one
reach the credential seam and stop, and the recorded answer is the sign-in
sentence. The test points `HOME` and `XDG_CONFIG_HOME` at an empty directory
for exactly this reason — a developer with a live session records the same
bytes as CI.
The gateway-backed round trip — a real recall answering from a real corpus,
watermarked — is `demos/ctx-5-recall.sh`, against a live stack.

**No `remember` that reaches the store.** Same reason. The write's admission,
its redaction scan and its four dispositions are pinned by `mcp::tests` in
`crates/synveda-cli/src/mcp.rs`, and the wire path by the observe suites.
