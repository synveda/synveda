#!/bin/sh
# CPR-45: Docker remains on the host. The preparation container sees no socket.
set -eu
set +x
umask 077
bundle=$(CDPATH= cd "$(dirname "$0")/../../.." && pwd -P)
state=${SYNVEDA_HOME:-${HOME:?HOME is required}/.synveda}/state
action=${1:-up}
shift "$(( $# > 0 ? 1 : 0 ))"
case "$action" in
  prepare|up|config|status|logs|stop|down|restart|credential|sample|backup|restore|reset) ;;
  *) echo 'usage: synveda-compose {prepare|up|config|status|logs|stop|down|restart|credential|sample|backup|restore|reset}' >&2; exit 64 ;;
esac
account=${1:-author}
if [ "$action" = credential ] || [ "$action" = backup ] || [ "$action" = restore ]; then
  [ "$#" -le 1 ] || exit 64
else
  [ "$#" -eq 0 ] || { echo 'only credential, backup and restore accept an argument' >&2; exit 64; }
fi
reset_confirmation=${SYNVEDA_CONFIRM_RESET:-}
restore_confirmation=${SYNVEDA_CONFIRM_RESTORE:-}
backup_id=${1:-}
[ "$(id -u)" -ne 0 ] || { echo 'run as an ordinary account with Docker access' >&2; exit 78; }
# Bind-mounted private state belongs to this host, never a remote Docker host.
# Resolve once so a context change cannot redirect a later lifecycle command.
if [ -n "${DOCKER_CONTEXT:-}" ] || [ -z "${DOCKER_HOST:-}" ]; then
  endpoint=$(docker context inspect "${DOCKER_CONTEXT:-$(docker context show)}" --format '{{.Endpoints.docker.Host}}')
else
  endpoint=$DOCKER_HOST
fi
case "$endpoint" in unix:///*) ;; *) echo 'evaluation requires a local Unix-socket Docker engine; remote contexts are refused' >&2; exit 78 ;; esac
case "$endpoint" in *[[:space:]]*) echo 'Docker endpoint contains whitespace' >&2; exit 78 ;; esac
export DOCKER_HOST=$endpoint
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH
case "$state:$bundle" in *[!a-zA-Z0-9_./:-]*) echo 'use deployment/state paths without whitespace or shell metacharacters' >&2; exit 78 ;; esac
[ ! -L "$state" ] || { echo 'state directory must not be a symlink' >&2; exit 78; }
mkdir -p "$state"
state=$(CDPATH= cd "$state" && pwd -P)
case "$action" in
  config|status|logs|credential) ;;
  *)
    lock=$state/.evaluation-operation
    if ! mkdir -m 700 "$lock" 2>/dev/null; then
      echo 'another evaluation operation owns state/.evaluation-operation; wait for it, or confirm its process/container has stopped before removing the empty lock directory' >&2
      exit 75
    fi
    trap 'rmdir "$lock"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM ;;
esac
IFS= read -r product_image < "$bundle/product-image"
case "$product_image" in *[!a-zA-Z0-9_./:@-]*|'') echo 'invalid release image reference' >&2; exit 78 ;; esac

utility() {
  docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --pids-limit 64 --memory 256m \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,nodev,size=32m \
    --mount "type=bind,source=$bundle,target=/bundle,readonly" \
    --mount "type=bind,source=$state,target=/state" \
    -e "SYNVEDA_HOST_BUNDLE=$bundle" -e "SYNVEDA_HOST_STATE=$state" \
    --entrypoint /usr/bin/timeout "$product_image" --signal=TERM --kill-after=5s 90s \
    node /bundle/deploy/compose/scripts/evaluation-recovery.mjs "$@"
}
if [ "$action" = backup ] || [ "$action" = restore ]; then
  case "$backup_id" in ''|*[!a-z0-9-]*|-*|*-|*--*) echo 'supply a new lowercase backup id, at most 64 characters' >&2; exit 64 ;; esac
  [ "${#backup_id}" -le 64 ] || exit 64
fi
if [ "$action" = restore ]; then
  [ "$restore_confirmation" = "restore:synveda-evaluation:$backup_id" ] || {
    echo "restore requires SYNVEDA_CONFIRM_RESTORE=restore:synveda-evaluation:$backup_id and an absent target database volume" >&2; exit 78;
  }
  if docker volume inspect synveda-evaluation_postgres-data >/dev/null 2>&1; then
    echo 'restore refuses retained data; use a separate Docker host or back up and deliberately reset the disposable evaluation first' >&2; exit 78
  fi
  utility restore "$backup_id"
fi
if [ "$action" = prepare ] || [ "$action" = up ]; then
  # Refuse creating replacement credentials for retained data, even after the
  # private directory was lost. Recovery requires the matching original keys.
  if docker volume inspect synveda-evaluation_postgres-data >/dev/null 2>&1 && \
      [ ! -f "$state/synveda-evaluation/secrets/.synveda-private-directory" ]; then
    echo 'retained synveda-evaluation_postgres-data requires its original state/secrets; restore them before startup' >&2
    exit 78
  fi
  docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --pids-limit 64 --memory 256m \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,nodev,size=32m \
    --mount "type=bind,source=$bundle,target=/bundle,readonly" \
    --mount "type=bind,source=$state,target=/state" \
    -e "SYNVEDA_HOST_BUNDLE=$bundle" -e "SYNVEDA_HOST_STATE=$state" \
    --entrypoint /usr/bin/timeout "$product_image" --signal=TERM --kill-after=5s 90s \
    /usr/bin/flock -w 30 /state/.prepare.lock \
    /usr/local/bin/node /bundle/deploy/compose/scripts/prepare-evaluation.mjs
  [ "$action" != prepare ] || exit 0
