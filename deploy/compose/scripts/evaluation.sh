#!/bin/sh
# Docker remains on the host. The preparation container sees no socket.
set -eu
set +x
umask 077
bundle=$(CDPATH= cd "$(dirname "$0")/../../.." && pwd -P)
state=${SYNVEDA_HOME:-${HOME:?HOME is required}/.synveda}/state
action=${1:-up}
shift "$(( $# > 0 ? 1 : 0 ))"
case "$action" in
  prepare|up|config|status|logs|stop|down|restart|credential|sample|backup|restore|reset|upgrade) ;;
  *) echo 'usage: synveda-compose {prepare|up|config|status|logs|stop|down|restart|credential|sample|backup|restore|reset|upgrade}' >&2; exit 64 ;;
esac
account=${1:-author}
if [ "$action" = credential ] || [ "$action" = backup ] || [ "$action" = restore ] || [ "$action" = upgrade ]; then
  [ "$#" -le 1 ] || exit 64
else
  [ "$#" -eq 0 ] || { echo 'only credential, backup, restore and upgrade accept an argument' >&2; exit 64; }
fi
reset_confirmation=${SYNVEDA_CONFIRM_RESET:-}
restore_confirmation=${SYNVEDA_CONFIRM_RESTORE:-}
upgrade_confirmation=${SYNVEDA_CONFIRM_UPGRADE:-}
upgrade_from=${SYNVEDA_UPGRADE_FROM:-}
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

# A recovery set is made by the release that will restore it.
release_utility() {
  utility_image=$1 utility_bundle=$2
  shift 2
  docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --pids-limit 64 --memory 256m \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,nodev,size=32m \
    --mount "type=bind,source=$utility_bundle,target=/bundle,readonly" \
    --mount "type=bind,source=$state,target=/state" \
    -e "SYNVEDA_HOST_BUNDLE=$utility_bundle" -e "SYNVEDA_HOST_STATE=$state" \
    --entrypoint /usr/bin/timeout "$utility_image" --signal=TERM --kill-after=5s 90s \
    node /bundle/deploy/compose/scripts/evaluation-recovery.mjs "$@"
}
utility() { release_utility "$product_image" "$bundle" "$@"; }
prepare_state() {
  docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --pids-limit 64 --memory 256m \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,nodev,size=32m \
    --mount "type=bind,source=$bundle,target=/bundle,readonly" \
    --mount "type=bind,source=$state,target=/state" \
    -e "SYNVEDA_HOST_BUNDLE=$bundle" -e "SYNVEDA_HOST_STATE=$state" \
    --entrypoint /usr/bin/timeout "$product_image" --signal=TERM --kill-after=5s 90s \
    /usr/bin/flock -w 30 /state/.prepare.lock \
    /usr/local/bin/node /bundle/deploy/compose/scripts/prepare-evaluation.mjs
}
if [ "$action" = backup ] || [ "$action" = restore ] || [ "$action" = upgrade ]; then
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
  prepare_state
  [ "$action" != prepare ] || exit 0
