#!/bin/sh
# Generate the local bundled-provider secret set without revealing values.
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

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
compose_dir=$(dirname "$script_dir")
configured_secret_dir=${SYNVEDA_SECRETS_DIR:-./secrets}
case "$configured_secret_dir" in
    /*) secret_dir=$configured_secret_dir ;;
    ./*) secret_dir=$compose_dir/${configured_secret_dir#./} ;;
    *) secret_dir=$compose_dir/$configured_secret_dir ;;
esac

if [ -L "$secret_dir" ]; then
    echo "generate-secrets: secret directory must not be a symlink" >&2
    exit 73
fi
umask 077
mkdir -p "$secret_dir"
chmod 700 "$secret_dir"

files='postgres_owner_password
synveda_migrator_password
synveda_gateway_password
synveda_worker_password
keycloak_database_password
keycloak_admin_username
keycloak_admin_password
synveda_migrator_database_url
synveda_gateway_database_url
synveda_worker_database_url
synveda_kms_key
synveda_kms_key_ref'

if [ "$force" -eq 0 ]; then
    for name in $files; do
        if [ -e "$secret_dir/$name" ] || [ -L "$secret_dir/$name" ]; then
            echo "generate-secrets: refusing to overwrite $name; rerun with --force" >&2
            exit 73
        fi
    done
fi

owner_password=$(openssl rand -hex 32)
migrator_password=$(openssl rand -hex 32)
gateway_password=$(openssl rand -hex 32)
worker_password=$(openssl rand -hex 32)
keycloak_password=$(openssl rand -hex 32)
admin_password=$(openssl rand -hex 32)
kms_key=$(openssl rand -hex 32)
kms_ref=$(openssl rand -hex 16)

write_secret() {
    name=$1
    value=$2
    target=$secret_dir/$name
    if [ -L "$target" ]; then
        echo "generate-secrets: refusing symlink target $name" >&2
        exit 73
    fi
    temporary=$(mktemp "$secret_dir/.${name}.XXXXXX")
    printf '%s\n' "$value" > "$temporary"
    chmod 600 "$temporary"
    mv -f "$temporary" "$target"
    echo "generated $name"
}

write_secret postgres_owner_password "$owner_password"
write_secret synveda_migrator_password "$migrator_password"
write_secret synveda_gateway_password "$gateway_password"
write_secret synveda_worker_password "$worker_password"
write_secret keycloak_database_password "$keycloak_password"
write_secret keycloak_admin_username synveda-bootstrap
write_secret keycloak_admin_password "$admin_password"
write_secret synveda_migrator_database_url \
    "postgres://synveda_migrator:${migrator_password}@postgres:5432/synveda"
write_secret synveda_gateway_database_url \
    "postgres://synveda_gateway:${gateway_password}@postgres:5432/synveda"
write_secret synveda_worker_database_url \
    "postgres://synveda_worker:${worker_password}@postgres:5432/synveda"
write_secret synveda_kms_key "$kms_key"
write_secret synveda_kms_key_ref "local:${kms_ref}"

unset owner_password migrator_password gateway_password worker_password
unset keycloak_password admin_password kms_key kms_ref
