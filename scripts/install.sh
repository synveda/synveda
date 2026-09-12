#!/bin/sh
# Synveda installer (OPS-8, ADR-0065).
#
# Invoke only the tag-bound copy named by a published release after inspecting
# it. The mutable source-branch script is not a public installation command.
#
# Downloads one release's binaries, console bundle and digest-bound Docker
# reference bundle. It installs artifacts only and never starts the deployment.
# There is no Rust toolchain and no source tree involved.
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
#   $SYNVEDA_HOME/reference/      immutable releases and validated current link
#   $SYNVEDA_HOME/state/          deployment secrets and authority state
#   $SYNVEDA_HOME/backups/        database and recovery-secret archives
#   $SYNVEDA_HOME/plugin/         the Claude marketplace and Codex hook runtime
#
# It touches **nothing** belonging to an editor or an AI client. Hooking one
# up is a separate, explicit step — `synveda plugin install` for Claude Code,
# `synveda mcp install --client …` for everything else — because an
# installer that silently reconfigures somebody's tools is not one to run
# from a pipe.
#
# Environment:
#   SYNVEDA_VERSION   compatible release tag. Omitting it queries the
#                     repository's latest release; do so only when that
#                     release's instructions declare this installer compatible.
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
bin_dir_explicit=no

say()  { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }
die()  { printf 'install: %s\n' "$*" >&2; exit 1; }

if [ -n "${SYNVEDA_BIN:-}" ]; then
  bin_dir_explicit=yes
  case "$BIN_DIR" in
    /*) ;;
    *) die "SYNVEDA_BIN must be an absolute directory path" ;;
  esac
  case "$BIN_DIR" in
    *//*|*/./*|*/../*|*/.|*/..|*/) die "SYNVEDA_BIN must be a normalized directory path" ;;
  esac
fi
case "$HOME_DIR" in
  /*) ;;
  *) die "SYNVEDA_HOME must be an absolute directory path" ;;
esac
case "$HOME_DIR" in
  *//*|*/./*|*/../*|*/.|*/..|*/) die "SYNVEDA_HOME must be a normalized directory path" ;;
