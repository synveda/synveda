#!/usr/bin/env sh
# Removes Synveda (OPS-10, ADR-0067) — the mirror of scripts/install.sh.
#
# POSIX sh, for install.sh's reason: this may have to run on a machine where
# something of ours is already broken, so it uses no bashism and nothing
# outside coreutils and, when a deployment is up, docker.
#
# What it removes, and nothing else:
#
#   $SYNVEDA_HOME/bin/            the gateway, and the CLI if it landed here
#   $SYNVEDA_HOME/console/        the admin console bundle
#   $SYNVEDA_HOME/profile/        compose file, Rauthy config, demo seeder
#   $SYNVEDA_HOME/plugin/         the Claude Code plugin marketplace
#   $SYNVEDA_HOME/data/           pidfile, log, rendered env, and kms.key
#   $SYNVEDA_BIN/synveda          the CLI, if the installer put it there
#
# **Your data survives by default.** `init` created four named Docker volumes
# and this stops the containers without touching them, because removing them
# is the only way to remove a tenant's memory — TEN-5 means a tenant row
# cannot be deleted, so the volume is the smallest unit of erasure that
# exists today. `--purge` removes them and says so.
#
# It touches **nothing** belonging to an editor or an AI client, exactly as
# install.sh promised not to. Those were separate, explicit steps and their
# removal is too: `synveda mcp uninstall --client <c>` and `synveda plugin
# uninstall`. This script reports what it found and names them.
#
# Environment:
#   SYNVEDA_HOME      install root                    (default ~/.synveda)
#   SYNVEDA_BIN       where the CLI went              (default /usr/local/bin)
#
# Usage:
#   ./uninstall.sh              remove the install, keep the data
#   ./uninstall.sh --purge      also destroy the volumes (a tenant's memory)
#   ./uninstall.sh --dry-run    list everything it would touch, change nothing
set -eu

HOME_DIR="${SYNVEDA_HOME:-$HOME/.synveda}"
BIN_DIR="${SYNVEDA_BIN:-/usr/local/bin}"

purge=no
dry_run=no
for arg in "$@"; do
  case "$arg" in
    --purge) purge=yes ;;
    --dry-run) dry_run=yes ;;
    -h|--help) sed -n '2,34p' "$0" | sed 's/^#\{1,\} \{0,1\}//'; exit 0 ;;
    *) printf 'uninstall: unknown option %s (try --help)\n' "$arg" >&2; exit 2 ;;
  esac
done

say()  { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }

# Everything routes through these two, so `--dry-run` cannot drift from what
# a real run does: there is one list of actions and a flag that stops short
# of performing them.
removed=0
remove_path() { # path label
  [ -e "$1" ] || [ -L "$1" ] || return 0
  removed=$((removed + 1))
  if [ "$dry_run" = yes ]; then
    say "    would remove  $1${2:+  ($2)}"
    return 0
  fi
  if rm -rf "$1" 2>/dev/null; then
    say "    removed  $1${2:+  ($2)}"
    return 0
  fi
  # The CLI's default home is root-owned on macOS, so the same sudo dance
  # install.sh does — and the same rule: a refusal must not kill the script
  # and leave a half-removed install (ADR-0065 amendment 6).
  if command -v sudo >/dev/null 2>&1 && sudo rm -rf "$1" 2>/dev/null; then
    say "    removed  $1  (via sudo)"
    return 0
  fi
  say "    COULD NOT REMOVE  $1 — remove it by hand:"
  say "        sudo rm -rf $1"
}

say "Synveda uninstall"
say ""
[ "$dry_run" = yes ] && { say "  --dry-run: nothing will be changed"; say ""; }

# ── 1. the deployment ───────────────────────────────────────────────────
compose_file="$HOME_DIR/profile/docker-compose.yml"
step "the deployment"
if [ ! -f "$compose_file" ]; then
  say "    no profile at $compose_file — nothing to stop"
elif ! command -v docker >/dev/null 2>&1; then
  say "    docker is not on PATH; skipping. If a deployment is running:"
  say "        docker compose -f $compose_file down"
else
  # The gateway is a *host* process on the default install (ADR-0055
  # decision 8), so compose does not own it and stopping the containers
  # would leave it running against a database that went away.
  pidfile="$HOME_DIR/data/gateway.pid"
  if [ -f "$pidfile" ]; then
    pid=$(cat "$pidfile" 2>/dev/null || true)
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      if [ "$dry_run" = yes ]; then
        say "    would stop the gateway (pid $pid)"
      else
        kill "$pid" 2>/dev/null || true
        say "    stopped the gateway (pid $pid)"
      fi
    fi
  fi
  if [ "$purge" = yes ]; then
    if [ "$dry_run" = yes ]; then
      say "    would run: docker compose down -v   (DESTROYS the volumes)"
    else
      docker compose -f "$compose_file" down -v >/dev/null 2>&1 ||
        say "    docker compose down -v reported a problem; continuing"
      say "    containers and volumes removed"
    fi
  else
    if [ "$dry_run" = yes ]; then
      say "    would run: docker compose down   (volumes kept)"
    else
      docker compose -f "$compose_file" down >/dev/null 2>&1 ||
        say "    docker compose down reported a problem; continuing"
      say "    containers stopped; volumes kept"
    fi
  fi
