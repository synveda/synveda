#!/usr/bin/env sh
# CPR-36: local, release and Helm deployment shapes converge on one runtime.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr36" "CPR-36 — one context platform across deployment shapes"

make check-deploy
cargo run -q -p synveda-cli --bin synveda -- db migrate
cargo test -p synveda-cli \
  init::tests::compose_gateway_login_is_rls_enforced \
  -- --exact --nocapture --test-threads=1
cargo test -p synveda-store --test epoch -- --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture

demo_finish
