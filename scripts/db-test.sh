#!/usr/bin/env bash
# `make db-test` — exact-role database acceptance on two disposable clusters.
#
# Normal workspace tests use the ordinary `synveda_gateway` credential against
# one migrated database. Database reset, schema-epoch mutation and role-drift
# cases run later and serially, with narrowly named test-only administrator
# settings; destructive lifecycle tests receive a second PostgreSQL cluster.
# No caller database or credential participates in this fixture.
# A caller may enable shell tracing before execution. Disable it before any
# generated credential can enter an expansion or child invocation.
set +x
set -euo pipefail
umask 077

cd "$(dirname "$0")/.."

case "${KEEP_TEST_DB:-}" in
  ""|1) ;;
  *)
    echo "db-test: KEEP_TEST_DB must be unset or 1" >&2
    exit 64
    ;;
esac

db_test_task=${SYNVEDA_DB_TEST_TASK:-workspace}
case "$db_test_task" in
  workspace|demo|product-evaluation|evaluation|longmemeval-evaluation|authority-fingerprints|sqlx-prepare) ;;
  *)
    echo "db-test: unknown SYNVEDA_DB_TEST_TASK" >&2
    exit 64
    ;;
esac
case "$db_test_task" in
  demo|product-evaluation|evaluation|longmemeval-evaluation) fast_fixture=true ;;
  authority-fingerprints|sqlx-prepare) fast_fixture=true ;;
  workspace) fast_fixture=false ;;
esac
if [ "$db_test_task" = authority-fingerprints ] && [ "$#" -ne 0 ]; then
  echo "db-test: authority-fingerprints takes no cargo-test arguments" >&2
  exit 64
fi
if [ "$db_test_task" = sqlx-prepare ] && [ "$#" -ne 0 ]; then
  echo "db-test: sqlx-prepare takes no cargo-test arguments" >&2
  exit 64
fi
if [ "$db_test_task" = demo ] && [ "$#" -lt 1 ]; then
  echo "db-test: demo requires a repository demo script" >&2
  exit 64
fi

docker_bin=${SYNVEDA_DOCKER_BIN:-docker}
command -v "$docker_bin" >/dev/null 2>&1 || {
  echo "db-test: Docker with the Compose plugin is required" >&2
  exit 69
}

