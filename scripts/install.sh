#!/bin/sh
# Synveda installer (OPS-8, ADR-0065).
#
#   curl -fsSL https://synveda.dev/install.sh | sh
#
# Downloads one release's binaries, console bundle and profile bundle, and
# leaves the machine able to run `synveda init`. Docker is the only other
# thing it needs; there is no Rust toolchain and no source tree involved.
#
# POSIX sh on purpose — this is the one file that runs before anything of
# ours does, on a machine we know nothing about, so it uses no bashism and
# nothing outside coreutils, tar, and curl or wget.
#
# What it writes, and nothing else:
#
#   $SYNVEDA_BIN/synveda          the CLI            (default /usr/local/bin)
#   $SYNVEDA_HOME/bin/synveda-gateway                (default ~/.synveda)
#   $SYNVEDA_HOME/bin/synveda-worker                 (default ~/.synveda)
#   $SYNVEDA_HOME/console/        the admin console bundle
#   $SYNVEDA_HOME/profile/        compose file, Rauthy config, version
#   $SYNVEDA_HOME/plugin/         the Claude Code plugin, as a marketplace
#
# It touches **nothing** belonging to an editor or an AI client. Hooking one
# up is a separate, explicit step — `synveda plugin install` for Claude Code,
# `synveda mcp install --client …` for everything else — because an
# installer that silently reconfigures somebody's tools is not one to run
# from a pipe.
#
# Environment:
#   SYNVEDA_VERSION   release tag to install (default: the latest release)
#   SYNVEDA_HOME      install root                    (default ~/.synveda)
#   SYNVEDA_BIN       where the CLI goes              (default /usr/local/bin)
#   SYNVEDA_BASE_URL  where to fetch assets from instead of a GitHub release.
#                     A mirror, or a `file:///path/to/assets` directory —
#                     which is how demos/ops-8-release-install.sh runs this
#                     script itself rather than a copy of what it does.
#                     Requires SYNVEDA_VERSION, since there is no release to
#                     ask which one is latest.
set -eu

REPO="${SYNVEDA_REPO:-synveda/synveda}"
HOME_DIR="${SYNVEDA_HOME:-$HOME/.synveda}"
BIN_DIR="${SYNVEDA_BIN:-/usr/local/bin}"

say()  { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }
die()  { printf 'install: %s\n' "$*" >&2; exit 1; }

# ── The platform ─────────────────────────────────────────────────────────
#
# Refused by name rather than guessed at (ADR-0065 decision 7). An installer
# that downloads x86_64 for an Intel Mac it did not recognise, or a glibc
# build for Alpine, fails later, further from the cause, and inside the
# product rather than in here.
os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)        target="darwin-arm64" ;;
  Linux/x86_64|Linux/amd64) target="linux-x86_64" ;;
  *)
    die "no release build for $os/$arch.

  This release ships macOS arm64 (Apple Silicon) and Linux x86_64.
  Everything still works from source on any platform Rust and Docker do:

    git clone https://github.com/$REPO
    cd synveda && cargo build --release -p synveda-cli -p synveda-gateway --bins
    ./target/release/synveda init

  If this platform matters to you, say so — adding one is a build matrix row."
    ;;
esac

# musl is a different libc, not a different architecture, and these binaries
# are built against glibc. Catching it here is the difference between a
# refusal that names the cause and a "not found" from the dynamic loader.
if [ "$os" = "Linux" ] && [ ! -e /lib/x86_64-linux-gnu/libc.so.6 ] \
   && [ ! -e /lib64/ld-linux-x86-64.so.2 ]; then
  die "this looks like a musl system (Alpine?), and the Linux build is glibc.
  Build from source, or run the gateway image and point a CLI at it."
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
  fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
  fetch_stdout() { wget -qO- "$1"; }
else
  die "neither curl nor wget is on PATH"
fi

command -v tar >/dev/null 2>&1 || die "tar is not on PATH"

# ── The release ──────────────────────────────────────────────────────────
version="${SYNVEDA_VERSION:-}"
if [ -z "$version" ]; then
  [ -z "${SYNVEDA_BASE_URL:-}" ] || die "SYNVEDA_BASE_URL needs SYNVEDA_VERSION too —
  a directory of assets cannot be asked which release is the latest."
  step "finding the latest release"
  version="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$version" ] || die "could not determine the latest release of $REPO.
  Pick one explicitly:  SYNVEDA_VERSION=v0.2.0 sh install.sh"