esac
if [ "$bin_dir_explicit" = yes ] && [ "$BIN_DIR" != "$HOME_DIR/bin" ]; then
  case "$BIN_DIR" in
    "$HOME_DIR"|"$HOME_DIR"/*)
      die "SYNVEDA_BIN must stay outside SYNVEDA_HOME or equal SYNVEDA_HOME/bin"
      ;;
  esac
  case "$HOME_DIR" in
    "$BIN_DIR"|"$BIN_DIR"/*)
      die "SYNVEDA_BIN must not contain SYNVEDA_HOME"
      ;;
  esac
fi

# The withdrawn Rauthy/profile installation is not a compatible input to the
# Keycloak reference deployment. Refuse it before downloads or mutation rather
# than carrying its configuration or silently orphaning its containers.
if [ -e "$HOME_DIR/profile" ] || [ -L "$HOME_DIR/profile" ]; then
  die "legacy $HOME_DIR/profile exists.
  Stop the retired deployment, move or remove that directory explicitly, then
  rerun this installer. It does not migrate Rauthy-era state."
fi

release_version_is_valid() {
  candidate=$1
  [ -n "$candidate" ] && [ "${#candidate}" -le 63 ] || return 1
  case "$candidate" in
    *[!0-9A-Za-z.-]* | *[!0-9A-Za-z]) return 1 ;;
  esac
  # Nineteen core digits are always inside Helm's unsigned parser range.
  LC_ALL=C printf '%s\n' "$candidate" | LC_ALL=C grep -Eq \
    '^(0|[1-9][0-9]{0,18})\.(0|[1-9][0-9]{0,18})\.(0|[1-9][0-9]{0,18})(-(0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?$'
}

select_release_version() {
  requested=$1
  case "$requested" in
    v*) selected_plain=${requested#v} ;;
    *) selected_plain=$requested ;;
  esac
  release_version_is_valid "$selected_plain" || die \
    "invalid release version; expected a label-safe vMAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH with an optional SemVer prerelease"
  plain=$selected_plain
  version="v$plain"
}

# An explicitly supplied version is untrusted input. Validate and normalize it
# before platform probes, downloads, temporary paths or filesystem mutation.
version="${SYNVEDA_VERSION:-}"
plain=""
if [ -n "$version" ]; then
  select_release_version "$version"
fi

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
  A source build may be possible on another platform, but the supported product
  topology is not a pair of ad hoc binaries. Use a reviewed checkout and follow
  deploy/compose/README.md, the canonical source-checkout guide. Its documented
  Docker prerequisites and still-pending live platform evidence apply.

  If this platform matters to you, say so — adding one is a build matrix row."
    ;;
esac

# musl is a different libc, not a different architecture, and these binaries
# are built against glibc. Catching it here is the difference between a
# refusal that names the cause and a "not found" from the dynamic loader.
if [ "$os" = "Linux" ] && [ ! -e /lib/x86_64-linux-gnu/libc.so.6 ] \
   && [ ! -e /lib64/ld-linux-x86-64.so.2 ]; then
  die "this looks like a musl system (Alpine?), and the Linux build is glibc.
  Build from source, or run the product image and point a CLI at its gateway."
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
if [ -z "$version" ]; then
  [ -z "${SYNVEDA_BASE_URL:-}" ] || die "SYNVEDA_BASE_URL needs SYNVEDA_VERSION too —
  a directory of assets cannot be asked which release is the latest."
  step "finding the latest release"
  version="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$version" ] || die "could not determine the latest release of $REPO.
  Pick a compatible published tag explicitly:
  SYNVEDA_VERSION=vMAJOR.MINOR.PATCH sh install.sh"
  select_release_version "$version"
fi
# Assets are named by the normalized plain version; GitHub tags retain one
# canonical leading `v`.
base="${SYNVEDA_BASE_URL:-https://github.com/$REPO/releases/download/$version}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

step "downloading synveda $version ($target)"
archive="synveda-$plain-$target.tar.gz"
reference="synveda-reference-$plain.tar.gz"
console="synveda-console-$plain.tar.gz"
plugin="synveda-plugin-$plain.tar.gz"
fetch "$base/$archive" "$work/$archive" || die "no asset $archive in release $version"
fetch "$base/$reference" "$work/$reference" || die "no asset $reference in release $version"
fetch "$base/$console" "$work/$console" || die "no asset $console in release $version"
fetch "$base/$plugin"  "$work/$plugin"  || die "no asset $plugin in release $version"

# ── Checksums ────────────────────────────────────────────────────────────
#
# These binaries are unsigned (decision 8), so this proves the download
# arrived intact and does *not* prove who built it. Said plainly at the end
# rather than implied by a checkmark here.
step "verifying checksums"
fetch "$base/SHA256SUMS" "$work/SHA256SUMS" 2>/dev/null ||
  die "release $version has no readable SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  checksum() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  checksum() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "sha256sum or shasum is required to verify release assets"
fi
for asset in "$archive" "$reference" "$console" "$plugin"; do
  want="$(awk -v asset="$asset" '
    $2 == asset { count += 1; digest = $1 }
    END { if (count != 1) exit 1; print digest }
  ' "$work/SHA256SUMS")" || die "$asset must appear exactly once in SHA256SUMS"
  printf '%s\n' "$want" | grep -Eq '^[0-9a-f]{64}$' ||
    die "$asset has an invalid SHA256SUMS digest"
  got="$(checksum "$work/$asset")"
  [ "$want" = "$got" ] || die "$asset failed its checksum.
  expected $want
  got      $got"
done

# ── Install ──────────────────────────────────────────────────────────────
step "installing to $HOME_DIR"
tar -xzf "$work/$archive"  -C "$work"
tar -xzf "$work/$console"  -C "$work"
tar -xzf "$work/$reference" -C "$work"
tar -xzf "$work/$plugin"   -C "$work"

require_plain_file() {
  file_path=$1
  file_label=$2
  [ -f "$file_path" ] && [ ! -L "$file_path" ] ||
    die "$file_label was missing or not a regular file"
}
require_plain_directory() {
  directory_path=$1
  directory_label=$2
  [ -d "$directory_path" ] && [ ! -L "$directory_path" ] ||
    die "$directory_label was missing or not a real directory"
}
require_plain_file "$work/synveda" "$archive synveda binary"
require_plain_file "$work/synveda-gateway" "$archive synveda-gateway binary"
require_plain_file "$work/synveda-worker" "$archive synveda-worker binary"
require_plain_directory "$work/console" "$console console root"
require_plain_directory "$work/plugin" "$plugin plugin root"
for codex_asset in package.json dist/hook.mjs dist/transcript.mjs \
  node_modules/@synveda/claude-code-adapter/package.json \
  node_modules/@synveda/claude-code-adapter/dist/session-runtime.mjs; do
  require_plain_file "$work/plugin/codex/$codex_asset" "$plugin Codex $codex_asset"
done
reference_bundle="$work/synveda-reference-$plain"
require_plain_directory "$reference_bundle" "$reference canonical root"
require_plain_file "$reference_bundle/synveda-compose" "$reference launcher"
[ -x "$reference_bundle/synveda-compose" ] ||
  die "$reference did not contain an executable synveda-compose launcher"
require_plain_file "$reference_bundle/environment.json" "$reference environment manifest"
require_plain_file "$reference_bundle/version" "$reference version"
require_plain_file "$reference_bundle/source-sha" "$reference source identity"
link_report=$(mktemp "$work/.archive-links.XXXXXX") ||
  die "release archive inspection could not be staged"
if ! find "$work/console" "$work/plugin" "$reference_bundle" \
  -type l -print -quit > "$link_report"; then
  die "release archives could not be inspected safely"
fi
if [ -s "$link_report" ]; then
  die "release archives must not contain symbolic links"
fi
rm -f "$link_report"
IFS= read -r packaged_version < "$reference_bundle/version" || packaged_version=
[ "$packaged_version" = "$plain" ] || die "$reference version does not match $version"
IFS= read -r source_sha < "$reference_bundle/source-sha" || source_sha=
printf '%s\n' "$source_sha" | grep -Eq '^[0-9a-f]{40}$' ||
  die "$reference source identity is invalid"
grep -Fq '"schema_version": 1' "$reference_bundle/environment.json" &&
  grep -Fq "\"release_version\": \"$plain\"" "$reference_bundle/environment.json" &&
  grep -Fq "\"source_sha\": \"$source_sha\"" "$reference_bundle/environment.json" ||
  die "$reference environment manifest does not match its release identity"

# Resolve and prove every recursively replaced destination before the first
# install mutation. A pre-existing unrelated directory is not ours to adopt.
require_plain_directory_or_absent() {
  directory_path=$1
  directory_label=$2
  if [ -e "$directory_path" ] || [ -L "$directory_path" ]; then
    [ -d "$directory_path" ] && [ ! -L "$directory_path" ] ||
      die "$directory_label is not a real directory: $directory_path"
  fi
}
reference_root="$HOME_DIR/reference"
releases_root="$reference_root/releases"
release_id="$plain-$source_sha"
release_target="$releases_root/$release_id"
release_stage="$releases_root/.install-$release_id-$$"
require_plain_directory_or_absent "$reference_root" "reference install root"
require_plain_directory_or_absent "$releases_root" "reference releases root"
if [ -e "$reference_root/current" ] && [ ! -L "$reference_root/current" ]; then
  die "$reference_root/current is not an installer-owned symlink"
fi
if [ -L "$reference_root/current" ]; then
  current_link=$(readlink "$reference_root/current") ||
    die "existing reference link could not be read"
  case "$current_link" in
    releases/*)
      current_name=${current_link#releases/}
      case "$current_name" in ''|*/*|*[!0-9A-Za-z.-]*) die "existing reference link was refused" ;; esac
      grep -Eq '^reference:[0-9A-Za-z.-]+:[0-9a-f]{40}$' \
        "$reference_root/$current_link/.synveda-installer-owned" 2>/dev/null ||
        die "existing reference link is not installer-owned"
      ;;
    *) die "existing reference link was refused" ;;
  esac