tmp_root=${TMPDIR:-/tmp}
tmp_root=${tmp_root%/}
case "$tmp_root" in
  ""|/|*[[:space:]]*|*//*|*/./*|*/../*|*/.|*/..)
    echo "db-test: temporary root has an unsafe shape" >&2
    exit 70
    ;;
  /*) ;;
  *)
    echo "db-test: temporary root must be absolute" >&2
    exit 70
    ;;
esac
# macOS exposes its physical per-user temporary tree through `/var`, which is
# itself a compatibility symlink to `/private/var`. Secret generation rightly
# rejects caller-controlled symlink ancestors, so resolve the already-existing
# temporary root once and create every private fixture beneath that physical
# path. `pwd -P` is portable across the supported Unix shells; no `readlink -f`
# dependency is introduced.
tmp_root=$(CDPATH= cd -- "$tmp_root" 2>/dev/null && pwd -P) || {
  echo "db-test: temporary root is unavailable" >&2
  exit 70
}
case "$tmp_root" in
  ""|/|*[[:space:]]*|*//*|*/./*|*/../*|*/.|*/..)
    echo "db-test: physical temporary root has an unsafe shape" >&2
    exit 70
    ;;
  /*) ;;
  *)
    echo "db-test: physical temporary root must be absolute" >&2
    exit 70
    ;;
esac
[ -d "$tmp_root" ] && [ ! -L "$tmp_root" ] || {
  echo "db-test: physical temporary root was refused" >&2
  exit 70
}
state_dir=$(mktemp -d "$tmp_root/synveda-db-test.XXXXXX")
chmod 700 "$state_dir"
state_prefix=$(dirname "$state_dir")/synveda-db-test.
case "$state_dir" in
  "$state_prefix"*) ;;
  *)
    echo "db-test: temporary state path failed its ownership check" >&2
    exit 70
    ;;
esac

state_token=${state_dir##*.}
state_token=$(printf '%s' "$state_token" | LC_ALL=C tr '[:upper:]' '[:lower:]')
case "$state_token" in
  ""|*[!a-z0-9]*)
    echo "db-test: temporary state token is not safe for a Compose project" >&2
    exit 70
    ;;
esac
project="synveda-db-test-$$-$state_token"
generator_project=synveda-development-acceptance-$state_token
manifest=deploy/compose/compose.db-test.yaml
secret_dir=$state_dir/generator/$generator_project/secrets
roles_file=$state_dir/database-roles.json
lifecycle_roles_file=$state_dir/lifecycle-database-roles.json
external_roles_file=$state_dir/external-database-roles.json
main_authority_dir=$state_dir/main-database-authority
lifecycle_authority_dir=$state_dir/lifecycle-database-authority
network_ownership_file=$state_dir/network-ownership.tsv
network_receipt_dir=$state_dir/network-receipts
: > "$network_ownership_file"
chmod 600 "$network_ownership_file"
mkdir "$network_receipt_dir"
chmod 700 "$network_receipt_dir"
owned_network_ids=()
owned_network_receipt_files=()
owned_network_count=0
network_logicals=(main-data lifecycle-data main-host lifecycle-host)
network_names=(
  "$project-main-data"
  "$project-lifecycle-data"
  "$project-main-host"
  "$project-lifecycle-host"
)
network_subnets=("" "" "" "")
network_attempt_counts=(0 0 0 0)
network_reservation_limit=64
cleanup_started=false

report_preserved_state() {
  local status=$?
  if [ "$status" -ne 0 ]; then
    echo >&2
    if [ "$cleanup_started" = true ]; then
      echo "db-test: failed during success-path cleanup; teardown for $project may be partial" >&2
      echo "db-test: private state and any remaining resources were retained" >&2
    else
      echo "db-test: failed; retained private fixture state and any created resources for $project" >&2
    fi
    echo "db-test: private state is in $state_dir (mode 0700; contains credentials)" >&2
    echo "db-test: network reservation evidence is in $network_ownership_file (mode 0600)" >&2
  fi
  return "$status"
}
trap report_preserved_state EXIT
trap 'exit 130' INT TERM

# Each logical network walks its own lane through a full-cycle permutation of
# the 2,048 /26 quartets in the IANA benchmarking range. Docker remains the
# race authority: only its exact overlap refusal is contention that created no
# resource and may advance the candidate. Every other failure retains the
# fixture and every network already created by this run. No existing Docker
# resource is listed or inspected.
network_seed=$(printf '%s\n' "$project" | cksum | awk '{print $1}')
network_start_slot=$((network_seed % 2048))
unset network_seed

network_candidate_subnet() {
  local logical_index=$1
  local attempt=$2
  local network_quartet_slot
  local network_slot
  local network_second
  local network_within
  local network_third
  local network_fourth

  # 1265 is odd and therefore coprime with 2048. The logical index selects one
  # of four disjoint /28 lanes, so no two candidates in this fixture coincide.
  network_quartet_slot=$(((network_start_slot + attempt * 1265) % 2048))
  network_slot=$((network_quartet_slot * 4 + logical_index))
  network_second=$((18 + network_slot / 4096))
  network_within=$((network_slot % 4096))
  network_third=$((network_within / 16))
  network_fourth=$(((network_within % 16) * 16))
  printf '198.%s.%s.%s/28\n' \
    "$network_second" "$network_third" "$network_fourth"
}

network_create_is_pool_contention() {
  local receipt_file=$1
  local error_file=$2
  local create_status=$3

  [ "$create_status" -eq 1 ] && [ ! -s "$receipt_file" ] && cmp -s -- \
    <(printf '%s\n' \
      'Error response from daemon: invalid pool request: Pool overlaps with other one on this address space') \
    "$error_file"
}

record_network_ownership() {
  local record_kind=$1
  local logical_name=$2
  local network_name=$3
  local subnet=$4
  local network_id=$5

  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$record_kind" "$logical_name" "$network_name" "$subnet" "$network_id" \
    >> "$network_ownership_file" || {
    echo "db-test: could not record network ownership" >&2
    return 70
  }
}

reserve_test_network() {
  local logical_index=$1
  local logical_name=$2
  local network_name=$3
  local internal=$4
  local attempt=0
  local created_network_id
  local existing_network_id
  local network_create_status
  local network_error_file
  local network_receipt_bytes
  local network_receipt_file
  local network_status_file
  local subnet

  while [ "$attempt" -lt "$network_reservation_limit" ]; do
    subnet=$(network_candidate_subnet "$logical_index" "$attempt") || {
      echo "db-test: could not derive a network candidate" >&2
      return 70
    }
    record_network_ownership \
      intent "$logical_name" "$network_name" "$subnet" - || return 70
    network_receipt_file=$network_receipt_dir/$logical_index-$attempt.stdout
    network_error_file=$network_receipt_dir/$logical_index-$attempt.stderr
    network_status_file=$network_receipt_dir/$logical_index-$attempt.status
    : > "$network_receipt_file" || {
      echo "db-test: could not prepare a private network receipt" >&2
      return 70
    }
    : > "$network_error_file" || {
      echo "db-test: could not prepare a private network receipt" >&2
      return 70
    }
    : > "$network_status_file" || {
      echo "db-test: could not prepare a private network receipt" >&2
      return 70
    }
    chmod 600 \
        "$network_receipt_file" "$network_error_file" "$network_status_file" || {
      echo "db-test: could not protect a private network receipt" >&2
      return 70
    }
    if [ "$internal" = true ]; then
      if "$docker_bin" network create \
        --driver bridge --internal --subnet "$subnet" \
        --label com.synveda.contract=cpr-45-db-test \
        --label "com.synveda.project=$project" \
        --label "com.synveda.network=$logical_name" \
        "$network_name" >"$network_receipt_file" 2>"$network_error_file"; then
        network_create_status=0
      else
        network_create_status=$?
      fi
    else
      if "$docker_bin" network create \
        --driver bridge --subnet "$subnet" \
        --label com.synveda.contract=cpr-45-db-test \
        --label "com.synveda.project=$project" \
        --label "com.synveda.network=$logical_name" \
        "$network_name" >"$network_receipt_file" 2>"$network_error_file"; then
        network_create_status=0
      else
        network_create_status=$?
      fi
    fi
    printf '%s\n' "$network_create_status" > "$network_status_file" || {
      echo "db-test: could not record the Docker reservation status" >&2
      return 70
    }
    if [ "$network_create_status" -eq 0 ]; then
      break
    fi
    if network_create_is_pool_contention \
        "$network_receipt_file" "$network_error_file" "$network_create_status"; then
      record_network_ownership \
        contended "$logical_name" "$network_name" "$subnet" - || return 70
      attempt=$((attempt + 1))
      continue
    fi
    echo "db-test: Docker could not reserve the $logical_name network" >&2
    return 69
  done
  if [ "$attempt" -eq "$network_reservation_limit" ]; then
    echo "db-test: no uncontended /28 is available for the $logical_name network" >&2
    return 69
  fi
  network_receipt_bytes=$(LC_ALL=C wc -c < "$network_receipt_file") || {
    echo "db-test: could not read a private network receipt" >&2
    return 70
  }
  if [ "$network_receipt_bytes" -ne 65 ]; then
    echo "db-test: Docker returned a malformed network identifier" >&2
    return 69
  fi
  IFS= read -r created_network_id < "$network_receipt_file" || {
    echo "db-test: Docker returned a malformed network identifier" >&2
    return 69
  }
  if [ "${#created_network_id}" -ne 64 ]; then
    echo "db-test: Docker returned a malformed network identifier" >&2
    return 69
  fi
  case "$created_network_id" in
    *[!0-9a-f]*)
      echo "db-test: Docker returned a malformed network identifier" >&2
      return 69
      ;;
  esac
  if [ "$owned_network_count" -gt 0 ]; then
    for existing_network_id in "${owned_network_ids[@]}"; do
      if [ "$existing_network_id" = "$created_network_id" ]; then
        echo "db-test: Docker reused a network identifier within one fixture" >&2
        return 69
      fi
    done
  fi
  record_network_ownership \
    owned "$logical_name" "$network_name" "$subnet" "$created_network_id" || return 70
  network_subnets[$logical_index]=$subnet
  network_attempt_counts[$logical_index]=$attempt
  owned_network_ids+=("$created_network_id")
  owned_network_receipt_files+=("$network_receipt_file")
  owned_network_count=$((owned_network_count + 1))
}

reserve_test_network 0 "${network_logicals[0]}" "${network_names[0]}" true
reserve_test_network 1 "${network_logicals[1]}" "${network_names[1]}" true
reserve_test_network 2 "${network_logicals[2]}" "${network_names[2]}" false
reserve_test_network 3 "${network_logicals[3]}" "${network_names[3]}" false

export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}
export SYNVEDA_DB_TEST_LIFECYCLE_DATA_NETWORK=${network_names[1]}
export SYNVEDA_DB_TEST_MAIN_HOST_NETWORK=${network_names[2]}
export SYNVEDA_DB_TEST_LIFECYCLE_HOST_NETWORK=${network_names[3]}
unset SYNVEDA_DB_TEST_MAIN_DATA_SUBNET SYNVEDA_DB_TEST_LIFECYCLE_DATA_SUBNET
unset SYNVEDA_DB_TEST_MAIN_HOST_SUBNET SYNVEDA_DB_TEST_LIFECYCLE_HOST_SUBNET
export SYNVEDA_DB_TEST_ROLES_FILE=$roles_file
export SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE=$lifecycle_roles_file
export SYNVEDA_DB_TEST_EXTERNAL_ROLES_FILE=$external_roles_file
export SYNVEDA_DB_TEST_MAIN_AUTHORITY_DIR=$main_authority_dir
export SYNVEDA_DB_TEST_LIFECYCLE_AUTHORITY_DIR=$lifecycle_authority_dir
export SYNVEDA_DB_TEST_UID=${SYNVEDA_DB_TEST_UID:-$(id -u)}
export SYNVEDA_DB_TEST_GID=${SYNVEDA_DB_TEST_GID:-$(id -g)}
export SYNVEDA_DB_TEST_SECRETS_DIR=$secret_dir
if [ -n "${SYNVEDA_DB_TEST_POSTGRES_IMAGE:-}" ]; then
  test_image_owned=false
else
  SYNVEDA_DB_TEST_POSTGRES_IMAGE=synveda-db-test-postgres:$project
  test_image_owned=true
fi
export SYNVEDA_DB_TEST_POSTGRES_IMAGE

compose() {
  "$docker_bin" compose --project-name "$project" --file "$manifest" "$@"
}

validate_owned_network_ledger() {
  local network_index
  local receipt_bytes
  local receipt_id
  local receipt_status

  [ "$owned_network_count" -eq 4 ] \
    && [ "${#owned_network_ids[@]}" -eq 4 ] \
    && [ "${#owned_network_receipt_files[@]}" -eq 4 ] || {
    echo "db-test: refusing cleanup without four immutable network identifiers" >&2
    return 1
  }
  [ -f "$network_ownership_file" ] && [ ! -L "$network_ownership_file" ] || {
    echo "db-test: refusing cleanup without the regular ownership ledger" >&2
    return 1
  }
  if ! cmp -s -- <(expected_owned_network_ledger) "$network_ownership_file"; then
    echo "db-test: refusing cleanup after ownership ledger drift" >&2
    return 1
  fi
  network_index=0
  while [ "$network_index" -lt 4 ]; do
    case "${owned_network_receipt_files[$network_index]}" in
      "$network_receipt_dir"/[0-3]-*.stdout) ;;
      *)
        echo "db-test: refusing cleanup without a fixture-local network receipt" >&2
        return 1
        ;;
    esac
    [ -f "${owned_network_receipt_files[$network_index]}" ] \
      && [ ! -L "${owned_network_receipt_files[$network_index]}" ] || {
      echo "db-test: refusing cleanup without a regular network receipt" >&2
      return 1
    }
    receipt_bytes=$(LC_ALL=C wc -c \
      < "${owned_network_receipt_files[$network_index]}") || return 1
    [ "$receipt_bytes" -eq 65 ] || {
      echo "db-test: refusing cleanup after network receipt drift" >&2
      return 1
    }
    IFS= read -r receipt_id \
      < "${owned_network_receipt_files[$network_index]}" || return 1
    [ "$receipt_id" = "${owned_network_ids[$network_index]}" ] || {
      echo "db-test: refusing cleanup after network receipt drift" >&2
      return 1
    }
    receipt_status=${owned_network_receipt_files[$network_index]%.stdout}.status
    [ -f "$receipt_status" ] && [ ! -L "$receipt_status" ] \
      && cmp -s -- <(printf '0\n') "$receipt_status" || {
      echo "db-test: refusing cleanup without a successful network receipt" >&2
      return 1
    }
    network_index=$((network_index + 1))
  done
}

expected_owned_network_ledger() {
  local network_attempt
  local network_index
  local subnet

  network_index=0
  while [ "$network_index" -lt 4 ]; do
    network_attempt=0
    while [ "$network_attempt" -le "${network_attempt_counts[$network_index]}" ]; do
      subnet=$(network_candidate_subnet \
        "$network_index" "$network_attempt")
      printf 'intent\t%s\t%s\t%s\t-\n' \
        "${network_logicals[$network_index]}" \
        "${network_names[$network_index]}" \
        "$subnet"
      if [ "$network_attempt" -lt "${network_attempt_counts[$network_index]}" ]; then
        printf 'contended\t%s\t%s\t%s\t-\n' \
          "${network_logicals[$network_index]}" \
          "${network_names[$network_index]}" \
          "$subnet"
      else
        printf 'owned\t%s\t%s\t%s\t%s\n' \
          "${network_logicals[$network_index]}" \
          "${network_names[$network_index]}" \
          "${network_subnets[$network_index]}" \
          "${owned_network_ids[$network_index]}"
      fi
      network_attempt=$((network_attempt + 1))
    done
    network_index=$((network_index + 1))
  done
}

cleanup_successful_fixture() {
  local network_index

  validate_owned_network_ledger
  cleanup_started=true
  compose down --volumes --remove-orphans
  network_index=$((owned_network_count - 1))
  while [ "$network_index" -ge 0 ]; do
    "$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null
    network_index=$((network_index - 1))
  done
  if [ "$test_image_owned" = true ]; then
    "$docker_bin" image rm "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" >/dev/null
  fi
  rm -R -- "$state_dir"
  trap - EXIT
}

private_evidence_file() {
  evidence_file=$1
  : > "$evidence_file"
  chmod 600 "$evidence_file"
}

assert_database_secrets_absent() {
  evidence_file=$1
  for secret_name in postgres_owner_password synveda_migrator_password \
      synveda_gateway_password synveda_worker_password keycloak_database_password \
      external_provider_password; do
    if LC_ALL=C grep -Fq -f "$secret_dir/$secret_name" "$evidence_file"; then
      echo "db-test: database credential entered captured evidence" >&2
      return 1
    fi
  done
  if LC_ALL=C grep -Fq "$state_dir" "$evidence_file"; then
    echo "db-test: private state path entered captured evidence" >&2
    return 1
  fi
}

assert_database_evidence_omits() {
  local evidence_file=$1
  shift
  assert_database_secrets_absent "$evidence_file"
  for prohibited_value in "$@"; do
    if LC_ALL=C grep -Fq "$prohibited_value" "$evidence_file"; then
      echo "db-test: protected fixture content entered captured evidence" >&2
      return 1
    fi
  done
}

assert_keycloak_admission_empty() {
  local label=$1
  local proof
  proof=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
\o /dev/null
select coalesce((
  select database.oid::bigint
    from pg_catalog.pg_database database
   where database.datname = 'keycloak'
), 0) as keycloak_database_oid
\gset
select 1 / case when :'keycloak_database_oid' <> '0' then 1 else 0 end;
\o
select (not exists (
  select 1
    from pg_catalog.pg_locks lock
   where lock.locktype = 'object'
     and lock.database = 0
     and lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass
     and lock.objid = :'keycloak_database_oid'::pg_catalog.oid
     and lock.objsubid = 0
     and lock.mode = 'RowExclusiveLock'
     and lock.pid is not null
))::text;
select pg_catalog.pg_stat_clear_snapshot()
\g /dev/null
select (not exists (
  select 1
    from pg_catalog.pg_stat_activity activity
   where activity.datid = :'keycloak_database_oid'::pg_catalog.oid
))::text;
select (not exists (
  select 1
    from pg_catalog.pg_prepared_xacts prepared
    join pg_catalog.pg_database database on database.datname = prepared.database
   where database.oid = :'keycloak_database_oid'::pg_catalog.oid
))::text;
SQL
  )
  [ "$proof" = "$(printf 'true\ntrue\ntrue')" ] || {
    echo "db-test: $label retained a Keycloak startup lock, session or prepared transaction" >&2
    return 1
  }
}

# Test isolation is intentional: ambient connection settings must not turn a
# repository gate into a mutation of an operator's database.
unset DATABASE_URL DATABASE_URL_FILE
unset SYNVEDA_MIGRATOR_DATABASE_URL SYNVEDA_MIGRATOR_DATABASE_URL_FILE
unset SYNVEDA_GATEWAY_DATABASE_URL SYNVEDA_GATEWAY_DATABASE_URL_FILE
unset SYNVEDA_WORKER_DATABASE_URL SYNVEDA_WORKER_DATABASE_URL_FILE
unset SYNVEDA_DATABASE_ROLES SYNVEDA_DATABASE_ROLES_FILE
unset SYNVEDA_EXPECTED_DATABASE_ROLE SYNVEDA_EXPECTED_DATABASE_ROLE_FILE
unset SYNVEDA_TEST_ADMIN_DATABASE_URL SYNVEDA_TEST_GATEWAY_DATABASE_URL
unset SYNVEDA_TEST_WORKER_DATABASE_URL
unset SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE
unset SYNVEDA_TEST_WORKER_DATABASE_URL_FILE SYNVEDA_CARGO_DATABASE_URL_FILE
unset SYNVEDA_TEST_DATABASE_URL_FILE
unset SYNVEDA_TEST_MIGRATOR_DATABASE_URL SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE
unset SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL
unset SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL
unset SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL
unset SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL_FILE
unset SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL_FILE
unset SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE
unset PGHOSTADDR PGHOST PGPORT PGUSER PGPASSWORD PGDATABASE
unset PGSSLROOTCERT PGSSLCERT PGSSLKEY PGSSLMODE PGSSLCERTMODE PGAPPNAME PGOPTIONS PGPASSFILE
unset PGGSSENCMODE PGGSSDELEGATION

mkdir -p "$state_dir/generator/$generator_project"
chmod 700 "$state_dir/generator" "$state_dir/generator/$generator_project"
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-$state_token \
SYNVEDA_SECRETS_DIR=$secret_dir \
SYNVEDA_DATABASE_AUTHORITY_DIR=$state_dir/generator/$generator_project/database-authority \
SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR=$state_dir/generator/$generator_project/keycloak-public-gate \
  deploy/compose/scripts/generate-secrets.sh >/dev/null
mkdir "$main_authority_dir" "$lifecycle_authority_dir"
chmod 700 "$main_authority_dir" "$lifecycle_authority_dir"
external_provider_password=$(openssl rand -hex 32)
printf '%s\n' "$external_provider_password" > "$secret_dir/external_provider_password"
unset external_provider_password
cp deploy/compose/configs/database/roles.reference.json "$roles_file"
cp deploy/compose/configs/database/roles.external-oidc.json "$lifecycle_roles_file"
printf '%s\n' '{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["cpr45_external_bootstrap"],"administrative_memberships":[{"member":"cpr45_external_bootstrap","grantor":"synveda_owner"}],"forbidden_databases":["keycloak","postgres","template1"],"isolated_peer_roles":["keycloak"]}' > "$external_roles_file"
chmod 600 "$secret_dir/external_provider_password" "$roles_file" \
  "$lifecycle_roles_file" "$external_roles_file"

published_port() {
  service=$1
  endpoint=$(compose port "$service" 5432)
  case "$endpoint" in
    127.0.0.1:[1-9][0-9]*) ;;
    *)
      echo "db-test: $service did not publish PostgreSQL on loopback" >&2
      return 1
      ;;
  esac
  port=${endpoint##*:}
  case "$port" in
    *[!0-9]*|"")
      echo "db-test: $service returned an invalid loopback port" >&2
      return 1
      ;;
  esac
  printf '%s\n' "$port"
}

write_database_url() {
  role=$1
  password_file=$2
  port=$3
  database=$4
  target=$5
  password_bytes=$(wc -c < "$password_file")
  [ "$password_bytes" -gt 1 ] && [ "$password_bytes" -le 4096 ] || {
    echo "db-test: generated database credential has an invalid size" >&2
    return 1
  }
  [ "$(tr -cd '\n' < "$password_file" | wc -c | tr -d ' ')" -eq 1 ] \
    && [ "$(tail -c 1 "$password_file" | od -An -tu1 | tr -d ' ')" = 10 ] || {
      echo "db-test: generated database credential must be one line" >&2
      return 1
    }
  : > "$target"
  chmod 600 "$target"
  printf 'postgresql://%s:' "$role" >> "$target"
  tr -d '\n' < "$password_file" >> "$target"
  printf '@127.0.0.1:%s/%s\n' "$port" "$database" >> "$target"
  unset password_bytes
}

run_main_database_preflight() {
  witness_file=$1
  env -u SYNVEDA_DB_TEST_SECRETS_DIR \
    SQLX_OFFLINE=true \
    SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
    SYNVEDA_DATABASE_EXPECTED_HOST=127.0.0.1 \
    SYNVEDA_DATABASE_EXPECTED_PORT=$main_port \
    SYNVEDA_DATABASE_EXPECTED_NAME=synveda \
    SYNVEDA_DATABASE_REQUIRED_PEER=keycloak \
    SYNVEDA_DATABASE_PEER_WITNESS_FILE=$witness_file \
    SYNVEDA_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
    SYNVEDA_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
    SYNVEDA_WORKER_DATABASE_URL_FILE=$main_worker_file \
      cargo run -q -p synveda-cli --bin synveda -- db preflight
}

run_main_database_authority_preflight() {
  env -u SYNVEDA_DB_TEST_SECRETS_DIR \
    -u SYNVEDA_DATABASE_REQUIRED_PEER \
    -u SYNVEDA_DATABASE_PEER_WITNESS_FILE \
    SQLX_OFFLINE=true \
    SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
    SYNVEDA_DATABASE_EXPECTED_HOST=127.0.0.1 \
    SYNVEDA_DATABASE_EXPECTED_PORT=$main_port \
    SYNVEDA_DATABASE_EXPECTED_NAME=synveda \
    SYNVEDA_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
    SYNVEDA_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
    SYNVEDA_WORKER_DATABASE_URL_FILE=$main_worker_file \
      cargo run -q -p synveda-cli --bin synveda -- db preflight
}

# Returns 0 only for exact readiness, 75 only for a closed transient shape,
# and 1 for every other status/output combination.
classify_main_database_authority_preflight() {
  local preflight_status=$1
  local stdout_file=$2
  local stderr_file=$3
  local retryable_error

  [ ! -s "$stdout_file" ] || return 1
  if [ "$preflight_status" -eq 0 ] && cmp -s -- \
      <(printf '%s\n' 'database target preflight complete') "$stderr_file"; then
    return 0
  fi
  [ "$preflight_status" -eq 1 ] || return 1
  for retryable_error in \
    'synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed' \
    'synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE preflight timed out' \
    'synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE authority or writable-target verification failed'; do
    if cmp -s -- <(printf '%s\n' "$retryable_error") "$stderr_file"; then
      return 75
    fi
  done
  return 1
}

wait_for_main_database_authority() {
  local attempt=1
  local classification
  local preflight_status
  local stdout_file=$state_dir/preflight-restart-readiness.stdout
  local stderr_file=$state_dir/preflight-restart-readiness.stderr

  while [ "$attempt" -le 3 ]; do
    private_evidence_file "$stdout_file"
    private_evidence_file "$stderr_file"
    if run_main_database_authority_preflight > "$stdout_file" 2> "$stderr_file"; then
      preflight_status=0
    else
      preflight_status=$?
    fi
    assert_database_secrets_absent "$stdout_file"
    assert_database_secrets_absent "$stderr_file"
    if classify_main_database_authority_preflight \
        "$preflight_status" "$stdout_file" "$stderr_file"; then
      rm -f "$stdout_file" "$stderr_file"
      return 0
    else
      classification=$?
    fi
    [ "$classification" -eq 75 ] || {
      echo "db-test: post-restart database authority readiness returned an invalid response" >&2
      return 1
    }
    [ "$attempt" -lt 3 ] || {
      echo "db-test: post-restart database authority readiness did not converge" >&2
      return 1
    }
    sleep 2
    attempt=$((attempt + 1))
  done
  return 1
}

compose config --quiet
compose build postgres-main
if [ "$fast_fixture" = true ]; then
  compose up --detach --wait postgres-main
else
  compose up --detach --wait postgres-main postgres-lifecycle
fi

# Demos and evaluations need the exact deployment authority, not the full
# destructive database acceptance suite. Converge one fresh main cluster in
# product order, prove its peer witness, migrate idempotently, then dispatch.
# The workspace task continues below through every hostile/two-cluster case.
if [ "$fast_fixture" = true ]; then
  compose run --rm --no-deps database-bootstrap-main
  compose run --rm --no-deps keycloak-database-bootstrap-main
  compose run --rm --no-deps database-bootstrap-main

  main_port=$(published_port postgres-main)
  main_migrator_file=$state_dir/main-migrator.url
  main_gateway_file=$state_dir/main-gateway.url
  main_worker_file=$state_dir/main-worker.url
  write_database_url synveda_migrator "$secret_dir/synveda_migrator_password" \
    "$main_port" synveda "$main_migrator_file"
  write_database_url synveda_gateway "$secret_dir/synveda_gateway_password" \
    "$main_port" synveda "$main_gateway_file"
  write_database_url synveda_worker "$secret_dir/synveda_worker_password" \
    "$main_port" synveda "$main_worker_file"

  main_witness_file=$main_authority_dir/keycloak-cluster.json
  [ -f "$main_witness_file" ] || {
    echo "db-test: fast fixture did not publish the Keycloak cluster witness" >&2
    exit 1
  }
  assert_database_secrets_absent "$main_witness_file"

  # A baseline-revision change makes the pinned authority fingerprints stale
  # before the product preflight can accept the new schema. Derive the four
  # content-free values from an isolated raw migration without treating that
  # report as migration acceptance. Online compilation uses only the exact
  # migrator URL; the ignored runtime reporter connects through the ordinary
  # gateway role and emits one complete set or none.
  if [ "$db_test_task" = authority-fingerprints ]; then
    sqlx_library_version=$(awk '
      $0 == "name = \"sqlx\"" { package = 1; next }
      package && $1 == "version" { gsub(/\"/, "", $3); print $3; exit }
    ' Cargo.lock)
    sqlx_cli_banner=$(cargo sqlx --version)
    [ -n "$sqlx_library_version" ] \
      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] || {
        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2
        exit 69
      }
    env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE \
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file \
          cargo sqlx migrate run --no-dotenv \
            --source crates/synveda-store/migrations
    env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE \
      SYNVEDA_REPORT_AUTHORITY_FINGERPRINTS=1 \
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file \
        scripts/cargo-with-database-url-file \
          cargo test -q -p synveda-store --lib \
            runtime_role::tests::report_live_catalog_fingerprints \
            -- --ignored --exact --nocapture
    unset sqlx_library_version sqlx_cli_banner
  fi

  # Cache regeneration cannot compile Synveda before the new cache exists:
  # one uncached production query would make that path circular. The pinned
  # SQLx CLI therefore applies only the transactional baseline first. After
  # prepare/check, the product preflight recognises that exact post-SQLx,
  # pre-epoch-stamp crash boundary and the normal migrator completes it.
  if [ "$db_test_task" = sqlx-prepare ]; then
    sqlx_library_version=$(awk '
      $0 == "name = \"sqlx\"" { package = 1; next }
      package && $1 == "version" { gsub(/\"/, "", $3); print $3; exit }
    ' Cargo.lock)
    sqlx_cli_banner=$(cargo sqlx --version)
    [ -n "$sqlx_library_version" ] \
      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] || {
        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2
        exit 69
      }
    env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE \
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file \
          cargo sqlx migrate run --no-dotenv \
            --source crates/synveda-store/migrations
    env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE \
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file \
          cargo sqlx prepare --no-dotenv --workspace -- --all-targets
    env -u SYNVEDA_DB_TEST_SECRETS_DIR -u SQLX_OFFLINE \
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file \
          cargo sqlx prepare --check --no-dotenv --workspace -- --all-targets
    unset sqlx_library_version sqlx_cli_banner
  fi

  if [ "$db_test_task" != authority-fingerprints ]; then
    run_main_database_preflight "$main_witness_file"
    for _ in 1 2; do
      env -u SYNVEDA_DB_TEST_SECRETS_DIR \
        SQLX_OFFLINE=true \
        DATABASE_URL_FILE=$main_migrator_file \
        SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
          cargo run -q -p synveda-cli --bin synveda -- db migrate
    done
    echo "db-test: fast exact-role database fixture bootstrapped and migrated"
  fi

  status=0
  case "$db_test_task" in
    authority-fingerprints|sqlx-prepare) ;;
    demo)
      demo_script=$1
      shift
      case "$demo_script" in
        demos/*.sh|./demos/*.sh|"$PWD"/demos/*.sh) ;;
        *)
          echo "db-test: demo task accepts only a repository demos/*.sh script" >&2
          status=64
          ;;
      esac
      if [ "$status" -eq 0 ] && { [ -L "$demo_script" ] || [ ! -f "$demo_script" ]; }; then
        echo "db-test: demo script must be a regular non-symlink file" >&2
        status=64
      fi
      if [ "$status" -eq 0 ]; then
        env -u SYNVEDA_DB_TEST_SECRETS_DIR \
          SYNVEDA_EXACT_ROLE_DEMO=1 \
          SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
          SQLX_OFFLINE=true \
          SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
          SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file \
          SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
            scripts/cargo-with-database-url-file sh "$demo_script" "$@" || status=$?
      fi
      ;;
    product-evaluation)
      if [ "$#" -ne 0 ]; then
        echo "db-test: product-evaluation takes no cargo-test arguments" >&2
        status=2
      else
        env -u SYNVEDA_DB_TEST_SECRETS_DIR \
          SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
          SQLX_OFFLINE=true \
          SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
          SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
            scripts/cargo-with-database-url-file node scripts/product-evaluation.mjs \
            || status=$?
      fi
      ;;
    evaluation)
      env -u SYNVEDA_DB_TEST_SECRETS_DIR \
        SYNVEDA_EVAL_EXACT_DATABASE=1 \
        SYNVEDA_EVAL_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
        SYNVEDA_EVAL_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
        SYNVEDA_EVAL_WORKER_DATABASE_URL_FILE=$main_worker_file \
        SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
        SYNVEDA_DATABASE_REQUIRED_PEER=keycloak \
          sh evals/run.sh "$@" || status=$?
      ;;
    longmemeval-evaluation)
      env -u SYNVEDA_DB_TEST_SECRETS_DIR \
        SYNVEDA_EVAL_EXACT_DATABASE=1 \
        SYNVEDA_EVAL_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
        SYNVEDA_EVAL_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
        SYNVEDA_EVAL_WORKER_DATABASE_URL_FILE=$main_worker_file \
        SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
        SYNVEDA_DATABASE_REQUIRED_PEER=keycloak \
          sh evals/run-longmemeval.sh "$@" || status=$?
      ;;
  esac
  [ "$status" -eq 0 ] || exit "$status"

  if [ "${KEEP_TEST_DB:-}" = 1 ]; then
    trap - EXIT
    echo "db-test: passed; retained isolated Compose project $project"
    echo "db-test: private state is in $state_dir (mode 0700; contains credentials)"
    exit 0
  fi
  cleanup_successful_fixture
  echo "db-test: passed; fast exact-role database volume removed"
  exit 0
fi

enable_hostile_database_logging() {
  service=$1
  compose exec -T "$service" \
    psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
alter system set log_statement = 'all';
alter system set log_min_messages = 'debug5';
alter system set log_min_error_statement = 'debug5';
alter system set log_error_verbosity = 'verbose';
alter system set log_min_duration_statement = 0;
alter system set log_min_duration_sample = 0;
alter system set log_statement_sample_rate = 1;
alter system set log_transaction_sample_rate = 1;
alter system set log_parameter_max_length = -1;
alter system set log_parameter_max_length_on_error = -1;
alter system set debug_print_parse = on;
alter system set debug_print_rewritten = on;
alter system set debug_print_plan = on;
select pg_catalog.pg_reload_conf();
SQL
  observed=$(compose exec -T "$service" \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select concat_ws(':', current_setting('log_min_messages'), current_setting('log_min_error_statement'), current_setting('log_error_verbosity'), current_setting('log_statement'), current_setting('log_min_duration_statement'), current_setting('log_min_duration_sample'), current_setting('log_statement_sample_rate'), current_setting('log_transaction_sample_rate'), current_setting('log_parameter_max_length'), current_setting('log_parameter_max_length_on_error'), current_setting('debug_print_parse'), current_setting('debug_print_rewritten'), current_setting('debug_print_plan'))")
  [ "$observed" = "debug5:debug5:verbose:all:0:0:1:1:-1:-1:on:on:on" ] || {
    echo "db-test: hostile PostgreSQL logging controls did not converge" >&2
    return 1
  }
}

disable_hostile_database_logging() {
  service=$1
  compose exec -T "$service" \
    psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
alter system reset log_statement;
alter system reset log_min_messages;
alter system reset log_min_error_statement;
alter system reset log_error_verbosity;
alter system reset log_min_duration_statement;
alter system reset log_min_duration_sample;
alter system reset log_statement_sample_rate;
alter system reset log_transaction_sample_rate;
alter system reset log_parameter_max_length;
alter system reset log_parameter_max_length_on_error;
alter system reset debug_print_parse;
alter system reset debug_print_rewritten;
alter system reset debug_print_plan;
select pg_catalog.pg_reload_conf();
SQL
}

assert_credential_server_log_clean() {
  evidence_file=$1
  assert_database_secrets_absent "$evidence_file"
  if LC_ALL=C grep -Fq 'SCRAM-SHA-256$' "$evidence_file"; then
    echo "db-test: PostgreSQL verifier entered captured server logs" >&2
    return 1
  fi
  if LC_ALL=C grep -Eiq \
      'alter role [a-z0-9_]+ with login inherit password' "$evidence_file"; then
    echo "db-test: password-bearing ALTER ROLE entered captured server logs" >&2
    return 1
  fi
}

enable_hostile_database_logging postgres-main

# Exercise a real server-side failure after COPY and dynamic password use while
# all thirteen standard PostgreSQL 17 logger paths are globally hostile. The
# production session contract must keep the unique COPY value out of client and
# server evidence, and the failed transaction must leave no probe role.
post_copy_stdout=$state_dir/post-copy-failure.stdout
post_copy_stderr=$state_dir/post-copy-failure.stderr
post_copy_server_log=$state_dir/post-copy-failure.postgres.log
private_evidence_file "$post_copy_stdout"
private_evidence_file "$post_copy_stderr"
private_evidence_file "$post_copy_server_log"
if compose exec -T postgres-main \
    psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    > "$post_copy_stdout" 2> "$post_copy_stderr" <<'SQL'
\o /dev/null
\set VERBOSITY terse
\set SHOW_CONTEXT never
set jit = off;
\i /usr/local/share/synveda/credential-log-contract.sql
begin;
create role cpr45_credential_log_probe nologin;
create temporary table pg_temp.cpr45_credential_log_probe (
  secret text not null
) using heap on commit drop;
\copy pg_temp.cpr45_credential_log_probe(secret) from stdin
cpr45-post-copy-server-log-sentinel
\.
do $credential$
declare
  probe_password text;
begin
  select secret into strict probe_password
    from pg_temp.cpr45_credential_log_probe;
  begin
    execute format(
      'alter role cpr45_credential_log_probe with password %L',
      probe_password
    );
    raise exception using message = 'controlled post-COPY probe failure';
  exception when query_canceled or assert_failure or others then
    raise exception using
      errcode = 'P0001',
      message = 'controlled post-COPY credential refusal';
  end;
end
$credential$;
SQL
then
  echo "db-test: controlled post-COPY credential failure unexpectedly succeeded" >&2
  exit 1
fi
LC_ALL=C grep -Fq 'controlled post-COPY credential refusal' "$post_copy_stderr" || {
  echo "db-test: controlled post-COPY failure missed the sanitised refusal" >&2
  exit 1
}
compose logs --no-color postgres-main > "$post_copy_server_log" 2>&1
if LC_ALL=C grep -Fq 'cpr45-post-copy-server-log-sentinel' \
    "$post_copy_stdout" "$post_copy_stderr" "$post_copy_server_log"; then
  echo "db-test: post-COPY credential entered captured evidence" >&2
  exit 1
fi
assert_credential_server_log_clean "$post_copy_server_log"
post_copy_role_count=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select count(*) from pg_catalog.pg_roles where rolname = 'cpr45_credential_log_probe'")
[ "$post_copy_role_count" = 0 ] || {
  echo "db-test: failed post-COPY probe retained its role" >&2
  exit 1
}
rm -f "$post_copy_stdout" "$post_copy_stderr" "$post_copy_server_log"
unset post_copy_stdout post_copy_stderr post_copy_server_log post_copy_role_count

sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | cut -d ' ' -f 1
  else
    echo "db-test: sha256sum or shasum is required for catalog evidence" >&2
    return 69
  fi
}

catalog_fingerprint() {
  service=$1
  database_name=$2
  global_digest_file=$state_dir/catalog-global.sha256
  local_digest_file=$state_dir/catalog-local.sha256
  : > "$global_digest_file"
  : > "$local_digest_file"
  chmod 600 "$global_digest_file" "$local_digest_file"
  if compose exec -T "$service" \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      > "$global_digest_file" <<'SQL'
with protected_roles as (
  select role.oid, role.rolname
    from pg_catalog.pg_roles role
   where role.rolname in (
     'synveda_owner', 'synveda_app', 'synveda_migrator',
     'synveda_gateway', 'synveda_worker', 'keycloak'
   )
), role_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    role.oid,
    role.rolname,
    role.rolsuper,
    role.rolinherit,
    role.rolcreaterole,
    role.rolcreatedb,
    role.rolcanlogin,
    role.rolreplication,
    role.rolbypassrls,
    role.rolconnlimit,
    role.rolvaliduntil,
    pg_catalog.encode(
      pg_catalog.sha256(
        pg_catalog.convert_to(coalesce(role.rolpassword, ''), 'UTF8')
      ),
      'hex'
    )
  ) order by role.rolname collate "C"), '[]'::jsonb) as value
  from pg_catalog.pg_authid role
  where role.oid in (select protected_roles.oid from protected_roles)
), membership_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    membership.oid,
    membership.roleid,
    membership.member,
    membership.grantor,
    membership.admin_option,
    membership.inherit_option,
    membership.set_option
  ) order by membership.oid), '[]'::jsonb) as value
  from pg_catalog.pg_auth_members membership
  where membership.roleid in (select protected_roles.oid from protected_roles)
     or membership.member in (select protected_roles.oid from protected_roles)
     or membership.grantor in (select protected_roles.oid from protected_roles)
), setting_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    settings.setdatabase,
    settings.setrole,
    settings.setconfig
  ) order by settings.setdatabase, settings.setrole), '[]'::jsonb) as value
  from pg_catalog.pg_db_role_setting settings
  where settings.setrole in (select protected_roles.oid from protected_roles)
     or settings.setdatabase in (
       select database.oid
         from pg_catalog.pg_database database
        where database.datname in (
          'synveda', 'keycloak', 'postgres', 'template0', 'template1'
        )
     )
), default_acl_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    defaults.oid,
    defaults.defaclrole,
    defaults.defaclnamespace,
    defaults.defaclobjtype,
    defaults.defaclacl
  ) order by defaults.oid), '[]'::jsonb) as value
  from pg_catalog.pg_default_acl defaults
  where defaults.defaclrole in (select protected_roles.oid from protected_roles)
     or exists (
       select 1
         from pg_catalog.aclexplode(defaults.defaclacl) acl
        where acl.grantee in (select protected_roles.oid from protected_roles)
     )
), database_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    database.oid,
    database.datname,
    database.datdba,
    database.encoding,
    database.datlocprovider,
    database.datistemplate,
    database.datallowconn,
    database.dathasloginevt,
    database.datconnlimit,
    database.dattablespace,
    database.datcollate,
    database.datctype,
    database.datlocale,
    database.daticurules,
    database.datcollversion,
    database.datacl
  ) order by database.datname collate "C"), '[]'::jsonb) as value
  from pg_catalog.pg_database database
  where database.datname in ('synveda', 'keycloak', 'postgres', 'template0', 'template1')
), shared_dependency_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    dependency.dbid,
    dependency.classid,
    dependency.objid,
    dependency.objsubid,
    dependency.refclassid,
    dependency.refobjid,
    dependency.deptype
  ) order by dependency.dbid, dependency.classid, dependency.objid,
             dependency.objsubid, dependency.refclassid,
             dependency.refobjid, dependency.deptype), '[]'::jsonb) as value
  from pg_catalog.pg_shdepend dependency
  where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
    and dependency.refobjid in (select protected_roles.oid from protected_roles)
), global_acl_rows as (
  select jsonb_build_array(
           'largeobject', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ) as value,
         concat_ws(':', 'largeobject', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable) as sort_key
    from pg_catalog.pg_largeobject_metadata object,
         lateral pg_catalog.aclexplode(object.lomacl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
  union all
  select jsonb_build_array(
           'foreign-data-wrapper', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'foreign-data-wrapper', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_foreign_data_wrapper object,
         lateral pg_catalog.aclexplode(object.fdwacl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
  union all
  select jsonb_build_array(
           'foreign-server', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'foreign-server', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_foreign_server object,
         lateral pg_catalog.aclexplode(object.srvacl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
  union all
  select jsonb_build_array(
           'language', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'language', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_language object,
         lateral pg_catalog.aclexplode(object.lanacl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
  union all
  select jsonb_build_array(
           'tablespace', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'tablespace', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_tablespace object,
         lateral pg_catalog.aclexplode(object.spcacl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
  union all
  select jsonb_build_array(
           'parameter', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'parameter', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_parameter_acl object,
         lateral pg_catalog.aclexplode(object.paracl) acl
   where acl.grantee = 0
      or acl.grantee in (select protected_roles.oid from protected_roles)
), global_acl_state as (
  select coalesce(
    jsonb_agg(global_acl_rows.value order by global_acl_rows.sort_key collate "C"),
    '[]'::jsonb
  ) as value
  from global_acl_rows
), canonical_state as (
  select jsonb_build_object(
    'roles', role_state.value,
    'memberships', membership_state.value,
    'settings', setting_state.value,
    'default-acls', default_acl_state.value,
    'databases', database_state.value,
    'shared-dependencies', shared_dependency_state.value,
    'global-acls', global_acl_state.value
  ) as value
  from role_state, membership_state, setting_state, default_acl_state,
       database_state, shared_dependency_state, global_acl_state
)
select pg_catalog.encode(
         pg_catalog.sha256(pg_catalog.convert_to(canonical_state.value::text, 'UTF8')),
         'hex'
       )
  from canonical_state;
SQL
  then
    :
  else
    echo "db-test: global catalog fingerprint query failed" >&2
    return 1
  fi
  if compose exec -T "$service" \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner \
      --dbname "$database_name" > "$local_digest_file" <<'SQL'
with public_inventory as (
  select concat_ws(':', 'namespace', namespace.oid) as sort_key,
         jsonb_build_array(
           'namespace', namespace.oid, namespace.nspname,
           namespace.nspowner, namespace.nspacl
         ) as value
    from pg_catalog.pg_namespace namespace
   where namespace.nspname = 'public'
      or namespace.nspname <> 'information_schema'
     and namespace.nspname !~ '^pg_'
  union all
  select concat_ws(':', 'class', object.oid),
         jsonb_build_array(
           'class', object.oid, namespace.nspname, object.relname,
           object.relowner, object.relkind, object.relpersistence,
           object.relrowsecurity, object.relforcerowsecurity,
           object.relreplident, object.relispartition, object.reloptions,
           object.relacl,
           case when object.relkind in ('v', 'm')
             then pg_catalog.pg_get_viewdef(object.oid, true) else '' end,
           coalesce(pg_catalog.pg_get_expr(object.relpartbound, object.oid), '')
         )
    from pg_catalog.pg_class object
    join pg_catalog.pg_namespace namespace on namespace.oid = object.relnamespace
   where namespace.nspname = 'public'
  union all
  select concat_ws(':', 'attribute', object.attrelid, object.attnum),
         jsonb_build_array(
           'attribute', object.attrelid, object.attnum, object.attname,
           object.atttypid, object.atttypmod, object.attnotnull,
           object.attidentity, object.attgenerated, object.attcollation,
           object.attacl,
           coalesce(
             pg_catalog.pg_get_expr(attribute_default.adbin, attribute_default.adrelid), ''
           )
         )
    from pg_catalog.pg_attribute object
    join pg_catalog.pg_class relation on relation.oid = object.attrelid
    join pg_catalog.pg_namespace namespace on namespace.oid = relation.relnamespace
    left join pg_catalog.pg_attrdef attribute_default
      on attribute_default.adrelid = object.attrelid
     and attribute_default.adnum = object.attnum
   where namespace.nspname = 'public'
     and object.attnum > 0
     and not object.attisdropped
  union all
  select concat_ws(':', 'routine', routine.oid),
         jsonb_build_array(
           'routine', routine.oid, namespace.nspname, routine.proname,
           pg_catalog.pg_get_function_identity_arguments(routine.oid),
           routine.proowner, language.lanname, routine.prokind,
           routine.prosecdef, routine.proleakproof, routine.provolatile,
           routine.proparallel, routine.proisstrict, routine.proretset,
           routine.prorettype, routine.proconfig, routine.proacl,
           coalesce(routine.prosrc, ''),
           coalesce(routine.probin, ''),
           coalesce(routine.prosqlbody::text, ''),
           case when routine.prokind in ('f', 'p')
             then pg_catalog.pg_get_functiondef(routine.oid)
             else ''
           end
         )
    from pg_catalog.pg_proc routine
    join pg_catalog.pg_namespace namespace on namespace.oid = routine.pronamespace
    join pg_catalog.pg_language language on language.oid = routine.prolang
   where namespace.nspname = 'public'
  union all
  select concat_ws(':', 'type', data_type.oid),
         jsonb_build_array(
           'type', data_type.oid, data_type.typname, data_type.typowner, data_type.typacl
         )
    from pg_catalog.pg_type data_type
    join pg_catalog.pg_namespace namespace on namespace.oid = data_type.typnamespace
   where namespace.nspname = 'public'
), public_state as (
  select coalesce(
    jsonb_agg(public_inventory.value order by public_inventory.sort_key collate "C"),
    '[]'::jsonb
  ) as value
  from public_inventory
), local_default_acl_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    defaults.oid,
    defaults.defaclrole,
    defaults.defaclnamespace,
    defaults.defaclobjtype,
    defaults.defaclacl
  ) order by defaults.oid), '[]'::jsonb) as value
  from pg_catalog.pg_default_acl defaults
), local_acl_rows as (
  select jsonb_build_array(
           'largeobject', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ) as value,
         concat_ws(':', 'largeobject', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable) as sort_key
    from pg_catalog.pg_largeobject_metadata object,
         lateral pg_catalog.aclexplode(object.lomacl) acl
  union all
  select jsonb_build_array(
           'foreign-data-wrapper', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'foreign-data-wrapper', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_foreign_data_wrapper object,
         lateral pg_catalog.aclexplode(object.fdwacl) acl
  union all
  select jsonb_build_array(
           'foreign-server', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'foreign-server', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_foreign_server object,
         lateral pg_catalog.aclexplode(object.srvacl) acl
  union all
  select jsonb_build_array(
           'language', object.oid, acl.grantor, acl.grantee,
           acl.privilege_type, acl.is_grantable
         ),
         concat_ws(':', 'language', object.oid, acl.grantor, acl.grantee,
                   acl.privilege_type, acl.is_grantable)
    from pg_catalog.pg_language object,
         lateral pg_catalog.aclexplode(object.lanacl) acl
), local_acl_state as (
  select coalesce(
    jsonb_agg(local_acl_rows.value order by local_acl_rows.sort_key collate "C"),
    '[]'::jsonb
  ) as value
  from local_acl_rows
), extension_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    extension.oid,
    extension.extname,
    extension.extowner,
    extension.extnamespace,
    extension.extrelocatable,
    extension.extversion,
    extension.extconfig,
    extension.extcondition
  ) order by extension.extname collate "C"), '[]'::jsonb) as value
  from pg_catalog.pg_extension extension
), extension_member_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    extension.extname,
    dependency.classid,
    dependency.objid,
    dependency.objsubid,
    dependency.refclassid,
    dependency.refobjid,
    dependency.refobjsubid,
    dependency.deptype
  ) order by extension.extname collate "C", dependency.classid,
             dependency.objid, dependency.objsubid), '[]'::jsonb) as value
  from pg_catalog.pg_depend dependency
  join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
  where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
    and dependency.deptype = 'e'
), local_shared_dependency_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    dependency.dbid,
    dependency.classid,
    dependency.objid,
    dependency.objsubid,
    dependency.refclassid,
    dependency.refobjid,
    dependency.deptype
  ) order by dependency.classid, dependency.objid, dependency.objsubid,
             dependency.refclassid, dependency.refobjid,
             dependency.deptype), '[]'::jsonb) as value
  from pg_catalog.pg_shdepend dependency
  join pg_catalog.pg_database database on database.datname = current_database()
  where dependency.dbid = database.oid
), trigger_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    trigger.oid,
    namespace.nspname,
    relation.relname,
    trigger.tgname,
    trigger.tgparentid,
    trigger.tgfoid,
    trigger.tgtype,
    trigger.tgenabled,
    trigger.tgisinternal,
    trigger.tgconstrrelid,
    trigger.tgconstrindid,
    trigger.tgconstraint,
    trigger.tgdeferrable,
    trigger.tginitdeferred,
    trigger.tgnargs,
    pg_catalog.encode(trigger.tgargs, 'hex'),
    trigger.tgoldtable,
    trigger.tgnewtable,
    pg_catalog.pg_get_triggerdef(trigger.oid, true)
  ) order by namespace.nspname collate "C", relation.relname collate "C",
             trigger.tgname collate "C"), '[]'::jsonb) as value
  from pg_catalog.pg_trigger trigger
  join pg_catalog.pg_class relation on relation.oid = trigger.tgrelid
  join pg_catalog.pg_namespace namespace on namespace.oid = relation.relnamespace
  where namespace.nspname = 'public'
), event_trigger_state as (
  select coalesce(jsonb_agg(jsonb_build_array(
    event_trigger.oid,
    event_trigger.evtname,
    event_trigger.evtevent,
    event_trigger.evtowner,
    event_trigger.evtfoid,
    event_trigger.evtenabled,
    event_trigger.evttags
  ) order by event_trigger.evtname collate "C"), '[]'::jsonb) as value
  from pg_catalog.pg_event_trigger event_trigger
), canonical_state as (
  select jsonb_build_object(
    'public', public_state.value,
    'default-acls', local_default_acl_state.value,
    'local-global-acls', local_acl_state.value,
    'extensions', extension_state.value,
    'extension-members', extension_member_state.value,
    'local-shared-dependencies', local_shared_dependency_state.value,
    'triggers', trigger_state.value,
    'event-triggers', event_trigger_state.value
  ) as value
  from public_state, local_default_acl_state, local_acl_state,
       extension_state, extension_member_state, local_shared_dependency_state,
       trigger_state, event_trigger_state
)
select pg_catalog.encode(
         pg_catalog.sha256(pg_catalog.convert_to(canonical_state.value::text, 'UTF8')),
         'hex'
       )
  from canonical_state;
SQL
  then
    :
  else
    echo "db-test: local catalog fingerprint query failed" >&2
    return 1
  fi
  global_digest=
  local_digest=
  IFS= read -r global_digest < "$global_digest_file" || [ -n "$global_digest" ]
  IFS= read -r local_digest < "$local_digest_file" || [ -n "$local_digest" ]
  case "$global_digest:$local_digest" in
    *[!0-9a-f:]*|:*|*:|*:*:*)
      echo "db-test: catalog fingerprint did not produce two SHA-256 digests" >&2
      return 1
      ;;
    *) ;;
  esac
  [ "${#global_digest}" -eq 64 ] && [ "${#local_digest}" -eq 64 ] || {
    echo "db-test: catalog fingerprint digest length was invalid" >&2
    return 1
  }
  printf '%s\n%s\n' "$global_digest" "$local_digest" | sha256_stream
}

assert_fresh_database_catalog() {
  service=$1
  fresh_state=$(compose exec -T "$service" \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select (select count(*) from pg_catalog.pg_roles where rolname in ('synveda_app','synveda_migrator','synveda_gateway','synveda_worker','keycloak'))::text || ':' || (select count(*) from pg_catalog.pg_database where datname in ('synveda','keycloak'))::text")
  [ "$fresh_state" = "0:0" ] || {
    echo "db-test: refused bootstrap changed the fresh protected catalog" >&2
    return 1
  }
}

exercise_target_event_trigger_refusal() {
  local database_name=$1
  local bootstrap_service=$2
  local display_name=$3
  local stdout_file=$state_dir/$database_name-event-trigger.stdout
  local stderr_file=$state_dir/$database_name-event-trigger.stderr
  local before after effect_count bootstrap_accepted quarantine_shape
  compose exec -T postgres-main \
    psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner \
      --dbname "$database_name" <<'SQL'
create table public.cpr45_target_ddl_effect (
  observed_at timestamptz not null default statement_timestamp()
);
create function public.cpr45_target_ddl_probe()
returns event_trigger
language plpgsql
as $probe$
begin
  insert into public.cpr45_target_ddl_effect default values;
end
$probe$;
create event trigger cpr45_target_ddl_probe
on ddl_command_start
when tag in ('CREATE TABLE')
execute function public.cpr45_target_ddl_probe();
SQL
  private_evidence_file "$stdout_file"
  private_evidence_file "$stderr_file"
  before=$(catalog_fingerprint postgres-main "$database_name")
  if compose run --rm --no-deps "$bootstrap_service" \
      > "$stdout_file" 2> "$stderr_file"; then
    bootstrap_accepted=true
  else
    bootstrap_accepted=false
  fi
  [ "$bootstrap_accepted" = false ] || {
    echo "db-test: $display_name bootstrap accepted a target event trigger" >&2
    return 1
  }
  LC_ALL=C grep -Fq \
    "database-bootstrap: $display_name local authority preflight was refused" \
    "$stderr_file" || {
      echo "db-test: $display_name target event trigger missed the local guard" >&2
      return 1
    }
  if [ "$database_name" = keycloak ]; then
    # A terminal Keycloak local-authority refusal is intentionally not
    # read-only at the global admission boundary: quarantine closes the
    # database, disables LOGIN and drains every admitted session. Prove that
    # fail-closed state, then act as the explicit operator to restore only
    # admission so the unchanged target-local catalog can be fingerprinted.
    assert_keycloak_admission_empty "$display_name event-trigger quarantine"
    quarantine_shape=$(compose exec -T postgres-main \
      psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
        --command "select (not database.datallowconn and not role.rolcanlogin and not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid where member.rolname = session_user and granted.rolname = 'keycloak'))::text from pg_catalog.pg_database database join pg_catalog.pg_roles role on role.oid = database.datdba where database.datname = 'keycloak' and role.rolname = 'keycloak'")
    [ "$quarantine_shape" = true ] || {
      echo "db-test: Keycloak event-trigger refusal did not close and drain admission" >&2
      return 1
    }
    compose exec -T postgres-main \
      psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
        --command 'alter role keycloak login; alter database keycloak allow_connections true'
  fi
  after=$(catalog_fingerprint postgres-main "$database_name")
  [ "$before" = "$after" ] || {
    echo "db-test: refused $display_name target event trigger changed catalog state" >&2
    return 1
  }
  effect_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner \
      --dbname "$database_name" \
      --command 'select count(*) from public.cpr45_target_ddl_effect')
  [ "$effect_count" = 0 ] || {
    echo "db-test: $display_name target DDL ran before event-trigger refusal" >&2
    return 1
  }
  assert_database_secrets_absent "$stdout_file"
  assert_database_secrets_absent "$stderr_file"
  compose exec -T postgres-main \
    psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner \
      --dbname "$database_name" \
      --command 'drop event trigger cpr45_target_ddl_probe; drop function public.cpr45_target_ddl_probe(); drop table public.cpr45_target_ddl_effect'
  rm -f "$stdout_file" "$stderr_file"
}

# CREATE TEMP TABLE is DDL and therefore cannot precede the event-trigger
# refusal. A hostile bootstrap-database trigger would otherwise commit its DML
# effect before a later guard failed.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
create table public.cpr45_bootstrap_ddl_effect (
  observed_at timestamptz not null default statement_timestamp()
);
create function public.cpr45_bootstrap_ddl_probe()
returns event_trigger
language plpgsql
as $probe$
begin
  insert into public.cpr45_bootstrap_ddl_effect default values;
end
$probe$;
create event trigger cpr45_bootstrap_ddl_probe
on ddl_command_start
when tag in ('CREATE TABLE')
execute function public.cpr45_bootstrap_ddl_probe();
SQL
stdout_file=$state_dir/bootstrap-event-trigger.stdout
stderr_file=$state_dir/bootstrap-event-trigger.stderr
server_log_file=$state_dir/bootstrap-event-trigger.postgres.log
private_evidence_file "$stdout_file"
private_evidence_file "$stderr_file"
private_evidence_file "$server_log_file"
fresh_before=$(catalog_fingerprint postgres-main postgres)
if compose run --rm --no-deps database-bootstrap-main \
    > "$stdout_file" 2> "$stderr_file"; then
  bootstrap_accepted=true
else
  bootstrap_accepted=false
fi
[ "$bootstrap_accepted" = false ] || {
  echo "db-test: bootstrap accepted a bootstrap-database event trigger" >&2
  exit 1
}
LC_ALL=C grep -Fq \
  'database-bootstrap: Synveda role or database convergence was refused' \
  "$stderr_file" || {
    echo "db-test: bootstrap event trigger missed the pre-DDL authority guard" >&2
    exit 1
  }
fresh_after=$(catalog_fingerprint postgres-main postgres)
[ "$fresh_before" = "$fresh_after" ] || {
  echo "db-test: refused bootstrap-database event trigger changed catalog state" >&2
  exit 1
}
ddl_effect_count=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'select count(*) from public.cpr45_bootstrap_ddl_effect')
[ "$ddl_effect_count" = 0 ] || {
  echo "db-test: bootstrap created DDL before refusing the event trigger" >&2
  exit 1
}
compose logs --no-color postgres-main > "$server_log_file" 2>&1
assert_database_secrets_absent "$stdout_file"
assert_database_secrets_absent "$stderr_file"
assert_database_secrets_absent "$server_log_file"
assert_fresh_database_catalog postgres-main
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'drop event trigger cpr45_bootstrap_ddl_probe; drop function public.cpr45_bootstrap_ddl_probe(); drop table public.cpr45_bootstrap_ddl_effect'
rm -f "$stdout_file" "$stderr_file" "$server_log_file"
unset stdout_file stderr_file server_log_file fresh_before fresh_after \
  bootstrap_accepted ddl_effect_count fresh_state

# Password content remains unread until every locked, read-only authority
# guard has passed. Exercise every mounted role password on a fresh cluster and
# prove malformed content neither reaches output nor leaves a role/database.
for credential_case in \
    synveda_migrator_password:database-bootstrap-main \
    synveda_gateway_password:database-bootstrap-main \
    synveda_worker_password:database-bootstrap-main \
    keycloak_database_password:keycloak-password-validator; do
  credential_name=${credential_case%%:*}
  bootstrap_service=${credential_case#*:}
  credential_candidate=$state_dir/$credential_name.malformed
  stdout_file=$state_dir/$credential_name.stdout
  stderr_file=$state_dir/$credential_name.stderr
  printf 'cpr45:malformed-password-sentinel\n' > "$credential_candidate"
  chmod 600 "$credential_candidate"
  private_evidence_file "$stdout_file"
  private_evidence_file "$stderr_file"
  fresh_before=$(catalog_fingerprint postgres-main postgres)
  if [ "$credential_name" = keycloak_database_password ]; then
    # Compose implementations may cache a project's secret bind at first use.
    # Run the exact image's production content validator in an isolated,
    # immutable container so this case cannot silently reuse the valid secret.
    if "$docker_bin" run --rm --network none --read-only \
        --user "$SYNVEDA_DB_TEST_UID:$SYNVEDA_DB_TEST_GID" \
        --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 64 \
        --tmpfs /tmp:rw,noexec,nosuid,nodev,mode=1777,size=1048576 \
        --mount "type=bind,source=$secret_dir/postgres_owner_password,target=/run/secrets/postgres_bootstrap_password,readonly" \
        --mount "type=bind,source=$credential_candidate,target=/run/secrets/keycloak_database_password,readonly" \
        --entrypoint /usr/local/bin/synveda-database-bootstrap \
        "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" validate-keycloak-password \
        > "$stdout_file" 2> "$stderr_file"; then
      bootstrap_accepted=true
    else
      bootstrap_accepted=false
    fi
  elif compose run --rm --no-deps \
      --volume "$credential_candidate:/run/secrets/$credential_name:ro" \
      "$bootstrap_service" > "$stdout_file" 2> "$stderr_file"; then
    bootstrap_accepted=true
  else
    bootstrap_accepted=false
  fi
  [ "$bootstrap_accepted" = false ] || {
    echo "db-test: bootstrap accepted malformed $credential_name" >&2
    exit 1
  }
  [ ! -s "$stdout_file" ] || {
    echo "db-test: malformed $credential_name produced stdout" >&2
    exit 1
  }
  if LC_ALL=C grep -Fq 'cpr45:malformed-password-sentinel' \
      "$stdout_file" "$stderr_file"; then
    echo "db-test: malformed database credential entered captured evidence" >&2
    exit 1
  fi
  LC_ALL=C grep -Fq \
    "database-bootstrap: $credential_name contains unsupported bytes" \
    "$stderr_file" || {
      echo "db-test: malformed $credential_name missed the content guard" >&2
      exit 1
    }
  assert_database_secrets_absent "$stdout_file"
  assert_database_secrets_absent "$stderr_file"
  fresh_after=$(catalog_fingerprint postgres-main postgres)
  [ "$fresh_before" = "$fresh_after" ] || {
    echo "db-test: malformed $credential_name changed protected catalog state" >&2
    exit 1
  }
  assert_fresh_database_catalog postgres-main
  rm -f "$credential_candidate" "$stdout_file" "$stderr_file"
done
unset credential_case credential_name bootstrap_service credential_candidate \
  stdout_file stderr_file \
  bootstrap_accepted fresh_before \
  fresh_after fresh_state

# The reference topology uses one PostgreSQL server but five independent
# principals. Refuse any credential collision in the production validator
# before a database connection or persistent mutation is possible.
credential_collision=$state_dir/database-credential-collision
stdout_file=$state_dir/database-credential-collision.stdout
stderr_file=$state_dir/database-credential-collision.stderr
# Generated sources are LF-terminated. Remove only that terminator so the live
# negative proves comparison is over PostgreSQL's effective value, not bytes.
tr -d '\n' < "$secret_dir/synveda_migrator_password" > "$credential_collision"
chmod 600 "$credential_collision"
private_evidence_file "$stdout_file"
private_evidence_file "$stderr_file"
fresh_before=$(catalog_fingerprint postgres-main postgres)
if "$docker_bin" run --rm --network none --read-only \
    --user "$SYNVEDA_DB_TEST_UID:$SYNVEDA_DB_TEST_GID" \
    --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 64 \
    --tmpfs /tmp:rw,noexec,nosuid,nodev,mode=1777,size=1048576 \
    --env SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD=true \
    --mount "type=bind,source=$secret_dir/postgres_owner_password,target=/run/secrets/postgres_bootstrap_password,readonly" \
    --mount "type=bind,source=$secret_dir/synveda_migrator_password,target=/run/secrets/synveda_migrator_password,readonly" \
    --mount "type=bind,source=$credential_collision,target=/run/secrets/synveda_gateway_password,readonly" \
    --mount "type=bind,source=$secret_dir/synveda_worker_password,target=/run/secrets/synveda_worker_password,readonly" \
    --mount "type=bind,source=$secret_dir/keycloak_database_password,target=/run/secrets/keycloak_database_password,readonly" \
    --entrypoint /usr/local/bin/synveda-database-bootstrap \
    "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" validate-synveda-passwords \
    > "$stdout_file" 2> "$stderr_file"; then
  echo "db-test: database bootstrap accepted reused credentials" >&2
  exit 1
fi
[ ! -s "$stdout_file" ] || {
  echo "db-test: reused database credential produced stdout" >&2
  exit 1
}
[ "$(cat "$stderr_file")" = \
  'database-bootstrap: database credentials must be pairwise distinct' ] || {
  echo "db-test: reused database credential missed the content-free refusal" >&2
  exit 1
}
assert_database_secrets_absent "$stdout_file"
assert_database_secrets_absent "$stderr_file"
fresh_after=$(catalog_fingerprint postgres-main postgres)
[ "$fresh_before" = "$fresh_after" ] || {
  echo "db-test: reused database credential changed protected catalog state" >&2
  exit 1
}
assert_fresh_database_catalog postgres-main
rm -f "$credential_collision" "$stdout_file" "$stderr_file"
unset credential_collision stdout_file stderr_file fresh_before fresh_after fresh_state

# The same deployment bootstrap owns both fixtures. Running the main
# convergence twice proves its ordinary idempotent re-run before any migration.
compose run --rm --no-deps database-bootstrap-main
compose run --rm --no-deps database-bootstrap-main

# A pre-existing protected role must not hide ownership in another database
# merely because its intended target does not exist yet. The digest covers the
# canonical logical mutation surface; it never prints a password verifier.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create role keycloak nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1; create schema cpr45_keycloak_dependency authorization keycloak"
main_before=$(catalog_fingerprint postgres-main postgres)
if main_refusal=$(compose run --rm --no-deps keycloak-database-bootstrap-main 2>&1); then
  echo "db-test: Keycloak bootstrap accepted absent-target shared ownership" >&2
  exit 1
fi
case "$main_refusal" in
  *"database-bootstrap: Keycloak role or database convergence was refused"*) ;;
  *)
    echo "db-test: Keycloak absent-target refusal did not reach the authority guard" >&2
    exit 1
    ;;
esac
unset main_refusal
main_after=$(catalog_fingerprint postgres-main postgres)
[ "$main_before" = "$main_after" ] || {
  echo "db-test: refused Keycloak absent-target bootstrap changed catalog state" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop schema cpr45_keycloak_dependency; drop role keycloak'

# An existing database with the wrong owner is unsafe input, not something the
# convergence job may repair after changing global role state. Prove both
# target branches preserve the same canonical logical catalog mutation surface.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create database keycloak with owner synveda_owner template template0 encoding 'UTF8'"
main_before=$(catalog_fingerprint postgres-main keycloak)
if main_refusal=$(compose run --rm --no-deps keycloak-database-bootstrap-main 2>&1); then
  echo "db-test: Keycloak bootstrap accepted a wrong-owner existing database" >&2
  exit 1
fi
case "$main_refusal" in
  *"database-bootstrap: Keycloak existing database shape was refused"*) ;;
  *)
    echo "db-test: Keycloak wrong-owner refusal did not reach the database-shape guard" >&2
    exit 1
    ;;
esac
unset main_refusal
main_after=$(catalog_fingerprint postgres-main keycloak)
[ "$main_before" = "$main_after" ] || {
  echo "db-test: refused Keycloak bootstrap changed protected catalog state" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop database keycloak with (force)'

# Recreate the first exact Keycloak crash boundary: the ordinary NOLOGIN role
# exists but CREATE DATABASE has not run. Hostile changes to a relaxed role
# dimension must be refused byte-for-byte; the exact self-granted SET row and
# infinite validity left later in the same phase must let the normal Compose
# ordering (Synveda first, Keycloak second) recover.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create role keycloak nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1"

expect_synveda_peer_refusal() {
  peer_case=$1
  peer_before=$(catalog_fingerprint postgres-main postgres)
  if peer_refusal=$(compose run --rm --no-deps database-bootstrap-main 2>&1); then
    echo "db-test: Synveda bootstrap accepted hostile $peer_case Keycloak recovery state" >&2
    return 1
  fi
  case "$peer_refusal" in
    *"database-bootstrap: Synveda role or database convergence was refused"*) ;;
    *)
      echo "db-test: hostile $peer_case Keycloak state missed the Synveda authority guard" >&2
      return 1
      ;;
  esac
  peer_after=$(catalog_fingerprint postgres-main postgres)
  [ "$peer_before" = "$peer_after" ] || {
    echo "db-test: refused $peer_case Keycloak state changed catalog authority" >&2
    return 1
  }
}

compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "alter role keycloak createdb"
expect_synveda_peer_refusal elevated-role
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "alter role keycloak nocreatedb valid until '2030-01-01 00:00:00+00'"
expect_synveda_peer_refusal finite-validity
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "alter role keycloak nologin valid until 'infinity'; grant keycloak to synveda_owner with admin false, inherit true, set true granted by synveda_owner"
compose run --rm --no-deps database-bootstrap-main
role_only_recovery=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not role.rolcanlogin and role.rolvaliduntil is not distinct from 'infinity'::timestamptz and not exists (select 1 from pg_catalog.pg_database where datname = 'keycloak'))::text from pg_catalog.pg_roles role where role.rolname = 'keycloak'")
[ "$role_only_recovery" = true ] || {
  echo "db-test: Synveda ordering changed the exact Keycloak role-only recovery state" >&2
  exit 1
}
compose run --rm --no-deps keycloak-database-bootstrap-main
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'drop database keycloak with (force)'
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'drop role keycloak'
unset peer_case peer_before peer_refusal peer_after role_only_recovery

# An old/open CREATE DATABASE result with a NULL ACL is never repaired: a
# session could already have entered through PUBLIC before a later REVOKE.
# Prove both ordered bootstraps refuse it without catalog mutation.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create role keycloak nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1 valid until 'infinity'; grant keycloak to synveda_owner with admin false, inherit true, set true granted by synveda_owner"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create database keycloak with owner keycloak template template0 encoding 'UTF8'"
keycloak_open_default_state=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (database.datallowconn and database.datacl is null and has_database_privilege('synveda_app', database.oid, 'CONNECT'))::text from pg_catalog.pg_database database where database.datname = 'keycloak'")
[ "$keycloak_open_default_state" = true ] || {
  echo "db-test: Keycloak open/default legacy fixture was not exact" >&2
  exit 1
}
open_default_before=$(catalog_fingerprint postgres-main keycloak)
if open_default_synveda_refusal=$(compose run --rm --no-deps database-bootstrap-main 2>&1); then
  echo "db-test: Synveda bootstrap repaired an open/default Keycloak database" >&2
  exit 1
fi
case "$open_default_synveda_refusal" in
  *"database-bootstrap: Synveda role or database convergence was refused"*) ;;
  *) echo "db-test: Synveda open/default refusal missed the peer authority guard" >&2; exit 1 ;;
esac
if open_default_keycloak_refusal=$(compose run --rm --no-deps keycloak-database-bootstrap-main 2>&1); then
  echo "db-test: Keycloak bootstrap repaired an open/default database" >&2
  exit 1
fi
case "$open_default_keycloak_refusal" in
  *"database-bootstrap: Keycloak existing database shape was refused"*) ;;
  *) echo "db-test: Keycloak open/default refusal missed the shape guard" >&2; exit 1 ;;
esac
open_default_after=$(catalog_fingerprint postgres-main keycloak)
[ "$open_default_before" = "$open_default_after" ] || {
  echo "db-test: open/default database refusal changed catalog state" >&2
  exit 1
}

expect_closed_keycloak_shape_refusal() {
  local label=$1
  local before after refusal
  # The target is deliberately closed. The global digest still covers its
  # database row and every setting scoped to its OID; use the maintenance
  # database only for the independent local half of the fingerprint.
  before=$(catalog_fingerprint postgres-main postgres)
  if refusal=$(compose run --rm --no-deps database-bootstrap-main 2>&1); then
    echo "db-test: Synveda bootstrap accepted $label closed Keycloak state" >&2
    return 1
  fi
  case "$refusal" in
    *"database-bootstrap: Synveda role or database convergence was refused"*) ;;
    *)
      echo "db-test: Synveda $label refusal missed the peer authority guard" >&2
      return 1
      ;;
  esac
  if refusal=$(compose run --rm --no-deps keycloak-database-bootstrap-main 2>&1); then
    echo "db-test: Keycloak bootstrap accepted $label closed database state" >&2
    return 1
  fi
  case "$refusal" in
    *"database-bootstrap: Keycloak existing database shape was refused"*) ;;
    *)
      echo "db-test: Keycloak $label refusal missed the database-shape guard" >&2
      return 1
      ;;
  esac
  after=$(catalog_fingerprint postgres-main postgres)
  [ "$before" = "$after" ] || {
    echo "db-test: $label closed-database refusal changed catalog state" >&2
    return 1
  }
  [ ! -e "$main_authority_dir/keycloak-cluster.json" ] || {
    echo "db-test: $label closed-database refusal published a peer witness" >&2
    return 1
  }
}

# The only CREATE DATABASE interruption state is closed with a NULL ACL. The
# ordered restart may converge roles while it remains closed only when every
# database-scoped setting is absent, no session survived the close and the
# later target-local preflight proves a pristine template-derived envelope.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop database keycloak with (force)'
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "alter role keycloak nologin valid until 'infinity'"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create database keycloak with owner keycloak template template0 encoding 'UTF8' allow_connections false"
keycloak_closed_state=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not database.datallowconn and database.datacl is null and not role.rolcanlogin)::text from pg_catalog.pg_database database join pg_catalog.pg_roles role on role.oid = database.datdba where database.datname = 'keycloak' and role.rolname = 'keycloak'")
[ "$keycloak_closed_state" = true ] || {
  echo "db-test: Keycloak closed-database recovery fixture was not exact" >&2
  exit 1
}
assert_keycloak_admission_empty "Keycloak closed-database recovery fixture"

# A setting for an unrelated role is still target-database state. Both ordered
# bootstraps must reject it, and the expanded fingerprint must observe it.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create role cpr45_keycloak_setting_probe nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1; alter role cpr45_keycloak_setting_probe in database keycloak set search_path = 'pg_catalog'"
keycloak_setting_shape=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select count(*) from pg_catalog.pg_db_role_setting settings join pg_catalog.pg_roles role on role.oid = settings.setrole join pg_catalog.pg_database database on database.oid = settings.setdatabase where role.rolname = 'cpr45_keycloak_setting_probe' and database.datname = 'keycloak'")
[ "$keycloak_setting_shape" = 1 ] || {
  echo "db-test: unprotected Keycloak database-setting fixture was not exact" >&2
  exit 1
}
expect_closed_keycloak_shape_refusal unprotected-setting
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'alter role cpr45_keycloak_setting_probe in database keycloak reset all; drop role cpr45_keycloak_setting_probe'

# Hold a real connection while closing admission. No bootstrap may classify
# that state as the zero-session CREATE boundary.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'alter database keycloak allow_connections true'
keycloak_session_stdout=$state_dir/keycloak-closed-session.stdout
keycloak_session_stderr=$state_dir/keycloak-closed-session.stderr
private_evidence_file "$keycloak_session_stdout"
private_evidence_file "$keycloak_session_stderr"
compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-closed-session \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command 'select pg_catalog.pg_sleep(120)' \
    > "$keycloak_session_stdout" 2> "$keycloak_session_stderr" &
keycloak_session_process=$!
keycloak_session_ready=false
keycloak_session_wait=0
while [ "$keycloak_session_wait" -lt 50 ]; do
  keycloak_session_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select count(*) from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-closed-session'")
  if [ "$keycloak_session_count" = 1 ]; then
    keycloak_session_ready=true
    break
  fi
  keycloak_session_wait=$((keycloak_session_wait + 1))
  sleep 0.1
done
[ "$keycloak_session_ready" = true ] || {
  echo "db-test: retained Keycloak session did not become observable" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'alter database keycloak allow_connections false'
expect_closed_keycloak_shape_refusal active-session
terminated_session=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select pg_catalog.pg_terminate_backend(pid)::text from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-closed-session'")
[ "$terminated_session" = true ] || {
  echo "db-test: retained Keycloak session was not terminated after evidence" >&2
  exit 1
}
wait "$keycloak_session_process" || :
assert_database_secrets_absent "$keycloak_session_stdout"
assert_database_secrets_absent "$keycloak_session_stderr"
rm -f "$keycloak_session_stdout" "$keycloak_session_stderr"

compose run --rm --no-deps database-bootstrap-main
compose run --rm --no-deps keycloak-database-bootstrap-main
keycloak_recovered=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (database.datallowconn and database.datacl is not null and not has_database_privilege('synveda_app', database.oid, 'CONNECT') and not has_database_privilege('synveda_migrator', database.oid, 'CONNECT') and not has_database_privilege('synveda_gateway', database.oid, 'CONNECT') and not has_database_privilege('synveda_worker', database.oid, 'CONNECT'))::text from pg_catalog.pg_database database where database.datname = 'keycloak'")
[ "$keycloak_recovered" = true ] || {
  echo "db-test: Keycloak closed-database recovery did not publish the terminal ACL" >&2
  exit 1
}

# An ordinary terminal database can already have pooled Keycloak sessions. A
# later target-local authority refusal must first close new admission, then
# terminate every admitted session before reporting a successful quarantine.
# Build an otherwise valid Keycloak-owned object before the initial preflight;
# an advisory-lock fence lets the harness add an invalid Synveda ACL only after
# that preflight has passed and the repeated target-local guard is waiting.
terminal_keycloak_content=cpr45-terminal-keycloak-content-sentinel
terminal_keycloak_object=cpr45_terminal_keycloak_probe
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
grant keycloak to synveda_owner
  with admin false, inherit true, set true granted by synveda_owner;
SQL
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak <<'SQL'
set role keycloak;
create table public.cpr45_terminal_keycloak_probe (value text primary key);
\copy public.cpr45_terminal_keycloak_probe(value) from stdin
cpr45-terminal-keycloak-content-sentinel
\.
reset role;
SQL
terminal_session_stdout=$state_dir/keycloak-terminal-session.stdout
terminal_session_stderr=$state_dir/keycloak-terminal-session.stderr
terminal_lock_stdout=$state_dir/keycloak-terminal-lock.stdout
terminal_lock_stderr=$state_dir/keycloak-terminal-lock.stderr
terminal_refusal_stdout=$state_dir/keycloak-terminal-quarantine.stdout
terminal_refusal_stderr=$state_dir/keycloak-terminal-quarantine.stderr
terminal_refusal_server_log=$state_dir/keycloak-terminal-quarantine.postgres.log
for evidence_file in "$terminal_session_stdout" "$terminal_session_stderr" \
    "$terminal_lock_stdout" "$terminal_lock_stderr" \
    "$terminal_refusal_stdout" "$terminal_refusal_stderr" \
    "$terminal_refusal_server_log"; do
  private_evidence_file "$evidence_file"
done
compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-terminal-session \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command 'set role keycloak; select pg_catalog.pg_sleep(120)' \
    > "$terminal_session_stdout" 2> "$terminal_session_stderr" &
terminal_session_process=$!
terminal_session_ready=false
terminal_session_wait=0
while [ "$terminal_session_wait" -lt 50 ]; do
  terminal_session_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select count(*) from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-terminal-session'")
  if [ "$terminal_session_count" = 1 ]; then
    terminal_session_ready=true
    break
  fi
  terminal_session_wait=$((terminal_session_wait + 1))
  sleep 0.1
done
[ "$terminal_session_ready" = true ] || {
  echo "db-test: terminal Keycloak session did not become observable" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'revoke keycloak from synveda_owner granted by synveda_owner'
terminal_membership_absent=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid where member.rolname = 'synveda_owner' and granted.rolname = 'keycloak'))::text")
[ "$terminal_membership_absent" = true ] || {
  echo "db-test: terminal Keycloak session retained owner membership" >&2
  exit 1
}

# Hold only the Keycloak-database target lock. The bootstrap can finish its
# initial preflight and maintenance-database global phase, enter its target
# phase, acquire the preceding target-session ordering lock and publish its
# fixed application marker, then become observably blocked at the exact
# repeated target-local boundary.
compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-quarantine-lock \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command "select pg_catalog.pg_advisory_lock(pg_catalog.hashtext('synveda.compose.bootstrap.keycloak')); select pg_catalog.pg_sleep(120)" \
    > "$terminal_lock_stdout" 2> "$terminal_lock_stderr" &
terminal_lock_process=$!
terminal_lock_ready=false
terminal_lock_wait=0
while [ "$terminal_lock_wait" -lt 100 ]; do
  terminal_lock_observed=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select exists (select 1 from pg_catalog.pg_stat_activity activity join pg_catalog.pg_locks lock on lock.pid = activity.pid where activity.datname = 'keycloak' and activity.application_name = 'cpr45-keycloak-quarantine-lock' and activity.state = 'active' and activity.wait_event = 'PgSleep' and lock.locktype = 'advisory' and lock.database = activity.datid and lock.granted and position('synveda.compose.bootstrap.keycloak' in activity.query) > 0)::text")
  if [ "$terminal_lock_observed" = true ]; then
    terminal_lock_ready=true
    break
  fi
  terminal_lock_wait=$((terminal_lock_wait + 1))
  sleep 0.1
done
[ "$terminal_lock_ready" = true ] || {
  echo "db-test: Keycloak quarantine lock did not become observable" >&2
  exit 1
}

compose run --rm --no-deps keycloak-database-bootstrap-main \
  > "$terminal_refusal_stdout" 2> "$terminal_refusal_stderr" &
terminal_bootstrap_process=$!
terminal_bootstrap_waiting=false
terminal_bootstrap_wait=0
while [ "$terminal_bootstrap_wait" -lt 200 ]; do
  terminal_bootstrap_lock_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select count(*) from pg_catalog.pg_stat_activity activity where activity.datname = 'keycloak' and activity.application_name = 'synveda-keycloak-bootstrap-target' and activity.usename = 'synveda_owner' and activity.backend_type = 'client backend' and activity.state = 'active' and activity.wait_event_type = 'Lock' and activity.wait_event = 'advisory' and (select count(*) from pg_catalog.pg_locks lock where lock.pid = activity.pid and lock.locktype = 'advisory' and lock.database = activity.datid and lock.granted) = 1 and (select count(*) from pg_catalog.pg_locks lock where lock.pid = activity.pid and lock.locktype = 'advisory' and lock.database = activity.datid and not lock.granted) = 1 and (select count(*) from pg_catalog.unnest(pg_catalog.pg_blocking_pids(activity.pid)) blocker(pid) join pg_catalog.pg_stat_activity holder on holder.pid = blocker.pid where holder.datid = activity.datid and holder.application_name = 'cpr45-keycloak-quarantine-lock') = 1")
  if [ "$terminal_bootstrap_lock_count" = 1 ]; then
    terminal_bootstrap_waiting=true
    break
  fi
  terminal_bootstrap_wait=$((terminal_bootstrap_wait + 1))
  sleep 0.1
done
[ "$terminal_bootstrap_waiting" = true ] || {
  echo "db-test: Keycloak bootstrap did not reach the locked target guard" >&2
  exit 1
}
kill -0 "$terminal_bootstrap_process" 2>/dev/null || {
  echo "db-test: Keycloak bootstrap exited before the locked target guard was released" >&2
  exit 1
}

# The first preflight accepted this Keycloak-owned object. Add only a forbidden
# direct Synveda ACL while the bootstrap is fenced, then remove the temporary
# SET ROLE authority before allowing its locked guard to continue.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'grant keycloak to synveda_owner with admin false, inherit true, set true granted by synveda_owner'
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak <<'SQL'
set role keycloak;
grant select on public.cpr45_terminal_keycloak_probe to synveda_app;
reset role;
SQL
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'revoke keycloak from synveda_owner granted by synveda_owner'
terminal_release_shape=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select ((select count(*) from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-terminal-session') = 1 and not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid where member.rolname = 'synveda_owner' and granted.rolname = 'keycloak'))::text")
[ "$terminal_release_shape" = true ] || {
  echo "db-test: terminal Keycloak fence lost its session or retained authority" >&2
  exit 1
}
terminal_lock_terminated=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select pg_catalog.pg_terminate_backend(pid, 5000)::text from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-quarantine-lock'")
[ "$terminal_lock_terminated" = true ] || {
  echo "db-test: Keycloak quarantine lock holder was not terminated" >&2
  exit 1
}
if wait "$terminal_lock_process"; then
  echo "db-test: terminated Keycloak quarantine lock holder exited successfully" >&2
  exit 1
fi
if wait "$terminal_bootstrap_process"; then
  echo "db-test: Keycloak bootstrap accepted terminal local-authority drift" >&2
  exit 1
fi
LC_ALL=C grep -Fxq \
  'database-bootstrap: Keycloak schema convergence was refused' \
  "$terminal_refusal_stderr" || {
    echo "db-test: terminal Keycloak drift missed the local quarantine guard" >&2
    exit 1
  }
if wait "$terminal_session_process"; then
  echo "db-test: quarantined Keycloak session remained usable" >&2
  exit 1
fi
assert_keycloak_admission_empty "terminal Keycloak quarantine"
terminal_keycloak_quarantine=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
select (
  not database.datallowconn
  and database.datacl is not null
  and not role.rolcanlogin
  and not exists (
    select 1 from pg_catalog.aclexplode(database.datacl) acl
      left join pg_catalog.pg_roles grantee on grantee.oid = acl.grantee
     where not (
       grantee.rolname = 'keycloak'
       and acl.grantor = database.datdba
       and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
       and not acl.is_grantable
     ) and not (
       grantee.rolname = session_user
       and acl.grantor = database.datdba
       and acl.privilege_type = 'CONNECT'
       and not acl.is_grantable
     )
  )
  and (
    select count(*) from pg_catalog.aclexplode(database.datacl) acl
     where acl.grantee = database.datdba
       and acl.grantor = database.datdba
       and acl.privilege_type in ('CREATE', 'CONNECT', 'TEMPORARY')
       and not acl.is_grantable
  ) = 3
  and (
    select count(*) from pg_catalog.aclexplode(database.datacl) acl
     where acl.grantee = (
       select owner_role.oid from pg_catalog.pg_roles owner_role
        where owner_role.rolname = session_user
     )
       and acl.grantor = database.datdba
       and acl.privilege_type = 'CONNECT'
       and not acl.is_grantable
  ) = 1
  and not exists (
    select 1
      from pg_catalog.pg_auth_members membership
      join pg_catalog.pg_roles member on member.oid = membership.member
      join pg_catalog.pg_roles granted on granted.oid = membership.roleid
     where member.rolname = session_user
       and granted.rolname = 'keycloak'
  )
)::text
  from pg_catalog.pg_database database
  join pg_catalog.pg_roles role on role.oid = database.datdba
 where database.datname = 'keycloak'
   and role.rolname = 'keycloak';
SQL
)
[ "$terminal_keycloak_quarantine" = true ] || {
  echo "db-test: terminal Keycloak target retained admission or sessions" >&2
  exit 1
}
[ ! -e "$main_authority_dir/keycloak-cluster.json" ] || {
  echo "db-test: terminal Keycloak quarantine published a witness" >&2
  exit 1
}
compose logs --no-color postgres-main > "$terminal_refusal_server_log" 2>&1
for evidence_file in "$terminal_session_stdout" "$terminal_session_stderr" \
    "$terminal_lock_stdout" "$terminal_lock_stderr" \
    "$terminal_refusal_stdout" "$terminal_refusal_stderr"; do
  assert_database_evidence_omits "$evidence_file" \
    "$terminal_keycloak_content" "$terminal_keycloak_object"
done
assert_database_evidence_omits \
  "$terminal_refusal_server_log" "$terminal_keycloak_content"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'alter database keycloak allow_connections true; grant keycloak to synveda_owner with admin false, inherit true, set true granted by synveda_owner'
terminal_keycloak_row=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command "set role keycloak; select count(*) from public.cpr45_terminal_keycloak_probe where value = 'cpr45-terminal-keycloak-content-sentinel'")
[ "$terminal_keycloak_row" = 1 ] || {
  echo "db-test: terminal Keycloak quarantine did not preserve target evidence" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
drop database keycloak with (force);
revoke keycloak from synveda_owner granted by synveda_owner;
SQL
rm -f "$terminal_session_stdout" "$terminal_session_stderr" \
  "$terminal_lock_stdout" "$terminal_lock_stderr" \
  "$terminal_refusal_stdout" "$terminal_refusal_stderr" \
  "$terminal_refusal_server_log"
unset terminal_keycloak_content terminal_keycloak_object terminal_session_stdout \
  terminal_session_stderr terminal_lock_stdout terminal_lock_stderr \
  terminal_refusal_stdout terminal_refusal_stderr terminal_refusal_server_log \
  evidence_file terminal_session_process terminal_session_ready \
  terminal_session_wait terminal_session_count terminal_membership_absent \
  terminal_lock_process terminal_lock_ready terminal_lock_wait \
  terminal_lock_observed terminal_bootstrap_process terminal_bootstrap_waiting \
  terminal_bootstrap_wait terminal_bootstrap_lock_count terminal_release_shape \
  terminal_lock_terminated terminal_keycloak_quarantine terminal_keycloak_row
compose run --rm --no-deps database-bootstrap-main
compose run --rm --no-deps keycloak-database-bootstrap-main

# A refusal after the global transaction opens Keycloak must use the already
# captured database OID to close admission, drain sessions and publish no
# witness. This test-only contract passes the first global include and fails
# the second, post-open include in the same bootstrap session.
post_open_contract=$state_dir/keycloak-post-open-cluster-authority.sql
post_open_session_stdout=$state_dir/keycloak-post-open-session.stdout
post_open_session_stderr=$state_dir/keycloak-post-open-session.stderr
post_open_refusal_stdout=$state_dir/keycloak-post-open-refusal.stdout
post_open_refusal_stderr=$state_dir/keycloak-post-open-refusal.stderr
for evidence_file in "$post_open_session_stdout" "$post_open_session_stderr" \
    "$post_open_refusal_stdout" "$post_open_refusal_stderr"; do
  private_evidence_file "$evidence_file"
done
cp deploy/compose/postgres/synveda-cluster-authority-contract.sql \
  "$post_open_contract"
printf '%s\n' \
  '' \
  '-- CPR-45 db-test-only post-open refusal.' \
  'select 1 / case when' \
  "  :'synveda_bootstrap_target' <> 'keycloak'" \
  "  or :'synveda_require_complete_roles' <> 'true'" \
  "  or :'synveda_allow_target_owner_membership' <> 'false'" \
  "  or :'synveda_allow_target_default_acl' <> 'false'" \
  'then 1 else 0 end;' >> "$post_open_contract"
chmod 600 "$post_open_contract"

compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-post-open-session \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command 'select pg_catalog.pg_sleep(120)' \
    > "$post_open_session_stdout" 2> "$post_open_session_stderr" &
post_open_session_process=$!
post_open_session_ready=false
post_open_session_wait=0
while [ "$post_open_session_wait" -lt 100 ]; do
  post_open_session_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select count(*) from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-post-open-session'")
  if [ "$post_open_session_count" = 1 ]; then
    post_open_session_ready=true
    break
  fi
  post_open_session_wait=$((post_open_session_wait + 1))
  sleep 0.1
done
[ "$post_open_session_ready" = true ] || {
  echo "db-test: post-open Keycloak session did not become observable" >&2
  exit 1
}

if compose run --rm --no-deps \
    --volume "$post_open_contract:/usr/local/share/synveda/cluster-authority-contract.sql:ro" \
    keycloak-database-bootstrap-main \
    > "$post_open_refusal_stdout" 2> "$post_open_refusal_stderr"; then
  echo "db-test: Keycloak bootstrap accepted a post-open authority failure" >&2
  exit 1
fi
LC_ALL=C grep -Fxq \
  'database-bootstrap: Keycloak role or database convergence was refused' \
  "$post_open_refusal_stderr" || {
    echo "db-test: post-open failure missed the global quarantine guard" >&2
    exit 1
  }
if wait "$post_open_session_process"; then
  echo "db-test: post-open quarantine left its Keycloak session usable" >&2
  exit 1
fi
assert_keycloak_admission_empty "post-open Keycloak failure"
post_open_shape=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not database.datallowconn and database.datacl is not null and not role.rolcanlogin and not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid where member.rolname = session_user and granted.rolname = 'keycloak'))::text from pg_catalog.pg_database database join pg_catalog.pg_roles role on role.oid = database.datdba where database.datname = 'keycloak' and role.rolname = 'keycloak'")
[ "$post_open_shape" = true ] || {
  echo "db-test: post-open failure did not close and drain Keycloak" >&2
  exit 1
}
[ ! -e "$main_authority_dir/keycloak-cluster.json" ] || {
  echo "db-test: post-open Keycloak failure published a witness" >&2
  exit 1
}
for evidence_file in "$post_open_session_stdout" "$post_open_session_stderr" \
    "$post_open_refusal_stdout" "$post_open_refusal_stderr"; do
  assert_database_secrets_absent "$evidence_file"
done
rm -f "$post_open_contract" "$post_open_session_stdout" \
  "$post_open_session_stderr" "$post_open_refusal_stdout" \
  "$post_open_refusal_stderr"

# Re-establish one exact terminal state for the crash-resume case below.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command 'alter role keycloak login; alter database keycloak allow_connections true'
compose run --rm --no-deps keycloak-database-bootstrap-main

# Simulate interruption after the quarantine closure transaction commits but
# before either an admitted visible session or a pre-pgstat startup is drained.
resume_session_stdout=$state_dir/keycloak-resume-session.stdout
resume_session_stderr=$state_dir/keycloak-resume-session.stderr
resume_startup_stdout=$state_dir/keycloak-resume-startup.stdout
resume_startup_stderr=$state_dir/keycloak-resume-startup.stderr
resume_refusal_stdout=$state_dir/keycloak-resume-refusal.stdout
resume_refusal_stderr=$state_dir/keycloak-resume-refusal.stderr
for evidence_file in "$resume_session_stdout" "$resume_session_stderr" \
    "$resume_startup_stdout" "$resume_startup_stderr" \
    "$resume_refusal_stdout" "$resume_refusal_stderr"; do
  private_evidence_file "$evidence_file"
done

compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-resume-session \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command 'select pg_catalog.pg_sleep(120)' \
    > "$resume_session_stdout" 2> "$resume_session_stderr" &
resume_session_process=$!
resume_session_ready=false
resume_session_wait=0
while [ "$resume_session_wait" -lt 100 ]; do
  resume_session_count=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
      --command "select count(*) from pg_catalog.pg_stat_activity where datname = 'keycloak' and application_name = 'cpr45-keycloak-resume-session'")
  if [ "$resume_session_count" = 1 ]; then
    resume_session_ready=true
    break
  fi
  resume_session_wait=$((resume_session_wait + 1))
  sleep 0.1
done
[ "$resume_session_ready" = true ] || {
  echo "db-test: crash-resume Keycloak session did not become observable" >&2
  exit 1
}

compose exec -T postgres-main \
  env PGAPPNAME=cpr45-keycloak-resume-startup \
    PGOPTIONS='-c post_auth_delay=120' \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command 'select 1' \
    > "$resume_startup_stdout" 2> "$resume_startup_stderr" &
resume_startup_process=$!
resume_startup_ready=false
resume_startup_wait=0
resume_startup_pid=
while [ "$resume_startup_wait" -lt 100 ]; do
  resume_startup_pid=$(compose exec -T postgres-main \
    psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
select lock.pid::text
  from pg_catalog.pg_locks lock
  join pg_catalog.pg_database database on database.oid = lock.objid
 where database.datname = 'keycloak'
   and database.datallowconn
   and lock.locktype = 'object'
   and lock.database = 0
   and lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass
   and lock.objsubid = 0
   and lock.mode = 'RowExclusiveLock'
   and lock.pid is not null
   and lock.granted
   and not exists (
     select 1
       from pg_catalog.pg_stat_activity activity
      where activity.pid = lock.pid
   )
 order by lock.pid;
SQL
  )
  case "$resume_startup_pid" in
    ''|*[!0-9]*) ;;
    *)
      resume_startup_ready=true
      break
      ;;
  esac
  resume_startup_wait=$((resume_startup_wait + 1))
  sleep 0.1
done
[ "$resume_startup_ready" = true ] || {
  echo "db-test: pre-pgstat Keycloak startup did not become observable" >&2
  exit 1
}
kill -0 "$resume_startup_process" 2>/dev/null || {
  echo "db-test: pre-pgstat Keycloak startup exited before closure" >&2
  exit 1
}

compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
begin;
alter role keycloak nologin;
alter database keycloak allow_connections false;
commit;
SQL

resume_retained_shape=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 \
    -v expected_startup_pid="$resume_startup_pid" \
    --username synveda_owner --dbname postgres <<'SQL'
select (
  (
    select count(*)
      from pg_catalog.pg_stat_activity activity
     where activity.datname = 'keycloak'
       and activity.application_name = 'cpr45-keycloak-resume-session'
  ) = 1
  and exists (
    select 1
      from pg_catalog.pg_locks lock
      join pg_catalog.pg_database database on database.oid = lock.objid
     where database.datname = 'keycloak'
       and lock.locktype = 'object'
       and lock.database = 0
       and lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass
       and lock.objsubid = 0
       and lock.mode = 'RowExclusiveLock'
       and lock.pid = :'expected_startup_pid'::integer
  )
)::text;
SQL
)
[ "$resume_retained_shape" = true ] || {
  echo "db-test: simulated quarantine crash did not retain both admission populations" >&2
  exit 1
}

if compose run --rm --no-deps keycloak-database-bootstrap-main \
    > "$resume_refusal_stdout" 2> "$resume_refusal_stderr"; then
  echo "db-test: interrupted Keycloak quarantine was reopened" >&2
  exit 1
fi
LC_ALL=C grep -Fxq \
  'database-bootstrap: interrupted Keycloak quarantine remains closed' \
  "$resume_refusal_stderr" || {
    echo "db-test: interrupted quarantine missed its resume guard" >&2
    exit 1
  }
if wait "$resume_session_process"; then
  echo "db-test: resumed quarantine left its visible session usable" >&2
  exit 1
fi
if wait "$resume_startup_process"; then
  echo "db-test: resumed quarantine admitted its delayed startup" >&2
  exit 1
fi
assert_keycloak_admission_empty "resumed Keycloak quarantine"
resume_shape=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not database.datallowconn and database.datacl is not null and not role.rolcanlogin and not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid where member.rolname = session_user and granted.rolname = 'keycloak'))::text from pg_catalog.pg_database database join pg_catalog.pg_roles role on role.oid = database.datdba where database.datname = 'keycloak' and role.rolname = 'keycloak'")
[ "$resume_shape" = true ] || {
  echo "db-test: resumed Keycloak quarantine did not finish its drain" >&2
  exit 1
}
[ ! -e "$main_authority_dir/keycloak-cluster.json" ] || {
  echo "db-test: resumed Keycloak quarantine published a witness" >&2
  exit 1
}
for evidence_file in "$resume_session_stdout" "$resume_session_stderr" \
    "$resume_startup_stdout" "$resume_startup_stderr" \
    "$resume_refusal_stdout" "$resume_refusal_stderr"; do
  assert_database_secrets_absent "$evidence_file"
done
rm -f "$resume_session_stdout" "$resume_session_stderr" \
  "$resume_startup_stdout" "$resume_startup_stderr" \
  "$resume_refusal_stdout" "$resume_refusal_stderr"
unset post_open_contract post_open_session_stdout post_open_session_stderr \
  post_open_refusal_stdout post_open_refusal_stderr post_open_session_process \
  post_open_session_ready post_open_session_wait post_open_session_count \
  post_open_shape resume_session_stdout resume_session_stderr \
  resume_startup_stdout resume_startup_stderr resume_refusal_stdout \
  resume_refusal_stderr resume_session_process resume_session_ready \
  resume_session_wait resume_session_count resume_startup_process \
  resume_startup_ready resume_startup_wait resume_startup_pid \
  resume_retained_shape resume_shape evidence_file

# Global metadata cannot distinguish a just-created closed database from a
# previously used one. A Keycloak-owned object therefore must fail the local
# pristine-envelope proof, re-close the database, disable LOGIN and publish no
# witness while preserving the object for explicit operator recovery.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
drop database keycloak with (force);
alter role keycloak nologin;
grant keycloak to synveda_owner
  with admin false, inherit true, set true granted by synveda_owner;
create database keycloak with owner keycloak template template0 encoding 'UTF8';
SQL
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak <<'SQL'
set role keycloak;
create table public.cpr45_stale_keycloak (value text primary key);
\copy public.cpr45_stale_keycloak(value) from stdin
cpr45-closed-keycloak-content-sentinel
\.
reset role;
SQL
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
revoke keycloak from synveda_owner granted by synveda_owner;
alter database keycloak allow_connections false;
SQL
compose run --rm --no-deps database-bootstrap-main
closed_keycloak_content=cpr45-closed-keycloak-content-sentinel
closed_keycloak_object=cpr45_stale_keycloak
closed_refusal_stdout=$state_dir/keycloak-closed-quarantine.stdout
closed_refusal_stderr=$state_dir/keycloak-closed-quarantine.stderr
closed_refusal_server_log=$state_dir/keycloak-closed-quarantine.postgres.log
for evidence_file in "$closed_refusal_stdout" "$closed_refusal_stderr" \
    "$closed_refusal_server_log"; do
  private_evidence_file "$evidence_file"
done
if compose run --rm --no-deps keycloak-database-bootstrap-main \
    > "$closed_refusal_stdout" 2> "$closed_refusal_stderr"; then
  echo "db-test: Keycloak bootstrap adopted a non-pristine closed database" >&2
  exit 1
fi
LC_ALL=C grep -Fxq \
  'database-bootstrap: Keycloak schema convergence was refused' \
  "$closed_refusal_stderr" || {
    echo "db-test: non-pristine Keycloak target missed the local guard" >&2
    exit 1
  }
assert_keycloak_admission_empty "non-pristine Keycloak quarantine"
keycloak_quarantine=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (not database.datallowconn and database.datacl is not null and not role.rolcanlogin and not exists (select 1 from pg_catalog.aclexplode(database.datacl) acl where acl.grantee = 0 and acl.privilege_type = 'CONNECT'))::text from pg_catalog.pg_database database join pg_catalog.pg_roles role on role.oid = database.datdba where database.datname = 'keycloak' and role.rolname = 'keycloak'")
[ "$keycloak_quarantine" = true ] || {
  echo "db-test: non-pristine Keycloak target was not quarantined" >&2
  exit 1
}
[ ! -e "$main_authority_dir/keycloak-cluster.json" ] || {
  echo "db-test: quarantined Keycloak target published a witness" >&2
  exit 1
}
compose logs --no-color postgres-main > "$closed_refusal_server_log" 2>&1
for evidence_file in "$closed_refusal_stdout" "$closed_refusal_stderr"; do
  assert_database_evidence_omits "$evidence_file" \
    "$closed_keycloak_content" "$closed_keycloak_object"
done
assert_database_evidence_omits \
  "$closed_refusal_server_log" "$closed_keycloak_content"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'alter database keycloak allow_connections true'
stale_keycloak_row=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname keycloak \
    --command "select count(*) from public.cpr45_stale_keycloak where value = 'cpr45-closed-keycloak-content-sentinel'")
[ "$stale_keycloak_row" = 1 ] || {
  echo "db-test: Keycloak quarantine did not preserve stale target evidence" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop database keycloak with (force)'
rm -f "$closed_refusal_stdout" "$closed_refusal_stderr" \
  "$closed_refusal_server_log"
compose run --rm --no-deps database-bootstrap-main
compose run --rm --no-deps keycloak-database-bootstrap-main

# A crash after the ACL transaction but before the credential transaction is
# the final finite state: terminal database ACL, NOLOGIN+infinity role and the
# temporary self-grant. The same ordered restart must restore LOGIN and remove
# the temporary authority.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "alter role keycloak nologin valid until 'infinity'; grant keycloak to synveda_owner with admin false, inherit true, set true granted by synveda_owner"
compose run --rm --no-deps database-bootstrap-main
compose run --rm --no-deps keycloak-database-bootstrap-main
keycloak_terminal_recovered=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select (role.rolcanlogin and role.rolvaliduntil is not distinct from 'infinity'::timestamptz and not exists (select 1 from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles granted on granted.oid = membership.roleid join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor where member.rolname = 'synveda_owner' and granted.rolname = 'keycloak' and grantor.rolname = 'synveda_owner'))::text from pg_catalog.pg_roles role where role.rolname = 'keycloak'")
[ "$keycloak_terminal_recovered" = true ] || {
  echo "db-test: Keycloak terminal-ACL restart did not restore exact login authority" >&2
  exit 1
}
unset keycloak_open_default_state open_default_before open_default_after \
  open_default_synveda_refusal open_default_keycloak_refusal \
  keycloak_closed_state keycloak_setting_shape keycloak_session_stdout \
  keycloak_session_stderr keycloak_session_process keycloak_session_ready \
  keycloak_session_wait keycloak_session_count terminated_session \
  keycloak_recovered keycloak_quarantine stale_keycloak_row \
  keycloak_terminal_recovered closed_keycloak_content closed_keycloak_object \
  closed_refusal_stdout closed_refusal_stderr closed_refusal_server_log \
  evidence_file

exercise_target_event_trigger_refusal \
  synveda database-bootstrap-main Synveda
exercise_target_event_trigger_refusal \
  keycloak keycloak-database-bootstrap-main Keycloak

# External-provider contracts may name a topology-specific forbidden set, but
# every element remains a string and bundled Keycloak must retain its own peer
# in that set. Prove both malformed forms are refused before any catalog write.
for role_contract_case in non-string missing-keycloak; do
  roles_candidate=$state_dir/database-roles.$role_contract_case
  case "$role_contract_case" in
    non-string)
      printf '%s\n' '{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["synveda_owner"],"administrative_memberships":[],"forbidden_databases":[1,"keycloak","postgres","template1"],"isolated_peer_roles":["keycloak"]}' > "$roles_candidate"
      ;;
    missing-keycloak)
      printf '%s\n' '{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["synveda_owner"],"administrative_memberships":[],"forbidden_databases":["postgres","template1"],"isolated_peer_roles":["keycloak"]}' > "$roles_candidate"
      ;;
  esac
  chmod 600 "$roles_candidate"
  stdout_file=$state_dir/role-contract-$role_contract_case.stdout
  stderr_file=$state_dir/role-contract-$role_contract_case.stderr
  private_evidence_file "$stdout_file"
  private_evidence_file "$stderr_file"
  main_before=$(catalog_fingerprint postgres-main keycloak)
  if compose run --rm --no-deps \
      --env SYNVEDA_POSTGRES_BUNDLED_CLUSTER=true \
      --volume "$roles_candidate:/run/secrets/database_roles.json:ro" \
      keycloak-database-bootstrap-main > "$stdout_file" 2> "$stderr_file"; then
    bootstrap_accepted=true
  else
    bootstrap_accepted=false
  fi
  [ "$bootstrap_accepted" = false ] || {
    echo "db-test: bootstrap accepted $role_contract_case forbidden database contract" >&2
    exit 1
  }
  [ ! -s "$stdout_file" ] || {
    echo "db-test: $role_contract_case role refusal produced stdout" >&2
    exit 1
  }
  LC_ALL=C grep -Fq \
    'database-bootstrap: Keycloak role or database convergence was refused' \
    "$stderr_file" || {
      echo "db-test: $role_contract_case role contract missed the authority guard" >&2
      exit 1
    }
  assert_database_secrets_absent "$stdout_file"
  assert_database_secrets_absent "$stderr_file"
  main_after=$(catalog_fingerprint postgres-main keycloak)
  [ "$main_before" = "$main_after" ] || {
    echo "db-test: $role_contract_case role refusal changed catalog state" >&2
    exit 1
  }
  rm -f "$roles_candidate" "$stdout_file" "$stderr_file"
done
unset roles_candidate role_contract_case stdout_file stderr_file \
  bootstrap_accepted main_before main_after

# A maintenance-database ACL regression must be refused before any credential
# read or catalog write, even when the target database is already terminal.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'grant connect on database postgres to public'
stdout_file=$state_dir/keycloak-maintenance-connect.stdout
stderr_file=$state_dir/keycloak-maintenance-connect.stderr
private_evidence_file "$stdout_file"
private_evidence_file "$stderr_file"
main_before=$(catalog_fingerprint postgres-main keycloak)
if compose run --rm --no-deps \
    --env SYNVEDA_POSTGRES_BUNDLED_CLUSTER=true \
    keycloak-database-bootstrap-main > "$stdout_file" 2> "$stderr_file"; then
  bootstrap_accepted=true
else
  bootstrap_accepted=false
fi
[ "$bootstrap_accepted" = false ] || {
  echo "db-test: external-provider Keycloak accepted maintenance-database CONNECT" >&2
  exit 1
}
LC_ALL=C grep -Fq \
  'database-bootstrap: Keycloak role or database convergence was refused' \
  "$stderr_file" || {
    echo "db-test: Keycloak maintenance CONNECT missed the authority guard" >&2
    exit 1
  }
assert_database_secrets_absent "$stdout_file"
assert_database_secrets_absent "$stderr_file"
main_after=$(catalog_fingerprint postgres-main keycloak)
[ "$main_before" = "$main_after" ] || {
  echo "db-test: refused Keycloak maintenance CONNECT changed catalog state" >&2
  exit 1
}
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'revoke connect on database postgres from public'
rm -f "$stdout_file" "$stderr_file"

# Every target-database role setting is rejected by the exact database-shape
# preflight before password files are read.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "alter role synveda_owner in database synveda set role = 'synveda_migrator'; alter role synveda_owner in database synveda set session_preload_libraries = 'auto_explain'"
stdout_file=$state_dir/bootstrap-target-setting.stdout
stderr_file=$state_dir/bootstrap-target-setting.stderr
server_log_file=$state_dir/bootstrap-target-setting.postgres.log
private_evidence_file "$stdout_file"
private_evidence_file "$stderr_file"
private_evidence_file "$server_log_file"
main_before=$(catalog_fingerprint postgres-main synveda)
if compose run --rm --no-deps database-bootstrap-main \
    > "$stdout_file" 2> "$stderr_file"; then
  bootstrap_accepted=true
else
  bootstrap_accepted=false
fi
[ "$bootstrap_accepted" = false ] || {
  echo "db-test: bootstrap accepted target-specific principal settings" >&2
  exit 1
}
LC_ALL=C grep -Fq \
  'database-bootstrap: Synveda existing database shape was refused' \
  "$stderr_file" || {
    echo "db-test: target-specific principal settings missed the authority guard" >&2
    exit 1
  }
main_after=$(catalog_fingerprint postgres-main synveda)
[ "$main_before" = "$main_after" ] || {
  echo "db-test: refused target-specific principal settings changed catalog state" >&2
  exit 1
}
residual_membership=$(compose exec -T postgres-main \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select count(*) from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles granted on granted.oid = membership.roleid join pg_catalog.pg_roles member on member.oid = membership.member where granted.rolname = 'synveda_migrator' and member.rolname = 'synveda_owner'")
[ "$residual_membership" = 0 ] || {
  echo "db-test: refused target-specific settings left owner membership" >&2
  exit 1
}
compose logs --no-color postgres-main > "$server_log_file" 2>&1
assert_database_secrets_absent "$stdout_file"
assert_database_secrets_absent "$stderr_file"
assert_database_secrets_absent "$server_log_file"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'alter role synveda_owner in database synveda reset all'
rm -f "$stdout_file" "$stderr_file" "$server_log_file"
unset stdout_file stderr_file server_log_file bootstrap_accepted main_before \
  main_after residual_membership

# Refusal cases deliberately invalidate the previously published witness.
# Re-converge the target before any application credential may trust it.
compose run --rm --no-deps keycloak-database-bootstrap-main
main_server_log=$state_dir/main-bootstrap.postgres.log
private_evidence_file "$main_server_log"
compose logs --no-color postgres-main > "$main_server_log" 2>&1
assert_credential_server_log_clean "$main_server_log"
disable_hostile_database_logging postgres-main
rm -f "$main_server_log"
unset main_server_log

# Prove the current external-PostgreSQL boundary with an ordinary PostgreSQL 17
# LOGIN+CREATEROLE+CREATEDB principal. Even after a provider preinstalls the
# extension set and exact role authority, the bootstrap command must refuse
# before mounted-input reads or catalog mutation until authenticated TLS is a
# supported product contract.
enable_hostile_database_logging postgres-lifecycle
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
create role cpr45_external_bootstrap
  nologin inherit nosuperuser createdb createrole noreplication nobypassrls
  connection limit -1;
create role cpr45_extension_installer
  nologin inherit superuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;
create role synveda_app
  nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;
create role synveda_migrator
  nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;
create role synveda_gateway
  nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;
create role synveda_worker
  nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;
create role keycloak
  nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls
  connection limit -1;

grant synveda_app, synveda_migrator, synveda_gateway, synveda_worker, keycloak
  to cpr45_external_bootstrap
  with admin true, inherit false, set false granted by synveda_owner;
grant pg_read_all_settings to cpr45_external_bootstrap
  with admin false, inherit true, set false granted by synveda_owner;
grant set on parameter session_preload_libraries to cpr45_external_bootstrap;
grant set on parameter log_min_messages to cpr45_external_bootstrap;
grant set on parameter log_min_error_statement to cpr45_external_bootstrap;
grant set on parameter log_error_verbosity to cpr45_external_bootstrap;
grant set on parameter log_statement to cpr45_external_bootstrap;
grant set on parameter log_min_duration_statement to cpr45_external_bootstrap;
grant set on parameter log_min_duration_sample to cpr45_external_bootstrap;
grant set on parameter log_statement_sample_rate to cpr45_external_bootstrap;
grant set on parameter log_transaction_sample_rate to cpr45_external_bootstrap;
grant set on parameter log_parameter_max_length to cpr45_external_bootstrap;
grant set on parameter log_parameter_max_length_on_error to cpr45_external_bootstrap;
grant set on parameter debug_print_parse to cpr45_external_bootstrap;
grant set on parameter debug_print_rewritten to cpr45_external_bootstrap;
grant set on parameter debug_print_plan to cpr45_external_bootstrap;

create database synveda
  with owner synveda_migrator template template0 encoding 'UTF8';
revoke connect, temporary on database postgres, template1, synveda from public;
grant connect, temporary on database postgres to cpr45_external_bootstrap;
set role synveda_migrator;
grant create, connect, temporary on database synveda to synveda_migrator;
grant connect on database synveda to cpr45_external_bootstrap;
reset role;

\connect synveda
set role cpr45_extension_installer;
create extension btree_gin with schema public version '1.3';
create extension vector with schema public version '0.8.6';
reset role;
reassign owned by cpr45_extension_installer to cpr45_external_bootstrap;
drop role cpr45_extension_installer;

set default_table_access_method = heap;
set client_encoding = 'UTF8';
set jit = off;
reset role;
\i /usr/local/share/synveda/credential-log-contract.sql
begin;
create temporary table pg_temp.cpr45_external_provider_credential (
  secret text not null
) using heap on commit drop;
\copy pg_temp.cpr45_external_provider_credential(secret) from '/run/secrets/external_provider_password'
do $credential$
declare
  provider_password text;
begin
  select secret into strict provider_password
    from pg_temp.cpr45_external_provider_credential;
  begin
    execute format(
      'alter role cpr45_external_bootstrap with login inherit password %L valid until ''infinity''',
      provider_password
    );
  exception when query_canceled or assert_failure or others then
    raise exception using
      errcode = 'P0001',
      message = 'External provider credential setup was refused';
  end;
end
$credential$;
commit;
SQL

external_stdout=$state_dir/external-provider-bootstrap.stdout
external_stderr=$state_dir/external-provider-bootstrap.stderr
external_server_log=$state_dir/external-provider-bootstrap.postgres.log
private_evidence_file "$external_stdout"
private_evidence_file "$external_stderr"
private_evidence_file "$external_server_log"
external_before=$(catalog_fingerprint postgres-lifecycle postgres)
for external_target in synveda keycloak; do
  if compose run --rm --no-deps database-bootstrap-external-lifecycle \
      "$external_target" >> "$external_stdout" 2>> "$external_stderr"; then
    echo "db-test: external-provider $external_target bootstrap bypassed the TLS gate" >&2
    exit 1
  fi
done
[ ! -s "$external_stdout" ] || {
  echo "db-test: external PostgreSQL refusal produced stdout" >&2
  exit 1
}
external_refusals=$(LC_ALL=C grep -Fc \
  'database-bootstrap: external PostgreSQL mutation is unavailable until the authenticated TLS bootstrap contract is implemented' \
  "$external_stderr")
[ "$external_refusals" = 2 ] || {
  echo "db-test: external PostgreSQL did not fail closed at the TLS gate" >&2
  exit 1
}
external_after=$(catalog_fingerprint postgres-lifecycle postgres)
[ "$external_before" = "$external_after" ] || {
  echo "db-test: refused external PostgreSQL bootstrap changed catalog state" >&2
  exit 1
}
external_shape=$(compose exec -T postgres-lifecycle \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select rolsuper::text || ':' || rolcreaterole::text || ':' || rolcreatedb::text || ':' || rolcanlogin::text from pg_catalog.pg_roles where rolname = 'cpr45_external_bootstrap'")
[ "$external_shape" = "false:true:true:true" ] || {
  echo "db-test: external bootstrap principal was not an ordinary CREATEROLE/CREATEDB login" >&2
  exit 1
}
external_keycloak_shape=$(compose exec -T postgres-lifecycle \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select count(*) from pg_catalog.pg_database where datname = 'keycloak'")
[ "$external_keycloak_shape" = 0 ] || {
  echo "db-test: refused external bootstrap created the Keycloak database" >&2
  exit 1
}
external_residual=$(compose exec -T postgres-lifecycle \
  psql -X -qAt -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
    --command "select count(*) from pg_catalog.pg_auth_members membership join pg_catalog.pg_roles granted on granted.oid = membership.roleid join pg_catalog.pg_roles member on member.oid = membership.member join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor where granted.rolname in ('synveda_migrator','keycloak') and member.rolname = 'cpr45_external_bootstrap' and grantor.rolname = 'cpr45_external_bootstrap'")
[ "$external_residual" = 0 ] || {
  echo "db-test: external bootstrap retained a target-owner membership" >&2
  exit 1
}
compose logs --no-color postgres-lifecycle > "$external_server_log" 2>&1
assert_database_secrets_absent "$external_stdout"
assert_database_secrets_absent "$external_stderr"
assert_credential_server_log_clean "$external_server_log"
disable_hostile_database_logging postgres-lifecycle

# Restore the lifecycle cluster to its intentional fresh state before the
# destructive reset/epoch suite. Every target is exact and belongs only to this
# isolated per-run fixture.
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
drop database synveda with (force);
revoke connect, temporary on database postgres from cpr45_external_bootstrap;
revoke synveda_app, synveda_migrator, synveda_gateway, synveda_worker, keycloak
  from cpr45_external_bootstrap granted by synveda_owner;
revoke pg_read_all_settings from cpr45_external_bootstrap granted by synveda_owner;
revoke set on parameter session_preload_libraries from cpr45_external_bootstrap;
revoke set on parameter log_min_messages from cpr45_external_bootstrap;
revoke set on parameter log_min_error_statement from cpr45_external_bootstrap;
revoke set on parameter log_error_verbosity from cpr45_external_bootstrap;
revoke set on parameter log_statement from cpr45_external_bootstrap;
revoke set on parameter log_min_duration_statement from cpr45_external_bootstrap;
revoke set on parameter log_min_duration_sample from cpr45_external_bootstrap;
revoke set on parameter log_statement_sample_rate from cpr45_external_bootstrap;
revoke set on parameter log_transaction_sample_rate from cpr45_external_bootstrap;
revoke set on parameter log_parameter_max_length from cpr45_external_bootstrap;
revoke set on parameter log_parameter_max_length_on_error from cpr45_external_bootstrap;
revoke set on parameter debug_print_parse from cpr45_external_bootstrap;
revoke set on parameter debug_print_rewritten from cpr45_external_bootstrap;
revoke set on parameter debug_print_plan from cpr45_external_bootstrap;
drop role synveda_gateway, synveda_worker, synveda_app, synveda_migrator, keycloak;
drop role cpr45_external_bootstrap;
SQL
rm -f "$external_stdout" "$external_stderr" "$external_server_log"
unset external_target external_before external_after external_refusals external_shape \
  external_keycloak_shape external_residual external_stdout external_stderr external_server_log

compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create role synveda_app nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1; create schema cpr45_synveda_dependency authorization synveda_app"
lifecycle_before=$(catalog_fingerprint postgres-lifecycle postgres)
if lifecycle_refusal=$(compose run --rm --no-deps database-bootstrap-lifecycle 2>&1); then
  echo "db-test: Synveda bootstrap accepted absent-target shared ownership" >&2
  exit 1
fi
case "$lifecycle_refusal" in
  *"database-bootstrap: Synveda role or database convergence was refused"*) ;;
  *)
    echo "db-test: Synveda absent-target refusal did not reach the authority guard" >&2
    exit 1
    ;;
esac
unset lifecycle_refusal
lifecycle_after=$(catalog_fingerprint postgres-lifecycle postgres)
[ "$lifecycle_before" = "$lifecycle_after" ] || {
  echo "db-test: refused Synveda absent-target bootstrap changed catalog state" >&2
  exit 1
}
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop schema cpr45_synveda_dependency; drop role synveda_app'

compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command "create database synveda with owner synveda_owner template template0 encoding 'UTF8'"
lifecycle_before=$(catalog_fingerprint postgres-lifecycle synveda)
if lifecycle_refusal=$(compose run --rm --no-deps database-bootstrap-lifecycle 2>&1); then
  echo "db-test: Synveda bootstrap accepted a wrong-owner existing database" >&2
  exit 1
fi
case "$lifecycle_refusal" in
  *"database-bootstrap: Synveda existing database shape was refused"*) ;;
  *)
    echo "db-test: Synveda wrong-owner refusal did not reach the database-shape guard" >&2
    exit 1
    ;;
esac
unset lifecycle_refusal
lifecycle_after=$(catalog_fingerprint postgres-lifecycle synveda)
[ "$lifecycle_before" = "$lifecycle_after" ] || {
  echo "db-test: refused Synveda bootstrap changed protected catalog state" >&2
  exit 1
}
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop database synveda with (force)'
compose run --rm --no-deps database-bootstrap-lifecycle

# A wrong-cluster preflight negative control needs a genuine content-free peer
# witness. Converge the bundled peer on this isolated cluster with only its two
# secrets, then remove its exact database, owner membership and role. The
# lifecycle suite intentionally uses the external-OIDC role contract, so prove
# that publishing the witness leaves no Keycloak catalogue state behind.
lifecycle_peer_before=$(catalog_fingerprint postgres-lifecycle synveda)
compose run --rm --no-deps keycloak-database-bootstrap-lifecycle
lifecycle_witness_file=$lifecycle_authority_dir/keycloak-cluster.json
[ -f "$lifecycle_witness_file" ] || {
  echo "db-test: lifecycle Keycloak bootstrap did not publish its cluster witness" >&2
  exit 1
}
assert_database_secrets_absent "$lifecycle_witness_file"
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres <<'SQL'
select 1 / case when not exists (
  select 1
    from pg_catalog.pg_auth_members membership
    join pg_catalog.pg_roles member on member.oid = membership.member
    join pg_catalog.pg_roles granted on granted.oid = membership.roleid
    join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
   where granted.rolname = 'keycloak'
      or member.rolname = 'keycloak'
      or grantor.rolname = 'keycloak'
) then 1 else 0 end;
drop database keycloak with (force);
drop role keycloak;
SQL
lifecycle_peer_after=$(catalog_fingerprint postgres-lifecycle synveda)
[ "$lifecycle_peer_before" = "$lifecycle_peer_after" ] || {
  echo "db-test: lifecycle peer-witness bootstrap left Keycloak catalog state" >&2
  exit 1
}
unset lifecycle_peer_before lifecycle_peer_after

# The lifecycle cluster must not already contain another migrator-owned
# database: its tests create, reset and drop one database at a time and the
# exact migrator sentinel refuses cross-database ownership.
compose exec -T postgres-lifecycle \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'drop database synveda with (force)'

main_port=$(published_port postgres-main)
lifecycle_port=$(published_port postgres-lifecycle)

main_owner_file=$state_dir/main-owner.url
main_migrator_file=$state_dir/main-migrator.url
main_gateway_file=$state_dir/main-gateway.url
main_worker_file=$state_dir/main-worker.url
lifecycle_owner_file=$state_dir/lifecycle-owner.url
lifecycle_migrator_file=$state_dir/lifecycle-migrator.url
lifecycle_gateway_file=$state_dir/lifecycle-gateway.url

write_database_url synveda_owner "$secret_dir/postgres_owner_password" \
  "$main_port" synveda "$main_owner_file"
write_database_url synveda_migrator "$secret_dir/synveda_migrator_password" \
  "$main_port" synveda "$main_migrator_file"
write_database_url synveda_gateway "$secret_dir/synveda_gateway_password" \
  "$main_port" synveda "$main_gateway_file"
write_database_url synveda_worker "$secret_dir/synveda_worker_password" \
  "$main_port" synveda "$main_worker_file"
write_database_url synveda_owner "$secret_dir/postgres_owner_password" \
  "$lifecycle_port" postgres "$lifecycle_owner_file"
write_database_url synveda_migrator "$secret_dir/synveda_migrator_password" \
  "$lifecycle_port" postgres "$lifecycle_migrator_file"
write_database_url synveda_gateway "$secret_dir/synveda_gateway_password" \
  "$lifecycle_port" postgres "$lifecycle_gateway_file"

expect_main_preflight_refusal() {
  local label=$1
  local witness_file=$2
  local expected_error=$3
  local stdout_file=$state_dir/preflight-$label.stdout
  local stderr_file=$state_dir/preflight-$label.stderr
  local before after actual_error
  private_evidence_file "$stdout_file"
  private_evidence_file "$stderr_file"
  before=$(catalog_fingerprint postgres-main synveda)
  if run_main_database_preflight "$witness_file" > "$stdout_file" 2> "$stderr_file"; then
    echo "db-test: database preflight accepted $label peer authority" >&2
    exit 1
  fi
  [ ! -s "$stdout_file" ] || {
    echo "db-test: refused $label database preflight produced stdout" >&2
    exit 1
  }
  actual_error=
  IFS= read -r actual_error < "$stderr_file" || [ -n "$actual_error" ]
  [ "$actual_error" = "$expected_error" ] \
    && [ "$(wc -l < "$stderr_file" | tr -d ' ')" -eq 1 ] || {
      echo "db-test: refused $label database preflight did not return one generic error" >&2
      exit 1
    }
  assert_database_secrets_absent "$stdout_file"
  assert_database_secrets_absent "$stderr_file"
  after=$(catalog_fingerprint postgres-main synveda)
  [ "$before" = "$after" ] || {
    echo "db-test: refused $label database preflight changed catalog state" >&2
    exit 1
  }
  rm -f "$stdout_file" "$stderr_file"
}

main_witness_file=$main_authority_dir/keycloak-cluster.json
[ -f "$main_witness_file" ] && [ -f "$lifecycle_witness_file" ] || {
  echo "db-test: database bootstrap did not publish both cluster witnesses" >&2
  exit 1
}
assert_database_secrets_absent "$main_witness_file"
assert_database_secrets_absent "$lifecycle_witness_file"
run_main_database_preflight "$main_witness_file"

peer_mismatch_error='synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE does not match the peer database cluster witness'
expect_main_preflight_refusal wrong-cluster "$lifecycle_witness_file" "$peer_mismatch_error"

tampered_witness_file=$state_dir/tampered-keycloak-cluster.json
sed 's/"database_oid":[0-9][0-9]*/"database_oid":1/' \
  "$main_witness_file" > "$tampered_witness_file"
