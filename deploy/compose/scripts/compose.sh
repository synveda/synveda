#!/bin/sh
# Canonical CPR-45 Compose selector and single-host lifecycle.
set -eu

# Shell ranges follow the process locale. Pin validation to bytewise ASCII so
# non-ASCII characters cannot collate into the closed DNS/image/name grammars.
LC_ALL=C
export LC_ALL

usage() {
    echo "usage: deploy/compose/scripts/compose.sh {config [--output PATH]|hosts-plan|hosts-status|hosts-install|hosts-remove|resolver-check|up [--initial-assets absent]|smoke|restart-gateway|down|reset}" >&2
    exit 64
}

action=${1:-}
case "$action" in
    config|hosts-plan|hosts-status|hosts-install|hosts-remove|resolver-check|up|smoke|restart-gateway|down|reset) ;;
    *) usage ;;
esac
shift
output=
initial_asset_state=existing
if [ "$action" = config ]; then
    case "${1:-}" in
        "") ;;
        --output)
            [ "$#" -eq 2 ] && [ -n "${2:-}" ] || usage
            output=$2
            shift 2
            ;;
        *) usage ;;
    esac
fi
if [ "$action" = up ]; then
    case "${1:-}" in
        "") ;;
        --initial-assets)
            [ "$#" -eq 2 ] && [ "${2:-}" = absent ] || usage
            initial_asset_state=absent
            shift 2
            ;;
        *) usage ;;
    esac
fi
[ "$#" -eq 0 ] || usage

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
compose_dir=$(dirname "$script_dir")
repo_root=$(CDPATH= cd "$compose_dir/../.." && pwd -P)
docker_bin=${SYNVEDA_DOCKER_BIN:-docker}
node_runner=$script_dir/run-node-closed
hosts_manager=$script_dir/manage-hosts-file.mjs

runtime=${SYNVEDA_COMPOSE_RUNTIME:-development}
postgres_mode=${SYNVEDA_POSTGRES_MODE:-bundled}
oidc_mode=${SYNVEDA_OIDC_MODE:-bundled}
profiles=${SYNVEDA_COMPOSE_PROFILES:-}
demo_profile=false
browser_acceptance_profile=false
lifecycle_timeout=${SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS:-900}
# The gateway declares a 30-second stop grace in compose.yaml. Leave bounded
# client and postflight margins around that daemon-side contract rather than
# handing the daemon the unrelated whole-lifecycle budget.
gateway_restart_stop_seconds=30
gateway_restart_runner_seconds=45
gateway_restart_health_seconds=120
gateway_restart_health_runner_seconds=125
gateway_restart_postflight_reserve_seconds=40
gateway_restart_orchestration_margin_seconds=5
gateway_restart_required_seconds=$((
    gateway_restart_runner_seconds +
    gateway_restart_health_runner_seconds +
    gateway_restart_postflight_reserve_seconds +
    gateway_restart_orchestration_margin_seconds
))

case "$lifecycle_timeout" in
    ''|0|0*|*[!0-9]*)
        echo "compose: SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS must be 240 through 3600" >&2
        exit 64
        ;;
esac
[ "$lifecycle_timeout" -ge 240 ] && [ "$lifecycle_timeout" -le 3600 ] || {
    echo "compose: SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS must be 240 through 3600" >&2
    exit 64
}

# These inputs are consumed by Node or OpenSSL before JavaScript can inspect
# them. Reference evidence refuses their ambient presence before the first
# helper process; recovery and explicit-HTTP development scrub them instead.
ambient_node_trust=false
if [ "${NODE_OPTIONS+x}" = x ] || [ "${NODE_EXTRA_CA_CERTS+x}" = x ] || \
    [ "${NODE_TLS_REJECT_UNAUTHORIZED+x}" = x ] || \
    [ "${NODE_USE_SYSTEM_CA+x}" = x ] || \
    [ "${NODE_USE_ENV_PROXY+x}" = x ] || [ "${SSL_CERT_FILE+x}" = x ] || \
    [ "${SSL_CERT_DIR+x}" = x ] || [ "${OPENSSL_CONF+x}" = x ] || \
    [ "${OPENSSL_CONF_INCLUDE+x}" = x ] || [ "${OPENSSL_MODULES+x}" = x ] || \
    [ "${OPENSSL_ENGINES+x}" = x ]; then
    ambient_node_trust=true
fi
case "$runtime:$action:$ambient_node_trust" in
    reference:config:true|reference:up:true|reference:smoke:true|\
    reference:restart-gateway:true)
        echo "compose: ambient host trust configuration is not accepted for reference evidence" >&2
        exit 78
        ;;
esac
unset NODE_OPTIONS NODE_EXTRA_CA_CERTS NODE_TLS_REJECT_UNAUTHORIZED \
    NODE_USE_SYSTEM_CA NODE_USE_ENV_PROXY SSL_CERT_FILE SSL_CERT_DIR \
    OPENSSL_CONF OPENSSL_CONF_INCLUDE OPENSSL_MODULES OPENSSL_ENGINES

# A development build is allowed to disclose the repository context only to
# the Docker daemon whose local Unix socket is proved below. Refuse every
# ambient BuildKit/Buildx/Bake selector before the first helper process. Other
# actions do not build and scrub the same controls so recovery never depends on
# a caller's build environment. DOCKER_CONFIG is deliberately retained for
# registry authentication; its builder state is isolated later with a private
# BUILDX_CONFIG directory.
ambient_build_control=false
if [ "${COMPOSE_BAKE+x}" = x ] || \
    [ "${COMPOSE_DOCKER_CLI_BUILD+x}" = x ] || \
    [ "${DOCKER_CLI_HINTS+x}" = x ] || [ "${DOCKER_CLI_HOOKS+x}" = x ] || \
    [ "${DOCKER_BUILDKIT+x}" = x ] || \
    [ "${DOCKER_DEFAULT_PLATFORM+x}" = x ] || \
    [ "${SOURCE_DATE_EPOCH+x}" = x ] || \
    [ "${BUILDKIT_COLORS+x}" = x ] || [ "${BUILDKIT_HOST+x}" = x ] || \
    [ "${BUILDKIT_PROGRESS+x}" = x ] || \
    [ "${BUILDKIT_NO_CLIENT_TOKEN+x}" = x ] || \
    [ "${BUILDKIT_TTY_LOG_LINES+x}" = x ] || \
    [ "${EXPERIMENTAL_BUILDKIT_SOURCE_POLICY+x}" = x ] || \
    [ "${BUILDX_BAKE_FILE+x}" = x ] || \
    [ "${BUILDX_BAKE_FILE_SEPARATOR+x}" = x ] || \
    [ "${BUILDX_BAKE_PATH_SEPARATOR+x}" = x ] || \
    [ "${BUILDX_BAKE_FILE_RELATIVE_PATHS+x}" = x ] || \
    [ "${BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP+x}" = x ] || \
    [ "${BUILDX_BAKE_GIT_AUTH_HEADER+x}" = x ] || \
    [ "${BUILDX_BAKE_GIT_AUTH_TOKEN+x}" = x ] || \
    [ "${BUILDX_BAKE_GIT_SSH+x}" = x ] || \
    [ "${BUILDX_BAKE_ENTITLEMENTS_FS+x}" = x ] || \
    [ "${BAKE_ALLOW_REMOTE_FS_ACCESS+x}" = x ] || \
    [ "${BAKE_CMD_CONTEXT+x}" = x ] || \
    [ "${BAKE_LOCAL_PLATFORM+x}" = x ] || \
    [ "${BUILDX_BUILDER+x}" = x ] || [ "${BUILDX_CONFIG+x}" = x ] || \
    [ "${BUILDX_CPU_PROFILE+x}" = x ] || \
    [ "${BUILDX_EXPERIMENTAL+x}" = x ] || \
    [ "${BUILDX_GIT_CHECK_DIRTY+x}" = x ] || \
    [ "${BUILDX_GIT_INFO+x}" = x ] || [ "${BUILDX_GIT_LABELS+x}" = x ] || \
    [ "${BUILDX_MEM_PROFILE+x}" = x ] || \
    [ "${BUILDX_METADATA_PROVENANCE+x}" = x ] || \
    [ "${BUILDX_METADATA_WARNINGS+x}" = x ] || \
    [ "${BUILDX_NO_DEFAULT_ATTESTATIONS+x}" = x ] || \
    [ "${BUILDX_NO_DEFAULT_OCI_ARTIFACT+x}" = x ] || \
    [ "${BUILDX_NO_DEFAULT_LOAD+x}" = x ] || \
    [ "${BUILDX_DEFAULT_POLICY+x}" = x ]; then
    ambient_build_control=true
fi
if [ "$runtime" = development ] && [ "$action" = up ] && \
    [ "$ambient_build_control" = true ]; then
    echo "compose: ambient host build configuration is not accepted for development builds" >&2
    exit 78
fi
unset COMPOSE_BAKE COMPOSE_DOCKER_CLI_BUILD DOCKER_CLI_HINTS DOCKER_CLI_HOOKS \
    DOCKER_BUILDKIT \
    DOCKER_DEFAULT_PLATFORM SOURCE_DATE_EPOCH BUILDKIT_COLORS BUILDKIT_HOST \
    BUILDKIT_PROGRESS BUILDKIT_NO_CLIENT_TOKEN BUILDKIT_TTY_LOG_LINES \
    EXPERIMENTAL_BUILDKIT_SOURCE_POLICY BUILDX_BAKE_FILE \
    BUILDX_BAKE_FILE_SEPARATOR BUILDX_BAKE_PATH_SEPARATOR \
    BUILDX_BAKE_FILE_RELATIVE_PATHS BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP \
    BUILDX_BAKE_GIT_AUTH_HEADER BUILDX_BAKE_GIT_AUTH_TOKEN \
    BUILDX_BAKE_GIT_SSH BUILDX_BAKE_ENTITLEMENTS_FS \
    BAKE_ALLOW_REMOTE_FS_ACCESS BAKE_CMD_CONTEXT BAKE_LOCAL_PLATFORM \
    BUILDX_BUILDER BUILDX_CONFIG BUILDX_CPU_PROFILE \
    BUILDX_EXPERIMENTAL BUILDX_GIT_CHECK_DIRTY BUILDX_GIT_INFO \
    BUILDX_GIT_LABELS BUILDX_MEM_PROFILE BUILDX_METADATA_PROVENANCE \
    BUILDX_METADATA_WARNINGS BUILDX_NO_DEFAULT_ATTESTATIONS \
    BUILDX_NO_DEFAULT_OCI_ARTIFACT BUILDX_NO_DEFAULT_LOAD \
    BUILDX_DEFAULT_POLICY
COMPOSE_BAKE=false
DOCKER_CLI_HOOKS=false
DOCKER_BUILDKIT=1
BUILDX_BUILDER=default
BUILDKIT_PROGRESS=plain
BUILDKIT_NO_CLIENT_TOKEN=false
BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP=1
BUILDX_GIT_CHECK_DIRTY=false
BUILDX_GIT_INFO=false
BUILDX_GIT_LABELS=false
BUILDX_NO_DEFAULT_ATTESTATIONS=true
export COMPOSE_BAKE DOCKER_CLI_HOOKS DOCKER_BUILDKIT BUILDX_BUILDER \
    BUILDKIT_PROGRESS BUILDKIT_NO_CLIENT_TOKEN BUILDX_GIT_CHECK_DIRTY \
    BUILDX_BAKE_DISABLE_VARS_ENV_LOOKUP BUILDX_GIT_INFO BUILDX_GIT_LABELS \
    BUILDX_NO_DEFAULT_ATTESTATIONS

