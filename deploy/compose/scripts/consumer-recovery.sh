#!/bin/sh
# OPS-12: explicit recovery of the candidate graph, never host-state volumes.
set -eu
set +x
umask 077
bundle=$(CDPATH= cd "$(dirname "$0")/../../.." && pwd -P)
action=${1:-}
backup_id=${2:-}
project=${COMPOSE_PROJECT_NAME:-synveda-local}
source=${3:-$project}
confirmation=${SYNVEDA_CONFIRM_RESTORE:-}
refuse() { echo "consumer recovery: $*" >&2; exit 78; }
valid_project() {
  case "$1" in synveda-local) return ;; synveda-local-acceptance-[a-z0-9]*) ;; *) refuse 'project must use the consumer layout' ;; esac
  suffix=${1#synveda-local-acceptance-}
  case "$suffix" in *[!a-z0-9-]*|*-|*--*) refuse 'invalid consumer project suffix' ;; esac
  [ "${#suffix}" -le 24 ] || refuse 'consumer project suffix is too long'
}
case "$action:$#" in backup:2|verify:2|restore:3) ;; *) refuse 'usage: synveda-recovery {backup|verify} ID | restore ID SOURCE_PROJECT' ;; esac
case "$backup_id" in ''|*[!a-z0-9-]*|-*|*-|*--*) refuse 'invalid lowercase backup id' ;; esac
[ "${#backup_id}" -le 64 ] || refuse 'backup id is too long'
valid_project "$project"
valid_project "$source"
if [ "$action" = restore ]; then
  [ "$project" != "$source" ] || refuse 'restore requires a different, empty target project'
  [ "$confirmation" = "$source:$backup_id:$project" ] || refuse "restore requires SYNVEDA_CONFIRM_RESTORE=$source:$backup_id:$project"
fi

# Pin one local engine for the whole ceremony. No utility receives its socket.
if [ -n "${DOCKER_CONTEXT:-}" ] || [ -z "${DOCKER_HOST:-}" ]; then
  endpoint=$(docker context inspect "${DOCKER_CONTEXT:-$(docker context show)}" --format '{{.Endpoints.docker.Host}}')
else endpoint=$DOCKER_HOST; fi
case "$endpoint" in unix:///*) ;; *) refuse 'recovery requires a local Unix-socket Docker engine' ;; esac
export DOCKER_HOST=$endpoint
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH
unset COMPOSE_FILE COMPOSE_PROFILES COMPOSE_ENV_FILES
for name in $(env | sed -n 's/^\(SYNVEDA_[A-Z0-9_]*\)=.*/\1/p'); do unset "$name"; done
export COMPOSE_PROJECT_NAME=$project SYNVEDA_BACKUP_ID=$backup_id SYNVEDA_RECOVERY_SOURCE=$source SYNVEDA_CONFIRM_RESTORE=$confirmation
runtime=$bundle/deploy/compose/consumer-runtime.yaml
overlay=$bundle/deploy/compose/consumer-restore.yaml
compose() {
  if [ "$action" = restore ]; then
    docker compose --env-file /dev/null --project-directory "$bundle" -p "$project" -f "$runtime" -f "$overlay" "$@"
  else
    docker compose --env-file /dev/null --project-directory "$bundle" -p "$project" -f "$runtime" "$@"
  fi
}
source_compose() {
  docker compose --env-file /dev/null --project-directory "$bundle" -p "$source" -f "$runtime" "$@"
}
state() { compose run --rm --no-deps recovery-state "$1" "$backup_id"; }
require_source_volume() {
  labels=$(docker volume inspect --format '{{index .Labels "com.docker.compose.project"}}:{{index .Labels "com.docker.compose.volume"}}' "${source}_$1")
  [ "$labels" = "$source:$1" ] || refuse 'source volume ownership does not match the consumer project'
}
if [ "$action" = backup ]; then
  require_source_volume installation
  require_source_volume postgres-data
fi
if [ "$action" = restore ]; then
  # Enumerate successfully, rather than treating inspect/network failures as absence.
  containers=$(docker ps -aq --filter "label=com.docker.compose.project=$project")
  [ -z "$containers" ] || refuse 'retained target containers refused'
  volumes=$(docker volume ls -q --filter "name=^${project}_")
  [ -z "$volumes" ] || refuse 'retained target volumes refused'
  networks=$(docker network ls -q --filter "label=com.docker.compose.project=$project")
  [ -z "$networks" ] || refuse 'retained target networks refused'
  containers=$(docker ps -aq --filter "label=com.docker.compose.project=$source")
  [ -z "$containers" ] || refuse 'source must be down before restoring its issuer and network configuration'
fi
if [ "$action" != backup ]; then require_source_volume recovery; fi
case "$action" in
  backup)
    # Validate the seal and reserve the immutable ID before stopping anything.
    state begin-backup
    trap 'echo "consumer recovery interrupted; preserve volumes and inspect .recovery-operation before any up or retry" >&2' EXIT
    # Stop every profile, including one-shot mutators and sample clients. Start
    # only PostgreSQL for its native dumps; no application writer can be started
    # by a dependency during the quiesced interval.
    compose --profile '*' stop --timeout 210
    compose up -d --no-deps --no-build --wait --wait-timeout 180 postgres
    compose run --rm --no-deps database-backup
    state snapshot
    compose run --rm --no-deps recovery-check verify "$backup_id"
    compose --profile '*' down
    state unlock
    trap - EXIT
    echo "Paired backup $source/$backup_id verified in ${source}_recovery. Source is down; all volumes retained."
    ;;
  verify) compose run --rm --no-deps recovery-check verify "$backup_id" ;;
  restore)
    # The read-only preflight uses the existing source volume, before allocating
    # either target volume. The restore helper repeats validation before copying.
    source_compose run --rm --no-deps recovery-check verify "$backup_id"
    trap 'echo "consumer restore interrupted; target remains private; preserve volumes and inspect .recovery-operation" >&2' EXIT
    state restore
    compose up -d --no-deps --no-build --wait --wait-timeout 180 postgres
    compose run --rm --no-deps database-bootstrap
    compose run --rm --no-deps keycloak-database-bootstrap
    compose run --rm --no-deps database-restore
    compose run --rm --no-deps database-bootstrap
    compose run --rm --no-deps keycloak-database-bootstrap
    compose run --rm --no-deps recovery-verify
    compose run --rm --no-deps recovery-key-refusal
    state unlock
    compose up -d --no-build --wait --wait-timeout 900
    trap - EXIT
    echo "Restored $source/$backup_id into private $project with the original keys. Verify browser/API access before resuming use."
    ;;
esac
