#!/bin/sh
# Generate the local bundled-provider secret set without revealing values.
# A caller may enable shell tracing before execution. Disable it before the
# first value-bearing expansion so generated credentials cannot reach logs.
set +x
set -eu
LC_ALL=C
export LC_ALL

force=0
if_missing=0
case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    --if-missing) if_missing=1 ;;
    *)
        echo "usage: deploy/compose/scripts/generate-secrets.sh [--force|--if-missing]" >&2
        exit 64
        ;;
esac
[ "$#" -le 1 ] || {
    echo "usage: deploy/compose/scripts/generate-secrets.sh [--force|--if-missing]" >&2
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
        *[!a-z0-9-]*|*-)
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

default_secret_dir=./runtime/$project/secrets
configured_secret_dir=${SYNVEDA_SECRETS_DIR:-$default_secret_dir}
configured_authority_dir=${SYNVEDA_DATABASE_AUTHORITY_DIR:-./runtime/$project/database-authority}
configured_gate_dir=${SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR:-./runtime/$project/keycloak-public-gate}
secret_dir=$(absolute_private_path "$configured_secret_dir" "$default_secret_dir" secret)
authority_dir=$(absolute_private_path "$configured_authority_dir" \
    "./runtime/$project/database-authority" database-authority)
gate_dir=$(absolute_private_path "$configured_gate_dir" \
    "./runtime/$project/keycloak-public-gate" keycloak-public-gate)

case "$secret_dir" in
    */"$project"/secrets) ;;
    *) echo "generate-secrets: secret directory must be scoped to project $project" >&2; exit 73 ;;
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
            "$compose_dir/runtime/$project/secrets"|"$compose_dir/runtime/$project/database-authority"|"$compose_dir/runtime/$project/keycloak-public-gate") ;;
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

# Serialise every absence check, stage and publication with lifecycle actions
# for this exact Compose project. A caller already holding the same verified
# lock (compose.sh) lends it to this child; standalone calls acquire it here.
# shellcheck source=deploy/compose/scripts/project-lock.sh
. "$script_dir/project-lock.sh"
secret_stage=
demo_stage=
generate_secrets_cleanup() {
    cleanup_status=$?
    trap '' HUP INT TERM
    trap - EXIT
    if [ -n "$demo_stage" ]; then
        rm -f -- "$demo_stage/keycloak_demo_admin_password" \
            "$demo_stage/keycloak_demo_member_password" 2>/dev/null || true
        rmdir -- "$demo_stage" 2>/dev/null || true
    fi
    if [ -n "$secret_stage" ]; then
        rmdir -- "$secret_stage/oidc-directory" 2>/dev/null || true
        for staged_name in ${files:-} .synveda-private-directory; do
            rm -f -- "$secret_stage/$staged_name" 2>/dev/null || true
        done
        rmdir -- "$secret_stage" 2>/dev/null || true
    fi
    if ! release_project_lock; then
        [ "$cleanup_status" -ne 0 ] || cleanup_status=73
    fi
    exit "$cleanup_status"
}
trap generate_secrets_cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
acquire_project_lock

base_files='postgres_owner_password
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
demo_files='keycloak_demo_admin_password
keycloak_demo_member_password'
files="$base_files
$demo_files"

validate_secret_inventory() {
    inventory_directory=$1
    for inventory_entry in \
        "$inventory_directory"/.[!.]* \
        "$inventory_directory"/..?* \
        "$inventory_directory"/*; do
        [ -e "$inventory_entry" ] || [ -L "$inventory_entry" ] || continue
        inventory_name=${inventory_entry##*/}
        case "$inventory_name" in
            "$marker_name"|oidc-directory|tls_cert|tls_key|\
            postgres_owner_password|synveda_migrator_password|\
            synveda_gateway_password|synveda_worker_password|\
            keycloak_database_password|keycloak_admin_username|\
            keycloak_admin_password|keycloak_convergence_admin_password|\
            synveda_migrator_database_url|synveda_gateway_database_url|\
            synveda_worker_database_url|synveda_kms_key|synveda_kms_key_ref|\
            keycloak_demo_admin_password|keycloak_demo_member_password) ;;
            *)
                echo "generate-secrets: existing secret set contains an unknown entry" >&2
                exit 73
                ;;
        esac
    done
}

