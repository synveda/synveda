#!/usr/bin/env sh
# TEN-2 acceptance demo: Postgres row-level security as the tenant backstop.
# AC (docs/backlog/TEN-2.md): direct SQL with the wrong tenant GUC returns
# zero rows on every tenant-scoped table.
#
# Flow: migrate -> admit two tenants (CLI) -> seed one record with history
# for tenant alpha (owner connection, RLS-exempt) -> then, as the
# non-superuser synveda_app role with a transaction-local GUC (ADR-0009),
# prove: wrong GUC sees zero rows on records, records_history, and
# records_versions; unset GUC sees zero; the right GUC sees exactly its own
# rows; and a cross-tenant INSERT is rejected by the policy's WITH CHECK.
# On Windows, run via Git Bash. Needs only the postgres service.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL

psql_db() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -U synveda -d synveda -qtAX -v ON_ERROR_STOP=1 "$@"
}

cargo build -p synveda-cli

echo "==> migrate + admit two tenants (synveda CLI)"
./target/debug/synveda db migrate
stamp="$(date +%s)-$$"
alpha=$(./target/debug/synveda tenant create \
  --slug "ten2-alpha-$stamp" --name "TEN-2 Alpha" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
beta=$(./target/debug/synveda tenant create \
  --slug "ten2-beta-$stamp" --name "TEN-2 Beta" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    alpha: $alpha"
echo "    beta:  $beta"

echo "==> tenant-scoped tables under forced RLS (any table with tenant_id)"
psql_db <<'SQL' | sed 's/^/    /'
select c.relname || '  enabled=' || c.relrowsecurity || ' forced=' || c.relforcerowsecurity
from pg_class c
join pg_namespace n on n.oid = c.relnamespace
where n.nspname = 'public' and c.relkind = 'r'
  and exists (select from pg_attribute a
              where a.attrelid = c.oid and a.attname = 'tenant_id'
                and not a.attisdropped)
order by c.relname;
SQL

echo "==> seed one record + one archived version for alpha (owner connection)"
rid=$(psql_db -v alpha="$alpha" <<'SQL'
insert into records (id, tenant_id, scope_id, owner_id, kind, class, content,
                     sensitivity, provenance, valid_from, tx_from)
values (gen_random_uuid(), :'alpha', gen_random_uuid(), gen_random_uuid(),
        'derived', 'fact', 'alpha-only memory', 'internal', '{}', now(), now())
returning id;
SQL
)
# A separate transaction, so the archive trigger records real history.
# (Stdin, not -c: psql skips variable interpolation for -c commands.)
psql_db -v rid="$rid" >/dev/null <<'SQL'
update records set content = 'alpha-only memory v2' where id = :'rid';
SQL
echo "    record: $rid (1 current row, 1 history row, 2 versions)"

# Prints alpha's visible row counts (records, records_history,
# records_versions) in one transaction as synveda_app with the GUC = $1
# ('' = unset). This is the AC's "direct SQL".
counts_as_app() {
  psql_db -v guc="$1" -v alpha="$alpha" <<'SQL' | tr '\n' ' '
begin;
set local role synveda_app;
select set_config('synveda.tenant_id', :'guc', true) as _ \gset
select count(*) from records where tenant_id = :'alpha';
select count(*) from records_history where tenant_id = :'alpha';
select count(*) from records_versions where tenant_id = :'alpha';
rollback;
SQL
}

echo "==> AC: wrong tenant GUC (beta) -> zero rows on every table"
got=$(counts_as_app "$beta")
if [ "$got" != "0 0 0 " ]; then
  echo "demo FAILED: wrong GUC saw rows (records/history/versions = $got)" >&2
  exit 1
fi
echo "    records=0 records_history=0 records_versions=0"

echo "==> unset GUC -> zero rows (the backstop fails closed)"
got=$(counts_as_app "")
if [ "$got" != "0 0 0 " ]; then
  echo "demo FAILED: unset GUC saw rows (records/history/versions = $got)" >&2
  exit 1
fi
echo "    records=0 records_history=0 records_versions=0"

echo "==> right tenant GUC (alpha) -> exactly its own rows"
got=$(counts_as_app "$alpha")
if [ "$got" != "1 1 2 " ]; then
  echo "demo FAILED: right GUC saw records/history/versions = $got, want 1 1 2" >&2
  exit 1
fi
echo "    records=1 records_history=1 records_versions=2"

echo "==> cross-tenant INSERT (GUC=beta, row for alpha) -> rejected"
if out=$(psql_db -v alpha="$alpha" -v beta="$beta" 2>&1 <<'SQL'
begin;
set local role synveda_app;
select set_config('synveda.tenant_id', :'beta', true) as _ \gset
insert into records (id, tenant_id, scope_id, owner_id, kind, class, content,
                     sensitivity, provenance, valid_from, tx_from)
values (gen_random_uuid(), :'alpha', gen_random_uuid(), gen_random_uuid(),
        'derived', 'fact', 'forged', 'internal', '{}', now(), now());
rollback;
SQL
); then
  echo "demo FAILED: cross-tenant insert was accepted" >&2
  exit 1
fi
echo "$out" | grep -q "row-level security" || {
  echo "demo FAILED: insert failed for the wrong reason: $out" >&2
  exit 1
}
echo "    rejected: new row violates row-level security policy"

echo ""
echo "TEN-2 row-level security backstop: acceptance criteria pass."
