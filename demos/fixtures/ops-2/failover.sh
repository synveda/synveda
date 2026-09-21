#!/bin/sh
# OPS-2: recover the pre-existing session through the public API after CNPG
# promotion. Connection failures are expected while readiness removes the
# gateway from its Service; each attempt and the whole recovery are bounded.
set -eu

WORK=${WORK_DIR:-/work}
GATEWAY=${SYNVEDA_GATEWAY:?SYNVEDA_GATEWAY must be set}
bearer=$(synveda auth token)
run=$(cat "$WORK/session-id")
deadline=$(($(date +%s) + 240))
attempt=0
while [ "$attempt" -lt 120 ]; do
  remaining=$((deadline - $(date +%s)))
  [ "$remaining" -gt 0 ] || break
  request_timeout=$remaining
  [ "$request_timeout" -le 10 ] || request_timeout=10
  attempt=$((attempt + 1))
  : > "$WORK/failover-body"
  # A conditional keeps curl's transport failure from triggering set -e.
  # Reuse the key because a timeout may follow a committed context run.
  if code=$(curl -sS --connect-timeout 5 --max-time "$request_timeout" \
    -o "$WORK/failover-body" -w "%{http_code}" \
    -X POST "$GATEWAY/v1/sessions/$run/context-runs" \
    -H "Authorization: Bearer $bearer" -H "Content-Type: application/json" \
    -H "Idempotency-Key: ops2-failover-$run" \
    -d '{"query":"when does the release train leave"}'); then
    case "$code" in
      2??)
        if grep -q '"id"' "$WORK/failover-body"; then
          echo "    the context run succeeded after the failover (attempt $attempt)"
          exit 0
        fi
        ;;
      4??)
        echo "the context run was refused after failover (HTTP $code)" >&2
        exit 1
        ;;
    esac
  fi
  remaining=$((deadline - $(date +%s)))
  [ "$remaining" -gt 0 ] || break
  [ "$remaining" -le 2 ] || remaining=2
  sleep "$remaining"
done
echo "the context run never recovered within 240 seconds" >&2
exit 1
