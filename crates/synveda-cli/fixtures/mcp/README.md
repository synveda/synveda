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

## Read this before citing the corpus as the AC

The two halves of a case have different provenance, and only one of them is
real:

| | where it comes from | what it is worth |
| --- | --- | --- |
| `expect` — the answers | **Recorded** from the shipped binary, over a real pipe, by the test | A regression guard. It fails when the server's protocol behaviour changes. |
| `send` — the questions | **Authored** from the spec and each client's documented behaviour | Only as good as the reading. It cannot discover a client that opens differently from the way the spec was read here. |

**No case is captured from Claude Desktop or Cursor.** Neither was available
to record from when the corpus was built, so decision 11 is *not yet
satisfied* and this suite must not be cited as evidence that it is. Every
case says so in its own `provenance` block, and the suite fails if one does
not. `provenance.kind` is `authored` today and becomes `captured` when real
frames land; the test prints the tally on every run.

The distinction is not pedantry. An authored frame agrees with what its
author expected the client to send, so a corpus of them is a mirror. What it
still buys is real: the server's answers are pinned, both eras are covered,
and a change to either fails here rather than in somebody's client.

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

| case | client / era / launch | what it is in the corpus for |
| --- | --- | --- |
| `claude-desktop-legacy` | claude-desktop · legacy · `--writes tool` | The `initialize` handshake, answered on the client's own terms rather than upgraded. Ends in a `tools/call` with no credential, which pins the failure posture ADR-0057 inverts from the hooks: a caller who *asked* is told, as `isError` content it can read, not as a protocol error the client renders opaquely. |
| `claude-desktop-modern` | claude-desktop · modern · `--writes tool` | The era the hand-written loop could not reach: `server/discover`, and the version carried per request in `_meta` with no handshake at all. Pins the supported list itself, because that list is what a client retries against. |
| `cursor-legacy` | cursor · legacy · `--writes tool` | The non-Anthropic client the AC names, opening at `2025-11-25` — the revision between the one CTX-5 pinned and the current one, so the corpus covers the middle rather than only the ends. Its `tools/call` uses `ids` where Claude Desktop's uses `query`, so both halves of the xor appear. |
| `cursor-modern` | cursor · modern · `--writes tool` | The same vendor on the current revision, so the corpus does not assume a client stays on the era it shipped with. Its `tools/call` passes `query` **and** `ids`, which is refused before the gateway is troubled — the one `tools/call` case here that reaches past the credential seam. |
| `claude-code-plugin` | claude-code · legacy · `--writes host` | The one launch this repository owns rather than a vendor: the plugin's entry point execs `--writes host`, so `tools/list` carries `recall` alone and `tools/call remember` is `-32602`. Decision 6 has two halves and this pins the second — a tool absent from the listing that still answered a call would not be absent. |
| `unsupported-version` | any · modern · `--writes tool` | Not a vendor's frame at all: what *any* client on a revision this server does not implement must be told. `-32022` carrying `{requested, supported}` is what lets it retry instead of fail. |

## What is not here, and why

**No gateway.** Every case is decided by the server alone, so the suite runs
in CI with nothing running. That bounds `tools/call`: three cases reach the
credential seam and stop, and the recorded answer is the sign-in sentence.
The gateway-backed round trip — a real recall answering from a real corpus,
watermarked — is `demos/ctx-5-recall.sh`, against a live stack.

**No `remember` that reaches the store.** Same reason. The write's admission,
its redaction scan and its four dispositions are pinned by `mcp::tests` in
`crates/synveda-cli/src/mcp.rs`, and the wire path by the observe suites.
