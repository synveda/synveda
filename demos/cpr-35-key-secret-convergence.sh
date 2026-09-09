#!/usr/bin/env sh
# CPR-35: stable tenant-secret identity, fail-closed references, durable DEK
# re-encryption and Knowledge-native export (ADR-0094).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr35" "CPR-35 — key and secret convergence"

cargo test -p synveda-store --test keys -- --test-threads=1
cargo test -p synveda-store --test knowledge \
  sealed_export_projection_contains_complete_knowledge_history_and_provenance \
  -- --exact --nocapture
cargo test -p synveda-gateway --test tools \
  stable_tool_secret_references_fail_closed_rotate_without_rewriting_versions_and_can_be_removed \
  -- --exact --test-threads=1
cargo test -p synveda-gateway --test directory_sync \
  an_unusable_stable_credential_never_falls_back_to_deployment_configuration \
  -- --exact --test-threads=1
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
cargo test -p synveda-cli \
  keys::tests::the_record_era_archive_magic_is_not_a_compatibility_reader \
  -- --exact

demo_finish
