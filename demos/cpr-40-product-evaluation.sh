#!/usr/bin/env sh
# CPR-40: deterministic product/trust evaluation definition and focused
# current-platform evidence. The full database-backed runner is `make
# eval-product`; the demo stays hermetic and fast enough for the corpus gate.
set -eu

echo "CPR-40 — context-platform product and trust evaluation"
echo "    Validate eighteen exact cases, eight independent outcome signals and six zero-count trust gates."
node scripts/product-evaluation.mjs --check
node --test scripts/product-evaluation.test.mjs

echo "    Prove the current Knowledge attribution parser ignores forged Record watermark text."
cargo test -p synveda-eval \
  client::tests::record_ids_are_recovered_from_current_knowledge_addresses_only

echo "    Prove deterministic capture classification records the explicit-choice ruleset version."
cargo test -p synveda-ingest \
  extraction::deterministic::tests::explicit_choice_forms_are_decisions_without_swallowing_procedures

echo "demo PASS: CPR-40 product evaluation is complete and trust gates remain zero-tolerance"
