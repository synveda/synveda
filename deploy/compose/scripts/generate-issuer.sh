#!/bin/sh
# Generate the provider-neutral bundled-OIDC issuer input without credentials.
# The file is still deployment authority, so replacement is explicit and the
# private project-scoped parent is required.
set -eu
LC_ALL=C
export LC_ALL

force=0
if_missing=0
case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    --if-missing) if_missing=1 ;;
    *) echo "usage: deploy/compose/scripts/generate-issuer.sh [--force|--if-missing]" >&2; exit 64 ;;
esac
[ "$#" -le 1 ] || {
    echo "usage: deploy/compose/scripts/generate-issuer.sh [--force|--if-missing]" >&2
    exit 64
}

runtime=${SYNVEDA_COMPOSE_RUNTIME:-development}
oidc_mode=${SYNVEDA_OIDC_MODE:-bundled}
[ "$oidc_mode" = bundled ] || {
    echo "generate-issuer: external OIDC requires an operator-supplied issuer file" >&2
    exit 64
}
case "$runtime" in
    development|reference) ;;
    *) echo "generate-issuer: runtime was refused" >&2; exit 64 ;;
esac

suffix=${SYNVEDA_COMPOSE_PROJECT_SUFFIX:-}
project=synveda-$runtime
if [ -n "$suffix" ]; then
    suffix_value=${suffix#acceptance-}
    [ "$suffix_value" != "$suffix" ] && [ -n "$suffix_value" ] && \
        [ "${#suffix_value}" -le 24 ] || {
        echo "generate-issuer: project suffix was refused" >&2
        exit 64
    }
    case "$suffix_value" in
        *[!a-z0-9-]*|*-)
            echo "generate-issuer: project suffix was refused" >&2
            exit 64
            ;;
    esac
    case "$suffix_value" in
        [a-z0-9]*) ;;
        *) echo "generate-issuer: project suffix was refused" >&2; exit 64 ;;
    esac
    project=$project-$suffix
fi

app_host=${SYNVEDA_APP_HOST:-app.synveda.test}
auth_host=${SYNVEDA_AUTH_HOST:-auth.synveda.test}
scheme=${SYNVEDA_PUBLIC_SCHEME:-http}
port=${SYNVEDA_DEV_HTTP_PORT:-8080}
tenant_id=${SYNVEDA_BOOTSTRAP_TENANT_ID:-019b53c0-7c00-7000-8000-000000000045}

valid_host() {
    case "$1" in
        ''|*[!a-z0-9.-]*|localhost|*.localhost|.*|*.|*..*) return 1 ;;
    esac
    [ "${#1}" -le 253 ] || return 1
    case "$1" in *[a-z]*) ;; *) return 1 ;; esac
    case "$1" in *.*) ;; *) return 1 ;; esac
    old_ifs=$IFS
    IFS=.
    set -- $1
    IFS=$old_ifs
    for label in "$@"; do
        [ -n "$label" ] && [ "${#label}" -le 63 ] || return 1
        case "$label" in -*|*-) return 1 ;; esac
    done
}
valid_host "$app_host" && valid_host "$auth_host" && [ "$app_host" != "$auth_host" ] || {
    echo "generate-issuer: hostnames were refused" >&2
    exit 64
}
case "$tenant_id" in
    ????????-????-7???-[89ab]???-????????????) ;;
    *) echo "generate-issuer: bootstrap tenant UUIDv7 was refused" >&2; exit 64 ;;
esac
case "$tenant_id" in
    *[!0-9a-f-]*) echo "generate-issuer: bootstrap tenant UUIDv7 was refused" >&2; exit 64 ;;