chmod 600 "$tampered_witness_file"
expect_main_preflight_refusal tampered "$tampered_witness_file" "$peer_mismatch_error"
rm -f "$tampered_witness_file"

# A server restart changes the writable generation marker while preserving the
# database OID. The old witness must fail until the idempotent Keycloak
# bootstrap commits again and atomically republishes it.
compose restart postgres-main >/dev/null
compose up --detach --wait postgres-main >/dev/null
# This fixture intentionally asks Compose for a dynamic loopback port. A
# recreate during the restart/up sequence may publish a different port, so
# refresh the private test URLs before asserting the stale cluster generation.
main_port=$(published_port postgres-main)
write_database_url synveda_owner "$secret_dir/postgres_owner_password" \
  "$main_port" synveda "$main_owner_file"
write_database_url synveda_migrator "$secret_dir/synveda_migrator_password" \
  "$main_port" synveda "$main_migrator_file"
write_database_url synveda_gateway "$secret_dir/synveda_gateway_password" \
  "$main_port" synveda "$main_gateway_file"
write_database_url synveda_worker "$secret_dir/synveda_worker_password" \
  "$main_port" synveda "$main_worker_file"
wait_for_main_database_authority
expect_main_preflight_refusal stale-after-restart "$main_witness_file" "$peer_mismatch_error"
compose run --rm --no-deps keycloak-database-bootstrap-main
run_main_database_preflight "$main_witness_file"