fi
[ -f "$state/evaluation.env" ] && [ -f "$state/evaluation-files" ] || {
  echo 'run ./synveda-compose prepare first; retain the generated state for every restart' >&2; exit 78;
}
# Prepared values own interpolation. Inherited source-development settings must
# not change the issuer, image, secret location or network on a later restart.
for name in $(env | sed -n 's/^\(SYNVEDA_[A-Z0-9_]*\)=.*/\1/p'); do unset "$name"; done
unset COMPOSE_FILE COMPOSE_PROFILES COMPOSE_ENV_FILES COMPOSE_PROJECT_NAME
# Explicit files and env input prevent automatic source builds or ambient .env
# selection. Do not source generated data as executable shell.
set -- --project-name synveda-evaluation --env-file "$state/evaluation.env"
while IFS= read -r fragment; do
  case "$fragment" in compose.yaml|compose.postgres.yaml|compose.keycloak.yaml|compose.keycloak-postgres.yaml|compose.evaluation.yaml|compose.demo.yaml) ;;
    *) echo 'unknown prepared Compose fragment; rerun preparation' >&2; exit 78 ;;
  esac
  set -- "$@" -f "$bundle/deploy/compose/$fragment"
done < "$state/evaluation-files"
case "$action" in
  up) docker compose "$@" up --detach --no-build --wait --wait-timeout 600
      echo 'Open the console URL printed by preparation. Run ./synveda-compose credential for deliberate first-login retrieval.' ;;
  config) docker compose "$@" config --quiet ;;
  status) docker compose "$@" ps --all ;;
  logs) docker compose "$@" logs --tail 100 ;;
  stop) docker compose "$@" stop ;;
  down) docker compose "$@" down ;;
  restart) docker compose "$@" restart gateway worker ;;
  sample)
    case "$(cat "$state/evaluation-files")" in *compose.demo.yaml*) ;; *) echo 'sample requires explicit demoAccounts=true on a fresh evaluation' >&2; exit 78 ;; esac
    echo 'Preparing fictional sample content for review using distinct demo accounts. This is synthetic replay; approval remains your explicit action.'
    docker compose "$@" -f "$bundle/deploy/compose/compose.browser-acceptance.yaml" \
      -f "$bundle/deploy/compose/compose.evaluation-sample.yaml" \
      run --rm --no-deps --entrypoint node browser-acceptance product-demo.mjs sample ;;
  backup)
    destination=$state/backups/$backup_id
    [ ! -e "$destination" ] && [ ! -L "$destination" ] || { echo 'backup id already exists; sets are never overwritten' >&2; exit 78; }
    mkdir -p "$state/backups"
    mkdir -m 700 "$destination" "$destination/database"
    export SYNVEDA_BACKUP_PROJECT=synveda-evaluation SYNVEDA_BACKUP_ID=$backup_id SYNVEDA_BACKUP_STAGING_DIR=$destination/database
    docker compose "$@" stop --timeout 210 gateway worker keycloak-realm-convergence keycloak
    if docker compose "$@" -f "$bundle/deploy/compose/compose.backup.yaml" run --rm --no-deps database-backup && utility snapshot "$backup_id"; then
      echo "Backup complete: $destination. Copy the entire private set to separately protected storage."
    else
      echo 'backup failed; incomplete set retained for inspection; resume with up after resolving the error' >&2; exit 78
    fi
    docker compose "$@" up --detach --no-build --wait --wait-timeout 600 ;;
  restore)
    export SYNVEDA_RESTORE_DATABASE_DIR=$state/backups/$backup_id/database
    export SYNVEDA_RESTORE_WRONG_KMS_KEY_FILE=$state/synveda-evaluation/secrets/synveda_kms_key
    docker compose "$@" up --detach --no-build --wait --wait-timeout 600 postgres
    docker compose "$@" run --rm --no-deps database-bootstrap
    docker compose "$@" run --rm --no-deps keycloak-database-bootstrap
    docker compose "$@" -f "$bundle/deploy/compose/compose.restore.yaml" run --rm --no-deps database-restore
    docker compose "$@" run --rm --no-deps database-bootstrap
    docker compose "$@" run --rm --no-deps keycloak-database-bootstrap
    docker compose "$@" -f "$bundle/deploy/compose/compose.restore.yaml" run --rm --no-deps recovery-verify
    docker compose "$@" up --detach --no-build --wait --wait-timeout 600
    echo 'Restored database pair with matching keys. Sign in and verify the expected workspace before resuming use.' ;;
  credential)
    [ -t 1 ] || { echo 'credential retrieval requires an interactive terminal; read the private password file directly for automation' >&2; exit 78; }
    case "$account" in author) role=admin ;; reviewer) role=member ;; approver|viewer) role=$account ;;
      *) echo 'credential account must be author, reviewer, approver or viewer' >&2; exit 64 ;; esac
    case "$(cat "$state/evaluation-files")" in *compose.demo.yaml*) ;; *) echo 'evaluation accounts were not enabled' >&2; exit 78 ;; esac
    printf 'Evaluation account: synveda-demo-%s\nPassword (keep private): ' "$role"
    cat "$state/synveda-evaluation/secrets/keycloak_demo_${role}_password" ;;
  reset)
    [ "$reset_confirmation" = 'delete:synveda-evaluation:postgres-data' ] || {
      echo 'reset requires SYNVEDA_CONFIRM_RESET=delete:synveda-evaluation:postgres-data; back up databases and matching keys first' >&2; exit 78;
    }
    docker compose "$@" down
    docker volume rm synveda-evaluation_postgres-data
    echo 'Evaluation database removed. Credentials and encryption keys are retained; ordinary down never removes data.' ;;
esac