fi
if [ "$action" = upgrade ]; then
  [ "$upgrade_confirmation" = "upgrade:synveda-evaluation:$backup_id" ] || {
    echo "upgrade requires SYNVEDA_CONFIRM_UPGRADE=upgrade:synveda-evaluation:$backup_id and SYNVEDA_UPGRADE_FROM naming the installed release's extracted bundle" >&2; exit 78;
  }
  case "$upgrade_from" in /*) ;; *) echo 'SYNVEDA_UPGRADE_FROM must be the absolute path of the installed release bundle' >&2; exit 78 ;; esac
  from=$(CDPATH= cd "$upgrade_from" && pwd -P)
  case "$from" in *[!a-zA-Z0-9_./:-]*) echo 'use an installed bundle path without whitespace or shell metacharacters' >&2; exit 78 ;; esac
  [ "$from" != "$bundle" ] || { echo 'run upgrade from the target bundle; SYNVEDA_UPGRADE_FROM names the installed one' >&2; exit 78; }
  IFS= read -r from_image < "$from/product-image"
  case "$from_image" in *[!a-zA-Z0-9_./:@-]*|'') echo 'invalid installed release image reference' >&2; exit 78 ;; esac
  [ "$from_image" != "$product_image" ] || { echo 'the installed and target bundles carry the same product image; nothing to upgrade' >&2; exit 78; }
  docker volume inspect synveda-evaluation_postgres-data >/dev/null 2>&1 && \
    [ -f "$state/synveda-evaluation/secrets/.synveda-private-directory" ] || {
      echo 'upgrade requires the retained synveda-evaluation database and its original state' >&2; exit 78;
    }
  receipt=$state/upgrades/$backup_id
  upgrade_phase=none
  if [ -f "$receipt" ]; then
    grep -qx "from_image=$from_image" "$receipt" && grep -qx "to_image=$product_image" "$receipt" || {
      echo 'this upgrade receipt names other releases; use a new backup id' >&2; exit 78;
    }
    upgrade_phase=$(sed -n 's/^phase=//p' "$receipt")
    case "$upgrade_phase" in
      writers-stopped|backed-up|prepared)
        for service in gateway worker; do
          [ -z "$(docker ps --quiet --filter label=com.docker.compose.project=synveda-evaluation --filter "label=com.docker.compose.service=$service" --filter status=running)" ] || {
            echo "$service is running again since this upgrade stopped it, so backup $backup_id may be stale; start a new upgrade with a new backup id" >&2; exit 78;
          }
        done ;;
    esac
  else
    # Only the release that prepared this installation may hand it over.
    grep -qx "SYNVEDA_PRODUCT_IMAGE='$from_image'" "$state/evaluation.env" || {
      echo 'SYNVEDA_UPGRADE_FROM is not the release this installation runs' >&2; exit 78;
    }
    [ ! -e "$state/backups/$backup_id" ] && [ ! -L "$state/backups/$backup_id" ] || { echo 'backup id already exists; sets are never overwritten' >&2; exit 78; }
    mkdir -p "$state/upgrades"
    printf 'format=synveda-evaluation-upgrade-v1\nbackup_id=%s\nfrom_bundle=%s\nfrom_image=%s\nto_image=%s\nphase=started\nstarted_at=%s\n' \
      "$backup_id" "$from" "$from_image" "$product_image" "$(date +%s)" > "$receipt.tmp"
    mv "$receipt.tmp" "$receipt"
    upgrade_phase=started
  fi
fi
[ -f "$state/evaluation.env" ] && [ -f "$state/evaluation-files" ] || {
  echo 'run ./synveda-compose prepare first; retain the generated state for every restart' >&2; exit 78;
}
# Prepared values own interpolation. Inherited source-development settings must
# not change the issuer, image, secret location or network on a later restart.
for name in $(env | sed -n 's/^\(SYNVEDA_[A-Z0-9_]*\)=.*/\1/p'); do unset "$name"; done
unset COMPOSE_FILE COMPOSE_PROFILES COMPOSE_ENV_FILES COMPOSE_PROJECT_NAME
if [ "$action" = upgrade ]; then
  record_phase() {
    sed '/^phase=/d' "$receipt" > "$receipt.tmp"
    printf 'phase=%s\n%s_at=%s\n' "$1" "$1" "$(date +%s)" >> "$receipt.tmp"
    mv "$receipt.tmp" "$receipt"
    upgrade_phase=$1
  }
  before_schema_change() {
    echo "upgrade stopped before any schema change; the database is still the installed release's. Restart it with $from/synveda-compose up, or fix the cause and rerun this command to resume." >&2
    exit 78
  }
  starting_compose() {
    fragments=$(cat "$state/evaluation-files")
    for fragment in $fragments; do
      case "$fragment" in compose.yaml|compose.postgres.yaml|compose.keycloak.yaml|compose.keycloak-postgres.yaml|compose.evaluation.yaml|compose.demo.yaml) ;;
        *) echo 'unknown prepared Compose fragment' >&2; return 78 ;;
      esac
    done
    # Validated fragment names and bundle path contain no field separators.
    # shellcheck disable=SC2046
    set -- --project-name synveda-evaluation --env-file "$state/evaluation.env" \
      $(for fragment in $fragments; do printf -- '-f %s/deploy/compose/%s ' "$from" "$fragment"; done) "$@"
    docker compose "$@"
  }
  if [ "$upgrade_phase" = started ]; then
    starting_compose stop --timeout 210 gateway worker keycloak-realm-convergence keycloak || before_schema_change
    record_phase writers-stopped
  fi
  if [ "$upgrade_phase" = writers-stopped ]; then
    destination=$state/backups/$backup_id
    [ ! -e "$destination" ] && [ ! -L "$destination" ] || { echo 'an interrupted upgrade left a partial backup; inspect it, then use a new backup id' >&2; before_schema_change; }
    mkdir -p "$state/backups"
    mkdir -m 700 "$destination" "$destination/database"
    export SYNVEDA_BACKUP_PROJECT=synveda-evaluation SYNVEDA_BACKUP_ID=$backup_id SYNVEDA_BACKUP_STAGING_DIR=$destination/database
    starting_compose -f "$from/deploy/compose/compose.backup.yaml" run --rm --no-deps database-backup && \
      release_utility "$from_image" "$from" snapshot "$backup_id" || before_schema_change
    unset SYNVEDA_BACKUP_PROJECT SYNVEDA_BACKUP_ID SYNVEDA_BACKUP_STAGING_DIR
    record_phase backed-up
  fi
  case "$upgrade_phase" in
    backed-up|prepared|migrating|migrated)
      prepare_state || before_schema_change
      [ "$upgrade_phase" != backed-up ] || record_phase prepared ;;
  esac
