#!/bin/sh
# Generate the local bundled-provider secret set without revealing values.
# A caller may enable shell tracing before execution. Disable it before the
# first value-bearing expansion so generated credentials cannot reach logs.
set +x
set -eu

force=0
case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    *)
        echo "usage: deploy/compose/scripts/generate-secrets.sh [--force]" >&2
        exit 64
        ;;
esac
[ "$#" -le 1 ] || {
    echo "usage: deploy/compose/scripts/generate-secrets.sh [--force]" >&2
    exit 64
}

command -v openssl >/dev/null 2>&1 || {
    echo "generate-secrets: openssl is required" >&2
    exit 69
}

runtime=${SYNVEDA_COMPOSE_RUNTIME:-development}
case "$runtime" in
    development|reference) ;;
    *)
        echo "generate-secrets: SYNVEDA_COMPOSE_RUNTIME must be development|reference" >&2
        exit 64
        ;;
esac
project=synveda-$runtime
suffix=${SYNVEDA_COMPOSE_PROJECT_SUFFIX:-}
if [ -n "$suffix" ]; then
    suffix_value=${suffix#acceptance-}
    if [ "$suffix_value" = "$suffix" ] || [ -z "$suffix_value" ] || \
        [ "${#suffix_value}" -gt 24 ]; then
        echo "generate-secrets: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
        exit 64
    fi
    case "$suffix_value" in
        *[!a-z0-9-]*)
            echo "generate-secrets: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    case "$suffix_value" in
        [a-z0-9]*) ;;
        *)
            echo "generate-secrets: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    project=$project-$suffix
fi

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
compose_dir=$(dirname "$script_dir")
repo_root=$(CDPATH= cd "$compose_dir/../.." && pwd -P)

absolute_private_path() {
    configured=$1
    default=$2
    label=$3
    case "$configured" in
        /*) path=$configured ;;
        "$default") path=$compose_dir/${configured#./} ;;
        *)
            echo "generate-secrets: custom $label path must be absolute" >&2
            exit 73
            ;;
    esac
    case "$path" in
        *//*|*/./*|*/../*|*/.|*/..|*[[:space:]]*)
            echo "generate-secrets: $label path has an unsafe shape" >&2
            exit 73
            ;;
    esac
    printf '%s\n' "$path"
}

configured_secret_dir=${SYNVEDA_SECRETS_DIR:-./secrets}
configured_authority_dir=${SYNVEDA_DATABASE_AUTHORITY_DIR:-./runtime/$project/database-authority}
configured_gate_dir=${SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR:-./runtime/$project/keycloak-public-gate}
secret_dir=$(absolute_private_path "$configured_secret_dir" ./secrets secret)
authority_dir=$(absolute_private_path "$configured_authority_dir" \
    "./runtime/$project/database-authority" database-authority)
gate_dir=$(absolute_private_path "$configured_gate_dir" \
    "./runtime/$project/keycloak-public-gate" keycloak-public-gate)

case "$secret_dir" in
    */secrets) ;;
    *) echo "generate-secrets: secret directory must use the dedicated secrets leaf" >&2; exit 73 ;;
esac
case "$authority_dir" in
    */"$project"/database-authority) ;;
    *)
        echo "generate-secrets: database-authority directory must be scoped to project $project" >&2
        exit 73
        ;;
esac
case "$gate_dir" in
    */"$project"/keycloak-public-gate) ;;
    *)
        echo "generate-secrets: keycloak-public-gate directory must be scoped to project $project" >&2
        exit 73
        ;;