# Every lifecycle temporary is rooted in one physical directory. A development
# build refuses a root inside the repository before any helper or lock because
# asset captures and Buildx state would otherwise enter the source context.
lifecycle_temp_root=${TMPDIR:-/tmp}
lifecycle_temp_root=$(CDPATH= cd "$lifecycle_temp_root" 2>/dev/null && pwd -P) || {
    echo "compose: lifecycle temporary root was unavailable" >&2
    exit 70
}
if [ "$runtime" = development ] && [ "$action" = up ]; then
    case "$lifecycle_temp_root" in
        "$repo_root"|"$repo_root"/*)
            echo "compose: lifecycle temporary root is not accepted inside the build context" >&2
            exit 78
            ;;
    esac

    # Preserve registry authentication byte-for-byte, but validate only the
    # effective config directory's location. Its contents are never opened or
    # copied. An explicit path must already be an accessible directory.
    build_docker_config_root=
    if [ "${DOCKER_CONFIG+x}" = x ]; then
        [ -n "$DOCKER_CONFIG" ] && [ -d "$DOCKER_CONFIG" ] || {
            echo "compose: Docker registry configuration path was refused" >&2
            exit 78
        }
        build_docker_config_root=$(CDPATH= cd "$DOCKER_CONFIG" 2>/dev/null && pwd -P) || {
            echo "compose: Docker registry configuration path was refused" >&2
            exit 78
        }
    else
        [ -n "${HOME:-}" ] && [ -d "$HOME" ] || {
            echo "compose: Docker registry configuration path was refused" >&2
            exit 78
        }
        build_docker_home=$(CDPATH= cd "$HOME" 2>/dev/null && pwd -P) || {
            echo "compose: Docker registry configuration path was refused" >&2
            exit 78
        }
        case "$build_docker_home" in
            "$repo_root"|"$repo_root"/*)
                echo "compose: Docker registry configuration is not accepted inside the build context" >&2
                exit 78
                ;;
        esac
        build_docker_config_path=$build_docker_home/.docker
        if [ -e "$build_docker_config_path" ] || [ -L "$build_docker_config_path" ]; then
            [ -d "$build_docker_config_path" ] || {
                echo "compose: Docker registry configuration path was refused" >&2
                exit 78
            }
            build_docker_config_root=$(CDPATH= cd "$build_docker_config_path" 2>/dev/null && pwd -P) || {
                echo "compose: Docker registry configuration path was refused" >&2
                exit 78
            }
        else
            build_docker_config_root=$build_docker_config_path
        fi
    fi
    case "$build_docker_config_root" in
        "$repo_root"|"$repo_root"/*)
            echo "compose: Docker registry configuration is not accepted inside the build context" >&2
            exit 78
            ;;
    esac
    build_docker_config_file=$build_docker_config_root/config.json
    if [ -L "$build_docker_config_file" ] || {
        [ -e "$build_docker_config_file" ] && [ ! -f "$build_docker_config_file" ]
    }; then
        echo "compose: Docker registry configuration file was refused" >&2
        exit 78
    fi
fi

lifecycle_started_at=$("$node_runner" "$script_dir/monotonic-seconds.mjs") || {
    echo "compose: lifecycle clock was unavailable" >&2
    exit 69
}
case "$lifecycle_started_at" in
    ''|*[!0-9]*)
        echo "compose: lifecycle clock was invalid" >&2
        exit 69
        ;;
esac
lifecycle_deadline=$((lifecycle_started_at + lifecycle_timeout))
lifecycle_last_remaining=$lifecycle_timeout
lifecycle_child_uncertain=false
bounded_runner_pending=false
bounded_runner_waiting=false
set_remaining_lifecycle_seconds() {
    lifecycle_now=$("$node_runner" "$script_dir/monotonic-seconds.mjs") || {
        echo "compose: lifecycle clock was unavailable" >&2
        return 69
    }
    case "$lifecycle_now" in
        ''|*[!0-9]*)
            echo "compose: lifecycle clock was invalid" >&2
            return 69
            ;;
    esac
    lifecycle_remaining=$((lifecycle_deadline - lifecycle_now))
    # CLOCK_MONOTONIC must not step backwards, but a non-increasing clamp also
    # prevents any platform/runtime anomaly from replenishing the budget.
    if [ "$lifecycle_remaining" -gt "$lifecycle_last_remaining" ]; then
        lifecycle_remaining=$lifecycle_last_remaining
    fi
    if [ "$lifecycle_remaining" -le 0 ]; then
        echo "compose: whole-operation lifecycle deadline expired" >&2
        return 124
    fi
    lifecycle_last_remaining=$lifecycle_remaining
}
run_bounded() {
    requested_seconds=$1
    shift
    set_remaining_lifecycle_seconds || return $?
    bounded_seconds=$lifecycle_remaining
    if [ "$requested_seconds" -lt "$bounded_seconds" ]; then
        bounded_seconds=$requested_seconds
    fi
    bounded_status_file=$(mktemp "$lifecycle_temp_root/synveda-compose-runner.XXXXXX") || {
        echo "compose: bounded runner status staging failed" >&2
        return 70
    }
    if ! chmod 600 "$bounded_status_file"; then
        rm -f -- "$bounded_status_file" 2>/dev/null || true
        bounded_status_file=
        return 70
    fi
    bounded_runner_pending=true
    "$node_runner" "$script_dir/run-with-deadline.mjs" --seconds "$bounded_seconds" \
        --status-file "$bounded_status_file" -- "$@" &
    bounded_runner_pid=$!
    bounded_status=0
    bounded_runner_waiting=true
    bounded_runner_pending=false
    wait "$bounded_runner_pid" || bounded_status=$?
    bounded_settlement_status=0
    settle_bounded_runner "$bounded_status" || bounded_settlement_status=$?
    if [ "$bounded_status" -ge 128 ]; then
        # An uncatchable signal delivered directly to the command can bypass
        # its authority-state cleanup even when the runner proves the process
        # group is gone. Parent-forwarded signals settle in compose_signal.
        lifecycle_child_uncertain=true
    fi
    if [ "$bounded_status" -eq 0 ] && [ "$bounded_settlement_status" -ne 0 ]; then
        bounded_status=$bounded_settlement_status
    fi
    bounded_runner_waiting=false
    bounded_runner_pid=
    return "$bounded_status"
}
bounded_runner_pid=
bounded_status_file=
bounded_capture_file=
bounded_output=
settle_bounded_runner() {
    settled_status=$1
    bounded_group_clean=false
    if [ -n "$bounded_status_file" ] && [ ! -L "$bounded_status_file" ] && \
        [ -f "$bounded_status_file" ]; then
        recorded_bounded_status=
        IFS= read -r recorded_bounded_status < "$bounded_status_file" || \
            recorded_bounded_status=
        if [ "$recorded_bounded_status" = "clean:$settled_status" ]; then
            bounded_group_clean=true
        fi
    fi
    if [ "$settled_status" -eq 125 ] || [ "$bounded_group_clean" != true ]; then
        lifecycle_child_uncertain=true
    fi
    if [ -n "$bounded_status_file" ]; then
        rm -f -- "$bounded_status_file" || return 70
        bounded_status_file=
    fi
}
capture_bounded_output() {
    capture_seconds=$1
    shift
    bounded_capture_file=$(mktemp "$lifecycle_temp_root/synveda-compose-output.XXXXXX") || {
        echo "compose: bounded output staging failed" >&2
        return 70
    }
    chmod 600 "$bounded_capture_file" || return 70
    capture_status=0
    run_bounded "$capture_seconds" "$@" > "$bounded_capture_file" || capture_status=$?
    if [ "$capture_status" -ne 0 ]; then
        rm -f -- "$bounded_capture_file" 2>/dev/null || true
        bounded_capture_file=
        return "$capture_status"
    fi
    bounded_output=$(cat -- "$bounded_capture_file") || {
        rm -f -- "$bounded_capture_file" 2>/dev/null || true
        bounded_capture_file=
        return 70
    }
    rm -f -- "$bounded_capture_file" || return 70
    bounded_capture_file=
}
propagate_bounded_failure() {
    case "$1" in
        124|125) exit "$1" ;;
    esac
}

# The wrapper owns provider/profile/file selection. Prevent Docker-native
# selector variables and an ambient .env from adding an unvalidated fragment.
unset COMPOSE_FILE COMPOSE_PROJECT_NAME COMPOSE_PROFILES COMPOSE_ENV_FILES
unset SYNVEDA_RENDER_PUBLIC_EDGE_SUBNET SYNVEDA_RENDER_PUBLIC_EDGE_GATEWAY \
    SYNVEDA_RENDER_APP_BACKEND_SUBNET SYNVEDA_RENDER_APP_BACKEND_GATEWAY \
    SYNVEDA_RENDER_DATA_SUBNET SYNVEDA_RENDER_DATA_GATEWAY \
    SYNVEDA_RENDER_KEYCLOAK_DATA_SUBNET SYNVEDA_RENDER_KEYCLOAK_DATA_GATEWAY \
    SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_SUBNET SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_SUBNET SYNVEDA_RENDER_IDENTITY_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_DYNAMIC_RANGE SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS \
    SYNVEDA_RENDER_TELEMETRY_SUBNET SYNVEDA_RENDER_TELEMETRY_GATEWAY \
    SYNVEDA_RENDER_APPLICATION_EGRESS_SUBNET SYNVEDA_RENDER_APPLICATION_EGRESS_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_EGRESS_SUBNET SYNVEDA_RENDER_IDENTITY_EGRESS_GATEWAY \
    SYNVEDA_RENDER_TELEMETRY_EGRESS_SUBNET SYNVEDA_RENDER_TELEMETRY_EGRESS_GATEWAY
export COMPOSE_DISABLE_ENV_FILE=1

case "$runtime" in
    development|reference) ;;
    *) echo "compose: SYNVEDA_COMPOSE_RUNTIME must be development|reference" >&2; exit 64 ;;
esac
case "$postgres_mode" in
    bundled|external) ;;
    *) echo "compose: SYNVEDA_POSTGRES_MODE must be bundled|external" >&2; exit 64 ;;
esac
case "$oidc_mode" in
    bundled|external) ;;
    *) echo "compose: SYNVEDA_OIDC_MODE must be bundled|external" >&2; exit 64 ;;
esac

old_ifs=$IFS
IFS=,
profile_count=0
profile_seen=,
case "$profiles" in
    ,*|*,|*,,*)
        echo "compose: empty profile selector was refused" >&2
        exit 64
        ;;
esac
for profile in $profiles; do
    case "$profile_seen" in
        *,"$profile",*)
            echo "compose: duplicate profile was refused" >&2
            exit 64
            ;;
    esac
    profile_seen=$profile_seen$profile,
    profile_count=$((profile_count + 1))
    case "$profile" in
        "") ;;
        semantic|observability|apalis-board|backup-test) ;;
        demo) demo_profile=true ;;
        browser-acceptance) browser_acceptance_profile=true ;;
        *)
            echo "compose: unsupported profile; allowed: semantic,observability,apalis-board,demo,backup-test,browser-acceptance" >&2
            exit 64
            ;;
    esac
done
IFS=$old_ifs

if [ "$demo_profile" = true ] && \
    { [ "$postgres_mode" != bundled ] || [ "$oidc_mode" != bundled ]; }; then
    echo "compose: demo profile requires bundled PostgreSQL and bundled OIDC" >&2
    exit 64
fi
if [ "$browser_acceptance_profile" = true ]; then
    [ "$profile_count" -eq 2 ] && [ "$demo_profile" = true ] || {
        echo "compose: browser acceptance requires exactly the demo,browser-acceptance profiles" >&2
        exit 64
    }
    [ "$runtime" = development ] && [ "$postgres_mode" = bundled ] && \
        [ "$oidc_mode" = bundled ] || {
        echo "compose: browser acceptance requires development with bundled PostgreSQL and bundled OIDC" >&2
        exit 64
    }
    case "$action" in
        config|up|smoke|down|reset) ;;
        *)
            echo "compose: browser acceptance is unavailable for this lifecycle action" >&2
            exit 64
            ;;
    esac
fi
if { [ "$action" = up ] || [ "$action" = reset ]; } && \
    [ "$postgres_mode" = external ]; then
    echo "compose: canonical start/reset is unavailable for external PostgreSQL in this checkpoint" >&2
    exit 69
fi

runtime_uid=${SYNVEDA_RUNTIME_UID:-$(id -u)}
runtime_gid=${SYNVEDA_RUNTIME_GID:-$(id -g)}
valid_runtime_id() {
    candidate=$1
    case "$candidate" in
        ''|0|0*|*[!0-9]*) return 1 ;;
    esac
    [ "${#candidate}" -le 10 ] && [ "$candidate" -le 2147483647 ]
}
if ! valid_runtime_id "$runtime_uid" || ! valid_runtime_id "$runtime_gid"; then
    echo "compose: SYNVEDA_RUNTIME_UID and SYNVEDA_RUNTIME_GID must be non-zero decimal integers" >&2
    exit 64
fi

suffix=${SYNVEDA_COMPOSE_PROJECT_SUFFIX:-}
project=synveda-$runtime
if [ -n "$suffix" ]; then
    suffix_value=${suffix#acceptance-}
    if [ "$suffix_value" = "$suffix" ] || [ -z "$suffix_value" ] || \
        [ "${#suffix_value}" -gt 24 ]; then
        echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
        exit 64
    fi
    case "$suffix_value" in
        *[!a-z0-9-]*|*-)
            echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    case "$suffix_value" in
        [a-z0-9]*) ;;
        *)
            echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    project=$project-$suffix
fi
if [ "$initial_asset_state" = absent ] && {
    [ "$runtime" != development ] || [ -z "$suffix" ];
}; then
    echo "compose: initial absence is restricted to suffixed development acceptance projects" >&2
    exit 64
fi
if [ "$browser_acceptance_profile" = true ]; then
    [ -n "$suffix" ] || {
        echo "compose: browser acceptance requires a suffixed development acceptance project" >&2
        exit 64
    }
    if [ "$action" = up ] && [ "$initial_asset_state" != absent ]; then
        echo "compose: browser acceptance up requires --initial-assets absent" >&2
        exit 64
    fi
fi

case "$action" in
    up|down|smoke|restart-gateway|reset)
        # Hold one exact-project exclusion across authority-file generation and
        # every Docker mutation. Child generators verify and borrow this lock.
        # shellcheck source=deploy/compose/scripts/project-lock.sh
        unset SYNVEDA_INTERNAL_PROJECT_LOCK_FILE SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER
        . "$script_dir/project-lock.sh"
        asset_config_file=
        status_file=
        buildx_config_dir=
        docker_mutation_uncertain=false
        docker_mutation_phase=
        compose_signal() {
            signal_name=$1
            signal_status=$2
            # A POSIX shell may defer a trap while waiting for a foreground
            # process. The deadline runner is deliberately a background child
            # so this handler can forward a parent-only signal immediately.
            trap '' HUP INT TERM
            if [ -n "$bounded_runner_pid" ]; then
                kill -"$signal_name" "$bounded_runner_pid" 2>/dev/null || true
                signal_wait_status=0
                wait "$bounded_runner_pid" 2>/dev/null || signal_wait_status=$?
                settle_bounded_runner "$signal_wait_status" 2>/dev/null || true
                bounded_runner_waiting=false
                bounded_runner_pid=
            elif [ "$bounded_runner_pending" = true ]; then
                # A signal between fork and $! publication cannot identify the
                # new process group safely. Retain the project lock so the
                # possibly-live child cannot overlap another lifecycle.
                lifecycle_child_uncertain=true
            fi
            exit "$signal_status"
        }
        compose_cleanup() {
            cleanup_status=$?
            # Ignore re-entrant signals until temporary state and the global
            # exact-project lock are released.
            trap '' HUP INT TERM
            trap - EXIT
            if [ -n "$asset_config_file" ] && \
                ! rm -f -- "$asset_config_file" 2>/dev/null; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=70
            fi
            if [ -n "$status_file" ] && ! rm -f -- "$status_file" 2>/dev/null; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=70
            fi
            if [ -n "$buildx_config_dir" ] && \
                ! rm -rf -- "$buildx_config_dir" 2>/dev/null; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=70
            fi
            if [ -n "$bounded_capture_file" ] && \
                ! rm -f -- "$bounded_capture_file" 2>/dev/null; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=70
            fi
            if [ -n "$bounded_status_file" ] && \
                ! rm -f -- "$bounded_status_file" 2>/dev/null; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=70
            fi
            if [ "$docker_mutation_uncertain" = true ]; then
                echo "compose: retained exact-project lock because Docker mutation state is uncertain ($docker_mutation_phase)" >&2
            elif [ "$lifecycle_child_uncertain" = true ]; then
                echo "compose: retained exact-project lock because a bounded child process group was not cleanly reaped" >&2
            elif ! release_project_lock; then
                [ "$cleanup_status" -ne 0 ] || cleanup_status=73
            fi
            exit "$cleanup_status"
        }
        trap compose_cleanup EXIT
        trap 'compose_signal HUP 129' HUP
        trap 'compose_signal INT 130' INT
        trap 'compose_signal TERM 143' TERM
        acquire_project_lock
        ;;
esac

app_host=${SYNVEDA_APP_HOST:-app.synveda.test}
auth_host=
if [ "$oidc_mode" = bundled ]; then
    auth_host=${SYNVEDA_AUTH_HOST:-auth.synveda.test}
fi
public_scheme=${SYNVEDA_PUBLIC_SCHEME:-http}
valid_host() {
    candidate=$1
    [ "${#candidate}" -le 253 ] || return 1
    case "$candidate" in
        ''|*[!a-z0-9.-]*|localhost|*.localhost|.*|*.|*..*) return 1 ;;
    esac
    case "$candidate" in
        *[a-z]*) ;;
        *) return 1 ;;
    esac
    case "$candidate" in
        *.*) ;;
        *) return 1 ;;
    esac
    previous_ifs=$IFS
    IFS=.
    set -- $candidate
    IFS=$previous_ifs
    for label in "$@"; do
        [ -n "$label" ] && [ "${#label}" -le 63 ] || return 1
        case "$label" in
            -*|*-) return 1 ;;
        esac
    done
}
valid_host "$app_host" || {
    echo "compose: application and identity hostnames must be lower-case DNS names" >&2
    exit 64
}
if [ "$oidc_mode" = bundled ]; then
    valid_host "$auth_host" || {
        echo "compose: application and identity hostnames must be lower-case DNS names" >&2
        exit 64
    }
    [ "$app_host" != "$auth_host" ] || {
        echo "compose: application and identity hostnames must differ" >&2
        exit 64
    }
fi

case "$runtime" in
    development)
        [ "$public_scheme" = http ] || {
            echo "compose: development uses explicit HTTP" >&2
            exit 64
        }
        case "$app_host" in
            *.test) ;;
            *) echo "compose: development hostnames must end in .test" >&2; exit 64 ;;
        esac
        if [ "$oidc_mode" = bundled ]; then
            case "$auth_host" in
                *.test) ;;
                *) echo "compose: development hostnames must end in .test" >&2; exit 64 ;;
            esac
        fi
        restart_policy=no
        public_port=${SYNVEDA_DEV_HTTP_PORT:-8080}
        valid_runtime_id "$public_port" && [ "$public_port" -ge 1024 ] && \
            [ "$public_port" -le 65535 ] && [ "$public_port" -ne 8443 ] || {
            echo "compose: SYNVEDA_DEV_HTTP_PORT must be a canonical integer from 1024 through 65535 except reserved port 8443" >&2
            exit 64
        }
        public_app_url=http://$app_host:$public_port
        # Browser and containers resolve the same issuer authority. Caddy must
        # therefore listen on the selected public port inside the network too;
        # host-only port translation would make the issuer unreachable from
        # the diagnostic, gateway and CLI containers.
        proxy_http_port=$public_port
        proxy_https_port=8443
        keycloak_ssl_required=NONE
        insecure_development_http=true
        runtime_overlay=dev
        caddy_app_config=$compose_dir/configs/caddy/app.dev.caddy
        caddy_identity_config=$compose_dir/configs/caddy/identity.dev.caddy
        ;;
    reference)
        [ "$public_scheme" = https ] || {
            echo "compose: reference mode requires HTTPS" >&2
            exit 64
        }
        case "$app_host" in
            *.test|*.localhost)
                echo "compose: reference hostnames must be operator DNS names" >&2
                exit 64
                ;;
        esac
        if [ "$oidc_mode" = bundled ]; then
            case "$auth_host" in
                *.test|*.localhost)
                    echo "compose: reference hostnames must be operator DNS names" >&2
                    exit 64
                    ;;
            esac
        fi
        [ "${SYNVEDA_TLS_MODE:-files}" = files ] || {
            echo "compose: this checkpoint accepts reference certificate-file mode only" >&2
            exit 64
        }
        restart_policy=unless-stopped
        public_port=443
        public_app_url=https://$app_host
        proxy_http_port=80
        proxy_https_port=443
        keycloak_ssl_required=EXTERNAL
        insecure_development_http=false
        runtime_overlay=reference
        caddy_app_config=$compose_dir/configs/caddy/app.reference.caddy
        caddy_identity_config=$compose_dir/configs/caddy/identity.reference.caddy
        ;;
esac

if [ "$oidc_mode" = bundled ]; then
    case "$runtime" in
        development) public_auth_url=http://$auth_host:$public_port ;;
        reference) public_auth_url=https://$auth_host ;;
    esac
fi

if [ "$oidc_mode" = external ]; then
    caddy_identity_config=$compose_dir/configs/caddy/identity.external.caddy
fi

run_hosts_manager() {
    hosts_manager_action=$1
    shift
    set -- "$hosts_manager" "$hosts_manager_action" \
        --runtime development --project "$project" --oidc "$oidc_mode" \
        --app-host "$app_host" "$@"
    if [ "$oidc_mode" = bundled ]; then
        set -- "$@" --auth-host "$auth_host"
    fi
    run_bounded "$lifecycle_timeout" "$node_runner" "$@"
}
hosts_confirmation() {
    hosts_confirmation_action=$1
    if [ "$oidc_mode" = bundled ]; then
        hosts_confirmation_identity=$auth_host
    else
        hosts_confirmation_identity=-
    fi
    printf '%s\n' "$hosts_confirmation_action:127.0.0.1:$project:$app_host:$hosts_confirmation_identity"
}

if [ "$action" = hosts-plan ]; then
    if [ "$runtime" = reference ]; then
        echo "reference mode uses operator DNS and has no managed hosts-file block"
    else
        run_hosts_manager plan
    fi
    exit 0
fi

if [ "$action" = hosts-status ]; then
    if [ "$runtime" = reference ]; then
        echo "reference mode uses operator DNS and has no managed hosts-file state"
    else
        run_hosts_manager status
    fi
    exit 0
fi

if [ "$action" = hosts-install ] || [ "$action" = hosts-remove ]; then
    [ "$runtime" = development ] || {
        echo "compose: reference mode never manages /etc/hosts" >&2
        exit 64
    }
    case "$action" in
        hosts-install)
            hosts_mutation=install
            hosts_confirmation_name=SYNVEDA_CONFIRM_HOSTS_INSTALL
            hosts_confirmation_value=${SYNVEDA_CONFIRM_HOSTS_INSTALL:-}
            ;;
        hosts-remove)
            hosts_mutation=remove
            hosts_confirmation_name=SYNVEDA_CONFIRM_HOSTS_REMOVE
            hosts_confirmation_value=${SYNVEDA_CONFIRM_HOSTS_REMOVE:-}
            ;;
    esac
    expected_hosts_confirmation=$(hosts_confirmation "$hosts_mutation")
    [ "$hosts_confirmation_value" = "$expected_hosts_confirmation" ] || {
        echo "compose: $hosts_mutation requires $hosts_confirmation_name=$expected_hosts_confirmation" >&2
        exit 64
    }
    [ -x /usr/bin/sudo ] && [ -x /usr/bin/env ] && [ -x /usr/bin/find ] && \
        [ -x /usr/bin/uname ] && [ -x /usr/bin/awk ] && [ -x /bin/ls ] || {
        echo "compose: the narrow hosts-file privilege runner is unavailable" >&2
        exit 69
    }
    node_platform=$(/usr/bin/uname -s 2>/dev/null) || node_platform=
    node_acl_tool=
    if [ "$node_platform" = Linux ]; then
        for acl_candidate in /usr/bin/getfacl /bin/getfacl; do
            [ -f "$acl_candidate" ] && [ -x "$acl_candidate" ] && [ ! -L "$acl_candidate" ] || continue
            trusted_acl_candidate=$(/usr/bin/find "$acl_candidate" -prune -type f -user root \
                ! -perm -0020 ! -perm -0002 -print 2>/dev/null) || continue
            [ "$trusted_acl_candidate" = "$acl_candidate" ] || continue
            node_acl_tool=$acl_candidate
            break
        done
    fi
    node_component_acl_free() {
        node_acl_component=$1
        case "$node_platform" in
            Darwin)
                node_acl_output=$(
                    /usr/bin/env -i LC_ALL=C PATH=/usr/bin:/bin \
                        /bin/ls -lde "$node_acl_component" 2>/dev/null
                ) || return 1
                case "$node_acl_output" in
                    *'
'*) return 1 ;;
                esac
                node_acl_mode=${node_acl_output%% *}
                case "$node_acl_mode" in *+) return 1 ;; esac
                ;;
            Linux)
                [ -n "$node_acl_tool" ] || return 1
                node_acl_output=$(
                    /usr/bin/env -i LC_ALL=C PATH=/usr/bin:/bin \
                        "$node_acl_tool" -c -p -n -- "$node_acl_component" 2>/dev/null
                ) || return 1
                printf '%s\n' "$node_acl_output" | \
                    /usr/bin/awk '
                        /^$/ { next }
                        /^#/ { bad = 1; next }
                        !/^(user|group|other)::[rwx-][rwx-][rwx-]$/ { bad = 1; next }
                        { split($0, part, ":"); seen[part[1]] += 1; count += 1 }
                        END {
                            if (bad || count != 3 || seen["user"] != 1 ||
                                seen["group"] != 1 || seen["other"] != 1) exit 1
                        }
                    ' || return 1
                ;;
            *) return 1 ;;
        esac
    }
    node_executable=
    for node_candidate in /usr/bin/node /usr/local/bin/node; do
        [ -f "$node_candidate" ] && [ -x "$node_candidate" ] && [ ! -L "$node_candidate" ] || continue
        case "$node_candidate" in
            /usr/bin/node) node_components='/ /usr /usr/bin /usr/bin/node' ;;
            /usr/local/bin/node) node_components='/ /usr /usr/local /usr/local/bin /usr/local/bin/node' ;;
        esac
        node_candidate_trusted=true
        for node_component in $node_components; do
            case "$node_component" in
                */node) node_component_type=f ;;
                *) node_component_type=d ;;
            esac
            trusted_component=$(/usr/bin/find "$node_component" -prune -type "$node_component_type" -user root \
                ! -perm -0020 ! -perm -0002 -print 2>/dev/null) || node_candidate_trusted=false
            [ "$trusted_component" = "$node_component" ] || node_candidate_trusted=false
            node_component_acl_free "$node_component" || node_candidate_trusted=false
        done
        [ "$node_candidate_trusted" = true ] || continue
        node_major=$(
            "$node_candidate" --use-bundled-ca -p 'process.versions.node.split(".")[0]' 2>/dev/null
        ) || continue
        case "$node_major" in ''|*[!0-9]*) continue ;; esac
        [ "$node_major" -ge 22 ] || continue
        node_executable=$node_candidate
        break
    done
    [ -n "$node_executable" ] || {
        echo "compose: Node executable identity was refused" >&2
        exit 69
    }
    set -- /usr/bin/sudo -- /usr/bin/env -i "$node_executable" \
        "$hosts_manager" "$hosts_mutation" \
        --runtime development --project "$project" --oidc "$oidc_mode" \
        --app-host "$app_host"
    if [ "$oidc_mode" = bundled ]; then
        set -- "$@" --auth-host "$auth_host"
    fi
    set -- "$@" --confirm "$expected_hosts_confirmation"
    exec "$@"
