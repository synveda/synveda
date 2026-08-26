#!/usr/bin/env sh
# CPR-41: the packaged PulseBoard command is an ordinary resumable public-API
# client; this hermetic evidence combines its CLI contract with the exact
# gateway bootstrap and cross-session product acceptances it orchestrates.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr41" "CPR-41 — one-command PulseBoard product walkthrough"

echo "    Verify the packaged command tree and secret-free local receipt boundary."
cargo test -p synveda-cli demo -- --nocapture
cargo run -q -p synveda-cli --bin synveda -- demo --help >/dev/null
cargo run -q -p synveda-cli --bin synveda -- demo start --help >/dev/null
cargo run -q -p synveda-cli --bin synveda -- demo status --help >/dev/null
cargo run -q -p synveda-cli --bin synveda -- demo reset --help >/dev/null

echo "    Prove exact first-profile adoption, concurrent one-winner semantics and ordinary matrix fallback."
cargo test -p synveda-gateway --test configuration_api -- --nocapture

echo "    Prove the PulseBoard capture, privacy, teammate reuse, supersession and context loop."
cargo test -p synveda-gateway --test capture_api \
  pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end \
  -- --exact --nocapture

demo_finish
