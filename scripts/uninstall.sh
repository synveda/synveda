#!/bin/sh
# Synveda release-artifact uninstall boundary (OPS-10 remains open).
#
# Deployment lifecycle and data custody belong to the installed canonical
# launcher. This script deliberately removes nothing until an installer-owned
# receipt can prove the exact deployed configuration and CLI destination.
#
# Stop while preserving volumes and state:
#   SYNVEDA_APP_HOST=... SYNVEDA_AUTH_HOST=... \
#     ~/.synveda/reference/current/synveda-compose down
#
# Destroy one bundled reference deployment only with the exact confirmation
# printed by that launcher's reset command. Backups remain a separate custody
# decision.
#
# Usage:
#   scripts/uninstall.sh
#   scripts/uninstall.sh --dry-run
set -eu

dry_run=false
case "$#:${1:-}" in
  0:) ;;
  1:--dry-run) dry_run=true ;;
  1:-h|1:--help)
    sed -n '2,18p' "$0" | sed 's/^#\{1,\} \{0,1\}//'
    exit 0
    ;;
  *)
    echo "uninstall: accepted options are --dry-run and --help" >&2
    exit 64
    ;;
esac

home_dir=${SYNVEDA_HOME:-${HOME:?HOME is required}/.synveda}

echo "Synveda automatic artifact removal is not yet available."
echo ""
echo "No file, process, container, volume, credential or client configuration was changed."
echo ""
echo "Use the installed canonical lifecycle first:"
echo "  $home_dir/reference/current/synveda-compose down"
echo ""
echo "For destructive deployment reset, use that launcher's exact printed"
echo "SYNVEDA_CONFIRM_RESET value. Database/KMS recovery backups are separate."
echo ""
echo "Then retain or remove these installer-owned paths deliberately:"
echo "  artifacts  $home_dir/reference  $home_dir/bin  $home_dir/console  $home_dir/plugin"
echo "  state      $home_dir/state      (contains keys; preserve unless reset succeeded)"
echo "  backups    $home_dir/backups    (never implied by deployment reset)"

if [ "$dry_run" = true ]; then
  echo ""
  echo "--dry-run complete: no mutation was attempted."
  exit 0
fi

echo "" >&2
echo "uninstall: refusing unproved removal; OPS-10 requires an installer-owned receipt" >&2
exit 69