# A later inherited grant can give the Keycloak login effective CONNECT into
# Synveda without changing either database ACL. The long-lived role sentinel
# must enforce the reverse edge of the same isolation contract.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'grant synveda_owner to keycloak granted by synveda_owner'
authority_error='synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE authority or writable-target verification failed'
expect_main_preflight_refusal inherited-peer-authority "$main_witness_file" "$authority_error"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'revoke synveda_owner from keycloak granted by synveda_owner'
run_main_database_preflight "$main_witness_file"

# PUBLIC receives CONNECT on a new PostgreSQL database by default. The
# Keycloak bootstrap must revoke it, and the Rust sentinel must detect any
# later restoration through effective (not merely direct) privileges.
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'grant connect on database keycloak to public'
preflight_stdout=$state_dir/cross-database-preflight.stdout
preflight_stderr=$state_dir/cross-database-preflight.stderr
private_evidence_file "$preflight_stdout"
private_evidence_file "$preflight_stderr"
if run_main_database_preflight "$main_witness_file" \
    > "$preflight_stdout" 2> "$preflight_stderr"; then
  echo "db-test: database preflight accepted cross-database CONNECT authority" >&2
  exit 1
fi
[ ! -s "$preflight_stdout" ] || {
  echo "db-test: refused database preflight produced stdout" >&2
  exit 1
}
preflight_error=
IFS= read -r preflight_error < "$preflight_stderr" || [ -n "$preflight_error" ]
[ "$preflight_error" = \
  'synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE authority or writable-target verification failed' ] \
  && [ "$(wc -l < "$preflight_stderr" | tr -d ' ')" -eq 1 ] || {
    echo "db-test: refused database preflight did not return one generic error" >&2
    exit 1
  }