fi

run_hosts_ownership_preflight() {
    if [ "$runtime" = development ]; then
        run_hosts_manager status --expect installed
    fi
}

run_resolver_preflight() {
    run_hosts_ownership_preflight
    set -- "$script_dir/check-host-resolution.mjs" \
        --runtime "$runtime" --oidc "$oidc_mode" \
        --app-host "$app_host" --docker-bin "$docker_bin"
    if [ "$oidc_mode" = bundled ]; then
        set -- "$@" --auth-host "$auth_host"
    fi
    run_bounded "$lifecycle_timeout" "$node_runner" "$@"
}
run_docker_preflight() {
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-host-resolution.mjs" \
        --docker-only true --docker-bin "$docker_bin"
}
pin_local_docker_endpoint() {
    capture_bounded_output "$lifecycle_timeout" "$node_runner" \
        "$script_dir/check-host-resolution.mjs" \
        --docker-only true --print-docker-endpoint true --docker-bin "$docker_bin" || return $?
    pinned_docker_endpoint=$bounded_output
    case "$pinned_docker_endpoint" in
        unix:///*) ;;
        *) echo "compose: validated Docker endpoint was refused" >&2; return 69 ;;
    esac
    case "$pinned_docker_endpoint" in
        *[[:space:]]*)
            echo "compose: validated Docker endpoint was refused" >&2
            return 69
            ;;
    esac
    DOCKER_HOST=$pinned_docker_endpoint
    export DOCKER_HOST
    unset DOCKER_CONTEXT
}
if [ "$action" = resolver-check ]; then
    run_resolver_preflight
    exit 0
