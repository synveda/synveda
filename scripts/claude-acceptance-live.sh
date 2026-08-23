#!/usr/bin/env bash
# CPR-14 tier 3: real Claude Code, installed marketplace plugin, live gateway.
# Exit 77 means the environment cannot run the live gate; it never means pass.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ -n "${SYNVEDA_CLAUDE_BIN:-}" ]; then
  CLAUDE_BIN=$SYNVEDA_CLAUDE_BIN
else
  CLAUDE_BIN=$(command -v claude || true)
fi
if [ -z "$CLAUDE_BIN" ]; then
  echo "CPR-14 live PENDING: no claude executable is on PATH" >&2
  exit 77
fi

version=$("$CLAUDE_BIN" --version 2>&1 || true)
auth=$("$CLAUDE_BIN" auth status 2>&1 || true)
echo "Claude Code: $version"
case "$auth" in
  *'"loggedIn": true'*|*'"loggedIn":true'*)
    echo "Claude authentication: available"
    ;;
  *)
    if [ -z "${ANTHROPIC_API_KEY:-}" ] && \
       [ -z "${ANTHROPIC_AUTH_TOKEN:-}" ] && \
       [ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" ]; then
      echo "CPR-14 live PENDING: Claude Code is not authenticated and no isolated-run credential is set" >&2
      exit 77
    fi
    echo "Claude authentication: isolated environment credential available"
    ;;
esac

# macOS subscription credentials live in Keychain and remain available under
# an isolated config directory. Linux/Windows OAuth credentials are a private
# file, so copy only that file into the disposable config rather than cloning
# settings, history or plugins from the user's real client state.
claude_config_source=${CLAUDE_CONFIG_DIR:-${HOME}/.claude}
if [ -f "$claude_config_source/.credentials.json" ]; then
  export SYNVEDA_CLAUDE_CREDENTIALS_FILE="$claude_config_source/.credentials.json"
fi

pnpm --filter @synveda/claude-code-adapter build
cargo build -p synveda-cli --bin synveda

SYNVEDA_CLAUDE_LIVE=1 \
SYNVEDA_CLAUDE_BIN="$CLAUDE_BIN" \
DATABASE_URL=${DATABASE_URL:-postgres://synveda:synveda-dev@localhost:5432/synveda} \
  bash scripts/db-test.sh \
    -p synveda-gateway \
    --test claude_lifecycle \
    an_installed_claude_executable_completes_the_session_plane \
    -- --ignored --exact --nocapture --test-threads=1
