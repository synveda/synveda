#!/usr/bin/env sh
# TEN-2 — forced row-level security is the tenant backstop.
#
# The epoch-3 baseline contains every current tenant table. This demo runs the
# schema-completeness assertion and representative wrong-tenant read/write
# probes through the non-BYPASSRLS application role. It deliberately seeds no
# application row with owner SQL: the tests use the current scope, Knowledge,
# session and governed-configuration stores and their normal constraints.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=${DATABASE_URL:-postgres://synveda:synveda-dev@localhost:5432/synveda}
SQLX_OFFLINE=true
export DATABASE_URL SQLX_OFFLINE

echo "==> every tenant table has enabled and forced RLS"
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact

echo "==> a wrong tenant sees no governed scope rows"
cargo test -p synveda-store --test rls \
  wrong_tenant_guc_sees_no_scope_rows -- --exact

echo "==> a cross-tenant scope write fails the WITH CHECK policy"
cargo test -p synveda-store --test rls \
  cross_tenant_scope_write_is_rejected -- --exact

echo "==> a wrong tenant sees no session-event rows"
cargo test -p synveda-store --test rls \
  wrong_tenant_guc_sees_no_session_event_rows -- --exact

echo
echo "TEN-2 forced-RLS completeness and tenant-isolation checks pass."
