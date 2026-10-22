#!/bin/sh
# OPS-5: one isolated PITR restore drill. It restores a custody generation into
# a disposable target project, records result, duration and generation age,
# then removes only that exact target. Schedule it with the host's scheduler.
set -eu
umask 077

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
compose_dir=$(dirname "$script_dir")
lifecycle=$script_dir/compose.sh

refuse() {
    echo "pitr-drill: $1" >&2
    exit "${2:-64}"
}

source_project=${SYNVEDA_RESTORE_SOURCE_PROJECT:-}
backup_id=${SYNVEDA_BACKUP_ID:-}
suffix=${SYNVEDA_PITR_DRILL_SUFFIX:-}
pool=${SYNVEDA_PITR_DRILL_IPV4_POOL:-}
result_root=${SYNVEDA_PITR_DRILL_RESULT_ROOT:-}
repository_config=${SYNVEDA_PITR_DRILL_REPOSITORY_CONFIG:-}
repository_ca=${SYNVEDA_PITR_DRILL_REPOSITORY_CA:-}
recovery_target=${SYNVEDA_PITR_TARGET:-latest}
custody_root=${SYNVEDA_RECOVERY_SECRETS_ROOT:-$compose_dir/backups/secrets/$source_project}
keep_target=${SYNVEDA_PITR_DRILL_KEEP:-false}

[ -n "$source_project" ] && [ -n "$backup_id" ] || refuse "set SYNVEDA_RESTORE_SOURCE_PROJECT and SYNVEDA_BACKUP_ID"
case "$suffix" in
    acceptance-drill-[a-z0-9]*) ;;
    *) refuse "SYNVEDA_PITR_DRILL_SUFFIX must start with acceptance-drill-" ;;
esac
[ -n "$pool" ] || refuse "set SYNVEDA_PITR_DRILL_IPV4_POOL to a private /24 unused by the source"
case "$result_root:$repository_config" in
    /*:/*) ;;
    *) refuse "SYNVEDA_PITR_DRILL_RESULT_ROOT and SYNVEDA_PITR_DRILL_REPOSITORY_CONFIG must be absolute" ;;
esac
case "$keep_target" in true|false) ;; *) refuse "SYNVEDA_PITR_DRILL_KEEP must be true|false" ;; esac
[ -d "$result_root" ] && [ ! -L "$result_root" ] && \
    [ "$(stat -c '%a' "$result_root")" = 700 ] || refuse "drill result root must be an existing mode-0700 directory" 78
[ -f "$repository_config" ] && [ ! -L "$repository_config" ] || refuse "drill repository configuration is unavailable" 78
binding=$custody_root/$backup_id/binding.json
[ -f "$binding" ] && [ ! -L "$binding" ] || refuse "custody generation $backup_id has no binding record" 78

runtime=${SYNVEDA_COMPOSE_RUNTIME:-development}
target_project=synveda-$runtime-$suffix
result_file=$result_root/$backup_id-$suffix.json
[ ! -e "$result_file" ] || refuse "drill result $result_file already exists; choose a new suffix" 73

# The target uses its own default project layout, never the source inputs.
unset SYNVEDA_SECRETS_DIR SYNVEDA_DATABASE_AUTHORITY_DIR SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR \
    SYNVEDA_OIDC_ISSUERS_FILE SYNVEDA_CONFIRM_RESET
SYNVEDA_COMPOSE_PROJECT_SUFFIX=$suffix
SYNVEDA_COMPOSE_IPV4_POOL=$pool
SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance
SYNVEDA_PITR_ENABLED=true
SYNVEDA_PITR_TARGET=$recovery_target
SYNVEDA_CONFIRM_RESTORE=$source_project:$backup_id:$target_project
export SYNVEDA_COMPOSE_PROJECT_SUFFIX SYNVEDA_COMPOSE_IPV4_POOL SYNVEDA_COMPOSE_PROFILES \
    SYNVEDA_PITR_ENABLED SYNVEDA_PITR_TARGET SYNVEDA_CONFIRM_RESTORE

started_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
started=$(date -u '+%s')
"$script_dir/generate-secrets.sh" --if-missing >/dev/null
"$script_dir/generate-issuer.sh" --if-missing >/dev/null
target_runtime=$compose_dir/runtime/$target_project
if [ ! -e "$target_runtime/pgbackrest.conf" ]; then
    install -m 0600 "$repository_config" "$target_runtime/pgbackrest.conf"
fi
if [ -n "$repository_ca" ] && [ ! -e "$target_runtime/pgbackrest-ca.pem" ]; then
    install -m 0600 "$repository_ca" "$target_runtime/pgbackrest-ca.pem"
fi

status=0
"$lifecycle" pitr-restore || status=$?
finished_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
duration=$(($(date -u '+%s') - started))

cleanup_status=0
if [ "$keep_target" = false ]; then
    "$lifecycle" down >/dev/null 2>&1 || cleanup_status=$?
    SYNVEDA_CONFIRM_RESET=$target_project "$lifecycle" reset >/dev/null 2>&1 || cleanup_status=$?
fi

result=passed
[ "$status" -eq 0 ] || result=failed
created_at=$(sed -n 's/.*"created_at":"\([0-9TZ:-]*\)".*/\1/p' "$binding")
generation_age=
if [ -n "$created_at" ]; then
    generation_age=$(($(date -u '+%s') - $(date -u -d "$created_at" '+%s' 2>/dev/null || echo "$started")))
