#!/bin/sh
# CPR-45 clean-Engine candidate preparation. Provider creation and Docker
# mutation deliberately remain later receipt phases.
set -eu
umask 077

case "${1:-}" in
    plan|status|verify) action=$1 ;;
    *) echo "usage: clean-engine-acceptance.sh {plan|status|verify}" >&2; exit 64 ;;
esac
[ "$#" -eq 1 ] || {
    echo "usage: clean-engine-acceptance.sh {plan|status|verify}" >&2
    exit 64
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
repo_root=$(CDPATH= cd "$script_dir/../../.." && pwd -P)
state_base=${SYNVEDA_CLEAN_ENGINE_STATE_BASE:-}
if [ -z "$state_base" ]; then
    [ -n "${HOME:-}" ] && [ -d "$HOME" ] || {
        echo "clean-engine: HOME or SYNVEDA_CLEAN_ENGINE_STATE_BASE is required" >&2
        exit 64
    }
    home_root=$(CDPATH= cd "$HOME" && pwd -P) || {
        echo "clean-engine: HOME was unavailable" >&2
        exit 69
    }
    state_base=$home_root/.local/state/synveda/compose-acceptance
fi
case "$state_base" in
    /*) ;;
    *) echo "clean-engine: state base must be absolute" >&2; exit 64 ;;
esac

set -- "$script_dir/run-node-closed" "$script_dir/clean-engine-state.mjs" \
    "$action" --repo-root "$repo_root" --state-base "$state_base"
if [ "$action" = plan ]; then
    [ -n "${SYNVEDA_COMPOSE_IPV4_POOL:-}" ] || {
        echo "clean-engine: SYNVEDA_COMPOSE_IPV4_POOL is required" >&2
        exit 64
    }
    set -- "$@" --ipv4-pool "$SYNVEDA_COMPOSE_IPV4_POOL" \
        --provider "${SYNVEDA_CLEAN_ENGINE_PROVIDER:-colima}"
fi
exec "$@"