fi
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
  up) docker compose "$@" up --detach --no-build --wait --wait-timeout 900
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
    docker compose "$@" up --detach --no-build --wait --wait-timeout 900 ;;
  upgrade)
    after_schema_change() {
      echo "upgrade stopped after the schema change began. The installed release refuses this database: fix the cause and rerun this command to resume, or restore backup $backup_id with $from/synveda-compose restore into an empty installation." >&2
      exit 78
    }
    case "$upgrade_phase" in
      prepared)
        docker compose "$@" up --detach --no-build --wait --wait-timeout 900 postgres || before_schema_change
        docker compose "$@" run --rm --no-deps database-bootstrap || before_schema_change
        docker compose "$@" run --rm --no-deps keycloak-database-bootstrap || before_schema_change
        docker compose "$@" run --rm --no-deps database-preflight || before_schema_change
        docker compose "$@" run --rm --no-deps --no-TTY migrate migration-check || before_schema_change
        record_phase migrating ;;
      migrating)
        docker compose "$@" up --detach --no-build --wait --wait-timeout 900 postgres || after_schema_change
        docker compose "$@" run --rm --no-deps database-bootstrap || after_schema_change
        docker compose "$@" run --rm --no-deps keycloak-database-bootstrap || after_schema_change ;;
    esac
    if [ "$upgrade_phase" = migrating ]; then
      docker compose "$@" run --rm --no-deps --no-TTY migrate || after_schema_change
      record_phase migrated
    fi
    if [ "$upgrade_phase" = migrated ]; then
      docker compose "$@" up --detach --no-build --wait --wait-timeout 900 || after_schema_change
      record_phase complete
    fi
    echo "Upgrade complete. Keep $from and backup $backup_id until this release is accepted; the installed release cannot serve the upgraded database." ;;
  restore)
    export SYNVEDA_RESTORE_DATABASE_DIR=$state/backups/$backup_id/database
    export SYNVEDA_RESTORE_WRONG_KMS_KEY_FILE=$state/synveda-evaluation/secrets/synveda_kms_key
    docker compose "$@" up --detach --no-build --wait --wait-timeout 900 postgres
    docker compose "$@" run --rm --no-deps database-bootstrap
    docker compose "$@" run --rm --no-deps keycloak-database-bootstrap
    docker compose "$@" -f "$bundle/deploy/compose/compose.restore.yaml" run --rm --no-deps database-restore
    docker compose "$@" run --rm --no-deps database-bootstrap
    docker compose "$@" run --rm --no-deps keycloak-database-bootstrap
    docker compose "$@" -f "$bundle/deploy/compose/compose.restore.yaml" run --rm --no-deps recovery-verify
    docker compose "$@" up --detach --no-build --wait --wait-timeout 900
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