umask 077
mode_of() {
    stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1" 2>/dev/null
}
owner_of() {
    stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1" 2>/dev/null
}
identity_of() {
    stat -c '%d:%i' "$1" 2>/dev/null || stat -f '%d:%i' "$1" 2>/dev/null
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
validate_private_parent() {
    directory=$1
    [ ! -L "$directory" ] && [ -d "$directory" ] && \
        [ "$(mode_of "$directory")" = 700 ] && \
        [ "$(owner_of "$directory")" = "$(id -u)" ] || {
        echo "generate-secrets: secret parent directory metadata was refused" >&2
        exit 73
    }
}
if [ -e "$secret_parent" ] || [ -L "$secret_parent" ]; then
    validate_private_parent "$secret_parent"
elif [ "$secret_dir" != "$compose_dir/runtime/$project/secrets" ]; then
    echo "generate-secrets: custom secret parent directory must already exist" >&2
    exit 73
fi
backup_dir=$secret_parent/previous-secrets
existing_secret_set=false
existing_secret_identity=
demo_extension_needed=false
missing_demo_files=
if { [ -e "$backup_dir" ] || [ -L "$backup_dir" ]; } && \
    [ ! -e "$secret_dir" ] && [ ! -L "$secret_dir" ]; then
    echo "generate-secrets: preserved previous secret set exists without an active set; explicit recovery is required" >&2
    exit 73
fi
if [ -e "$secret_dir" ] || [ -L "$secret_dir" ]; then
    require_marker "$secret_dir"
    validate_secret_inventory "$secret_dir"
    existing_secret_identity=$(identity_of "$secret_dir") || {
        echo "generate-secrets: existing secret-set identity was unavailable" >&2
        exit 73
    }
    if [ "$force" -eq 0 ] && [ "$if_missing" -eq 0 ]; then
        echo "generate-secrets: refusing to replace an existing secret set" >&2
        exit 73
    fi
    if [ "$force" -eq 1 ]; then
        [ "${SYNVEDA_CONFIRM_SECRET_REPLACEMENT:-}" = "$project" ] || {
            echo "generate-secrets: --force requires SYNVEDA_CONFIRM_SECRET_REPLACEMENT=$project" >&2
            exit 73
        }
        [ ! -e "$backup_dir" ] && [ ! -L "$backup_dir" ] || {
            echo "generate-secrets: preserved previous secret set already exists" >&2
            exit 73
        }
    fi
    for name in $base_files; do
        candidate=$secret_dir/$name
        [ ! -L "$candidate" ] && [ -f "$candidate" ] && \
            [ "$(mode_of "$candidate")" = 600 ] && \
            [ "$(owner_of "$candidate")" = "$(id -u)" ] || {
            echo "generate-secrets: existing secret set is incomplete or unsafe" >&2
            exit 73
        }
    done
    demo_present=0
    for name in $demo_files; do
        candidate=$secret_dir/$name
        if [ -e "$candidate" ] || [ -L "$candidate" ]; then
            [ ! -L "$candidate" ] && [ -f "$candidate" ] && \
                [ "$(mode_of "$candidate")" = 600 ] && \
                [ "$(owner_of "$candidate")" = "$(id -u)" ] || {
                echo "generate-secrets: existing demo secret extension is unsafe" >&2
                exit 73
            }
            demo_present=$((demo_present + 1))
        else
            missing_demo_files="$missing_demo_files $name"
        fi
    done
    for demo_name in $demo_files; do
        [ -f "$secret_dir/$demo_name" ] || continue
        for protected_name in keycloak_admin_password \
            keycloak_convergence_admin_password; do
            cmp -s -- "$secret_dir/$demo_name" \
                "$secret_dir/$protected_name" && {
                echo "generate-secrets: existing demo secret extension is unsafe" >&2
                exit 73
            }
        done
    done
    if [ "$demo_present" -eq 2 ] && \
        cmp -s -- "$secret_dir/keycloak_demo_admin_password" \
            "$secret_dir/keycloak_demo_member_password"; then
        echo "generate-secrets: existing demo secret extension is unsafe" >&2
        exit 73
    fi
    if [ "$demo_present" -lt 2 ]; then
        [ "$if_missing" -eq 1 ] || [ "$force" -eq 1 ] || {
            echo "generate-secrets: refusing to replace an existing secret set" >&2
            exit 73
        }
        [ "$force" -eq 1 ] || demo_extension_needed=true
    fi
    directory_credentials=$secret_dir/oidc-directory
    [ ! -L "$directory_credentials" ] && [ -d "$directory_credentials" ] && \
        [ "$(mode_of "$directory_credentials")" = 700 ] && \
        [ "$(owner_of "$directory_credentials")" = "$(id -u)" ] || {
        echo "generate-secrets: OIDC directory credential directory is incomplete or unsafe" >&2
        exit 73
    }
    existing_secret_set=true
fi

require_unchanged_secret_set() {
    [ "$existing_secret_set" = true ] && \
        [ ! -L "$secret_dir" ] && [ -d "$secret_dir" ] && \
        [ "$(identity_of "$secret_dir" 2>/dev/null || true)" = \
            "$existing_secret_identity" ] || {
        echo "generate-secrets: existing secret set changed during validation" >&2
        exit 73
    }
    require_marker "$secret_dir"
}

validate_complete_secret_set() {
    require_unchanged_secret_set
    validate_secret_inventory "$secret_dir"
    for complete_name in $files; do
        complete_candidate=$secret_dir/$complete_name
        [ ! -L "$complete_candidate" ] && [ -f "$complete_candidate" ] && \
            [ "$(mode_of "$complete_candidate")" = 600 ] && \
            [ "$(owner_of "$complete_candidate")" = "$(id -u)" ] || {
            echo "generate-secrets: completed secret set is incomplete or unsafe" >&2
            exit 73
        }
    done
    complete_directory_credentials=$secret_dir/oidc-directory
    [ ! -L "$complete_directory_credentials" ] && \
        [ -d "$complete_directory_credentials" ] && \
        [ "$(mode_of "$complete_directory_credentials")" = 700 ] && \
        [ "$(owner_of "$complete_directory_credentials")" = "$(id -u)" ] || {
        echo "generate-secrets: completed OIDC directory is incomplete or unsafe" >&2
        exit 73
    }
    for complete_demo_name in $demo_files; do
        for complete_protected_name in keycloak_admin_password \
            keycloak_convergence_admin_password; do
            cmp -s -- "$secret_dir/$complete_demo_name" \
                "$secret_dir/$complete_protected_name" && {
                echo "generate-secrets: completed demo secret extension is unsafe" >&2
                exit 73
            }
        done
    done
    cmp -s -- "$secret_dir/keycloak_demo_admin_password" \
        "$secret_dir/keycloak_demo_member_password" && {
        echo "generate-secrets: completed demo secret extension is unsafe" >&2
        exit 73
    }
    return 0
}

# Only mutate after every target, ancestor, existing secret set and overlap
# check has passed. Existing state leaves need a project marker; their
# metadata is validated, never silently repaired.
if [ ! -e "$secret_parent" ]; then
    mkdir -p -m 700 "$secret_parent" || {
        echo "generate-secrets: project runtime directory could not be created" >&2
        exit 73
    }
    chmod 700 "$secret_parent"
    validate_private_parent "$secret_parent"
fi
ensure_private_directory "$authority_dir"
ensure_private_directory "$gate_dir"

if [ "$existing_secret_set" = true ] && [ "$if_missing" -eq 1 ]; then
    require_unchanged_secret_set
    if [ "$demo_extension_needed" = true ]; then
        demo_stage=$(mktemp -d "$secret_parent/.synveda-demo-secret-stage.XXXXXX") || {
            echo "generate-secrets: demo secret staging directory could not be created" >&2
            exit 73
        }
        chmod 700 "$demo_stage"
        cleanup_demo_stage() {
            [ -n "$demo_stage" ] || return 0
            rm -f -- "$demo_stage/keycloak_demo_admin_password" \
                "$demo_stage/keycloak_demo_member_password" 2>/dev/null || return 1
            rmdir -- "$demo_stage" 2>/dev/null
        }
        for name in $missing_demo_files; do
            while :; do
                openssl rand -hex 32 > "$demo_stage/$name"
                chmod 600 "$demo_stage/$name"
                collision=false
                for existing_name in keycloak_admin_password \
                    keycloak_convergence_admin_password $demo_files; do
                    [ "$existing_name" = "$name" ] && continue
                    existing_path=$secret_dir/$existing_name
                    [ -f "$existing_path" ] || existing_path=$demo_stage/$existing_name
                    if [ -f "$existing_path" ] && \
                        cmp -s -- "$demo_stage/$name" "$existing_path"; then
                        collision=true
                    fi
                done
                [ "$collision" = false ] && break
            done
        done
        for name in $missing_demo_files; do
            staged_demo=$demo_stage/$name
            target_demo=$secret_dir/$name
            staged_demo_identity=$(identity_of "$staged_demo") || exit 73
            ln "$staged_demo" "$target_demo" || {
                echo "generate-secrets: demo secret extension could not be installed" >&2
                exit 73
            }
            [ ! -L "$target_demo" ] && [ -f "$target_demo" ] && \
                [ "$(identity_of "$target_demo" 2>/dev/null || true)" = \
                    "$staged_demo_identity" ] && \
                [ "$(mode_of "$target_demo")" = 600 ] && \
                [ "$(owner_of "$target_demo")" = "$(id -u)" ] || {
                echo "generate-secrets: demo secret extension changed during publication" >&2
                exit 73
            }
            rm -f -- "$staged_demo" || {
                echo "generate-secrets: demo secret staging cleanup failed" >&2
                exit 73
            }
            echo "generated $name"
        done
        cleanup_demo_stage
        demo_stage=
    fi
    validate_complete_secret_set
    echo "existing secret set validated"
    exit 0
fi

secret_stage=$(mktemp -d "$secret_parent/.synveda-secret-stage.XXXXXX") || {
    echo "generate-secrets: secret staging directory could not be created" >&2
    exit 73
}
chmod 700 "$secret_stage"
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
demo_admin_password=$(openssl rand -hex 32)
demo_member_password=$(openssl rand -hex 32)
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
write_secret keycloak_demo_admin_password "$demo_admin_password"
write_secret keycloak_demo_member_password "$demo_member_password"
write_secret synveda_migrator_database_url \
    "postgres://synveda_migrator:${migrator_password}@postgres:5432/synveda"
write_secret synveda_gateway_database_url \
    "postgres://synveda_gateway:${gateway_password}@postgres:5432/synveda"
write_secret synveda_worker_database_url \
    "postgres://synveda_worker:${worker_password}@postgres:5432/synveda"
write_secret synveda_kms_key "$kms_key"
write_secret synveda_kms_key_ref "local:${kms_ref}"

unset owner_password migrator_password gateway_password worker_password
unset keycloak_password admin_password convergence_admin_password
unset demo_admin_password demo_member_password kms_key kms_ref

publish_secret_stage() {
    [ ! -e "$secret_dir" ] && [ ! -L "$secret_dir" ] || return 1
    staged_identity=$(identity_of "$secret_stage") || return 1
    staged_basename=${secret_stage##*/}
    mv -n -- "$secret_stage" "$secret_dir" || return 1
    nested_stage=$secret_dir/$staged_basename
    if [ ! -L "$nested_stage" ] && [ -d "$nested_stage" ] && \
        [ "$(identity_of "$nested_stage")" = "$staged_identity" ]; then
        # BSD/GNU mv directory semantics may place a source inside a destination
        # created after the precheck. Keep our exact inode tracked so the EXIT
        # cleanup removes it and never leaves a second credential set active.
        secret_stage=$nested_stage
        return 1
    fi
    [ ! -e "$secret_stage" ] && [ ! -L "$secret_stage" ]
}

if [ "$existing_secret_set" = true ]; then
    require_unchanged_secret_set
    secret_basename=${secret_dir##*/}
    nested_backup=$backup_dir/$secret_basename
    preserve_status=0
    mv -n -- "$secret_dir" "$backup_dir" || preserve_status=$?
    if [ "$preserve_status" -eq 0 ] && \
        [ ! -L "$backup_dir" ] && [ -d "$backup_dir" ] && \
        [ "$(identity_of "$backup_dir" 2>/dev/null || true)" = \
            "$existing_secret_identity" ] && \
        [ ! -e "$secret_dir" ] && [ ! -L "$secret_dir" ]; then
        :
    elif [ ! -L "$secret_dir" ] && [ -d "$secret_dir" ] && \
        [ "$(identity_of "$secret_dir" 2>/dev/null || true)" = \
            "$existing_secret_identity" ]; then
        echo "generate-secrets: previous secret set remained active; preservation was refused" >&2
        exit 73
    elif [ ! -L "$nested_backup" ] && [ -d "$nested_backup" ] && \
        [ "$(identity_of "$nested_backup" 2>/dev/null || true)" = \
            "$existing_secret_identity" ]; then
        echo "generate-secrets: previous secret set was preserved at $nested_backup; foreign backup state was refused" >&2
        exit 73
    else
        echo "generate-secrets: previous secret set location became uncertain; preservation was refused" >&2
        exit 73
    fi
    if ! publish_secret_stage; then
        echo "generate-secrets: staged secret set could not be installed; previous set remains at $backup_dir" >&2
        exit 73
    fi
    echo "preserved previous secret set; this is not a credential-rotation workflow"
else
    publish_secret_stage || {
        echo "generate-secrets: staged secret set could not be installed" >&2
        exit 73
    }
fi
secret_stage=
validate_secret_inventory "$secret_dir"
for name in $files; do
    echo "generated $name"
done