fi
printf '{"format":"synveda-pitr-drill-v1","source_project":"%s","backup_id":"%s","target_project":"%s","recovery_target":"%s","started_at":"%s","finished_at":"%s","duration_seconds":%s,"result":"%s","exit_status":%s,"cleanup_status":%s,"generation_created_at":"%s","generation_age_seconds":%s}\n' \
    "$source_project" "$backup_id" "$target_project" "$recovery_target" "$started_at" \
    "$finished_at" "$duration" "$result" "$status" "$cleanup_status" "$created_at" \
    "${generation_age:-null}" > "$result_file"
metrics=$result_root/pitr-drill.prom
success_time=$(sed -n 's/^synveda_pitr_drill_last_success_timestamp_seconds //p' "$metrics" 2>/dev/null || true)
case "$success_time" in *[!0-9]*) success_time= ;; esac
[ "$result" = failed ] || success_time=$(date -u '+%s')
metrics_temp=$(mktemp "$result_root/.pitr-drill.prom.XXXXXX")
{
    echo "# HELP synveda_pitr_drill_success Whether the last isolated PITR restore drill passed."
    echo "# TYPE synveda_pitr_drill_success gauge"
    echo "synveda_pitr_drill_success $([ "$result" = passed ] && echo 1 || echo 0)"
    echo "# HELP synveda_pitr_drill_duration_seconds Wall time of the last drill, first command to browser login."
    echo "# TYPE synveda_pitr_drill_duration_seconds gauge"
    echo "synveda_pitr_drill_duration_seconds $duration"
    echo "# HELP synveda_pitr_drill_generation_age_seconds Age of the custody generation the last drill restored."
    echo "# TYPE synveda_pitr_drill_generation_age_seconds gauge"
    echo "synveda_pitr_drill_generation_age_seconds ${generation_age:-0}"
    if [ -n "$success_time" ]; then
        echo "# HELP synveda_pitr_drill_last_success_timestamp_seconds When a drill last passed."
        echo "# TYPE synveda_pitr_drill_last_success_timestamp_seconds gauge"
        echo "synveda_pitr_drill_last_success_timestamp_seconds $success_time"
    fi
} > "$metrics_temp"
mv -f "$metrics_temp" "$metrics"
echo "pitr-drill: $result in ${duration}s for $source_project/$backup_id into $target_project; record $result_file"
[ "$status" -eq 0 ] || exit "$status"
[ "$cleanup_status" -eq 0 ] || refuse "drill passed but its disposable target was not removed; inspect $target_project" 70
