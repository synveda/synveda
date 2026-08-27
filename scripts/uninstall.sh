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
#   $SYNVEDA_HOME/data/           runtime state; keeps kms.key by default
#   $SYNVEDA_BIN/synveda          the CLI, if the installer put it there
#
# **Your data and its key survive by default.** `init` created three named
# Docker volumes and a local key-encryption key. This stops the containers
# without touching the volumes and retains `data/kms.key`, because keeping
# encrypted rows while destroying their only key is data loss. `--purge`
# explicitly removes both and says so.
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
#   ./uninstall.sh --purge      destroy the volumes and data/kms.key
#   ./uninstall.sh --dry-run    list everything it would touch, change nothing
set -eu

HOME_DIR="${SYNVEDA_HOME:-$HOME/.synveda}"
BIN_DIR="${SYNVEDA_BIN:-/usr/local/bin}"

purge=no
dry_run=no
purge_status=not-requested
for arg in "$@"; do
  case "$arg" in
    --purge) purge=yes ;;
    --dry-run) dry_run=yes ;;
    -h|--help) sed -n '2,35p' "$0" | sed 's/^#\{1,\} \{0,1\}//'; exit 0 ;;
    *) printf 'uninstall: unknown option %s (try --help)\n' "$arg" >&2; exit 2 ;;
  esac
done

say()  { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }

# Everything routes through these two, so `--dry-run` cannot drift from what
# a real run does: there is one list of actions and a flag that stops short
# of performing them.
removed=0
failures=0
key_preserved=no
remove_path() { # path label
  [ -e "$1" ] || [ -L "$1" ] || return 0
  if [ "$dry_run" = yes ]; then
    removed=$((removed + 1))
    say "    would remove  $1${2:+  ($2)}"
    return 0
  fi
  if rm -rf "$1" 2>/dev/null; then
    removed=$((removed + 1))
    say "    removed  $1${2:+  ($2)}"
    return 0
  fi
  # The CLI's default home is root-owned on macOS, so the same sudo dance
  # install.sh does — and the same rule: a refusal must not kill the script
  # and leave a half-removed install (ADR-0065 amendment 6).
  if command -v sudo >/dev/null 2>&1 && sudo rm -rf "$1" 2>/dev/null; then
    removed=$((removed + 1))
    say "    removed  $1  (via sudo)"
    return 0
  fi
  failures=$((failures + 1))
  say "    COULD NOT REMOVE  $1 — remove it by hand:"
  say "        sudo rm -rf $1"
}