esac
case "$runtime:$scheme" in
    development:http)
        case "$app_host:$auth_host" in
            *.test:*.test) ;;
            *) echo "generate-issuer: development hostnames must end in .test" >&2; exit 64 ;;
        esac
        case "$port" in
            ''|0|0*|*[!0-9]*) echo "generate-issuer: development port was refused" >&2; exit 64 ;;
        esac
        [ "$port" -ge 1024 ] && [ "$port" -le 65535 ] && [ "$port" -ne 8443 ] || {
            echo "generate-issuer: development port was refused" >&2
            exit 64
        }
        issuer=http://$auth_host:$port/realms/synveda
        ;;
    reference:https)
        case "$app_host:$auth_host" in
            *.test:*|*:*.test|*.localhost:*|*:*.localhost)
                echo "generate-issuer: reference hostnames were refused" >&2
                exit 64
                ;;
        esac
        issuer=https://$auth_host/realms/synveda
        ;;
    *) echo "generate-issuer: runtime scheme was refused" >&2; exit 64 ;;
esac

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
compose_dir=$(dirname "$script_dir")
# Issuer input and secret state form one project authority contract, so use the
# same lock as the lifecycle and secret generator.
# shellcheck source=deploy/compose/scripts/project-lock.sh
. "$script_dir/project-lock.sh"
stage=
generate_issuer_cleanup() {
    cleanup_status=$?
    trap '' HUP INT TERM
    trap - EXIT
    [ -z "$stage" ] || rm -f -- "$stage" 2>/dev/null || true
    if ! release_project_lock; then
        [ "$cleanup_status" -ne 0 ] || cleanup_status=73
    fi
    exit "$cleanup_status"
}
trap generate_issuer_cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
acquire_project_lock
default_target=./runtime/$project/issuers.json
configured=${SYNVEDA_OIDC_ISSUERS_FILE:-$default_target}
case "$configured" in
    /*) target=$configured ;;
    "$default_target") target=$compose_dir/${configured#./} ;;
    *) echo "generate-issuer: custom issuer path must be absolute" >&2; exit 73 ;;
esac
case "$target" in
    *//*|*/./*|*/../*|*/.|*/..|*[[:space:]])
        echo "generate-issuer: issuer path was refused" >&2
        exit 73
        ;;
esac
case "$target" in
    */"$project"/issuers.json) ;;
    *)
        echo "generate-issuer: issuer input must be scoped to project $project" >&2
        exit 73
        ;;
esac
parent=$(dirname "$target")
[ ! -L "$parent" ] && [ -d "$parent" ] || {
    echo "generate-issuer: private project runtime directory is missing" >&2
    exit 73
}
mode=$(stat -c '%a' "$parent" 2>/dev/null || stat -f '%Lp' "$parent" 2>/dev/null)
owner=$(stat -c '%u' "$parent" 2>/dev/null || stat -f '%u' "$parent" 2>/dev/null)
[ "$mode" = 700 ] && [ "$owner" = "$(id -u)" ] || {
    echo "generate-issuer: private project runtime directory metadata was refused" >&2
    exit 73
}

previous=$target.previous
existing_target=false
target_identity=
identity_of() {
    stat -c '%d:%i' "$1" 2>/dev/null || stat -f '%d:%i' "$1" 2>/dev/null
}
if [ -e "$target" ] || [ -L "$target" ]; then
    [ ! -L "$target" ] && [ -f "$target" ] || {
        echo "generate-issuer: existing issuer input was refused" >&2
        exit 73
    }
    target_mode=$(stat -c '%a' "$target" 2>/dev/null || stat -f '%Lp' "$target" 2>/dev/null)
    target_owner=$(stat -c '%u' "$target" 2>/dev/null || stat -f '%u' "$target" 2>/dev/null)
    [ "$target_mode" = 600 ] && [ "$target_owner" = "$(id -u)" ] || {
        echo "generate-issuer: existing issuer input metadata was refused" >&2
        exit 73
    }
    target_identity=$(identity_of "$target") || {
        echo "generate-issuer: existing issuer input identity was unavailable" >&2
        exit 73
    }
    if [ "$force" -eq 0 ] && [ "$if_missing" -eq 0 ]; then
        echo "generate-issuer: refusing to replace existing issuer input" >&2
        exit 73
    fi
    if [ "$force" -eq 1 ]; then
        [ "${SYNVEDA_CONFIRM_ISSUER_REPLACEMENT:-}" = "$project" ] || {
            echo "generate-issuer: --force requires exact project confirmation" >&2
            exit 73
        }
        [ ! -e "$previous" ] && [ ! -L "$previous" ] || {
            echo "generate-issuer: preserved previous issuer input already exists" >&2
            exit 73
        }
    fi
    existing_target=true
