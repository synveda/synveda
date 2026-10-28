#!/usr/bin/env sh
# Database fault for load phases: `start cpu_contention` runs four bounded
# CPU-bound queries in a loop on the fixture database; `stop` ends them.
# Connection settings come from libpq environment variables and state from
# EVAL_LOAD_FAULT_STATE, both supplied by evals/load.sh.
set -eu

[ "$#" -eq 2 ] && [ "$2" = cpu_contention ] || {
  echo "load-fault: usage: load-fault-hook.sh start|stop cpu_contention" >&2
  exit 64
}
state=${EVAL_LOAD_FAULT_STATE:?EVAL_LOAD_FAULT_STATE is required}
mkdir -p "$state"
pids=$state/pids

case "$1" in
  start)
    [ ! -s "$pids" ] || {
      echo "load-fault: contention is already running" >&2
      exit 70
    }
    for _ in 1 2 3 4; do
      (
        while :; do
          psql -X -q -At -c "select count(*) from generate_series(1, 3000000)" >/dev/null 2>&1 || sleep 1
        done
      ) &
      echo $! >>"$pids"
    done
    ;;
  stop)
    if [ -f "$pids" ]; then
      while read -r pid; do
        kill -TERM "$pid" 2>/dev/null || true
      done <"$pids"
      rm -f "$pids"
    fi
    ;;
  *)
    echo "load-fault: unknown action $1" >&2
    exit 64
    ;;
esac