fi

# ── 2. what the installer wrote ─────────────────────────────────────────
step "what the installer wrote"
# `data/` carries kms.key, so it is called out before it goes rather than
# after (ADR-0067 decision 2). Records are not sealed under it — ADR-0064
# decision 7 — so kept data stays readable; console sessions, tenant
# secrets and any `synveda tenant export` archive do not.
if [ -f "$HOME_DIR/data/kms.key" ] && [ "$purge" = no ]; then
  say ""
  say "    NOTE: $HOME_DIR/data/kms.key is about to be removed, and you are"
  say "    keeping your data. Records stay readable — they are not sealed —"
  say "    but console sessions, tenant secrets and any \`tenant export\`"
  say "    archive can never be opened again without this file."
  say ""
  say "    Copy it first if any of that matters:"
  say "        cp $HOME_DIR/data/kms.key ~/synveda-kms.key.backup"
  say ""
fi
remove_path "$HOME_DIR/bin" "gateway binary"
remove_path "$HOME_DIR/console" "admin console"
remove_path "$HOME_DIR/profile" "compose profile and demo seeder"
remove_path "$HOME_DIR/plugin" "Claude Code plugin marketplace"
remove_path "$HOME_DIR/data" "pidfile, log, and kms.key"
# Only if it is now empty: somebody may keep their own things in here, and
# `rmdir` refusing a non-empty directory is the check rather than a test we
# would have to write.
[ "$dry_run" = yes ] || rmdir "$HOME_DIR" 2>/dev/null || true

# ── 3. the CLI ──────────────────────────────────────────────────────────
step "the CLI"
remove_path "$BIN_DIR/synveda" "installed CLI"
# The sudo fallback puts it under the install root instead, which step 2
# already removed with `bin/` — this only reports it, so the summary is
# honest about where it went.
still=$(command -v synveda 2>/dev/null || true)
if [ -n "$still" ]; then
  say "    still on PATH: $still"
  say "    (not ours to remove — the installer did not put it there)"
fi

# ── 4. somebody else's config ───────────────────────────────────────────
# Reported, never removed. install.sh wrote none of this and said so; an
# uninstaller that reached into an editor's settings without being asked
# would be breaking the same promise from the other end (ADR-0067
# decision 3).
step "AI clients you connected yourself"
found_clients=no
for candidate in \
  "$HOME/Library/Application Support/Claude/claude_desktop_config.json" \
  "$HOME/.cursor/mcp.json" \
  "$HOME/.config/zed/settings.json" \
  "$HOME/Library/Application Support/Code/User/mcp.json" \
  "$HOME/.config/Code/User/mcp.json" \
  "$HOME/.codeium/windsurf/mcp_config.json" \
  "$HOME/.continue/config.json"; do
  [ -f "$candidate" ] || continue
  grep -q '"synveda"' "$candidate" 2>/dev/null || continue
  found_clients=yes
  say "    $candidate"
done
if [ "$found_clients" = yes ]; then
  say ""
  say "    These are yours, so this script does not edit them. Remove our"
  say "    entry — and only ours — with:"
  say "        synveda mcp uninstall --client <name>"
else
  say "    none found"
fi
if [ -d "$HOME/.claude" ]; then
  say "    Claude Code: \`synveda plugin uninstall\` (before removing the CLI)"
fi
if [ -f "$HOME/.config/synveda/credentials.json" ]; then
  say "    stored logins: $HOME/.config/synveda/credentials.json"
  remove_path "$HOME/.config/synveda/credentials.json" "stored logins"
fi

# ── the summary ─────────────────────────────────────────────────────────
say ""
if [ "$dry_run" = yes ]; then
  say "--dry-run: $removed path(s) would be removed. Nothing was changed."
  exit 0
fi
if [ "$removed" -eq 0 ]; then
  # Idempotent, and says so rather than failing: the moment somebody runs
  # this twice is the moment something went wrong the first time.
  say "Nothing of ours was installed here. Nothing to do."
  exit 0
fi
say "Removed $removed path(s)."
if [ "$purge" = yes ]; then
  say ""
  say "The volumes are gone, and with them every tenant's memory in this"
  say "deployment. There was no smaller unit to offer: a tenant row cannot be"
  say "deleted yet (TEN-5), so the volume is this product's only erasure."
  say "This was not a GDPR erasure — it removed a deployment, not a subject."
else
  say ""
  say "Your data is still here, in four Docker volumes:"
  say "    pg-data  rauthy-data  tei-cache  gateway-search"
  say "Reinstalling and running \`synveda init\` picks them up again — unless"
  say "the release you reinstall serves a different schema epoch, which it"
  say "will tell you at startup (CPR-2). Starting over on the same install is"
  say "\`synveda reset --database --force\`, which destroys the database and"
  say "leaves everything else; this script is for removing the product."
  say "To destroy the volumes — which is the only way to remove a tenant's"
  say "memory:"
  say "    ./uninstall.sh --purge"
  say "or, if you have already removed the profile:"
  say "    docker volume rm pg-data rauthy-data tei-cache gateway-search"
fi
