#!/usr/bin/env sh
# Shared CPR-13 harness for feature demos re-pointed from the retired hierarchy
# and global runtime routes. Callers still own their narrative and focused
# acceptance command; this file only gives each one a real, isolated epoch-3
# Postgres database.

demo_start() {
  if [ "$#" -ne 2 ]; then
    echo "usage: demo_start <slug> <title>" >&2
    exit 2
  fi

  DEMO_SLUG=$1
  DEMO_TITLE=$2
  DEMO_REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
  DEMO_COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
  DEMO_DATABASE="synveda_${DEMO_SLUG}_demo_$$"
  export DEMO_REPO_ROOT DEMO_COMPOSE DEMO_DATABASE

  cd "$DEMO_REPO_ROOT"
  # shellcheck disable=SC2086 # the compose command intentionally has words.
  $DEMO_COMPOSE up --detach --wait postgres
  # shellcheck disable=SC2086
  $DEMO_COMPOSE exec -T postgres createdb -U synveda "$DEMO_DATABASE"
  # shellcheck disable=SC2086
  $DEMO_COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda \
    -d "$DEMO_DATABASE" -c \
    "create extension if not exists vector; create extension if not exists btree_gin" \
    >/dev/null

  DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DEMO_DATABASE"
  SQLX_OFFLINE=true
  export DATABASE_URL SQLX_OFFLINE
  trap demo_cleanup EXIT HUP INT TERM

  echo "==> $DEMO_TITLE"
  echo "    isolated schema epoch 3 database: $DEMO_DATABASE"
}

demo_cleanup() {
  if [ -n "${DEMO_DATABASE:-}" ]; then
    # shellcheck disable=SC2086
    $DEMO_COMPOSE exec -T postgres dropdb -U synveda --if-exists --force \
      "$DEMO_DATABASE" >/dev/null 2>&1 || true
    DEMO_DATABASE=
  fi
}

demo_finish() {
  echo ""
  echo "$DEMO_TITLE: current context-platform acceptance evidence passes."
}
