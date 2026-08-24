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
# An exported credential takes precedence over Claude Code's native credential
# store. Check the latter in isolation first: a stale or malformed environment
# token must not shadow an otherwise valid installed-client login during this
# installed-client acceptance gate.
native_auth=$(
  unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN
  "$CLAUDE_BIN" auth status 2>&1 || true
)
use_native_auth=0
echo "Claude Code: $version"
case "$native_auth" in
  *'"loggedIn": true'*|*'"loggedIn":true'*)
    echo "Claude authentication: native credential available"
    use_native_auth=1
    unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN
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

# Move only the credential into the disposable config rather than cloning
# settings, history or plugins from the user's real client state. Linux and
# Windows commonly expose the OAuth credential as a private file; macOS keeps
# the default-profile item in Keychain.
claude_config_source=${CLAUDE_CONFIG_DIR:-${HOME}/.claude}
if [ -f "$claude_config_source/.credentials.json" ]; then
  export SYNVEDA_CLAUDE_CREDENTIALS_FILE="$claude_config_source/.credentials.json"
elif [ "$use_native_auth" -eq 1 ] && [ "$(uname -s)" = Darwin ]; then
  # Setting either HOME or CLAUDE_CONFIG_DIR changes the Keychain namespace in
  # Claude Code 2.1.241. Hand the default-profile item to the isolated config
  # as a private, short-lived file; never print or retain its contents.
  credential_handoff=$(mktemp "${TMPDIR:-/tmp}/synveda-claude-credentials.XXXXXX")
  chmod 600 "$credential_handoff"
  cleanup_credential_handoff() {
    if [ -n "${credential_handoff:-}" ] && [ -f "$credential_handoff" ]; then
      rm -f -- "$credential_handoff"
    fi
  }
  trap cleanup_credential_handoff EXIT HUP INT TERM
  if ! security find-generic-password -s "Claude Code-credentials" -w >"$credential_handoff"; then
    echo "CPR-14 live PENDING: the native Claude credential could not be copied into the isolated configuration" >&2
    exit 77
  fi
  export SYNVEDA_CLAUDE_CREDENTIALS_FILE="$credential_handoff"
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