fi
# Assets are named by the version without its leading `v`, matching the
# crate version `synveda init` compares a profile against.
plain="${version#v}"
base="${SYNVEDA_BASE_URL:-https://github.com/$REPO/releases/download/$version}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

step "downloading synveda $version ($target)"
archive="synveda-$plain-$target.tar.gz"
profile="synveda-profile-$plain.tar.gz"
console="synveda-console-$plain.tar.gz"
plugin="synveda-plugin-$plain.tar.gz"
fetch "$base/$archive" "$work/$archive" || die "no asset $archive in release $version"
fetch "$base/$profile" "$work/$profile" || die "no asset $profile in release $version"
fetch "$base/$console" "$work/$console" || die "no asset $console in release $version"
fetch "$base/$plugin"  "$work/$plugin"  || die "no asset $plugin in release $version"

# ── Checksums ────────────────────────────────────────────────────────────
#
# These binaries are unsigned (decision 8), so this proves the download
# arrived intact and does *not* prove who built it. Said plainly at the end
# rather than implied by a checkmark here.
step "verifying checksums"
if fetch "$base/SHA256SUMS" "$work/SHA256SUMS" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    checksum() { sha256sum "$1" | cut -d' ' -f1; }
  elif command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
  else
    checksum() { echo ""; }
  fi
  for asset in "$archive" "$profile" "$console" "$plugin"; do
    want="$(grep " $asset\$" "$work/SHA256SUMS" | cut -d' ' -f1 | head -n 1)"
    got="$(checksum "$work/$asset")"
    if [ -z "$got" ]; then
      say "    no sha256 tool on PATH — checksums not verified"
      break
    fi
    [ -n "$want" ] || die "$asset is not listed in SHA256SUMS"
    [ "$want" = "$got" ] || die "$asset failed its checksum.
  expected $want
  got      $got"
  done
else
  say "    release has no SHA256SUMS — skipping"
fi

# ── Install ──────────────────────────────────────────────────────────────
step "installing to $HOME_DIR"
mkdir -p "$HOME_DIR/bin"
tar -xzf "$work/$archive"  -C "$work"
tar -xzf "$work/$console"  -C "$work"
tar -xzf "$work/$profile"  -C "$work"
tar -xzf "$work/$plugin"   -C "$work"

[ -f "$work/synveda" ]          || die "$archive did not contain a synveda binary"
[ -f "$work/synveda-gateway" ]  || die "$archive did not contain a synveda-gateway binary"
[ -f "$work/synveda-worker" ]   || die "$archive did not contain a synveda-worker binary"

install_file() { # src dst — install(1) is not on every minimal image
  cp "$1" "$2.tmp" && chmod 755 "$2.tmp" && mv "$2.tmp" "$2"
}

sudo_install_file() { # src dst — install_file's rename dance, as root
  # Not `sudo cp` onto the target. Writing over a Mach-O in place leaves a
  # binary whose signature no longer matches its contents, and macOS kills
  # the next run with SIGKILL — "Killed: 9", no explanation. Copy beside it
  # and rename, which is atomic and leaves no window where the file on disk
  # is half a binary. This is an upgrade's failure, not a first install's,
  # so it is the path least likely to be noticed before a user hits it.
  sudo cp "$1" "$2.tmp" || return 1
  # Past the first sudo, the timestamp is cached: cleanup will not re-prompt.
  if sudo chmod 755 "$2.tmp" && sudo mv -f "$2.tmp" "$2"; then
    return 0
  fi
  sudo rm -f "$2.tmp" || true
  return 1
}

install_file "$work/synveda-gateway" "$HOME_DIR/bin/synveda-gateway"
install_file "$work/synveda-worker" "$HOME_DIR/bin/synveda-worker"
rm -rf "$HOME_DIR/console"
cp -R "$work/console" "$HOME_DIR/console"

# The Claude Code plugin, as the marketplace `synveda plugin install` points
# Claude Code at. Unpacked here and installed *nowhere* — this script does
# not touch `~/.claude`.
rm -rf "$HOME_DIR/plugin"
cp -R "$work/plugin" "$HOME_DIR/plugin"

# The profile directory is replaced wholesale, so that a file dropped from
# one release does not linger into the next — with one exception. `.env` is
# the *deployment's* configuration (the issuer, the tenant, the embedder),
# written into this directory by `synveda init` because that is where
# compose reads it from. It is state that happens to live among release
# content, and deleting it would leave a gateway that starts with no issuer
# and refuses every request. Re-running `init` would rewrite it, but an
# installer that silently unconfigures a working deployment is not a thing
# to make somebody discover.
if [ -f "$HOME_DIR/profile/.env" ]; then
  cp "$HOME_DIR/profile/.env" "$work/carried.env"
  say "    carrying over the existing deployment configuration (.env)"