assert_database_secrets_absent "$preflight_stdout"
assert_database_secrets_absent "$preflight_stderr"
compose exec -T postgres-main \
  psql -X -q -v ON_ERROR_STOP=1 --username synveda_owner --dbname postgres \
  --command 'revoke connect on database keycloak from public'
run_main_database_preflight "$main_witness_file"
rm -f "$preflight_stdout" "$preflight_stderr"
unset preflight_stdout preflight_stderr preflight_error peer_mismatch_error authority_error \
  main_witness_file lifecycle_witness_file tampered_witness_file

for _ in 1 2; do
  SQLX_OFFLINE=true \
  DATABASE_URL_FILE=$main_migrator_file \
  SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
    cargo run -q -p synveda-cli --bin synveda -- db migrate
done
echo "db-test: isolated exact-role database bootstrapped and migrated"

status=0
case "${SYNVEDA_DB_TEST_TASK:-workspace}" in
  workspace)
    SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
    SQLX_OFFLINE=true \
    SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
    SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file \
    SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      scripts/cargo-with-database-url-file cargo test --workspace "$@" || status=$?

    # A focused invocation (for example `make claude-acceptance`) owns its test
    # filter. The full gate additionally runs privileged/drift suites one at a
    # time so cluster-global role mutations cannot race ordinary binaries.
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-store --lib \
          runtime_role::tests::routine_and_trigger_drift_are_refused_and_transactionally_restored -- \
          --exact --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-store --test access -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-store --test scopes -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-store --test rls -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-audit --test tamper -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-vedaflow --test object_store -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-gateway --test okf_api -- \
          --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-gateway --test capture_api \
          capture_worker_reproves_a_preflight_lease_before_calling_the_extractor -- \
          --exact --ignored --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
      SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
      SYNVEDA_TEST_WORKER_DATABASE_URL_FILE=$main_worker_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-cli --bin synveda \
          init::tests::compose_runtime_logins_are_distinct_and_rls_enforced \
          -- --exact --nocapture --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
      SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
      SYNVEDA_TEST_WORKER_DATABASE_URL_FILE=$main_worker_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-gateway --test observability \
          governed_routes_open_only_for_exact_authority_and_terminal_drift_stays_closed \
          -- --exact --nocapture --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$roles_file \
      SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file \
      SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE=$main_owner_file \
      SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE=$main_gateway_file \
      SYNVEDA_TEST_WORKER_DATABASE_URL_FILE=$main_worker_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-gateway --test worker_process \
          exact_worker_role_is_ready_and_authority_drift_is_terminal \
          -- --exact --nocapture --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$lifecycle_roles_file \
      SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL_FILE=$lifecycle_owner_file \
      SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL_FILE=$lifecycle_migrator_file \
      SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE=$lifecycle_gateway_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-store --test epoch -- --test-threads=1 || status=$?
    fi
    if [ "$status" -eq 0 ] && [ "$#" -eq 0 ]; then
      SYNVEDA_CARGO_DATABASE_URL_FILE=$main_gateway_file \
      SQLX_OFFLINE=true \
      SYNVEDA_DATABASE_ROLES_FILE=$lifecycle_roles_file \
      SYNVEDA_EPOCH_TEST_ADMIN_DATABASE_URL_FILE=$lifecycle_owner_file \
      SYNVEDA_EPOCH_TEST_MIGRATOR_DATABASE_URL_FILE=$lifecycle_migrator_file \
      SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE=$lifecycle_gateway_file \
        scripts/cargo-with-database-url-file cargo test -p synveda-gateway --test observability \
          readyz_refuses_a_database_that_is_not_at_this_schema_epoch \
          -- --exact --nocapture --test-threads=1 || status=$?
    fi
    ;;
esac

if [ "$status" -ne 0 ]; then
  exit "$status"
fi

if [ "${KEEP_TEST_DB:-}" = 1 ]; then
  trap - EXIT
  echo "db-test: passed; retained isolated Compose project $project"
  echo "db-test: private state is in $state_dir (mode 0700; contains credentials)"
  exit 0
fi

cleanup_successful_fixture
echo "db-test: passed; isolated database volumes removed"
