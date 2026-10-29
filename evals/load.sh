#!/usr/bin/env sh
# Mixed public-API load, fault and soak profiles against a fresh exact-role
# stack: one gateway and one core worker built with EVAL_PRODUCT_PROFILE
# (release by default), PostgreSQL from the database fixture, and a loopback
# 1024-dimension stub embedding provider whose delay and error share each
# phase sets.
#
#   evals/load.sh evals/load/profiles/<name>.json
#
# EVAL_LOAD_REPORT names where the final report is copied; the checkpoint
# itself stays in the fixture's private scratch state until the run ends.
# EVAL_LOAD_RESUME_ATTEMPTS (default 3) bounds how often an interrupted
# harness resumes its own checkpoint against the same live stack.
set -eu

cd "$(dirname "$0")/.."

if [ "${SYNVEDA_EVAL_EXACT_DATABASE:-}" != 1 ]; then
  SYNVEDA_DB_TEST_TASK=load-evaluation
  export SYNVEDA_DB_TEST_TASK
  exec bash scripts/db-test.sh "$@"
fi

[ "$#" -eq 1 ] && [ -f "$1" ] || {
  echo "load: usage: evals/load.sh <profile.json>" >&2
  exit 64
}
for load_tool in psql python3; do
  command -v "$load_tool" >/dev/null 2>&1 || {
    echo "load: $load_tool is required for database samples, faults and environment facts" >&2
    exit 69
  }
done
load_profile=$1
load_attempts=${EVAL_LOAD_RESUME_ATTEMPTS:-3}
case "$load_attempts" in
  ''|*[!0-9]*|0)
    echo "load: EVAL_LOAD_RESUME_ATTEMPTS must be a positive integer" >&2
    exit 64
    ;;
esac
[ "$load_attempts" -le 20 ] || {
  echo "load: EVAL_LOAD_RESUME_ATTEMPTS must be at most 20" >&2
  exit 64
}

load_field() { # json-path field
  python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"
}
EVAL_LOAD_AGENTS=$(load_field "$load_profile" agents)
EVAL_LOAD_NOISY_AGENTS=$(load_field "$load_profile" noisy_agents)
load_seconds=$(python3 -c 'import json, sys
print(sum(phase["seconds"] for phase in json.load(open(sys.argv[1]))["phases"]))' "$load_profile")
# Every bearer must outlive the run, its setup and a resumed tail.
EVAL_LOAD_TOKEN_TTL_SECS=$((load_seconds + 3600))
EVAL_PRODUCT_PROFILE=${EVAL_PRODUCT_PROFILE:-release}
# The reference deployment's per-process pool, not the evaluation fixture's.
SYNVEDA_DB_MAX_CONNECTIONS=${EVAL_LOAD_DB_MAX_CONNECTIONS:-8}
SYNVEDA_WORKER_DB_MAX_CONNECTIONS=${EVAL_LOAD_DB_MAX_CONNECTIONS:-8}
export EVAL_LOAD_AGENTS EVAL_LOAD_NOISY_AGENTS EVAL_LOAD_TOKEN_TTL_SECS EVAL_PRODUCT_PROFILE
export SYNVEDA_DB_MAX_CONNECTIONS SYNVEDA_WORKER_DB_MAX_CONNECTIONS

. evals/lib.sh

LOAD_PROVIDER_PID=
LOAD_SAMPLER_PID=
load_finish() {
  load_status=$1
  trap - EXIT INT TERM
  if [ -n "${EVAL_STATE:-}" ] && [ -f "$EVAL_STATE/load-fault/pids" ]; then
    EVAL_LOAD_FAULT_STATE=$EVAL_STATE/load-fault sh evals/load-fault-hook.sh stop cpu_contention || true
  fi
  for load_pid in $LOAD_SAMPLER_PID $LOAD_PROVIDER_PID; do
    kill -TERM "$load_pid" 2>/dev/null || true
  done
  eval_finish "$load_status"
}
trap 'load_finish $?' EXIT
trap 'load_finish 130' INT
trap 'load_finish 143' TERM

load_dir=$(eval_product_dir)
SQLX_OFFLINE=true cargo build -p synveda-eval
if [ "$load_dir" = release ]; then
  SQLX_OFFLINE=true cargo build --release -p synveda-eval
fi
LOAD_PROVIDER_URL=http://127.0.0.1:8161
eval_port_free "$LOAD_PROVIDER_URL"
"./target/$load_dir/synveda-eval" provider-stub --listen 127.0.0.1:8161 --dimension 1024 &
LOAD_PROVIDER_PID=$!
SYNVEDA_EMBEDDER=tei
SYNVEDA_TEI_URL=$LOAD_PROVIDER_URL
SYNVEDA_EMBEDDER_MODEL=load-stub/1024
export SYNVEDA_EMBEDDER SYNVEDA_TEI_URL SYNVEDA_EMBEDDER_MODEL

eval_up

# Database samples and the CPU-contention fault use the fixture's migrator
# login through libpq environment variables, never argv.
load_database_environment() {
  python3 - "$eval_migrator_database_file" <<'PY'
import shlex, sys, urllib.parse
url = urllib.parse.urlsplit(open(sys.argv[1]).read().strip())
for key, value in (("PGHOST", url.hostname), ("PGPORT", url.port or 5432),
                   ("PGUSER", urllib.parse.unquote(url.username or "")),
                   ("PGPASSWORD", urllib.parse.unquote(url.password or "")),
                   ("PGDATABASE", url.path.lstrip("/"))):
    print(f"export {key}={shlex.quote(str(value))}")
PY
}
eval "$(load_database_environment)"
export PGCONNECT_TIMEOUT=5

LOAD_SAMPLES=$EVAL_STATE/database-samples.jsonl
: >"$LOAD_SAMPLES"
(
  while :; do
    psql -X -q -At -v ON_ERROR_STOP=1 -c "select json_build_object(
      'at', now(),
      'database_bytes', pg_database_size(current_database()),
      'connections', (select coalesce(json_object_agg(key, total), '{}'::json) from (
          select coalesce(usename, 'unknown') || ':' || coalesce(state, 'none') as key, count(*) as total
          from pg_stat_activity where datname = current_database() group by 1) as activity),
      'audit_log_bytes', pg_total_relation_size('public.audit_log'),
      'session_events_bytes', pg_total_relation_size('public.session_events'),
      'capture_candidates_bytes', pg_total_relation_size('public.capture_candidates'),
      'knowledge_revisions_bytes', pg_total_relation_size('public.knowledge_revisions'),
      'knowledge_embeddings_bytes', pg_total_relation_size('public.knowledge_revision_embeddings'),
      'audit_log_rows_estimate', (select reltuples::bigint from pg_class where oid = 'public.audit_log'::regclass),
      'session_events_rows_estimate', (select reltuples::bigint from pg_class where oid = 'public.session_events'::regclass))" \
      >>"$LOAD_SAMPLES" 2>/dev/null || true
    sleep 15
  done
) &
LOAD_SAMPLER_PID=$!