fi
rm -rf "$HOME_DIR/profile"
cp -R "$work/synveda-profile-$plain" "$HOME_DIR/profile"
# An `if`, not `[ … ] && cp`: under `set -eu` a two-command AND list whose
# test is false fails the whole statement and takes the script with it.
if [ -f "$work/carried.env" ]; then
  cp "$work/carried.env" "$HOME_DIR/profile/.env"
fi

step "installing the CLI to $BIN_DIR"
cli_installed=""
if [ -w "$BIN_DIR" ]; then
  install_file "$work/synveda" "$BIN_DIR/synveda"
  cli_installed=yes
elif command -v sudo >/dev/null 2>&1; then
  say "    $BIN_DIR is not writable — asking sudo"
  if sudo_install_file "$work/synveda" "$BIN_DIR/synveda"; then
    cli_installed=yes
  fi
fi

if [ -z "$cli_installed" ]; then
  # sudo being *absent* is not the only way this arrives here, and treating
  # it as such is what this branch used to get wrong: it tested `command -v
  # sudo` and let a sudo that ran and *refused* kill the script under
  # `set -e`. Refusal is the ordinary case, not the exotic one — a managed
  # machine where the user is not an admin, a pipe with no terminal to
  # prompt on (CI, a Dockerfile, `ssh host 'curl … | sh'`), or somebody who
  # simply declines. By then the gateway, console, profile and plugin are
  # all installed, so dying here left a complete install with no CLI on
  # PATH and a raw sudo error as the explanation.
  #
  # The CLI goes where this user can certainly write instead. An install
  # that needs no privileges should not require them.
  install_file "$work/synveda" "$HOME_DIR/bin/synveda"
  say "    could not write $BIN_DIR — installed to $HOME_DIR/bin instead"
  BIN_DIR="$HOME_DIR/bin"
fi

# macOS quarantines anything a browser downloaded and refuses to run it.
# `curl | sh` sets no such attribute, but somebody who fetched an asset by
# hand will have one, so strip it from what we wrote either way.
if [ "$os" = "Darwin" ] && command -v xattr >/dev/null 2>&1; then
  xattr -d com.apple.quarantine "$BIN_DIR/synveda" 2>/dev/null || true
  xattr -d com.apple.quarantine "$HOME_DIR/bin/synveda-gateway" 2>/dev/null || true
  xattr -d com.apple.quarantine "$HOME_DIR/bin/synveda-worker" 2>/dev/null || true
fi

# ── What it got ──────────────────────────────────────────────────────────
say ""
say "synveda $version installed."
say ""
say "  CLI       $BIN_DIR/synveda"
say "  gateway   $HOME_DIR/bin/synveda-gateway"
say "  worker    $HOME_DIR/bin/synveda-worker"
say "  profile   $HOME_DIR/profile"
say "  console   $HOME_DIR/console"
say "  plugin    $HOME_DIR/plugin        (not installed into any client)"
say ""
if [ "$os" = "Darwin" ]; then
  say "  These binaries are unsigned and not notarized — macOS will say so if you"
  say "  run one it did not see this script write."
  say ""
fi
# Every line below is a `synveda …` command, so all of them are wrong unless
# the directory the CLI landed in is on PATH. That is routine after the
# fallback above, and true of ~/.local/bin on plenty of machines besides —
# so say it here rather than let somebody meet "command not found" on the
# very first thing this script told them to run.
case ":${PATH}:" in
  *":$BIN_DIR:"*) ;;
  *)
    say "  $BIN_DIR is not on your PATH. Add it to your shell profile:"
    say ""
    say "    export PATH=\"$BIN_DIR:\$PATH\""
    say ""
    ;;
esac
say "Docker has to be running. Then:"
say ""
say "  synveda init                       # one runtime, schema and tenant"
say "  synveda login                      # identity, principal scope, first grant"
say ""
say "  http://127.0.0.1:8120/console/     # the admin console"
say "  Advanced > Configuration           # bind personal, team or enterprise data"
say ""
say "To give an AI client your team's governed memory:"
say ""
say "  synveda plugin install             # Claude Code: hooks + MCP, one command"
say "  synveda mcp install --client claude-desktop   # or cursor, or zed"
say ""
say "Docs: https://github.com/$REPO/blob/main/docs/INSTALL.md"
