#!/usr/bin/env sh
# FLOW-1 acceptance demo: the VedaFlow object store.
# Acceptance: identical content deduplicates and history stays immutable under
# concurrent writers. The current model uses
# BLAKE3 content-addressed objects/trees/commits/refs in Postgres, with
# commits recording author identity, signature, and policy-pack snapshot
# hash (ADR-0003, object model in ADR-0030).
#
# Flow: migrate -> admit a tenant -> write governed history through the
# real API: the same bytes address the same object and dedup, the same
# bytes as a *skill* address differently (kind is in the hash), a commit
# records its author, its policy pack, and an Ed25519 signature that
# verifies against the commit address alone -> eight concurrent writers
# race one ref through compare-and-swap and every commit survives ->
# the ref refuses to rewind unless forced (FLOW-7's call, by name) ->
# THE OTHER HALF: the append-only triggers refuse a direct UPDATE even
# for the superuser, and when an attacker with database credentials
# suppresses triggers and rewrites a row anyway, verification names it
# -> restore the row, verification agrees again -> the property suite
# runs both properties, including 8 writers x 5 commits on one ref.
# On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE

cargo build -p synveda-cli
cargo build -p synveda-vedaflow --example object_store

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "flow1-demo-$(date +%s)-$$" --name "FLOW-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"

echo
echo "==> the object store, through its own API"
echo "    (content addressing + dedup, a signed commit carrying its policy"
echo "     pack, and eight writers racing one ref)"
cargo run -q -p synveda-vedaflow --example object_store -- "$tenant_id"

echo
echo "==> history is append-only: a direct UPDATE, as the table owner"
if psql_c "update vedaflow_commits set message = 'rewritten'
           where tenant_id = '$tenant_id';" 2>&1 | grep -q 'append-only'; then
  echo "    refused by the trigger (FLOW-1, ADR-0030) — as it must be"
else
  echo "    FAIL: vedaflow_commits accepted an UPDATE"
  exit 1
fi
if psql_c "delete from vedaflow_objects where tenant_id = '$tenant_id';" 2>&1 \
     | grep -q 'append-only'; then
  echo "    DELETE on vedaflow_objects refused too"
else
  echo "    FAIL: vedaflow_objects accepted a DELETE"
  exit 1
fi

echo
echo "==> THE ATTACK: database credentials, triggers suppressed"
echo "    (session_replication_role = replica — what no trigger can stop)"
psql_c "set session_replication_role = replica;
        update vedaflow_objects
        set content = content || '\\x21'::bytea, size_bytes = size_bytes + 1
        where tenant_id = '$tenant_id'
          and hash = (select hash from vedaflow_objects
                      where tenant_id = '$tenant_id'
                      order by hash limit 1);" >/dev/null
echo "    one object's content rewritten in place"
printf "    verification says: "
cargo run -q -p synveda-vedaflow --example object_store -- "$tenant_id" verify

echo
echo "==> restore the byte, and verification agrees again"
psql_c "set session_replication_role = replica;
        update vedaflow_objects
        set content = substring(content from 1 for octet_length(content) - 1),
            size_bytes = size_bytes - 1
        where tenant_id = '$tenant_id'
          and hash = (select hash from vedaflow_objects
                      where tenant_id = '$tenant_id'
                      order by hash limit 1);" >/dev/null
printf "    verification says: "
cargo run -q -p synveda-vedaflow --example object_store -- "$tenant_id" verify

echo
echo "==> THE AC: the property suite (dedup; immutable under concurrency)"
cargo test -p synveda-vedaflow --test object_store -- --nocapture --test-threads=1

echo
echo "==> and the adversarial RLS suite, with the six VedaFlow tables in it"
cargo test -p synveda-store --test rls

echo
echo "FLOW-1 demo complete."