fi

umask 077
stage=$(mktemp "$parent/.issuers.XXXXXX") || {
    echo "generate-issuer: staging failed" >&2
    exit 73
}
cleanup() { rm -f -- "$stage" 2>/dev/null || true; }
chmod 600 "$stage"
printf '[\n  {\n    "issuer": "%s",\n    "client_id": "synveda",\n    "audience": "synveda-api",\n    "tenant": {"static": {"tenant_id": "%s"}},\n    "login_scopes": ["openid", "profile", "email"]\n  }\n]\n' \
    "$issuer" "$tenant_id" > "$stage"

if [ "$existing_target" = true ] && [ "$if_missing" -eq 1 ]; then
    [ ! -L "$target" ] && [ -f "$target" ] && \
        [ "$(identity_of "$target" 2>/dev/null || true)" = "$target_identity" ] && \
        [ "$(stat -c '%a' "$target" 2>/dev/null || stat -f '%Lp' "$target" 2>/dev/null)" = 600 ] && \
        [ "$(stat -c '%u' "$target" 2>/dev/null || stat -f '%u' "$target" 2>/dev/null)" = "$(id -u)" ] || {
        echo "generate-issuer: existing issuer input changed during validation" >&2
        exit 73
    }
    if ! cmp -s -- "$target" "$stage"; then
        echo "generate-issuer: existing issuer input differs from the selected contract" >&2
        exit 73
    fi
    cleanup
    stage=
    echo "existing project-scoped issuer configuration validated"
    exit 0
fi

if [ "$existing_target" = true ]; then
    [ ! -L "$target" ] && [ -f "$target" ] && \
        [ "$(identity_of "$target" 2>/dev/null || true)" = "$target_identity" ] && \
        [ "$(stat -c '%a' "$target" 2>/dev/null || stat -f '%Lp' "$target" 2>/dev/null)" = 600 ] && \
        [ "$(stat -c '%u' "$target" 2>/dev/null || stat -f '%u' "$target" 2>/dev/null)" = "$(id -u)" ] || {
        echo "generate-issuer: existing issuer input changed during validation" >&2
        exit 73
    }
    ln "$target" "$previous" || {
        echo "generate-issuer: previous input could not be preserved" >&2
        exit 73
    }
    [ "$(identity_of "$previous" 2>/dev/null || true)" = "$target_identity" ] && \
        [ "$(identity_of "$target" 2>/dev/null || true)" = "$target_identity" ] || {
        echo "generate-issuer: existing issuer input changed during preservation" >&2
        exit 73
    }
    # The stage and target share a directory, so replacement is atomic. The
    # preserved hard link remains recovery evidence without an unlink window
    # in which the authoritative target could disappear.
    if ! mv -f -- "$stage" "$target"; then
        echo "generate-issuer: staged input could not be installed" >&2
        exit 73
    fi
else
    # Hard-link publication is a no-clobber create. A competing publisher is
    # refused even if it appears after the initial absence check.
    ln "$stage" "$target" || {
        echo "generate-issuer: staged input could not be installed" >&2
        exit 73
    }
    rm -f -- "$stage" || {
        echo "generate-issuer: staging cleanup failed" >&2
        exit 73
    }
fi
stage=
echo "generated project-scoped issuer configuration"