load_environment=$EVAL_STATE/load-environment.json
load_server=$(psql -X -q -At -c "select current_setting('server_version') || ', pgvector ' ||
  (select extversion from pg_extension where extname = 'vector') || ', shared_buffers ' ||
  current_setting('shared_buffers') || ', work_mem ' || current_setting('work_mem') ||
  ', jit ' || current_setting('jit') || ', max_connections ' || current_setting('max_connections')")
case "$(uname -s)" in
  Darwin) load_hardware="$(sysctl -n hw.model) $(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.ncpu) CPUs, $(($(sysctl -n hw.memsize) / 1073741824)) GiB" ;;
  *) load_hardware="$(uname -m), $(nproc) CPUs, $(awk '/MemTotal/ {printf "%d GiB", $2 / 1048576}' /proc/meminfo)" ;;
esac
load_engine=$(docker info --format '{{.OperatingSystem}} {{.ServerVersion}}, {{.NCPU}} CPUs, {{.MemTotal}} bytes' 2>/dev/null || echo unknown)
load_commit=$(git rev-parse HEAD)
git diff --quiet HEAD -- || load_commit="$load_commit+uncommitted"
python3 - "$load_environment" "$load_commit" "$load_dir" "$load_hardware" "$load_engine" "$load_server" "$SYNVEDA_DB_MAX_CONNECTIONS" <<'PY'
import json, sys
path, commit, build, hardware, engine, server, pool = sys.argv[1:]
json.dump({
    "source_commit": commit,
    "product_build": f"synveda-gateway and synveda-worker, cargo {build} profile",
    "topology": f"one gateway and one core worker as host processes, {pool} database connections each; PostgreSQL in the exact-role database fixture container; loopback HTTP",
    "hardware": f"host {hardware}; container engine {engine}",
    "database": f"PostgreSQL {server}; database fixture defaults",
    "embedding": "loopback stub TEI provider, deterministic 1024-dimension vectors, model load-stub/1024; no model inference",
    "identities": "ordinary service identities with fixture-issued HS256 bearers through gateway authentication, tenant resolution, admission, Cedar and forced RLS; RS256/JWKS verification is not exercised",
    "notes": [
        "deterministic Capture extractor",
        "measured gateway admission at product defaults (enforce); the fixture's bootstrap gateway observes admission while provisioning agents",
    ],
}, open(path, "w"), indent=2)
PY

EVAL_LOAD_FAULT_STATE=$EVAL_STATE/load-fault
export EVAL_LOAD_FAULT_STATE
load_report=$EVAL_STATE/load-report.json
load_attempt=0
load_resume=
until "./target/$load_dir/synveda-eval" load \
    --env "$EVAL_ENV" \
    --profile "$load_profile" \
    --report "$load_report" \
    --environment "$load_environment" \
    --worker-url "$EVAL_WORKER_URL" \
    --provider-url "$LOAD_PROVIDER_URL" \
    --fault-hook evals/load-fault-hook.sh \
    --gateway-pid "$EVAL_PID" \
    --worker-pid "$EVAL_WORKER_PID" \
    --database-samples "$LOAD_SAMPLES" \
    $load_resume; do
  load_attempt=$((load_attempt + 1))
  [ "$load_attempt" -lt "$load_attempts" ] && [ -f "$load_report" ] || {
    echo "load: the harness stopped after $load_attempt attempt(s)" >&2
    exit 1
  }
  echo "load: resuming the checkpoint (attempt $((load_attempt + 1)) of $load_attempts)" >&2
  load_resume=--resume
done

"./target/$load_dir/synveda-eval" load-check --report "$load_report" --baselines evals/load/baselines
if [ -n "${EVAL_LOAD_REPORT:-}" ]; then
  cp "$load_report" "$EVAL_LOAD_REPORT"
  echo "load: report copied to $EVAL_LOAD_REPORT"
fi
