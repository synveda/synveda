#!/usr/bin/env sh
# CTX-6: deterministic Claude compact/restart replay on the exact-role fixture.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx6" "CTX-6 — governed checkpoint restart"
echo "    Replays two compact boundaries in one batch and checks checkpoint, recent-event and budget evidence."
cargo test -p synveda-gateway --test sessions_api \
  compact_replay_keeps_both_checkpoint_windows_and_the_uncovered_tail \
  -- --exact --nocapture
echo "    Replays a concurrent duplicate marker, a missing source and a late event."
cargo test -p synveda-gateway --test sessions_api \
  concurrent_boundary_replay_marks_missing_evidence_and_keeps_late_events \
  -- --exact --nocapture
demo_finish