fi
case "$action" in
    up|smoke|restart-gateway) run_hosts_ownership_preflight ;;
esac
case "$action" in
    up|down|smoke|restart-gateway|reset) pin_local_docker_endpoint ;;
esac

compose_ipv4_pool_set=${SYNVEDA_COMPOSE_IPV4_POOL+x}
compose_ipv4_pool=${SYNVEDA_COMPOSE_IPV4_POOL:-172.30.240.0/24}
if [ "$runtime" = reference ] || [ -n "$suffix" ]; then
    [ -n "$compose_ipv4_pool_set" ] && [ -n "${SYNVEDA_COMPOSE_IPV4_POOL:-}" ] || {
        echo "compose: reference and acceptance projects require an explicit SYNVEDA_COMPOSE_IPV4_POOL" >&2
        exit 64
    }
fi

valid_ipv4_octet() {
    case "$1" in
        0|[1-9]|[1-9][0-9]|[1-9][0-9][0-9]) [ "$1" -le 255 ] ;;
        *) return 1 ;;
    esac
}
split_ipv4() {
    address=$1
    previous_ifs=$IFS
    IFS=.
    set -- $address
    IFS=$previous_ifs
    [ "$#" -eq 4 ] || return 1
    valid_ipv4_octet "$1" && valid_ipv4_octet "$2" && \
        valid_ipv4_octet "$3" && valid_ipv4_octet "$4" || return 1
    [ "$address" = "$1.$2.$3.$4" ] || return 1
    ipv4_a=$1
    ipv4_b=$2
    ipv4_c=$3
    ipv4_d=$4
}
validate_private_24() {
    pool_candidate=$1
    case "$pool_candidate" in
        */24) pool_network=${pool_candidate%/24} ;;
        *) echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be a private /24 CIDR" >&2; exit 64 ;;
    esac
    split_ipv4 "$pool_network" && [ "$ipv4_d" -eq 0 ] || {
        echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be a canonical private IPv4 /24 CIDR" >&2
        exit 64
    }
    case "$ipv4_a" in
        10) ;;
        172)
            [ "$ipv4_b" -ge 16 ] && [ "$ipv4_b" -le 31 ] || {
                echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2
                exit 64
            }
            ;;
        192)
            [ "$ipv4_b" -eq 168 ] || {
                echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2
                exit 64
            }
            ;;
        *) echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2; exit 64 ;;
    esac
}

validate_private_24 "$compose_ipv4_pool"
pool_prefix=$ipv4_a.$ipv4_b.$ipv4_c

# Ten current networks consume fixed /28 slots from one operator-selected /24.
# The proxy is fixed at identity slot +2 while dynamic identity endpoints are
# confined to the upper /29, so Docker cannot allocate the trusted address to
# Keycloak or a one-shot convergence container first.
identity_subnet=$pool_prefix.0/28
identity_gateway=$pool_prefix.1
proxy_identity_address=$pool_prefix.2
identity_dynamic_range=$pool_prefix.8/29
public_edge_subnet=$pool_prefix.16/28
public_edge_gateway=$pool_prefix.17
app_backend_subnet=$pool_prefix.32/28
app_backend_gateway=$pool_prefix.33
synveda_data_subnet=$pool_prefix.48/28
synveda_data_gateway=$pool_prefix.49
keycloak_data_subnet=$pool_prefix.64/28
keycloak_data_gateway=$pool_prefix.65
keycloak_management_subnet=$pool_prefix.80/28
keycloak_management_gateway=$pool_prefix.81
telemetry_subnet=$pool_prefix.96/28
telemetry_gateway=$pool_prefix.97
application_egress_subnet=$pool_prefix.112/28
application_egress_gateway=$pool_prefix.113
identity_egress_subnet=$pool_prefix.128/28
identity_egress_gateway=$pool_prefix.129
telemetry_egress_subnet=$pool_prefix.144/28
telemetry_egress_gateway=$pool_prefix.145

bootstrap_tenant_id=${SYNVEDA_BOOTSTRAP_TENANT_ID:-019b53c0-7c00-7000-8000-000000000045}
bootstrap_tenant_slug=${SYNVEDA_BOOTSTRAP_TENANT_SLUG:-reference}
bootstrap_tenant_name=${SYNVEDA_BOOTSTRAP_TENANT_NAME:-Synveda Reference}
case "$bootstrap_tenant_id" in
    ????????-????-7???-[89ab]???-????????????) ;;
    *) echo "compose: bootstrap tenant UUIDv7 was refused" >&2; exit 64 ;;
esac
case "$bootstrap_tenant_id" in
    *[!0-9a-f-]*) echo "compose: bootstrap tenant UUIDv7 was refused" >&2; exit 64 ;;
esac
case "$bootstrap_tenant_slug" in
    [a-z0-9]*) ;;
    *) echo "compose: bootstrap tenant slug was refused" >&2; exit 64 ;;
esac
case "$bootstrap_tenant_slug" in
    *[!a-z0-9-]*|*-|*--*) echo "compose: bootstrap tenant slug was refused" >&2; exit 64 ;;
esac
[ "${#bootstrap_tenant_slug}" -le 63 ] || {
    echo "compose: bootstrap tenant slug was refused" >&2
    exit 64
}
case "$bootstrap_tenant_name" in
    ''|-*|*[!A-Za-z0-9._' '-]*)
        echo "compose: bootstrap tenant name was refused" >&2
        exit 64
        ;;
esac
case "$bootstrap_tenant_name" in
    *[A-Za-z0-9]*) ;;
    *) echo "compose: bootstrap tenant name was refused" >&2; exit 64 ;;
esac
[ "${#bootstrap_tenant_name}" -le 128 ] || {
    echo "compose: bootstrap tenant name was refused" >&2
    exit 64
}