fi
if [ -e "$release_target" ] || [ -L "$release_target" ]; then
  [ -d "$release_target" ] && [ ! -L "$release_target" ] &&
    grep -Fxq "reference:$plain:$source_sha" \
      "$release_target/.synveda-installer-owned" 2>/dev/null ||
    die "existing release path is not owned by this installer: $release_target"
fi
for managed_name in console plugin; do
  managed_path="$HOME_DIR/$managed_name"
  if [ -e "$managed_path" ] || [ -L "$managed_path" ]; then
    [ -d "$managed_path" ] && [ ! -L "$managed_path" ] &&
      grep -Fxq "$managed_name" "$managed_path/.synveda-installer-owned" 2>/dev/null ||
      die "existing $managed_path is not owned by this installer"
  fi
done
require_plain_directory_or_absent "$HOME_DIR/bin" "binary install path"
if [ "$bin_dir_explicit" = yes ]; then
  require_plain_directory_or_absent "$BIN_DIR" "SYNVEDA_BIN"
fi
require_plain_file_or_absent() {
  file_path=$1
  file_label=$2
  if [ -e "$file_path" ] || [ -L "$file_path" ]; then
    [ -f "$file_path" ] && [ ! -L "$file_path" ] ||
      die "$file_label is not a regular file: $file_path"
  fi
}
for binary_target in \
  "$HOME_DIR/bin/synveda" \
  "$HOME_DIR/bin/synveda-gateway" \
  "$HOME_DIR/bin/synveda-worker" \
  "$BIN_DIR/synveda"; do
  require_plain_file_or_absent "$binary_target" "binary install target"
  [ ! -e "$binary_target.tmp" ] && [ ! -L "$binary_target.tmp" ] ||
    die "temporary binary install path already exists: $binary_target.tmp"