esac
path_is_within() {
    candidate=$1
    directory=$2
    [ "$candidate" = "$directory" ] || case "$candidate" in
        "$directory"/*) return 0 ;;
        *) return 1 ;;
    esac
}

for private_path in "$secret_dir" "$authority_dir" "$gate_dir"; do
    if path_is_within "$private_path" "$repo_root"; then
        case "$private_path" in
            "$compose_dir/secrets"|"$compose_dir/runtime/$project/database-authority"|"$compose_dir/runtime/$project/keycloak-public-gate") ;;
            *)
                echo "generate-secrets: in-repository private paths must use the ignored Compose roots" >&2
                exit 73
                ;;
        esac
    fi
    probe=$private_path
    while [ "$probe" != / ]; do
        [ ! -L "$probe" ] || {
            echo "generate-secrets: private path ancestors must not be symlinks" >&2
            exit 73
        }
        probe=$(dirname "$probe")
    done
done

reject_directory_overlap() {
    first=$1
    second=$2
    label=$3
    if path_is_within "$first" "$second" || path_is_within "$second" "$first"; then
        echo "generate-secrets: $label directories must not overlap" >&2
        exit 73
    fi
}
reject_directory_overlap "$secret_dir" "$authority_dir" \
    secret-and-database-authority
reject_directory_overlap "$secret_dir" "$gate_dir" \
    secret-and-keycloak-public-gate
reject_directory_overlap "$authority_dir" "$gate_dir" \
    database-authority-and-keycloak-public-gate

files='postgres_owner_password
synveda_migrator_password
synveda_gateway_password
synveda_worker_password
keycloak_database_password
keycloak_admin_username
keycloak_admin_password
keycloak_convergence_admin_password
synveda_migrator_database_url
synveda_gateway_database_url
synveda_worker_database_url
synveda_kms_key
synveda_kms_key_ref'

umask 077
mode_of() {
    stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1" 2>/dev/null
}
owner_of() {
    stat -f '%u' "$1" 2>/dev/null || stat -c '%u' "$1" 2>/dev/null
}
marker_name=.synveda-private-directory
marker_value=project:$project

require_marker() {
    directory=$1
    marker=$directory/$marker_name
    [ ! -L "$directory" ] && [ -d "$directory" ] && \
        [ "$(mode_of "$directory")" = 700 ] && \
        [ "$(owner_of "$directory")" = "$(id -u)" ] || {
        echo "generate-secrets: existing private directory metadata was refused" >&2
        exit 73
    }
    [ ! -L "$marker" ] && [ -f "$marker" ] && \
        [ "$(mode_of "$marker")" = 600 ] && \
        [ "$(owner_of "$marker")" = "$(id -u)" ] || {
        echo "generate-secrets: existing private directory is not owned by this project" >&2
        exit 73
    }
    IFS= read -r recorded_marker < "$marker" || recorded_marker=
    [ "$recorded_marker" = "$marker_value" ] || {
        echo "generate-secrets: existing private directory is not owned by this project" >&2
        exit 73
    }
}

ensure_private_directory() {
    directory=$1
    if [ -e "$directory" ] || [ -L "$directory" ]; then
        require_marker "$directory"
        return
    fi
    mkdir -p -m 700 "$directory" || {
        echo "generate-secrets: private directory could not be created" >&2
        exit 73
    }
    [ ! -L "$directory" ] && [ -d "$directory" ] || {
        echo "generate-secrets: private directory creation was refused" >&2
        exit 73
    }
    chmod 700 "$directory"
    printf '%s\n' "$marker_value" > "$directory/$marker_name"
    chmod 600 "$directory/$marker_name"
}

secret_parent=$(dirname "$secret_dir")
[ ! -L "$secret_parent" ] && [ -d "$secret_parent" ] || {
    echo "generate-secrets: secret parent directory must already exist" >&2
    exit 73
}
backup_dir=$(dirname "$authority_dir")/previous-secrets
if [ -e "$secret_dir" ] || [ -L "$secret_dir" ]; then
    require_marker "$secret_dir"
    if [ "$force" -eq 0 ]; then
        echo "generate-secrets: refusing to replace an existing secret set" >&2
        exit 73
    fi
    [ "${SYNVEDA_CONFIRM_SECRET_REPLACEMENT:-}" = "$project" ] || {
        echo "generate-secrets: --force requires SYNVEDA_CONFIRM_SECRET_REPLACEMENT=$project" >&2
        exit 73
    }
    [ ! -e "$backup_dir" ] && [ ! -L "$backup_dir" ] || {
        echo "generate-secrets: preserved previous secret set already exists" >&2
        exit 73
    }
    for name in $files; do
        candidate=$secret_dir/$name
        [ ! -L "$candidate" ] && [ -f "$candidate" ] && \
            [ "$(mode_of "$candidate")" = 600 ] && \
            [ "$(owner_of "$candidate")" = "$(id -u)" ] || {
            echo "generate-secrets: existing secret set is incomplete or unsafe" >&2
            exit 73
        }
    done
    directory_credentials=$secret_dir/oidc-directory
    [ ! -L "$directory_credentials" ] && [ -d "$directory_credentials" ] && \
        [ "$(mode_of "$directory_credentials")" = 700 ] && \
        [ "$(owner_of "$directory_credentials")" = "$(id -u)" ] || {
        echo "generate-secrets: OIDC directory credential directory is incomplete or unsafe" >&2
        exit 73
    }
fi

# Only mutate after every target, ancestor, existing secret set and overlap
# check has passed. Existing state leaves need a project marker; their
# metadata is validated, never silently repaired.
ensure_private_directory "$authority_dir"
ensure_private_directory "$gate_dir"

secret_stage=$(mktemp -d "$secret_parent/.synveda-secret-stage.XXXXXX") || {
    echo "generate-secrets: secret staging directory could not be created" >&2
    exit 73
}
chmod 700 "$secret_stage"
cleanup_stage() {
    [ -n "${secret_stage:-}" ] || return 0
    rmdir -- "$secret_stage/oidc-directory" 2>/dev/null || return 1
    for staged_name in $files "$marker_name"; do
        rm -f -- "$secret_stage/$staged_name" 2>/dev/null || return 1
    done
    rmdir -- "$secret_stage" 2>/dev/null
}
trap 'cleanup_stage || true' EXIT HUP INT TERM
printf '%s\n' "$marker_value" > "$secret_stage/$marker_name"
chmod 600 "$secret_stage/$marker_name"
mkdir -m 700 "$secret_stage/oidc-directory"

owner_password=$(openssl rand -hex 32)
migrator_password=$(openssl rand -hex 32)
gateway_password=$(openssl rand -hex 32)
worker_password=$(openssl rand -hex 32)
keycloak_password=$(openssl rand -hex 32)
admin_password=$(openssl rand -hex 32)
convergence_admin_password=$(openssl rand -hex 32)
kms_key=$(openssl rand -hex 32)
kms_ref=$(openssl rand -hex 16)

write_secret() {
    name=$1
    value=$2
    target=$secret_stage/$name
    printf '%s\n' "$value" > "$target"
    chmod 600 "$target"
}

write_secret postgres_owner_password "$owner_password"
write_secret synveda_migrator_password "$migrator_password"
write_secret synveda_gateway_password "$gateway_password"
write_secret synveda_worker_password "$worker_password"
write_secret keycloak_database_password "$keycloak_password"
write_secret keycloak_admin_username synveda-bootstrap
write_secret keycloak_admin_password "$admin_password"
write_secret keycloak_convergence_admin_password "$convergence_admin_password"
write_secret synveda_migrator_database_url \
    "postgres://synveda_migrator:${migrator_password}@postgres:5432/synveda"
write_secret synveda_gateway_database_url \
    "postgres://synveda_gateway:${gateway_password}@postgres:5432/synveda"
write_secret synveda_worker_database_url \
    "postgres://synveda_worker:${worker_password}@postgres:5432/synveda"
write_secret synveda_kms_key "$kms_key"
write_secret synveda_kms_key_ref "local:${kms_ref}"

unset owner_password migrator_password gateway_password worker_password
unset keycloak_password admin_password convergence_admin_password kms_key kms_ref

if [ -e "$secret_dir" ] || [ -L "$secret_dir" ]; then
    mv "$secret_dir" "$backup_dir" || {
        echo "generate-secrets: previous secret set could not be preserved" >&2
        exit 73
    }
    if ! mv "$secret_stage" "$secret_dir"; then
        mv "$backup_dir" "$secret_dir" 2>/dev/null || true
        echo "generate-secrets: staged secret set could not be installed" >&2
        exit 73
    fi
    echo "preserved previous secret set; this is not a credential-rotation workflow"
else
    mv "$secret_stage" "$secret_dir" || {
        echo "generate-secrets: staged secret set could not be installed" >&2
        exit 73
    }
fi
secret_stage=
trap - EXIT HUP INT TERM
for name in $files; do
    echo "generated $name"
done