for setting in DATABASE_URL SYNVEDA_MIGRATOR_DATABASE_URL SYNVEDA_GATEWAY_DATABASE_URL \
    SYNVEDA_WORKER_DATABASE_URL \
    SYNVEDA_KMS_KEY SYNVEDA_KMS_KEY_REF POSTGRES_PASSWORD KC_DB_PASSWORD KC_BOOTSTRAP_ADMIN_USERNAME \
    KC_BOOTSTRAP_ADMIN_PASSWORD SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD \
    SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD; do
    case "$setting" in
        DATABASE_URL) present=${DATABASE_URL+x} ;;
        SYNVEDA_MIGRATOR_DATABASE_URL) present=${SYNVEDA_MIGRATOR_DATABASE_URL+x} ;;
        SYNVEDA_GATEWAY_DATABASE_URL) present=${SYNVEDA_GATEWAY_DATABASE_URL+x} ;;
        SYNVEDA_WORKER_DATABASE_URL) present=${SYNVEDA_WORKER_DATABASE_URL+x} ;;
        SYNVEDA_KMS_KEY) present=${SYNVEDA_KMS_KEY+x} ;;
        SYNVEDA_KMS_KEY_REF) present=${SYNVEDA_KMS_KEY_REF+x} ;;
        POSTGRES_PASSWORD) present=${POSTGRES_PASSWORD+x} ;;
        KC_DB_PASSWORD) present=${KC_DB_PASSWORD+x} ;;
        KC_BOOTSTRAP_ADMIN_USERNAME) present=${KC_BOOTSTRAP_ADMIN_USERNAME+x} ;;
        KC_BOOTSTRAP_ADMIN_PASSWORD) present=${KC_BOOTSTRAP_ADMIN_PASSWORD+x} ;;
        SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD) present=${SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD+x} ;;
        SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD) present=${SYNVEDA_KEYCLOAK_DEMO_ADMIN_PASSWORD+x} ;;
        SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD) present=${SYNVEDA_KEYCLOAK_DEMO_MEMBER_PASSWORD+x} ;;
    esac
    [ -z "${present:-}" ] || {
        echo "compose: direct secret setting $setting is forbidden; use the role-specific file" >&2
        exit 78
    }
done

if [ "$action" = up ]; then
    run_resolver_preflight
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-network-preflight.mjs" \
        --project "$project" --pool "$compose_ipv4_pool" --docker-bin "$docker_bin"
    run_bounded "$lifecycle_timeout" env \
        "SYNVEDA_COMPOSE_RUNTIME=$runtime" \
        "SYNVEDA_POSTGRES_MODE=$postgres_mode" \
        "SYNVEDA_OIDC_MODE=$oidc_mode" \
        "SYNVEDA_COMPOSE_PROJECT_SUFFIX=$suffix" \
        "SYNVEDA_APP_HOST=$app_host" \
        "SYNVEDA_PUBLIC_SCHEME=$public_scheme" \
        "SYNVEDA_DEV_HTTP_PORT=$public_port" \
        "$script_dir/generate-secrets.sh" --if-missing
    if [ "$oidc_mode" = bundled ]; then
        run_bounded "$lifecycle_timeout" env \
            "SYNVEDA_COMPOSE_RUNTIME=$runtime" \
            "SYNVEDA_OIDC_MODE=$oidc_mode" \
            "SYNVEDA_COMPOSE_PROJECT_SUFFIX=$suffix" \
            "SYNVEDA_APP_HOST=$app_host" \
            "SYNVEDA_AUTH_HOST=$auth_host" \
            "SYNVEDA_PUBLIC_SCHEME=$public_scheme" \
            "SYNVEDA_DEV_HTTP_PORT=$public_port" \
            "SYNVEDA_BOOTSTRAP_TENANT_ID=$bootstrap_tenant_id" \
            "$script_dir/generate-issuer.sh" --if-missing
    fi
fi

absolute_from_compose() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        ./*) printf '%s/%s\n' "$compose_dir" "${1#./}" ;;
        *) printf '%s/%s\n' "$compose_dir" "$1" ;;
    esac
}

secret_dir=$(absolute_from_compose "${SYNVEDA_SECRETS_DIR:-./runtime/$project/secrets}")
issuer_file=$(absolute_from_compose "${SYNVEDA_OIDC_ISSUERS_FILE:-./runtime/$project/issuers.json}")
database_authority_dir=$(absolute_from_compose "${SYNVEDA_DATABASE_AUTHORITY_DIR:-./runtime/$project/database-authority}")
keycloak_public_gate_dir=$(absolute_from_compose "${SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR:-./runtime/$project/keycloak-public-gate}")
if [ "${SYNVEDA_DATABASE_ROLES_FILE+x}" = x ]; then
    [ -n "$SYNVEDA_DATABASE_ROLES_FILE" ] || {
        echo "compose: SYNVEDA_DATABASE_ROLES_FILE must not be empty" >&2
        exit 78
    }
    database_roles_input=$SYNVEDA_DATABASE_ROLES_FILE
elif [ "$postgres_mode" = external ]; then
    echo "compose: external PostgreSQL requires an explicit topology-specific SYNVEDA_DATABASE_ROLES_FILE" >&2
    exit 78
elif [ "$oidc_mode" = bundled ]; then
    database_roles_input=./configs/database/roles.reference.json
else
    database_roles_input=./configs/database/roles.external-oidc.json
fi
database_roles_file=$(absolute_from_compose "$database_roles_input")
reject_sensitive_build_context_path() {
    candidate=$1
    allowed_root=$2
    label=$3
    case "$candidate" in
        "$repo_root"|"$repo_root"/*)
            case "$candidate" in
                "$allowed_root"|"$allowed_root"/*) ;;
                *)
                    echo "compose: $label must be outside the Docker build context or under its ignored Compose root" >&2
                    exit 78
                    ;;
            esac
            ;;
    esac
}
reject_sensitive_build_context_path "$secret_dir" "$compose_dir/runtime" secret-directory
reject_sensitive_build_context_path "$issuer_file" "$compose_dir/runtime" issuer-configuration
if [ "$oidc_mode" = bundled ]; then
    reject_sensitive_build_context_path "$database_authority_dir" "$compose_dir/runtime" database-authority
    reject_sensitive_build_context_path "$keycloak_public_gate_dir" "$compose_dir/runtime" keycloak-public-gate
fi
mode_of() {
    stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1" 2>/dev/null
}
owner_of() {
    stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1" 2>/dev/null
}
group_of() {
    stat -c '%g' "$1" 2>/dev/null || stat -f '%g' "$1" 2>/dev/null
}
size_of() {
    stat -c '%s' "$1" 2>/dev/null || stat -f '%z' "$1" 2>/dev/null
}
require_private_directory() {
    directory=$1
    label=$2
    [ ! -L "$directory" ] && [ -d "$directory" ] || {
        echo "compose: $label directory is missing or is a symlink" >&2
        exit 78
    }
    [ "$(mode_of "$directory")" = 700 ] || {
        echo "compose: $label directory must have mode 0700" >&2
        exit 78
    }
    [ "$(owner_of "$directory")" = "$runtime_uid" ] && \
        [ "$(group_of "$directory")" = "$runtime_gid" ] || {
        echo "compose: $label directory must be owned by the runtime UID:GID" >&2
        exit 78
    }
}
require_private_directory "$secret_dir" secret
secret_dir=$(CDPATH= cd "$secret_dir" && pwd -P)
case "$secret_dir" in
    */"$project"/secrets) ;;
    *)
        echo "compose: secret directory must be scoped to project $project" >&2
        exit 78
        ;;
esac
reject_sensitive_build_context_path "$secret_dir" "$compose_dir/runtime" secret-directory
oidc_directory_secret_dir=$secret_dir/oidc-directory
require_private_directory "$oidc_directory_secret_dir" oidc-directory-secret
oidc_directory_secret_dir=$(CDPATH= cd "$oidc_directory_secret_dir" && pwd -P)
reject_sensitive_build_context_path "$oidc_directory_secret_dir" "$compose_dir/runtime" oidc-directory-secret
if [ "$oidc_mode" = bundled ]; then
    require_private_directory "$database_authority_dir" database-authority
    database_authority_dir=$(CDPATH= cd "$database_authority_dir" && pwd -P)
    reject_sensitive_build_context_path "$database_authority_dir" "$compose_dir/runtime" database-authority
    require_private_directory "$keycloak_public_gate_dir" keycloak-public-gate
    keycloak_public_gate_dir=$(CDPATH= cd "$keycloak_public_gate_dir" && pwd -P)
    reject_sensitive_build_context_path "$keycloak_public_gate_dir" "$compose_dir/runtime" keycloak-public-gate
    case "$database_authority_dir" in
        */"$project"/database-authority) ;;
        *)
            echo "compose: database-authority directory must be scoped to project $project" >&2
            exit 78
            ;;
    esac
    case "$keycloak_public_gate_dir" in
        */"$project"/keycloak-public-gate) ;;
        *)
            echo "compose: keycloak-public-gate directory must be scoped to project $project" >&2
            exit 78
            ;;
    esac
fi
issuer_parent=$(dirname "$issuer_file")
require_private_directory "$issuer_parent" issuer-configuration
issuer_parent=$(CDPATH= cd "$issuer_parent" && pwd -P)
issuer_file=$issuer_parent/$(basename "$issuer_file")
if [ "$oidc_mode" = bundled ]; then
    case "$issuer_file" in
        */"$project"/issuers.json) ;;
        *)
            echo "compose: bundled issuer input must be scoped to project $project" >&2
            exit 78
            ;;
    esac
fi
reject_sensitive_build_context_path "$issuer_file" "$compose_dir/runtime" issuer-configuration
require_private_file() {
    file=$1
    label=$2
    [ ! -L "$file" ] && [ -f "$file" ] || {
        echo "compose: required $label file is missing or is a symlink" >&2
        exit 78
    }
    mode=$(mode_of "$file")
    [ "$mode" = 600 ] || {
        echo "compose: required $label file must have mode 0600" >&2
        exit 78
    }
    [ "$(owner_of "$file")" = "$runtime_uid" ] && \
        [ "$(group_of "$file")" = "$runtime_gid" ] || {
        echo "compose: required $label file must be owned by the runtime UID:GID" >&2
        exit 78
    }
    [ -s "$file" ] || {
        echo "compose: required $label file must not be empty" >&2
        exit 78
    }
}

for name in synveda_migrator_database_url synveda_gateway_database_url \
    synveda_worker_database_url synveda_kms_key synveda_kms_key_ref; do
    require_private_file "$secret_dir/$name" "$name"
done
if [ "$postgres_mode" = bundled ]; then
    require_private_file "$secret_dir/postgres_owner_password" postgres_owner_password
    require_private_file "$secret_dir/synveda_migrator_password" synveda_migrator_password
    require_private_file "$secret_dir/synveda_gateway_password" synveda_gateway_password
    require_private_file "$secret_dir/synveda_worker_password" synveda_worker_password
fi
if [ "$oidc_mode" = bundled ]; then
    require_private_file "$secret_dir/postgres_owner_password" postgres_owner_password
    require_private_file "$secret_dir/keycloak_database_password" keycloak_database_password
    require_private_file "$secret_dir/keycloak_admin_username" keycloak_admin_username
    require_private_file "$secret_dir/keycloak_admin_password" keycloak_admin_password
    require_private_file "$secret_dir/keycloak_convergence_admin_password" \
        keycloak_convergence_admin_password
    if [ "$demo_profile" = true ]; then
        require_private_file "$secret_dir/keycloak_demo_admin_password" \
            keycloak_demo_admin_password
        require_private_file "$secret_dir/keycloak_demo_member_password" \
            keycloak_demo_member_password
    fi
fi
if [ "$runtime" = reference ]; then
    require_private_file "$secret_dir/tls_cert" tls_cert
    require_private_file "$secret_dir/tls_key" tls_key
    case "$action" in
        config|up|smoke|restart-gateway)
            set_remaining_lifecycle_seconds
            set -- "$script_dir/check-tls-inputs.mjs" \
                --cert-file "$secret_dir/tls_cert" \
                --key-file "$secret_dir/tls_key" \
                --oidc "$oidc_mode" --app-host "$app_host" \
                --valid-for-seconds "$lifecycle_remaining"
            if [ "$oidc_mode" = bundled ]; then
                set -- "$@" --auth-host "$auth_host"
            fi
            run_bounded "$lifecycle_timeout" "$node_runner" "$@"
            ;;
    esac
fi
require_private_file "$issuer_file" issuer_configuration

[ ! -L "$database_roles_file" ] && [ -f "$database_roles_file" ] || {
    echo "compose: database role contract file is missing or is a symlink" >&2
    exit 78
}
database_roles_bytes=$(size_of "$database_roles_file") || {
    echo "compose: database role contract size cannot be inspected" >&2
    exit 78
}
case "$database_roles_bytes" in
    ''|*[!0-9]*)
        echo "compose: database role contract size cannot be inspected" >&2
        exit 78
        ;;
