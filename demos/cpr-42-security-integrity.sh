#!/usr/bin/env sh
# CPR-42: repeat the cross-layer security inventory and the four adapter
# regressions found by the adversarial audit. Database-backed cases named by
# the inventory run in `make db-test`; this script remains a focused package
# acceptance proof.
set -eu

echo "CPR-42 — context-platform security and product integrity"

echo "    Prove deletion or widening of a named security boundary fails the gate."
node --test scripts/check-context-security.test.mjs
node scripts/check-context-security.mjs

echo "    Prove CLI delivery holds corrupt or cross-gateway spool state."
cargo test -p synveda-cli session::tests

echo "    Prove automatic hooks hold tampered/refused state and diagnostics redact it."
npm test --prefix adapters/claude-code

echo "demo PASS: CPR-42 adversarial boundaries and confirmed fixes hold"