done
for private_path in \
  "$HOME_DIR/state" \
  "$HOME_DIR/state/synveda-reference" \
  "$HOME_DIR/backups" \
  "$HOME_DIR/backups/database" \
  "$HOME_DIR/backups/database/synveda-reference" \
  "$HOME_DIR/backups/secrets" \
  "$HOME_DIR/backups/secrets/synveda-reference"; do
  if [ -e "$private_path" ] || [ -L "$private_path" ]; then
    [ -d "$private_path" ] && [ ! -L "$private_path" ] ||
      die "mutable deployment path is not a directory: $private_path"
  fi
done

mkdir -p "$HOME_DIR/bin" || die "could not create binary install path: $HOME_DIR/bin"
if [ "$bin_dir_explicit" = yes ]; then
  mkdir -p "$BIN_DIR" || die "could not create SYNVEDA_BIN: $BIN_DIR"
fi

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
printf '%s\n' console > "$HOME_DIR/console/.synveda-installer-owned"

# The Claude marketplace and Codex runtime share this owned archive root.
# Neither client is configured here; its normal trust/setup flow remains explicit.
rm -rf "$HOME_DIR/plugin"
cp -R "$work/plugin" "$HOME_DIR/plugin"
printf '%s\n' plugin > "$HOME_DIR/plugin/.synveda-installer-owned"

# Releases are immutable inputs. Mutable secrets, keys and backups use sibling
# roots, so repointing `current` cannot switch or delete deployment authority.
mkdir -p "$releases_root"
rm -rf "$release_stage"
cp -R "$reference_bundle" "$release_stage"
printf 'reference:%s:%s\n' "$plain" "$source_sha" \
  > "$release_stage/.synveda-installer-owned"
if [ -e "$release_target" ]; then
  rm -rf "$release_target"
fi
mv "$release_stage" "$release_target"

current_next="$reference_root/.current-$$"
rm -f "$current_next"
ln -s "releases/$release_id" "$current_next"
# POSIX mv follows an existing symlink-to-directory on common platforms. The
# current link was proved installer-owned above, so remove exactly that link
# before renaming the fully staged replacement. If the rename fails, immutable
# releases remain intact and a rerun restores current.
rm -f "$reference_root/current"
mv "$current_next" "$reference_root/current"

ensure_private_directory() {
  private_dir=$1
  if [ -e "$private_dir" ] || [ -L "$private_dir" ]; then
    [ -d "$private_dir" ] && [ ! -L "$private_dir" ] ||
      die "mutable deployment path is not a directory: $private_dir"
  else
    mkdir -p "$private_dir"
  fi
  chmod 700 "$private_dir"
}
for private_dir in \
  "$HOME_DIR/state" \
  "$HOME_DIR/state/synveda-reference" \
  "$HOME_DIR/backups" \
  "$HOME_DIR/backups/database" \
  "$HOME_DIR/backups/database/synveda-reference" \
  "$HOME_DIR/backups/secrets" \
  "$HOME_DIR/backups/secrets/synveda-reference"; do
  ensure_private_directory "$private_dir"
done

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
  # simply declines. By then the gateway, console, reference and plugin are
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
say "  reference $HOME_DIR/reference/current"
say "  state     $HOME_DIR/state                 (preserved across upgrades)"
say "  backups   $HOME_DIR/backups               (preserved across upgrades)"
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
say "The digest-bound Docker reference bundle is installed but not started."
say ""
say "Configure real DNS and TLS as documented, then use:"
say ""
say "  SYNVEDA_APP_HOST=app.example.com \\\"
say "  SYNVEDA_AUTH_HOST=auth.example.com \\\"
say "    $HOME_DIR/reference/current/synveda-compose up"
say ""
say "The installer never starts containers. Clean-host and published-image"
say "acceptance evidence remains documented separately."
say ""
say "  https://github.com/$REPO/blob/$source_sha/docs/INSTALL.md"
say ""
say "After the gateway is running, client setup remains explicit:"
say ""
say "  synveda plugin install             # Claude Code: hooks + MCP, one command"
say "  synveda mcp install --client claude-desktop   # or cursor, or zed"
say "  Codex hook (Node 22+): $HOME_DIR/plugin/codex/dist/hook.mjs"
say "  Codex setup: https://github.com/$REPO/blob/$source_sha/docs/integrations/codex.md"
say ""
say "Docs: https://github.com/$REPO/blob/$source_sha/docs/INSTALL.md"