esac
[ "$database_roles_bytes" -gt 0 ] && [ "$database_roles_bytes" -le 4096 ] || {
    echo "compose: database role contract must contain 1 through 4096 bytes" >&2
    exit 78
}
database_roles_parent=$(CDPATH= cd "$(dirname "$database_roles_file")" && pwd -P)
database_roles_file=$database_roles_parent/$(basename "$database_roles_file")

path_is_within() {
    candidate=$1
    directory=$2
    [ "$candidate" = "$directory" ] || case "$candidate" in
        "$directory"/*) return 0 ;;
        *) return 1 ;;
    esac
}
reject_directory_overlap() {
    first=$1
    second=$2
    label=$3
    if path_is_within "$first" "$second" || path_is_within "$second" "$first"; then
        echo "compose: $label directories must not overlap" >&2
        exit 78
    fi
}
reject_file_in_directory() {
    file=$1
    directory=$2
    label=$3
    if path_is_within "$file" "$directory"; then
        echo "compose: $label file must not be inside runtime authority state" >&2
        exit 78
    fi
}
if [ "$oidc_mode" = bundled ]; then
    reject_directory_overlap "$secret_dir" "$database_authority_dir" \
        secret-and-database-authority
    reject_directory_overlap "$secret_dir" "$keycloak_public_gate_dir" \
        secret-and-keycloak-public-gate
    reject_directory_overlap "$database_authority_dir" "$keycloak_public_gate_dir" \
        database-authority-and-keycloak-public-gate
    for state_directory in "$database_authority_dir" "$keycloak_public_gate_dir"; do
        reject_file_in_directory "$issuer_file" "$state_directory" issuer-configuration
        reject_file_in_directory "$database_roles_file" "$state_directory" database-role-contract
    done
fi
reject_file_in_directory "$issuer_file" "$secret_dir" issuer-configuration
reject_file_in_directory "$database_roles_file" "$secret_dir" database-role-contract

if [ "$oidc_mode" = bundled ]; then
    oidc_issuer=$public_auth_url/realms/synveda
else
    oidc_issuer=${SYNVEDA_OIDC_ISSUER:-}
    case "$oidc_issuer" in
        http://*|https://*) ;;
        *)
            echo "compose: external OIDC requires an absolute SYNVEDA_OIDC_ISSUER" >&2
            exit 64
            ;;
    esac
    case "$oidc_issuer" in
        *'@'*|*'?'*|*'#'*|*[[:space:]]*)
            echo "compose: SYNVEDA_OIDC_ISSUER must not contain credentials, whitespace, a query or a fragment" >&2
            exit 64
            ;;
    esac
    if [ "$runtime" = reference ]; then
        case "$oidc_issuer" in
            https://*) ;;
            *) echo "compose: reference external OIDC requires HTTPS" >&2; exit 64 ;;
        esac
    fi
fi
issuer_bytes=$(size_of "$issuer_file") || {
    echo "compose: issuer configuration size cannot be inspected" >&2
    exit 78
}
case "$issuer_bytes" in
    ''|*[!0-9]*)
        echo "compose: issuer configuration size cannot be inspected" >&2
        exit 78
        ;;
esac
[ "$issuer_bytes" -le 1048576 ] || {
    echo "compose: issuer configuration exceeds the 1048576 byte startup bound" >&2
    exit 78
}

postgres_bootstrap_url_set=${SYNVEDA_POSTGRES_BOOTSTRAP_URL+x}
database_expected_host=
database_expected_port=
database_expected_name=
if [ "$postgres_mode" = bundled ]; then
    [ -z "${postgres_bootstrap_url_set:-}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL is not accepted with bundled PostgreSQL" >&2
        exit 64
    }
    postgres_bootstrap_url=postgresql://synveda_owner@postgres:5432/postgres
    postgres_bundled_cluster=true
    database_expected_host=postgres
    database_expected_port=5432
    database_expected_name=synveda
elif [ "$oidc_mode" = bundled ]; then
    postgres_bootstrap_url=${SYNVEDA_POSTGRES_BOOTSTRAP_URL:-}
    case "$postgres_bootstrap_url" in
        postgres://*|postgresql://*) ;;
        *)
            echo "compose: external PostgreSQL with bundled Keycloak requires SYNVEDA_POSTGRES_BOOTSTRAP_URL" >&2
            exit 64
            ;;
    esac
    case "$postgres_bootstrap_url" in
        *'?'*|*'#'*|*[![:print:]]*|*' '*)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must contain no query, fragment or whitespace" >&2
            exit 64
            ;;
    esac
    pg_location=${postgres_bootstrap_url#*://}
    pg_authority=${pg_location%%/*}
    pg_database=${pg_location#*/}
    [ "$pg_authority" != "$pg_location" ] && [ -n "$pg_database" ] && \
        [ "$pg_database" = "${pg_database##*/}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must identify one database" >&2
        exit 64
    }
    case "$pg_authority" in
        *@*)
            pg_user=${pg_authority%%@*}
            pg_endpoint=${pg_authority#*@}
            ;;
        *)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must name a login without a password" >&2
            exit 64
            ;;
    esac
    case "$pg_user" in
        ''|*:*|*'@'*|*[!A-Za-z0-9_-]*)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must name a login without a password" >&2
            exit 64
            ;;
    esac
    pg_host=${pg_endpoint%:*}
    pg_port=${pg_endpoint##*:}
    [ "$pg_host" != "$pg_endpoint" ] && valid_host "$pg_host" && \
    valid_runtime_id "$pg_port" && [ "$pg_port" -le 65535 ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must identify one canonical DNS host and port" >&2
        exit 64
    }
    postgres_bundled_cluster=false
    database_expected_host=$pg_host
    database_expected_port=$pg_port
    database_expected_name=synveda
else
    [ -z "${postgres_bootstrap_url_set:-}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL is not accepted without a bundled database bootstrap" >&2
        exit 64
    }
    postgres_bootstrap_url=
    postgres_bundled_cluster=false
fi

product_image=${SYNVEDA_PRODUCT_IMAGE:-synveda/product:dev}
postgres_image=${SYNVEDA_POSTGRES_IMAGE:-synveda/postgres:17.11-dev}
keycloak_image=${SYNVEDA_KEYCLOAK_IMAGE:-synveda/keycloak:26.7.2-dev}
caddy_image=${SYNVEDA_CADDY_IMAGE:-synveda/proxy:2.11.4-dev}
otel_image=${SYNVEDA_OTEL_COLLECTOR_IMAGE:-otel/opentelemetry-collector-contrib:0.159.0@sha256:1f2c54a30e713fac6b3ae77a1ec84010c2007e29ced8ec666214fc2f6739c1cc}
browser_image=${SYNVEDA_BROWSER_IMAGE:-synveda/browser-acceptance:1.62.1-dev}
valid_image_reference() {
    case "$1" in
        ''|*[!A-Za-z0-9_./:@+-]*) return 1 ;;
    esac
}
digest_image() {
    candidate=$1
    valid_image_reference "$candidate" || return 1
    case "$candidate" in *@sha256:*) ;; *) return 1 ;; esac
    digest=${candidate##*@sha256:}
    repository=${candidate%@sha256:*}
    [ -n "$repository" ] && [ "${#digest}" -eq 64 ] || return 1
    case "$repository" in *@*) return 1 ;; esac
    case "$digest" in
        *[!0-9a-f]*) return 1 ;;
    esac
}
for image_reference in "$product_image" "$postgres_image" "$keycloak_image" \
    "$caddy_image" "$otel_image"; do
    valid_image_reference "$image_reference" || {
        echo "compose: image references must use the closed OCI reference character set" >&2
        exit 64
    }
done
if [ "$browser_acceptance_profile" = true ]; then
    valid_image_reference "$browser_image" || {
        echo "compose: browser acceptance image must use the closed OCI reference character set" >&2
        exit 64
    }
    browser_seccomp_profile=$compose_dir/browser/seccomp_profile.json
    [ ! -L "$browser_seccomp_profile" ] && [ -f "$browser_seccomp_profile" ] || {
        echo "compose: reviewed browser seccomp profile was unavailable" >&2
        exit 78
    }
    browser_seccomp_parent=$(CDPATH= cd "$(dirname "$browser_seccomp_profile")" && pwd -P) || {
        echo "compose: reviewed browser seccomp profile was unavailable" >&2
        exit 78
    }
    browser_seccomp_profile=$browser_seccomp_parent/$(basename "$browser_seccomp_profile")
    run_bounded 30 "$node_runner" "$script_dir/check-browser-seccomp.mjs" \
        --profile "$browser_seccomp_profile"
fi
if [ "$runtime" = reference ]; then
    digest_image "$product_image" || {
        echo "compose: reference product image must use an OCI sha256 digest" >&2
        exit 64
    }
    if [ "$postgres_mode" = bundled ] || [ "$oidc_mode" = bundled ]; then
        digest_image "$postgres_image" || {
            echo "compose: reference PostgreSQL server/client image must use an OCI sha256 digest" >&2
            exit 64
        }
    fi
    if [ "$oidc_mode" = bundled ]; then
        digest_image "$keycloak_image" || {
            echo "compose: reference Keycloak image must use an OCI sha256 digest" >&2
            exit 64
        }
    fi
    digest_image "$caddy_image" || {
        echo "compose: reference proxy image must use an OCI sha256 digest" >&2
        exit 64
    }
fi
digest_image "$otel_image" || {
    echo "compose: Collector image must use an OCI sha256 digest" >&2
    exit 64
}
keycloak_database_url=
keycloak_database_url_set=${SYNVEDA_KEYCLOAK_DATABASE_URL+x}
if [ "$oidc_mode" = bundled ]; then
    if [ "$postgres_mode" = bundled ]; then
        [ -z "${keycloak_database_url_set:-}" ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL is not accepted with bundled PostgreSQL" >&2
            exit 64
        }
        keycloak_database_url=jdbc:postgresql://postgres:5432/keycloak
    else
        keycloak_database_url=${SYNVEDA_KEYCLOAK_DATABASE_URL:-}
        case "$keycloak_database_url" in
            jdbc:postgresql://*/*) ;;
            *)
                echo "compose: external PostgreSQL with bundled Keycloak requires SYNVEDA_KEYCLOAK_DATABASE_URL" >&2
                exit 64
                ;;
        esac
        case "$keycloak_database_url" in
            *@*|*'?'*|*'#'*)
                echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must be credential-free and contain no query or fragment" >&2
                exit 64
                ;;
        esac
        jdbc_location=${keycloak_database_url#jdbc:postgresql://}
        jdbc_host_port=${jdbc_location%%/*}
        jdbc_database=${jdbc_location#*/}
        [ "$jdbc_host_port" != "$jdbc_location" ] && \
            [ -n "$jdbc_database" ] && [ "$jdbc_database" = "${jdbc_database##*/}" ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must identify one host, port and database" >&2
            exit 64
        }
        jdbc_host=${jdbc_host_port%:*}
        jdbc_port=${jdbc_host_port##*:}
        jdbc_database_valid=true
        case "$jdbc_database" in
            [A-Za-z0-9_]*) ;;
            *) jdbc_database_valid=false ;;
        esac
        case "$jdbc_database" in
            *[!A-Za-z0-9_-]*) jdbc_database_valid=false ;;
            *) ;;
        esac
        [ "$jdbc_host" != "$jdbc_host_port" ] && valid_host "$jdbc_host" && \
            valid_runtime_id "$jdbc_port" && [ "$jdbc_port" -le 65535 ] && \
            [ "${#jdbc_database}" -le 63 ] && [ "$jdbc_database_valid" = true ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must identify one canonical DNS host, port and database" >&2
            exit 64
        }
        [ "$pg_database" = postgres ] && [ "$jdbc_database" = keycloak ] && \
            [ "$pg_host" = "$jdbc_host" ] && [ "$pg_port" = "$jdbc_port" ] || {
            echo "compose: Keycloak bootstrap and JDBC settings must use one endpoint, with postgres and keycloak databases" >&2
            exit 64
        }
    fi