# Remove the runtime state while retaining the one file required to open the
# persistent database again. Iterating the directory rather than moving the
# key out and back means an interruption cannot strand or lose the key.
remove_runtime_state_preserving_key() {
  data_dir="$HOME_DIR/data"
  key_path="$data_dir/kms.key"
  [ -e "$data_dir" ] || [ -L "$data_dir" ] || return 0
  if [ -L "$data_dir" ]; then
    failures=$((failures + 1))
    say "    COULD NOT CLEAN  $data_dir — refusing to traverse a symlink"
    say "    the linked directory and any key inside it were left untouched"
    return 0
  fi
  if [ ! -d "$data_dir" ]; then
    remove_path "$data_dir" "unexpected runtime-state path"
    return 0
  fi

  if [ -e "$key_path" ] || [ -L "$key_path" ]; then
    key_preserved=yes
    if [ "$dry_run" = yes ]; then
      say "    would preserve $key_path  (key for the retained volumes)"
    else
      say "    preserved  $key_path  (key for the retained volumes)"
    fi
  fi

  # The three patterns cover ordinary and hidden entries in POSIX sh. An
  # unmatched pattern is skipped by the existence check in remove_path.
  for state_path in "$data_dir"/* "$data_dir"/.[!.]* "$data_dir"/..?*; do
    [ "$state_path" = "$key_path" ] && continue
    remove_path "$state_path" "runtime state"
  done
  [ "$dry_run" = yes ] || rmdir "$data_dir" 2>/dev/null || true
}

say "Synveda uninstall"
say ""
[ "$dry_run" = yes ] && { say "  --dry-run: nothing will be changed"; say ""; }

# ── 1. the deployment ───────────────────────────────────────────────────
compose_file="$HOME_DIR/profile/docker-compose.yml"
step "the deployment"
if [ ! -f "$compose_file" ]; then
  if [ "$purge" = yes ]; then
    purge_status=failed
    say "    no profile at $compose_file — cannot verify volume removal"
    say "    the retained-volume key will not be removed"
  else
    say "    no profile at $compose_file — nothing to stop"
  fi
elif ! command -v docker >/dev/null 2>&1; then
  [ "$purge" = yes ] && purge_status=failed
  say "    docker is not on PATH; skipping. If a deployment is running:"
  if [ "$purge" = yes ]; then
    say "        docker compose -f $compose_file down -v"
    say "    the retained-volume key will not be removed"
  else
    say "        docker compose -f $compose_file down"
  fi
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
      purge_status=planned
      say "    would run: docker compose down -v   (DESTROYS the volumes)"
    elif docker compose -f "$compose_file" down -v >/dev/null 2>&1; then
      purge_status=completed
      say "    containers and volumes removed"
    else
      purge_status=failed
      say "    docker compose down -v failed; volumes may remain"
      say "    the retained-volume key will not be removed"
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
remove_path "$HOME_DIR/bin" "gateway binary"
remove_path "$HOME_DIR/console" "admin console"
remove_path "$HOME_DIR/profile" "compose profile and demo seeder"
remove_path "$HOME_DIR/plugin" "Claude Code plugin marketplace"
if [ "$purge_status" = completed ] || [ "$purge_status" = planned ]; then
  remove_path "$HOME_DIR/data" "runtime state and kms.key; explicit purge"
else
  remove_runtime_state_preserving_key
fi
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
if [ "$purge" = yes ] && [ "$purge_status" != completed ]; then
  say "Purge did not complete. The volumes may survive, so their key remains at:"
  say "    $HOME_DIR/data/kms.key"
  say "Remove the volumes successfully before explicitly deleting that key."
  exit 1
fi
if [ "$failures" -ne 0 ]; then
  say "Uninstall was incomplete: $failures path(s) could not be removed safely."
  exit 1
fi
if [ "$removed" -eq 0 ]; then
  # Idempotent, and says so rather than failing: the moment somebody runs
  # this twice is the moment something went wrong the first time.
  if [ "$key_preserved" = yes ]; then
    say "Nothing else was installed here. The retained-volume key remains at:"
    say "    $HOME_DIR/data/kms.key"
  else
    say "Nothing of ours was installed here. Nothing to do."
  fi
  exit 0
fi
say "Removed $removed path(s)."
if [ "$purge" = yes ]; then
  say ""
  say "The volumes and $HOME_DIR/data/kms.key are gone, and with them every"
  say "tenant's memory in this deployment. There was no smaller unit to offer:"
  say "a tenant row cannot be deleted yet (TEN-5), so the volume is this"
  say "product's only deployment-level erasure."
  say "This was not a GDPR erasure — it removed a deployment, not a subject."
else
  say ""
  say "Your data is still here, in three Docker volumes:"
  say "    pg-data  rauthy-data  tei-cache"
  say "Its key is still here:"
  say "    $HOME_DIR/data/kms.key"
  say "Reinstalling and running \`synveda init\` reuses both — unless"
  say "the release you reinstall serves a different schema epoch, which it"
  say "will tell you at startup (CPR-2). Starting over on the same install is"
  say "\`synveda reset --database --force\`, which destroys the database and"
  say "leaves everything else; this script is for removing the product."
  say "To destroy them now, delete the three Compose volumes first and only"
  say "then delete the key:"
  say "    docker volume rm synveda_pg-data synveda_rauthy-data synveda_tei-cache && \\"
  say "      rm $HOME_DIR/data/kms.key"
fi
