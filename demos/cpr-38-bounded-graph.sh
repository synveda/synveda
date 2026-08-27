#!/usr/bin/env sh
# CPR-38: anchor-first bounded Knowledge graph retrieval.
#
# The acceptance drives public Knowledge/VedaFlow/session/context APIs. It
# proves a two-hop answer improves over the same governed profile with graph
# expansion disabled, then proves a denied private endpoint contributes no
# id, revision, content, count or path detail. Hashes-only retention keeps
# verifiable path digests without Knowledge addresses.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr38" "CPR-38 — bounded Knowledge graph retrieval"
echo "    Public APIs create Knowledge and governed support resolutions; ContextRun expands at most two authorised hops."
cargo test -p synveda-gateway --test context_runs \
  bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace -- --nocapture
demo_finish
