#!/bin/sh
# Generate the provider-neutral bundled-OIDC issuer input without credentials.
# The file is still deployment authority, so replacement is explicit and the
# private project-scoped parent is required.
set -eu
LC_ALL=C
export LC_ALL

force=0
case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    *) echo "usage: deploy/compose/scripts/generate-issuer.sh [--force]" >&2; exit 64 ;;
esac
[ "$#" -le 1 ] || {
    echo "usage: deploy/compose/scripts/generate-issuer.sh [--force]" >&2
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
parent=$(dirname "$target")
[ ! -L "$parent" ] && [ -d "$parent" ] || {
    echo "generate-issuer: private project runtime directory is missing" >&2
    exit 73
}
mode=$(stat -f '%Lp' "$parent" 2>/dev/null || stat -c '%a' "$parent" 2>/dev/null)
owner=$(stat -f '%u' "$parent" 2>/dev/null || stat -c '%u' "$parent" 2>/dev/null)
[ "$mode" = 700 ] && [ "$owner" = "$(id -u)" ] || {
    echo "generate-issuer: private project runtime directory metadata was refused" >&2
    exit 73
}

previous=$target.previous
if [ -e "$target" ] || [ -L "$target" ]; then
    [ ! -L "$target" ] && [ -f "$target" ] || {
        echo "generate-issuer: existing issuer input was refused" >&2
        exit 73
    }
    target_mode=$(stat -f '%Lp' "$target" 2>/dev/null || stat -c '%a' "$target" 2>/dev/null)
    target_owner=$(stat -f '%u' "$target" 2>/dev/null || stat -c '%u' "$target" 2>/dev/null)
    [ "$target_mode" = 600 ] && [ "$target_owner" = "$(id -u)" ] || {
        echo "generate-issuer: existing issuer input metadata was refused" >&2
        exit 73
    }
    [ "$force" -eq 1 ] && \
        [ "${SYNVEDA_CONFIRM_ISSUER_REPLACEMENT:-}" = "$project" ] || {
        echo "generate-issuer: refusing to replace existing issuer input" >&2
        exit 73
    }
    [ ! -e "$previous" ] && [ ! -L "$previous" ] || {
        echo "generate-issuer: preserved previous issuer input already exists" >&2
        exit 73
    }
fi

umask 077
stage=$(mktemp "$parent/.issuers.XXXXXX") || {
    echo "generate-issuer: staging failed" >&2
    exit 73
}
cleanup() { rm -f -- "$stage" 2>/dev/null || true; }
trap cleanup EXIT HUP INT TERM
chmod 600 "$stage"
printf '[\n  {\n    "issuer": "%s",\n    "client_id": "synveda",\n    "audience": "synveda-api",\n    "tenant": {"static": {"tenant_id": "%s"}},\n    "login_scopes": ["openid", "profile", "email"]\n  }\n]\n' \
    "$issuer" "$tenant_id" > "$stage"

if [ -e "$target" ]; then
    ln "$target" "$previous" || {
        echo "generate-issuer: previous input could not be preserved" >&2
        exit 73
    }
fi
if ! mv "$stage" "$target"; then
    echo "generate-issuer: staged input could not be installed" >&2
    exit 73
fi
stage=
trap - EXIT HUP INT TERM
echo "generated project-scoped issuer configuration"