else
    [ -z "${keycloak_database_url_set:-}" ] || {
        echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL is not accepted with external OIDC" >&2
        exit 64
    }
fi

capture_bounded_output 30 "$docker_bin" compose version --short || {
    compose_version_status=$?
    propagate_bounded_failure "$compose_version_status"
    echo "compose: Docker Compose is required" >&2
    exit 69
}
compose_version=$bounded_output
version_numbers=$(printf '%s\n' "$compose_version" | sed -E 's/^[^0-9]*([0-9]+)\.([0-9]+)\.([0-9]+).*/\1 \2 \3/')
set -- $version_numbers
[ "$#" -eq 3 ] || {
    echo "compose: could not parse Docker Compose version" >&2
    exit 69
}
if [ "$1" -lt 2 ] || \
    { [ "$1" -eq 2 ] && [ "$2" -lt 33 ]; } || \
    { [ "$1" -eq 2 ] && [ "$2" -eq 33 ] && [ "$3" -lt 1 ]; }; then
    echo "compose: Docker Compose 2.33.1 or newer is required" >&2
    exit 69
fi

export SYNVEDA_COMPOSE_RUNTIME=$runtime
export SYNVEDA_POSTGRES_MODE=$postgres_mode
export SYNVEDA_OIDC_MODE=$oidc_mode
export SYNVEDA_RUNTIME_UID=$runtime_uid
export SYNVEDA_RUNTIME_GID=$runtime_gid
export SYNVEDA_COMPOSE_RESTART_POLICY=$restart_policy
export SYNVEDA_APP_HOST=$app_host
export SYNVEDA_PUBLIC_SCHEME=$public_scheme
export SYNVEDA_PUBLIC_PORT=$public_port
export SYNVEDA_PUBLIC_APP_URL=$public_app_url
if [ "$oidc_mode" = bundled ]; then
    export SYNVEDA_AUTH_HOST=$auth_host
    export SYNVEDA_PUBLIC_AUTH_URL=$public_auth_url
else
    unset SYNVEDA_AUTH_HOST SYNVEDA_PUBLIC_AUTH_URL
fi
export SYNVEDA_INSECURE_DEVELOPMENT_HTTP=$insecure_development_http
export SYNVEDA_OIDC_ISSUER=$oidc_issuer
export SYNVEDA_BOOTSTRAP_TENANT_ID=$bootstrap_tenant_id
export SYNVEDA_BOOTSTRAP_TENANT_SLUG=$bootstrap_tenant_slug
export SYNVEDA_BOOTSTRAP_TENANT_NAME=$bootstrap_tenant_name
export SYNVEDA_POSTGRES_BOOTSTRAP_URL=$postgres_bootstrap_url
export SYNVEDA_POSTGRES_BUNDLED_CLUSTER=$postgres_bundled_cluster
export SYNVEDA_DATABASE_EXPECTED_HOST=$database_expected_host
export SYNVEDA_DATABASE_EXPECTED_PORT=$database_expected_port
export SYNVEDA_DATABASE_EXPECTED_NAME=$database_expected_name
export SYNVEDA_DEV_HTTP_PORT=$public_port
export SYNVEDA_PROXY_HTTP_PORT=$proxy_http_port
export SYNVEDA_PROXY_HTTPS_PORT=$proxy_https_port
export SYNVEDA_RENDER_PUBLIC_EDGE_SUBNET=$public_edge_subnet
export SYNVEDA_RENDER_PUBLIC_EDGE_GATEWAY=$public_edge_gateway
export SYNVEDA_RENDER_APP_BACKEND_SUBNET=$app_backend_subnet
export SYNVEDA_RENDER_APP_BACKEND_GATEWAY=$app_backend_gateway
export SYNVEDA_RENDER_DATA_SUBNET=$synveda_data_subnet
export SYNVEDA_RENDER_DATA_GATEWAY=$synveda_data_gateway
export SYNVEDA_RENDER_KEYCLOAK_DATA_SUBNET=$keycloak_data_subnet
export SYNVEDA_RENDER_KEYCLOAK_DATA_GATEWAY=$keycloak_data_gateway
export SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_SUBNET=$keycloak_management_subnet
export SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_GATEWAY=$keycloak_management_gateway
export SYNVEDA_RENDER_IDENTITY_SUBNET=$identity_subnet
export SYNVEDA_RENDER_IDENTITY_GATEWAY=$identity_gateway
export SYNVEDA_RENDER_IDENTITY_DYNAMIC_RANGE=$identity_dynamic_range
export SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS=$proxy_identity_address
export SYNVEDA_RENDER_TELEMETRY_SUBNET=$telemetry_subnet
export SYNVEDA_RENDER_TELEMETRY_GATEWAY=$telemetry_gateway
export SYNVEDA_RENDER_APPLICATION_EGRESS_SUBNET=$application_egress_subnet
export SYNVEDA_RENDER_APPLICATION_EGRESS_GATEWAY=$application_egress_gateway
export SYNVEDA_RENDER_IDENTITY_EGRESS_SUBNET=$identity_egress_subnet
export SYNVEDA_RENDER_IDENTITY_EGRESS_GATEWAY=$identity_egress_gateway
export SYNVEDA_RENDER_TELEMETRY_EGRESS_SUBNET=$telemetry_egress_subnet
export SYNVEDA_RENDER_TELEMETRY_EGRESS_GATEWAY=$telemetry_egress_gateway
export SYNVEDA_CADDY_APP_CONFIG=$caddy_app_config
export SYNVEDA_CADDY_IDENTITY_CONFIG=$caddy_identity_config
export SYNVEDA_SECRETS_DIR=$secret_dir
export SYNVEDA_OIDC_DIRECTORY_SECRETS_DIR=$oidc_directory_secret_dir
export SYNVEDA_OIDC_ISSUERS_FILE=$issuer_file
export SYNVEDA_DATABASE_ROLES_FILE=$database_roles_file
export SYNVEDA_DATABASE_AUTHORITY_DIR=$database_authority_dir
export SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR=$keycloak_public_gate_dir
export SYNVEDA_PRODUCT_IMAGE=$product_image
export SYNVEDA_POSTGRES_IMAGE=$postgres_image
export SYNVEDA_KEYCLOAK_IMAGE=$keycloak_image
export SYNVEDA_KEYCLOAK_DATABASE_URL=$keycloak_database_url
export SYNVEDA_KEYCLOAK_SSL_REQUIRED=$keycloak_ssl_required
export SYNVEDA_CADDY_IMAGE=$caddy_image
export SYNVEDA_OTEL_COLLECTOR_IMAGE=$otel_image
if [ "$browser_acceptance_profile" = true ]; then
    export SYNVEDA_BROWSER_IMAGE=$browser_image
    export SYNVEDA_BROWSER_SECCOMP_PROFILE=$browser_seccomp_profile
else
    unset SYNVEDA_BROWSER_IMAGE SYNVEDA_BROWSER_SECCOMP_PROFILE
fi

set -- compose --project-directory "$compose_dir" \
    --env-file "$compose_dir/.env.example" -p "$project" \
    -f "$compose_dir/compose.yaml" -f "$compose_dir/compose.$runtime_overlay.yaml"
