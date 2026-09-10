#!/usr/bin/env sh
# TEN-2: forced RLS is the tenant-isolation backstop.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ten2" "TEN-2 — forced-RLS tenant isolation"

cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
cargo test -p synveda-store --test rls \
  wrong_tenant_guc_sees_no_scope_rows -- --exact --nocapture
cargo test -p synveda-store --test rls \
  cross_tenant_scope_write_is_rejected -- --exact --nocapture
cargo test -p synveda-store --test rls \
  wrong_tenant_guc_sees_no_session_event_rows -- --exact --nocapture

demo_finish
