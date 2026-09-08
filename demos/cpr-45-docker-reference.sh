#!/usr/bin/env sh
# CPR-45: the three bounded live reference gates reuse one canonical lifecycle.
# Successful acceptance and restore targets remain available for inspection.
set -eu

case "${1:-acceptance}" in
    acceptance|backup|restore-smoke) action=${1:-acceptance} ;;
    *) echo "usage: demos/cpr-45-docker-reference.sh [acceptance|backup|restore-smoke]" >&2; exit 64 ;;
esac

exec "$(dirname "$0")/../deploy/compose/scripts/compose.sh" "$action"