if [ "$runtime" = development ] && [ "$postgres_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.postgres.dev.yaml"
fi
if [ "$runtime" = development ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak.dev.yaml"
fi
if [ "$postgres_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.postgres.yaml"
fi
if [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak.yaml"
fi
if [ "$postgres_mode" = bundled ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak-postgres.yaml"
fi
if [ "$postgres_mode" = external ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak-external-postgres.yaml"
fi
if [ "$postgres_mode" = external ]; then
    set -- "$@" -f "$compose_dir/compose.external-postgres.yaml"
fi
if [ "$postgres_mode" = external ] || [ "$oidc_mode" = external ]; then
    set -- "$@" -f "$compose_dir/compose.external.yaml"
fi
if [ "$demo_profile" = true ]; then
    set -- "$@" -f "$compose_dir/compose.demo.yaml"
fi
if [ "$browser_acceptance_profile" = true ]; then
    set -- "$@" -f "$compose_dir/compose.browser-acceptance.yaml"
fi
old_ifs=$IFS
IFS=,
for profile in $profiles; do
    [ -z "$profile" ] || set -- "$@" --profile "$profile"
done
IFS=$old_ifs

prepare_asset_contract() {
    asset_config_file=$(mktemp "$lifecycle_temp_root/synveda-compose-assets.XXXXXX") || exit 70
    chmod 600 "$asset_config_file"
    run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
        config --format json > "$asset_config_file"
    if [ "$initial_asset_state" = absent ]; then
        prove_assets_absent
    else
        prove_assets_existing
    fi
}

prove_assets_absent() {
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-compose-assets.mjs" \
        --config-file "$asset_config_file" --project "$project" \
        --docker-bin "$docker_bin" --state absent
}

prove_assets_existing() {
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-compose-assets.mjs" \
        --config-file "$asset_config_file" --project "$project" \
        --docker-bin "$docker_bin" --state existing
}

prove_assets_converged() {
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-compose-assets.mjs" \
        --config-file "$asset_config_file" --project "$project" \
        --docker-bin "$docker_bin" --state converged
}

prove_assets_stopped() {
    run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/check-compose-assets.mjs" \
        --config-file "$asset_config_file" --project "$project" \
        --docker-bin "$docker_bin" --state stopped
}

prepare_local_build_boundary() {
    build_context_status=0
    capture_bounded_output 30 "$docker_bin" context show || build_context_status=$?
    if [ "$build_context_status" -ne 0 ]; then
        propagate_bounded_failure "$build_context_status"
        echo "compose: pinned Docker build context was unavailable" >&2
        return 69
    fi
    [ "$bounded_output" = default ] || {
        echo "compose: pinned Docker build context was refused" >&2
        return 69
    }
    buildx_config_dir=$(mktemp -d "$lifecycle_temp_root/synveda-compose-buildx.XXXXXX") || {
        echo "compose: private Buildx state creation failed" >&2
        return 70
    }
    chmod 700 "$buildx_config_dir" || {
        echo "compose: private Buildx state permissions failed" >&2
        return 70
    }
    [ ! -L "$buildx_config_dir" ] && [ -d "$buildx_config_dir" ] || {
        echo "compose: private Buildx state was refused" >&2
        return 70
    }
    BUILDX_CONFIG=$buildx_config_dir
    export BUILDX_CONFIG
    run_bounded 25 "$node_runner" "$script_dir/check-local-builder.mjs" \
        --docker-bin "$docker_bin" || {
        builder_status=$?
        propagate_bounded_failure "$builder_status"
        return "$builder_status"
    }
}

run_runtime_smoke() {
    status_file=$(mktemp "$lifecycle_temp_root/synveda-compose-status.XXXXXX") || exit 70
    run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
        ps --all --format json > "$status_file"
    set -- "$script_dir/check-runtime-smoke.mjs" \
        --status-file "$status_file" --runtime "$runtime" \
        --postgres "$postgres_mode" --oidc "$oidc_mode" \
        --browser "$browser_acceptance_profile" \
        --app-url "$public_app_url" --issuer "$oidc_issuer"
    run_bounded "$lifecycle_timeout" "$node_runner" "$@"
    rm -f -- "$status_file"
    status_file=
}

wait_for_browser_acceptance() {
    browser_identity_status=0
    capture_bounded_output 30 "$docker_bin" "$@" \
        ps --all --quiet --no-trunc browser-acceptance || browser_identity_status=$?
    if [ "$browser_identity_status" -ne 0 ]; then
        propagate_bounded_failure "$browser_identity_status"
        echo "compose: browser acceptance container identity was unavailable" >&2
        return 69
    fi
    browser_container_identity=$bounded_output
    if [ "${#browser_container_identity}" -ne 64 ]; then
        echo "compose: browser acceptance container identity was refused" >&2
        return 78
    fi
    case "$browser_container_identity" in
        *[!0-9a-f]*)
            echo "compose: browser acceptance container identity was refused" >&2
            return 78
            ;;
    esac
    browser_wait_status=0
    capture_bounded_output "$lifecycle_timeout" "$docker_bin" container wait \
        "$browser_container_identity" || browser_wait_status=$?
    if [ "$browser_wait_status" -ne 0 ]; then
        propagate_bounded_failure "$browser_wait_status"
        echo "compose: browser acceptance result was unavailable" >&2
        return 69
    fi
    case "$bounded_output" in
        0) ;;
        ''|*[!0-9]*)
            echo "compose: browser acceptance result was malformed" >&2
            return 78
            ;;
        *)
            echo "compose: browser acceptance failed" >&2
            return 78
            ;;
    esac
}

capture_gateway_container_identity() {
    gateway_identity_status=0
    capture_bounded_output 30 "$docker_bin" "$@" \
        ps --all --quiet --no-trunc gateway || gateway_identity_status=$?
    if [ "$gateway_identity_status" -ne 0 ]; then
        propagate_bounded_failure "$gateway_identity_status"
        echo "compose: exact gateway container identity was unavailable" >&2
        return 69
    fi
    gateway_container_identity=$bounded_output
    if [ "${#gateway_container_identity}" -ne 64 ]; then
        echo "compose: exact gateway container identity was refused" >&2
        return 78
    fi
    case "$gateway_container_identity" in
        *[!0-9a-f]*)
            echo "compose: exact gateway container identity was refused" >&2
            return 78
            ;;
    esac
}

case "$action" in
    config)
        if [ -n "$output" ]; then
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
                config --format json --output "$output"
        else
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" config --quiet
        fi
        echo "canonical Compose configuration valid for $project ($postgres_mode PostgreSQL, $oidc_mode OIDC)"
        ;;
    up)
        prepare_asset_contract "$@"
        if [ "$runtime" = development ]; then
            prepare_local_build_boundary
            docker_mutation_phase=compose-build
            docker_mutation_uncertain=true
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
                build --builder default
            docker_mutation_uncertain=false
            docker_mutation_phase=
        fi
        docker_mutation_uncertain=true
        docker_mutation_phase=compose-up
        if [ "$browser_acceptance_profile" = true ]; then
            # An unreferenced one-shot is not portable through Compose's
            # `up --wait` classification. Converge the normal graph with the
            # fixture scaled to zero, then start exactly that already-built
            # service without recreating its proved-ready dependencies.
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
                up --no-build --detach --wait --wait-timeout "$lifecycle_timeout" \
                --force-recreate --scale browser-acceptance=0
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
                up --no-build --detach --no-deps --force-recreate \
                browser-acceptance
            browser_acceptance_status=0
            wait_for_browser_acceptance "$@" || browser_acceptance_status=$?
            case "$browser_acceptance_status" in
                0) ;;
                78)
                    docker_mutation_uncertain=false
                    docker_mutation_phase=
                    exit 78
                    ;;
                *) exit "$browser_acceptance_status" ;;
            esac
        else
            run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
                up --no-build --detach --wait --wait-timeout "$lifecycle_timeout" \
                --force-recreate
        fi
        asset_convergence_status=0
        prove_assets_converged || asset_convergence_status=$?
        case "$asset_convergence_status" in
            0) ;;
            78)
                # The Docker mutation completed and exact immutable container
                # inspection proved a deterministic contract violation. Leave
                # the project recoverable through down or force-recreate.
                docker_mutation_uncertain=false
                docker_mutation_phase=
                exit 78
                ;;
            *) exit "$asset_convergence_status" ;;
        esac
        docker_mutation_uncertain=false
        docker_mutation_phase=
        if [ "$browser_acceptance_profile" = true ]; then
            run_resolver_preflight
            run_runtime_smoke "$@"
        fi
        echo "canonical Compose services converged for $project"
        ;;
    down)
        run_docker_preflight
        prepare_asset_contract "$@"
        docker_mutation_uncertain=true
        docker_mutation_phase=compose-down
        run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
            down --timeout "$lifecycle_timeout"
        prove_assets_stopped
        docker_mutation_uncertain=false
        docker_mutation_phase=
        echo "canonical Compose services stopped for $project; persistent data retained"
        ;;
    smoke)
        prepare_asset_contract "$@"
        prove_assets_converged
        run_resolver_preflight
        run_runtime_smoke "$@"
        echo "canonical Compose smoke passed for $project"
        ;;
    restart-gateway)
        prepare_asset_contract "$@"
        prove_assets_converged
        run_resolver_preflight
        # Refuse to turn a pre-existing degraded graph into restart evidence.
        run_runtime_smoke "$@"
        capture_gateway_container_identity "$@"
        gateway_container_identity_before=$gateway_container_identity
        set_remaining_lifecycle_seconds
        [ "$lifecycle_remaining" -ge "$gateway_restart_required_seconds" ] || {
            echo "compose: insufficient lifecycle budget remains for a bounded gateway restart" >&2
            exit 124
        }
        docker_mutation_uncertain=true
        docker_mutation_phase=compose-restart-gateway
        run_bounded "$gateway_restart_runner_seconds" "$docker_bin" "$@" \
            restart --no-deps --timeout "$gateway_restart_stop_seconds" gateway
        # `restart` does not wait for health. Reuse the exact rendered graph
        # without recreating the container and let Compose enforce its health
        # contract before the public smoke is repeated.
        run_bounded "$gateway_restart_health_runner_seconds" "$docker_bin" "$@" \
            up --no-build --detach --wait --wait-timeout "$gateway_restart_health_seconds" \
            --no-deps --no-recreate gateway
        # Repeat the same captured asset, resolver and runtime checks after the
        # mutation. The pre-mutation render remains the comparison authority.
        prove_assets_converged
        run_resolver_preflight
        run_runtime_smoke "$@"
        capture_gateway_container_identity "$@"
        [ "$gateway_container_identity" = "$gateway_container_identity_before" ] || {
            echo "compose: gateway container identity changed during restart" >&2
            exit 78
        }
        docker_mutation_uncertain=false
        docker_mutation_phase=
        echo "canonical Compose gateway restart passed for $project"
        ;;
    reset)
        [ "${SYNVEDA_CONFIRM_RESET:-}" = "$project" ] || {
            echo "compose: reset requires SYNVEDA_CONFIRM_RESET=$project" >&2
            exit 64
        }
        run_docker_preflight
        prepare_asset_contract "$@"
        if [ "$oidc_mode" = bundled ]; then
            run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/reset-runtime-state.mjs" \
                --mode check --project "$project" \
                --authority-dir "$database_authority_dir" \
                --gate-dir "$keycloak_public_gate_dir"
        fi
        volume_name=${project}_postgres-data
        volume_format='{{.Name}}|{{.Driver}}|{{.Scope}}|{{json .Options}}|{{index .Labels "com.docker.compose.project"}}|{{index .Labels "com.docker.compose.volume"}}|{{index .Labels "com.synveda.contract"}}|{{index .Labels "com.synveda.volume"}}'
        volume_expected_prefix="$volume_name|local|local|"
        volume_expected_suffix="|$project|postgres-data|cpr-45|postgres-data"
        list_named_volume() {
            capture_bounded_output 30 "$docker_bin" volume ls --quiet \
                --filter "name=^${volume_name}$"
        }
        list_labelled_project_volume() {
            capture_bounded_output 30 "$docker_bin" volume ls --quiet \
                --filter "label=com.docker.compose.project=$project" \
                --filter "label=com.docker.compose.volume=postgres-data"
        }
        volume_present=false
        list_named_volume || {
            inventory_status=$?
            propagate_bounded_failure "$inventory_status"
            echo "compose: named project data volume inventory was unavailable" >&2
            exit 69
        }
        named_volume_candidates=$bounded_output
        list_labelled_project_volume || {
            inventory_status=$?
            propagate_bounded_failure "$inventory_status"
            echo "compose: project data volume inventory was unavailable" >&2
            exit 69
        }
        labelled_volume_candidates=$bounded_output
        case "$named_volume_candidates:$labelled_volume_candidates" in
            :) ;;
            "$volume_name:$volume_name") volume_present=true ;;
            *)
                echo "compose: exact project data volume inventory was refused" >&2
                exit 78
                ;;
        esac
        if [ "$volume_present" = true ]; then
            capture_bounded_output 30 "$docker_bin" volume inspect \
                --format "$volume_format" "$volume_name" || {
                inspection_status=$?
                propagate_bounded_failure "$inspection_status"
                echo "compose: exact project data volume inspection failed" >&2
                exit 69
            }
            volume_contract=$bounded_output
            case "$volume_contract" in
                "$volume_expected_prefix"null"$volume_expected_suffix"|\
                "$volume_expected_prefix"'{}'"$volume_expected_suffix") ;;
                *)
                    echo "compose: exact project data volume contract was refused" >&2
                    exit 78
                    ;;
            esac
        fi
        docker_mutation_uncertain=true
        docker_mutation_phase=compose-down-for-reset
        run_bounded "$lifecycle_timeout" "$docker_bin" "$@" \
            down --timeout "$lifecycle_timeout"
        prove_assets_stopped
        docker_mutation_uncertain=false
        docker_mutation_phase=
        list_named_volume || {
            inventory_status=$?
            propagate_bounded_failure "$inventory_status"
            echo "compose: named project data volume inventory was unavailable after shutdown" >&2
            exit 69
        }
        named_volume_candidates_after=$bounded_output
        list_labelled_project_volume || {
            inventory_status=$?
            propagate_bounded_failure "$inventory_status"
            echo "compose: project data volume inventory was unavailable after shutdown" >&2
            exit 69
        }
        labelled_volume_candidates_after=$bounded_output
        if [ "$volume_present" = true ]; then
            [ "$named_volume_candidates_after" = "$volume_name" ] && \
                [ "$labelled_volume_candidates_after" = "$volume_name" ] || {
                echo "compose: exact project data volume changed during reset" >&2
                exit 78
            }
            capture_bounded_output 30 "$docker_bin" volume inspect \
                --format "$volume_format" "$volume_name" || {
                inspection_status=$?
                propagate_bounded_failure "$inspection_status"
                echo "compose: exact project data volume disappeared during reset" >&2
                exit 70
            }
            volume_contract=$bounded_output
            case "$volume_contract" in
                "$volume_expected_prefix"null"$volume_expected_suffix"|\
                "$volume_expected_prefix"'{}'"$volume_expected_suffix") ;;
                *)
                    echo "compose: exact project data volume changed during reset" >&2
                    exit 78
                    ;;
            esac
            docker_mutation_uncertain=true
            docker_mutation_phase=project-volume-removal
            run_bounded 30 "$docker_bin" volume rm "$volume_name" >/dev/null || {
                echo "compose: exact project data volume removal failed" >&2
                exit 70
            }
            list_named_volume || {
                inventory_status=$?
                propagate_bounded_failure "$inventory_status"
                echo "compose: named project data volume inventory was unavailable after removal" >&2
                exit 69
            }
            named_volume_candidates_final=$bounded_output
            list_labelled_project_volume || {
                inventory_status=$?
                propagate_bounded_failure "$inventory_status"
                echo "compose: project data volume inventory was unavailable after removal" >&2
                exit 69
            }
            labelled_volume_candidates_final=$bounded_output
            [ -z "$named_volume_candidates_final" ] && \
                [ -z "$labelled_volume_candidates_final" ] || {
                echo "compose: exact project data volume remains after reset" >&2
                exit 78
            }
            docker_mutation_uncertain=false
            docker_mutation_phase=
        else
            [ -z "$named_volume_candidates_after" ] && \
                [ -z "$labelled_volume_candidates_after" ] || {
                echo "compose: project data volume appeared during reset" >&2
                exit 78
            }
        fi
        if [ "$oidc_mode" = bundled ]; then
            run_bounded "$lifecycle_timeout" "$node_runner" "$script_dir/reset-runtime-state.mjs" \
                --mode apply --project "$project" \
                --authority-dir "$database_authority_dir" \
                --gate-dir "$keycloak_public_gate_dir"
        fi
        echo "canonical Compose data reset for $project; secrets, issuer and KMS key retained"
        ;;
esac
