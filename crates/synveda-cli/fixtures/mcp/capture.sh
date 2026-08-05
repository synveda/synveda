#!/bin/sh
# Capture a real client's MCP frames (ADPT-2, ADR-0057 decision 11).
#
# The corpus beside this file has real *answers* — every `expect` was
# produced by the shipped binary — and authored *questions*. Decision 11
# asks for each client's real frames, and this is how they get made: put
# this script in the client's config where `synveda` would go, use the
# client normally, and every frame in both directions lands on disk.
#
#   synveda mcp install --client cursor --print   # the entry to adapt
#
# then edit the `command` to point here, and add the directory:
#
#   {
#     "mcpServers": {
#       "synveda": {
#         "command": "/abs/path/to/capture.sh",
#         "env": { "SYNVEDA_MCP_CAPTURE": "/tmp/synveda-capture" }
#       }
#     }
#   }
#
# Restart the client, exercise it — let it list tools, ask it to recall
# something, ask it to remember something — then quit. You will have:
#
#   $SYNVEDA_MCP_CAPTURE/in.jsonl    what the client sent, one frame a line
#   $SYNVEDA_MCP_CAPTURE/out.jsonl   what the server answered
#
# `in.jsonl` is the half the corpus wants. Fold it into a case's `send`
# frames, set `provenance.kind` to `captured` with a note naming the client
# and version, and re-record:
#
#   SYNVEDA_RECORD_MCP=1 cargo test -p synveda-cli --test mcp_corpus
#
# This is a shell script rather than a flag on the server on purpose. A
# recording mode inside `synveda mcp` would be a product surface that
# exists only for a test, and it would be one more thing between a client
# and the protocol at exactly the layer this feature is trying to keep
# thin. `tee` already does this, for any client, with nothing shipped.
#
# It writes every frame verbatim, so treat the output as you would a
# transcript: a `remember` call carries whatever the model composed.

set -eu

: "${SYNVEDA_MCP_CAPTURE:=${TMPDIR:-/tmp}/synveda-mcp-capture}"
: "${SYNVEDA_CLI:=synveda}"
: "${SYNVEDA_MCP_WRITES:=tool}"

mkdir -p "$SYNVEDA_MCP_CAPTURE"

# stdin is teed on the way in and stdout on the way out; stderr is left
# alone so the client's own log still shows what the server said about
# itself. `exec` so signals reach the pipeline rather than this shell.
exec tee -a "$SYNVEDA_MCP_CAPTURE/in.jsonl" \
  | "$SYNVEDA_CLI" mcp --writes "$SYNVEDA_MCP_WRITES" \
  | tee -a "$SYNVEDA_MCP_CAPTURE/out.jsonl"
