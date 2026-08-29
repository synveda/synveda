#!/usr/bin/env sh
# Shared CPR-13/CPR-45 harness for feature demos. The first source replaces the
# caller with the deployment-owned exact-role fixture. That fixture then runs
# the same demo again with an already-migrated epoch-3 database, an ordinary
# gateway login and the one private migrator file used only for tenant
# admission. The outer fixture owns cleanup, including failed-state retention.

DEMO_REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
if [ "${SYNVEDA_EXACT_ROLE_DEMO:-}" != 1 ]; then
  exec env SYNVEDA_DB_TEST_TASK=demo \
    bash "$DEMO_REPO_ROOT/scripts/db-test.sh" "$0" "$@"
fi

[ -n "${DATABASE_URL:-}" ] \
  && [ -f "${SYNVEDA_TEST_DATABASE_URL_FILE:-}" ] \
  && [ -f "${SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE:-}" ] \
  && [ -f "${SYNVEDA_DATABASE_ROLES_FILE:-}" ] || {
  echo "demo: exact-role database fixture is incomplete" >&2
  exit 78
}

demo_start() {
  if [ "$#" -ne 2 ]; then
    echo "usage: demo_start <slug> <title>" >&2
    exit 2
  fi

  DEMO_SLUG=$1
  DEMO_TITLE=$2
  export DEMO_REPO_ROOT DEMO_SLUG DEMO_TITLE

  cd "$DEMO_REPO_ROOT"

  echo "==> $DEMO_TITLE"
  echo "    isolated exact-role schema epoch 3 database"
}

demo_finish() {
  echo ""
  echo "$DEMO_TITLE: current context-platform acceptance evidence passes."
}
