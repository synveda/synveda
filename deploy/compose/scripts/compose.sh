#!/bin/sh
# Canonical CPR-45 Compose selector. This checkpoint deliberately exposes only
# deterministic validation; lifecycle actions arrive with convergence jobs.
set -eu

# Shell ranges follow the process locale. Pin validation to bytewise ASCII so
# non-ASCII characters cannot collate into the closed DNS/image/name grammars.
LC_ALL=C
export LC_ALL

usage() {
    echo "usage: deploy/compose/scripts/compose.sh config [--output PATH]" >&2
    exit 64
}

[ "${1:-}" = config ] || usage
shift
output=
case "${1:-}" in
    "") ;;
    --output)
        [ "$#" -eq 2 ] && [ -n "${2:-}" ] || usage
        output=$2
        shift 2
        ;;
    *) usage ;;
esac
[ "$#" -eq 0 ] || usage

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
compose_dir=$(dirname "$script_dir")
repo_root=$(CDPATH= cd "$compose_dir/../.." && pwd -P)
docker_bin=${SYNVEDA_DOCKER_BIN:-docker}

runtime=${SYNVEDA_COMPOSE_RUNTIME:-development}
postgres_mode=${SYNVEDA_POSTGRES_MODE:-bundled}
oidc_mode=${SYNVEDA_OIDC_MODE:-bundled}
profiles=${SYNVEDA_COMPOSE_PROFILES:-}

# The wrapper owns provider/profile/file selection. Prevent Docker-native
# selector variables and an ambient .env from adding an unvalidated fragment.
unset COMPOSE_FILE COMPOSE_PROJECT_NAME COMPOSE_PROFILES COMPOSE_ENV_FILES
unset SYNVEDA_RENDER_PUBLIC_EDGE_SUBNET SYNVEDA_RENDER_PUBLIC_EDGE_GATEWAY \
    SYNVEDA_RENDER_APP_BACKEND_SUBNET SYNVEDA_RENDER_APP_BACKEND_GATEWAY \
    SYNVEDA_RENDER_DATA_SUBNET SYNVEDA_RENDER_DATA_GATEWAY \
    SYNVEDA_RENDER_KEYCLOAK_DATA_SUBNET SYNVEDA_RENDER_KEYCLOAK_DATA_GATEWAY \
    SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_SUBNET SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_SUBNET SYNVEDA_RENDER_IDENTITY_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_DYNAMIC_RANGE SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS \
    SYNVEDA_RENDER_TELEMETRY_SUBNET SYNVEDA_RENDER_TELEMETRY_GATEWAY \
    SYNVEDA_RENDER_APPLICATION_EGRESS_SUBNET SYNVEDA_RENDER_APPLICATION_EGRESS_GATEWAY \
    SYNVEDA_RENDER_IDENTITY_EGRESS_SUBNET SYNVEDA_RENDER_IDENTITY_EGRESS_GATEWAY \
    SYNVEDA_RENDER_TELEMETRY_EGRESS_SUBNET SYNVEDA_RENDER_TELEMETRY_EGRESS_GATEWAY
export COMPOSE_DISABLE_ENV_FILE=1

case "$runtime" in
    development|reference) ;;
    *) echo "compose: SYNVEDA_COMPOSE_RUNTIME must be development|reference" >&2; exit 64 ;;
esac
case "$postgres_mode" in
    bundled|external) ;;
    *) echo "compose: SYNVEDA_POSTGRES_MODE must be bundled|external" >&2; exit 64 ;;
esac
case "$oidc_mode" in
    bundled|external) ;;
    *) echo "compose: SYNVEDA_OIDC_MODE must be bundled|external" >&2; exit 64 ;;
esac

old_ifs=$IFS
IFS=,
for profile in $profiles; do
    case "$profile" in
        "") ;;
        semantic|observability|apalis-board|demo|backup-test) ;;
        *)
            echo "compose: unsupported profile; allowed: semantic,observability,apalis-board,demo,backup-test" >&2
            exit 64
            ;;
    esac
done
IFS=$old_ifs

runtime_uid=${SYNVEDA_RUNTIME_UID:-$(id -u)}
runtime_gid=${SYNVEDA_RUNTIME_GID:-$(id -g)}
valid_runtime_id() {
    candidate=$1
    case "$candidate" in
        ''|0|0*|*[!0-9]*) return 1 ;;
    esac
    [ "${#candidate}" -le 10 ] && [ "$candidate" -le 2147483647 ]
}
if ! valid_runtime_id "$runtime_uid" || ! valid_runtime_id "$runtime_gid"; then
    echo "compose: SYNVEDA_RUNTIME_UID and SYNVEDA_RUNTIME_GID must be non-zero decimal integers" >&2
    exit 64
fi

suffix=${SYNVEDA_COMPOSE_PROJECT_SUFFIX:-}
project=synveda-$runtime
if [ -n "$suffix" ]; then
    suffix_value=${suffix#acceptance-}
    if [ "$suffix_value" = "$suffix" ] || [ -z "$suffix_value" ] || \
        [ "${#suffix_value}" -gt 24 ]; then
        echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
        exit 64
    fi
    case "$suffix_value" in
        *[!a-z0-9-]*|*-)
            echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    case "$suffix_value" in
        [a-z0-9]*) ;;
        *)
            echo "compose: project suffix must match acceptance-[a-z0-9][a-z0-9-]{0,23}" >&2
            exit 64
            ;;
    esac
    project=$project-$suffix
fi

app_host=${SYNVEDA_APP_HOST:-app.synveda.test}
if [ "$oidc_mode" = bundled ]; then
    auth_host=${SYNVEDA_AUTH_HOST:-auth.synveda.test}
fi
public_scheme=${SYNVEDA_PUBLIC_SCHEME:-http}
valid_host() {
    candidate=$1
    [ "${#candidate}" -le 253 ] || return 1
    case "$candidate" in
        ''|*[!a-z0-9.-]*|localhost|*.localhost|.*|*.|*..*) return 1 ;;
    esac
    case "$candidate" in
        *[a-z]*) ;;
        *) return 1 ;;
    esac
    case "$candidate" in
        *.*) ;;
        *) return 1 ;;
    esac
    previous_ifs=$IFS
    IFS=.
    set -- $candidate
    IFS=$previous_ifs
    for label in "$@"; do
        [ -n "$label" ] && [ "${#label}" -le 63 ] || return 1
        case "$label" in
            -*|*-) return 1 ;;
        esac
    done
}
valid_host "$app_host" || {
    echo "compose: application and identity hostnames must be lower-case DNS names" >&2
    exit 64
}
if [ "$oidc_mode" = bundled ]; then
    valid_host "$auth_host" || {
        echo "compose: application and identity hostnames must be lower-case DNS names" >&2
        exit 64
    }
    [ "$app_host" != "$auth_host" ] || {
        echo "compose: application and identity hostnames must differ" >&2
        exit 64
    }
fi

case "$runtime" in
    development)
        [ "$public_scheme" = http ] || {
            echo "compose: development uses explicit HTTP" >&2
            exit 64
        }
        case "$app_host" in
            *.test) ;;
            *) echo "compose: development hostnames must end in .test" >&2; exit 64 ;;
        esac
        if [ "$oidc_mode" = bundled ]; then
            case "$auth_host" in
                *.test) ;;
                *) echo "compose: development hostnames must end in .test" >&2; exit 64 ;;
            esac
        fi
        restart_policy=no
        public_port=${SYNVEDA_DEV_HTTP_PORT:-8080}
        valid_runtime_id "$public_port" && [ "$public_port" -ge 1024 ] && \
            [ "$public_port" -le 65535 ] && [ "$public_port" -ne 8443 ] || {
            echo "compose: SYNVEDA_DEV_HTTP_PORT must be a canonical integer from 1024 through 65535 except reserved port 8443" >&2
            exit 64
        }
        public_app_url=http://$app_host:$public_port
        # Browser and containers resolve the same issuer authority. Caddy must
        # therefore listen on the selected public port inside the network too;
        # host-only port translation would make the issuer unreachable from
        # the diagnostic, gateway and CLI containers.
        proxy_http_port=$public_port
        proxy_https_port=8443
        keycloak_ssl_required=NONE
        insecure_development_http=true
        runtime_overlay=dev
        caddy_app_config=$compose_dir/configs/caddy/app.dev.caddy
        caddy_identity_config=$compose_dir/configs/caddy/identity.dev.caddy
        ;;
    reference)
        [ "$public_scheme" = https ] || {
            echo "compose: reference mode requires HTTPS" >&2
            exit 64
        }
        case "$app_host" in
            *.test|*.localhost)
                echo "compose: reference hostnames must be operator DNS names" >&2
                exit 64
                ;;
        esac
        if [ "$oidc_mode" = bundled ]; then
            case "$auth_host" in
                *.test|*.localhost)
                    echo "compose: reference hostnames must be operator DNS names" >&2
                    exit 64
                    ;;
            esac
        fi
        [ "${SYNVEDA_TLS_MODE:-files}" = files ] || {
            echo "compose: this checkpoint accepts reference certificate-file mode only" >&2
            exit 64
        }
        restart_policy=unless-stopped
        public_port=443
        public_app_url=https://$app_host
        proxy_http_port=80
        proxy_https_port=443
        keycloak_ssl_required=EXTERNAL
        insecure_development_http=false
        runtime_overlay=reference
        caddy_app_config=$compose_dir/configs/caddy/app.reference.caddy
        caddy_identity_config=$compose_dir/configs/caddy/identity.reference.caddy
        ;;
esac

if [ "$oidc_mode" = bundled ]; then
    case "$runtime" in
        development) public_auth_url=http://$auth_host:$public_port ;;
        reference) public_auth_url=https://$auth_host ;;
    esac
fi

if [ "$oidc_mode" = external ]; then
    caddy_identity_config=$compose_dir/configs/caddy/identity.external.caddy
fi

compose_ipv4_pool_set=${SYNVEDA_COMPOSE_IPV4_POOL+x}
compose_ipv4_pool=${SYNVEDA_COMPOSE_IPV4_POOL:-172.30.240.0/24}
if [ "$runtime" = reference ] || [ -n "$suffix" ]; then
    [ -n "$compose_ipv4_pool_set" ] && [ -n "${SYNVEDA_COMPOSE_IPV4_POOL:-}" ] || {
        echo "compose: reference and acceptance projects require an explicit SYNVEDA_COMPOSE_IPV4_POOL" >&2
        exit 64
    }
fi

valid_ipv4_octet() {
    case "$1" in
        0|[1-9]|[1-9][0-9]|[1-9][0-9][0-9]) [ "$1" -le 255 ] ;;
        *) return 1 ;;
    esac
}
split_ipv4() {
    address=$1
    previous_ifs=$IFS
    IFS=.
    set -- $address
    IFS=$previous_ifs
    [ "$#" -eq 4 ] || return 1
    valid_ipv4_octet "$1" && valid_ipv4_octet "$2" && \
        valid_ipv4_octet "$3" && valid_ipv4_octet "$4" || return 1
    [ "$address" = "$1.$2.$3.$4" ] || return 1
    ipv4_a=$1
    ipv4_b=$2
    ipv4_c=$3
    ipv4_d=$4
}
validate_private_24() {
    pool_candidate=$1
    case "$pool_candidate" in
        */24) pool_network=${pool_candidate%/24} ;;
        *) echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be a private /24 CIDR" >&2; exit 64 ;;
    esac
    split_ipv4 "$pool_network" && [ "$ipv4_d" -eq 0 ] || {
        echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be a canonical private IPv4 /24 CIDR" >&2
        exit 64
    }
    case "$ipv4_a" in
        10) ;;
        172)
            [ "$ipv4_b" -ge 16 ] && [ "$ipv4_b" -le 31 ] || {
                echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2
                exit 64
            }
            ;;
        192)
            [ "$ipv4_b" -eq 168 ] || {
                echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2
                exit 64
            }
            ;;
        *) echo "compose: SYNVEDA_COMPOSE_IPV4_POOL must be private" >&2; exit 64 ;;
    esac
}

validate_private_24 "$compose_ipv4_pool"
pool_prefix=$ipv4_a.$ipv4_b.$ipv4_c

# Ten current networks consume fixed /28 slots from one operator-selected /24.
# The proxy is fixed at identity slot +2 while dynamic identity endpoints are
# confined to the upper /29, so Docker cannot allocate the trusted address to
# Keycloak or a one-shot convergence container first.
identity_subnet=$pool_prefix.0/28
identity_gateway=$pool_prefix.1
proxy_identity_address=$pool_prefix.2
identity_dynamic_range=$pool_prefix.8/29
public_edge_subnet=$pool_prefix.16/28
public_edge_gateway=$pool_prefix.17
app_backend_subnet=$pool_prefix.32/28
app_backend_gateway=$pool_prefix.33
synveda_data_subnet=$pool_prefix.48/28
synveda_data_gateway=$pool_prefix.49
keycloak_data_subnet=$pool_prefix.64/28
keycloak_data_gateway=$pool_prefix.65
keycloak_management_subnet=$pool_prefix.80/28
keycloak_management_gateway=$pool_prefix.81
telemetry_subnet=$pool_prefix.96/28
telemetry_gateway=$pool_prefix.97
application_egress_subnet=$pool_prefix.112/28
application_egress_gateway=$pool_prefix.113
identity_egress_subnet=$pool_prefix.128/28
identity_egress_gateway=$pool_prefix.129
telemetry_egress_subnet=$pool_prefix.144/28
telemetry_egress_gateway=$pool_prefix.145

for setting in DATABASE_URL SYNVEDA_MIGRATOR_DATABASE_URL SYNVEDA_GATEWAY_DATABASE_URL \
    SYNVEDA_WORKER_DATABASE_URL \
    SYNVEDA_KMS_KEY SYNVEDA_KMS_KEY_REF POSTGRES_PASSWORD KC_DB_PASSWORD KC_BOOTSTRAP_ADMIN_USERNAME \
    KC_BOOTSTRAP_ADMIN_PASSWORD SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD; do
    case "$setting" in
        DATABASE_URL) present=${DATABASE_URL+x} ;;
        SYNVEDA_MIGRATOR_DATABASE_URL) present=${SYNVEDA_MIGRATOR_DATABASE_URL+x} ;;
        SYNVEDA_GATEWAY_DATABASE_URL) present=${SYNVEDA_GATEWAY_DATABASE_URL+x} ;;
        SYNVEDA_WORKER_DATABASE_URL) present=${SYNVEDA_WORKER_DATABASE_URL+x} ;;
        SYNVEDA_KMS_KEY) present=${SYNVEDA_KMS_KEY+x} ;;
        SYNVEDA_KMS_KEY_REF) present=${SYNVEDA_KMS_KEY_REF+x} ;;
        POSTGRES_PASSWORD) present=${POSTGRES_PASSWORD+x} ;;
        KC_DB_PASSWORD) present=${KC_DB_PASSWORD+x} ;;
        KC_BOOTSTRAP_ADMIN_USERNAME) present=${KC_BOOTSTRAP_ADMIN_USERNAME+x} ;;
        KC_BOOTSTRAP_ADMIN_PASSWORD) present=${KC_BOOTSTRAP_ADMIN_PASSWORD+x} ;;
        SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD) present=${SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD+x} ;;
    esac
    [ -z "${present:-}" ] || {
        echo "compose: direct secret setting $setting is forbidden; use the role-specific file" >&2
        exit 78
    }
done

absolute_from_compose() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        ./*) printf '%s/%s\n' "$compose_dir" "${1#./}" ;;
        *) printf '%s/%s\n' "$compose_dir" "$1" ;;
    esac
}

secret_dir=$(absolute_from_compose "${SYNVEDA_SECRETS_DIR:-./secrets}")
issuer_file=$(absolute_from_compose "${SYNVEDA_OIDC_ISSUERS_FILE:-./runtime/$project/issuers.json}")
database_authority_dir=$(absolute_from_compose "${SYNVEDA_DATABASE_AUTHORITY_DIR:-./runtime/$project/database-authority}")
keycloak_public_gate_dir=$(absolute_from_compose "${SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR:-./runtime/$project/keycloak-public-gate}")
if [ "${SYNVEDA_DATABASE_ROLES_FILE+x}" = x ]; then
    [ -n "$SYNVEDA_DATABASE_ROLES_FILE" ] || {
        echo "compose: SYNVEDA_DATABASE_ROLES_FILE must not be empty" >&2
        exit 78
    }
    database_roles_input=$SYNVEDA_DATABASE_ROLES_FILE
elif [ "$postgres_mode" = external ]; then
    echo "compose: external PostgreSQL requires an explicit topology-specific SYNVEDA_DATABASE_ROLES_FILE" >&2
    exit 78
elif [ "$oidc_mode" = bundled ]; then
    database_roles_input=./configs/database/roles.reference.json
else
    database_roles_input=./configs/database/roles.external-oidc.json
fi
database_roles_file=$(absolute_from_compose "$database_roles_input")
reject_sensitive_build_context_path() {
    candidate=$1
    allowed_root=$2
    label=$3
    case "$candidate" in
        "$repo_root"|"$repo_root"/*)
            case "$candidate" in
                "$allowed_root"|"$allowed_root"/*) ;;
                *)
                    echo "compose: $label must be outside the Docker build context or under its ignored Compose root" >&2
                    exit 78
                    ;;
            esac
            ;;
    esac
}
reject_sensitive_build_context_path "$secret_dir" "$compose_dir/secrets" secret-directory
reject_sensitive_build_context_path "$issuer_file" "$compose_dir/runtime" issuer-configuration
if [ "$oidc_mode" = bundled ]; then
    reject_sensitive_build_context_path "$database_authority_dir" "$compose_dir/runtime" database-authority
    reject_sensitive_build_context_path "$keycloak_public_gate_dir" "$compose_dir/runtime" keycloak-public-gate
fi
mode_of() {
    stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1" 2>/dev/null
}
owner_of() {
    stat -f '%u' "$1" 2>/dev/null || stat -c '%u' "$1" 2>/dev/null
}
group_of() {
    stat -f '%g' "$1" 2>/dev/null || stat -c '%g' "$1" 2>/dev/null
}
size_of() {
    stat -f '%z' "$1" 2>/dev/null || stat -c '%s' "$1" 2>/dev/null
}
require_private_directory() {
    directory=$1
    label=$2
    [ ! -L "$directory" ] && [ -d "$directory" ] || {
        echo "compose: $label directory is missing or is a symlink" >&2
        exit 78
    }
    [ "$(mode_of "$directory")" = 700 ] || {
        echo "compose: $label directory must have mode 0700" >&2
        exit 78
    }
    [ "$(owner_of "$directory")" = "$runtime_uid" ] && \
        [ "$(group_of "$directory")" = "$runtime_gid" ] || {
        echo "compose: $label directory must be owned by the runtime UID:GID" >&2
        exit 78
    }
}
require_private_directory "$secret_dir" secret
secret_dir=$(CDPATH= cd "$secret_dir" && pwd -P)
reject_sensitive_build_context_path "$secret_dir" "$compose_dir/secrets" secret-directory
oidc_directory_secret_dir=$secret_dir/oidc-directory
require_private_directory "$oidc_directory_secret_dir" oidc-directory-secret
oidc_directory_secret_dir=$(CDPATH= cd "$oidc_directory_secret_dir" && pwd -P)
reject_sensitive_build_context_path "$oidc_directory_secret_dir" "$compose_dir/secrets" oidc-directory-secret
if [ "$oidc_mode" = bundled ]; then
    require_private_directory "$database_authority_dir" database-authority
    database_authority_dir=$(CDPATH= cd "$database_authority_dir" && pwd -P)
    reject_sensitive_build_context_path "$database_authority_dir" "$compose_dir/runtime" database-authority
    require_private_directory "$keycloak_public_gate_dir" keycloak-public-gate
    keycloak_public_gate_dir=$(CDPATH= cd "$keycloak_public_gate_dir" && pwd -P)
    reject_sensitive_build_context_path "$keycloak_public_gate_dir" "$compose_dir/runtime" keycloak-public-gate
    case "$database_authority_dir" in
        */"$project"/database-authority) ;;
        *)
            echo "compose: database-authority directory must be scoped to project $project" >&2
            exit 78
            ;;
    esac
    case "$keycloak_public_gate_dir" in
        */"$project"/keycloak-public-gate) ;;
        *)
            echo "compose: keycloak-public-gate directory must be scoped to project $project" >&2
            exit 78
            ;;
    esac
fi
issuer_parent=$(dirname "$issuer_file")
require_private_directory "$issuer_parent" issuer-configuration
issuer_parent=$(CDPATH= cd "$issuer_parent" && pwd -P)
issuer_file=$issuer_parent/$(basename "$issuer_file")
reject_sensitive_build_context_path "$issuer_file" "$compose_dir/runtime" issuer-configuration
require_private_file() {
    file=$1
    label=$2
    [ ! -L "$file" ] && [ -f "$file" ] || {
        echo "compose: required $label file is missing or is a symlink" >&2
        exit 78
    }
    mode=$(mode_of "$file")
    [ "$mode" = 600 ] || {
        echo "compose: required $label file must have mode 0600" >&2
        exit 78
    }
    [ "$(owner_of "$file")" = "$runtime_uid" ] && \
        [ "$(group_of "$file")" = "$runtime_gid" ] || {
        echo "compose: required $label file must be owned by the runtime UID:GID" >&2
        exit 78
    }
    [ -s "$file" ] || {
        echo "compose: required $label file must not be empty" >&2
        exit 78
    }
}

for name in synveda_migrator_database_url synveda_gateway_database_url \
    synveda_worker_database_url synveda_kms_key synveda_kms_key_ref; do
    require_private_file "$secret_dir/$name" "$name"
done
if [ "$postgres_mode" = bundled ]; then
    require_private_file "$secret_dir/postgres_owner_password" postgres_owner_password
    require_private_file "$secret_dir/synveda_migrator_password" synveda_migrator_password
    require_private_file "$secret_dir/synveda_gateway_password" synveda_gateway_password
    require_private_file "$secret_dir/synveda_worker_password" synveda_worker_password
fi
if [ "$oidc_mode" = bundled ]; then
    require_private_file "$secret_dir/postgres_owner_password" postgres_owner_password
    require_private_file "$secret_dir/keycloak_database_password" keycloak_database_password
    require_private_file "$secret_dir/keycloak_admin_username" keycloak_admin_username
    require_private_file "$secret_dir/keycloak_admin_password" keycloak_admin_password
    require_private_file "$secret_dir/keycloak_convergence_admin_password" \
        keycloak_convergence_admin_password
fi
if [ "$runtime" = reference ]; then
    require_private_file "$secret_dir/tls_cert" tls_cert
    require_private_file "$secret_dir/tls_key" tls_key
fi
require_private_file "$issuer_file" issuer_configuration

[ ! -L "$database_roles_file" ] && [ -f "$database_roles_file" ] || {
    echo "compose: database role contract file is missing or is a symlink" >&2
    exit 78
}
database_roles_bytes=$(size_of "$database_roles_file") || {
    echo "compose: database role contract size cannot be inspected" >&2
    exit 78
}
case "$database_roles_bytes" in
    ''|*[!0-9]*)
        echo "compose: database role contract size cannot be inspected" >&2
        exit 78
        ;;
esac
[ "$database_roles_bytes" -gt 0 ] && [ "$database_roles_bytes" -le 4096 ] || {
    echo "compose: database role contract must contain 1 through 4096 bytes" >&2
    exit 78
}
database_roles_parent=$(CDPATH= cd "$(dirname "$database_roles_file")" && pwd -P)
database_roles_file=$database_roles_parent/$(basename "$database_roles_file")

path_is_within() {
    candidate=$1
    directory=$2
    [ "$candidate" = "$directory" ] || case "$candidate" in
        "$directory"/*) return 0 ;;
        *) return 1 ;;
    esac
}
reject_directory_overlap() {
    first=$1
    second=$2
    label=$3
    if path_is_within "$first" "$second" || path_is_within "$second" "$first"; then
        echo "compose: $label directories must not overlap" >&2
        exit 78
    fi
}
reject_file_in_directory() {
    file=$1
    directory=$2
    label=$3
    if path_is_within "$file" "$directory"; then
        echo "compose: $label file must not be inside runtime authority state" >&2
        exit 78
    fi
}
if [ "$oidc_mode" = bundled ]; then
    reject_directory_overlap "$secret_dir" "$database_authority_dir" \
        secret-and-database-authority
    reject_directory_overlap "$secret_dir" "$keycloak_public_gate_dir" \
        secret-and-keycloak-public-gate
    reject_directory_overlap "$database_authority_dir" "$keycloak_public_gate_dir" \
        database-authority-and-keycloak-public-gate
    for state_directory in "$database_authority_dir" "$keycloak_public_gate_dir"; do
        reject_file_in_directory "$issuer_file" "$state_directory" issuer-configuration
        reject_file_in_directory "$database_roles_file" "$state_directory" database-role-contract
    done
fi
reject_file_in_directory "$issuer_file" "$secret_dir" issuer-configuration
reject_file_in_directory "$database_roles_file" "$secret_dir" database-role-contract

if [ "$oidc_mode" = bundled ]; then
    oidc_issuer=$public_auth_url/realms/synveda
else
    oidc_issuer=${SYNVEDA_OIDC_ISSUER:-}
    case "$oidc_issuer" in
        http://*|https://*) ;;
        *)
            echo "compose: external OIDC requires an absolute SYNVEDA_OIDC_ISSUER" >&2
            exit 64
            ;;
    esac
    case "$oidc_issuer" in
        *'@'*|*'?'*|*'#'*|*[[:space:]]*)
            echo "compose: SYNVEDA_OIDC_ISSUER must not contain credentials, whitespace, a query or a fragment" >&2
            exit 64
            ;;
    esac
    if [ "$runtime" = reference ]; then
        case "$oidc_issuer" in
            https://*) ;;
            *) echo "compose: reference external OIDC requires HTTPS" >&2; exit 64 ;;
        esac
    fi
fi
issuer_bytes=$(size_of "$issuer_file") || {
    echo "compose: issuer configuration size cannot be inspected" >&2
    exit 78
}
case "$issuer_bytes" in
    ''|*[!0-9]*)
        echo "compose: issuer configuration size cannot be inspected" >&2
        exit 78
        ;;
esac
[ "$issuer_bytes" -le 1048576 ] || {
    echo "compose: issuer configuration exceeds the 1048576 byte startup bound" >&2
    exit 78
}

postgres_bootstrap_url_set=${SYNVEDA_POSTGRES_BOOTSTRAP_URL+x}
database_expected_host=
database_expected_port=
database_expected_name=
if [ "$postgres_mode" = bundled ]; then
    [ -z "${postgres_bootstrap_url_set:-}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL is not accepted with bundled PostgreSQL" >&2
        exit 64
    }
    postgres_bootstrap_url=postgresql://synveda_owner@postgres:5432/postgres
    postgres_bundled_cluster=true
    database_expected_host=postgres
    database_expected_port=5432
    database_expected_name=synveda
elif [ "$oidc_mode" = bundled ]; then
    postgres_bootstrap_url=${SYNVEDA_POSTGRES_BOOTSTRAP_URL:-}
    case "$postgres_bootstrap_url" in
        postgres://*|postgresql://*) ;;
        *)
            echo "compose: external PostgreSQL with bundled Keycloak requires SYNVEDA_POSTGRES_BOOTSTRAP_URL" >&2
            exit 64
            ;;
    esac
    case "$postgres_bootstrap_url" in
        *'?'*|*'#'*|*[![:print:]]*|*' '*)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must contain no query, fragment or whitespace" >&2
            exit 64
            ;;
    esac
    pg_location=${postgres_bootstrap_url#*://}
    pg_authority=${pg_location%%/*}
    pg_database=${pg_location#*/}
    [ "$pg_authority" != "$pg_location" ] && [ -n "$pg_database" ] && \
        [ "$pg_database" = "${pg_database##*/}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must identify one database" >&2
        exit 64
    }
    case "$pg_authority" in
        *@*)
            pg_user=${pg_authority%%@*}
            pg_endpoint=${pg_authority#*@}
            ;;
        *)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must name a login without a password" >&2
            exit 64
            ;;
    esac
    case "$pg_user" in
        ''|*:*|*'@'*|*[!A-Za-z0-9_-]*)
            echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must name a login without a password" >&2
            exit 64
            ;;
    esac
    pg_host=${pg_endpoint%:*}
    pg_port=${pg_endpoint##*:}
    [ "$pg_host" != "$pg_endpoint" ] && valid_host "$pg_host" && \
    valid_runtime_id "$pg_port" && [ "$pg_port" -le 65535 ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL must identify one canonical DNS host and port" >&2
        exit 64
    }
    postgres_bundled_cluster=false
    database_expected_host=$pg_host
    database_expected_port=$pg_port
    database_expected_name=synveda
else
    [ -z "${postgres_bootstrap_url_set:-}" ] || {
        echo "compose: SYNVEDA_POSTGRES_BOOTSTRAP_URL is not accepted without a bundled database bootstrap" >&2
        exit 64
    }
    postgres_bootstrap_url=
    postgres_bundled_cluster=false
fi

product_image=${SYNVEDA_PRODUCT_IMAGE:-synveda/product:dev}
postgres_image=${SYNVEDA_POSTGRES_IMAGE:-synveda/postgres:17.11-dev}
keycloak_image=${SYNVEDA_KEYCLOAK_IMAGE:-synveda/keycloak:26.7.2-dev}
caddy_image=${SYNVEDA_CADDY_IMAGE:-synveda/proxy:2.11.4-dev}
otel_image=${SYNVEDA_OTEL_COLLECTOR_IMAGE:-otel/opentelemetry-collector-contrib:0.159.0@sha256:1f2c54a30e713fac6b3ae77a1ec84010c2007e29ced8ec666214fc2f6739c1cc}
valid_image_reference() {
    case "$1" in
        ''|*[!A-Za-z0-9_./:@+-]*) return 1 ;;
    esac
}
digest_image() {
    candidate=$1
    valid_image_reference "$candidate" || return 1
    case "$candidate" in *@sha256:*) ;; *) return 1 ;; esac
    digest=${candidate##*@sha256:}
    repository=${candidate%@sha256:*}
    [ -n "$repository" ] && [ "${#digest}" -eq 64 ] || return 1
    case "$repository" in *@*) return 1 ;; esac
    case "$digest" in
        *[!0-9a-f]*) return 1 ;;
    esac
}
for image_reference in "$product_image" "$postgres_image" "$keycloak_image" \
    "$caddy_image" "$otel_image"; do
    valid_image_reference "$image_reference" || {
        echo "compose: image references must use the closed OCI reference character set" >&2
        exit 64
    }
done
if [ "$runtime" = reference ]; then
    digest_image "$product_image" || {
        echo "compose: reference product image must use an OCI sha256 digest" >&2
        exit 64
    }
    if [ "$postgres_mode" = bundled ] || [ "$oidc_mode" = bundled ]; then
        digest_image "$postgres_image" || {
            echo "compose: reference PostgreSQL server/client image must use an OCI sha256 digest" >&2
            exit 64
        }
    fi
    if [ "$oidc_mode" = bundled ]; then
        digest_image "$keycloak_image" || {
            echo "compose: reference Keycloak image must use an OCI sha256 digest" >&2
            exit 64
        }
    fi
    digest_image "$caddy_image" || {
        echo "compose: reference proxy image must use an OCI sha256 digest" >&2
        exit 64
    }
fi
digest_image "$otel_image" || {
    echo "compose: Collector image must use an OCI sha256 digest" >&2
    exit 64
}
keycloak_database_url=
keycloak_database_url_set=${SYNVEDA_KEYCLOAK_DATABASE_URL+x}
if [ "$oidc_mode" = bundled ]; then
    if [ "$postgres_mode" = bundled ]; then
        [ -z "${keycloak_database_url_set:-}" ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL is not accepted with bundled PostgreSQL" >&2
            exit 64
        }
        keycloak_database_url=jdbc:postgresql://postgres:5432/keycloak
    else
        keycloak_database_url=${SYNVEDA_KEYCLOAK_DATABASE_URL:-}
        case "$keycloak_database_url" in
            jdbc:postgresql://*/*) ;;
            *)
                echo "compose: external PostgreSQL with bundled Keycloak requires SYNVEDA_KEYCLOAK_DATABASE_URL" >&2
                exit 64
                ;;
        esac
        case "$keycloak_database_url" in
            *@*|*'?'*|*'#'*)
                echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must be credential-free and contain no query or fragment" >&2
                exit 64
                ;;
        esac
        jdbc_location=${keycloak_database_url#jdbc:postgresql://}
        jdbc_host_port=${jdbc_location%%/*}
        jdbc_database=${jdbc_location#*/}
        [ "$jdbc_host_port" != "$jdbc_location" ] && \
            [ -n "$jdbc_database" ] && [ "$jdbc_database" = "${jdbc_database##*/}" ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must identify one host, port and database" >&2
            exit 64
        }
        jdbc_host=${jdbc_host_port%:*}
        jdbc_port=${jdbc_host_port##*:}
        jdbc_database_valid=true
        case "$jdbc_database" in
            [A-Za-z0-9_]*) ;;
            *) jdbc_database_valid=false ;;
        esac
        case "$jdbc_database" in
            *[!A-Za-z0-9_-]*) jdbc_database_valid=false ;;
            *) ;;
        esac
        [ "$jdbc_host" != "$jdbc_host_port" ] && valid_host "$jdbc_host" && \
            valid_runtime_id "$jdbc_port" && [ "$jdbc_port" -le 65535 ] && \
            [ "${#jdbc_database}" -le 63 ] && [ "$jdbc_database_valid" = true ] || {
            echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL must identify one canonical DNS host, port and database" >&2
            exit 64
        }
        [ "$pg_database" = postgres ] && [ "$jdbc_database" = keycloak ] && \
            [ "$pg_host" = "$jdbc_host" ] && [ "$pg_port" = "$jdbc_port" ] || {
            echo "compose: Keycloak bootstrap and JDBC settings must use one endpoint, with postgres and keycloak databases" >&2
            exit 64
        }
    fi
else
    [ -z "${keycloak_database_url_set:-}" ] || {
        echo "compose: SYNVEDA_KEYCLOAK_DATABASE_URL is not accepted with external OIDC" >&2
        exit 64
    }
fi

compose_version=$("$docker_bin" compose version --short 2>/dev/null) || {
    echo "compose: Docker Compose is required" >&2
    exit 69
}
version_numbers=$(printf '%s\n' "$compose_version" | sed -E 's/^[^0-9]*([0-9]+)\.([0-9]+)\.([0-9]+).*/\1 \2 \3/')
set -- $version_numbers
[ "$#" -eq 3 ] || {
    echo "compose: could not parse Docker Compose version" >&2
    exit 69
}
if [ "$1" -lt 2 ] || \
    { [ "$1" -eq 2 ] && [ "$2" -lt 33 ]; } || \
    { [ "$1" -eq 2 ] && [ "$2" -eq 33 ] && [ "$3" -lt 1 ]; }; then
    echo "compose: Docker Compose 2.33.1 or newer is required" >&2
    exit 69
fi

export SYNVEDA_COMPOSE_RUNTIME=$runtime
export SYNVEDA_POSTGRES_MODE=$postgres_mode
export SYNVEDA_OIDC_MODE=$oidc_mode
export SYNVEDA_RUNTIME_UID=$runtime_uid
export SYNVEDA_RUNTIME_GID=$runtime_gid
export SYNVEDA_COMPOSE_RESTART_POLICY=$restart_policy
export SYNVEDA_APP_HOST=$app_host
export SYNVEDA_PUBLIC_SCHEME=$public_scheme
export SYNVEDA_PUBLIC_PORT=$public_port
export SYNVEDA_PUBLIC_APP_URL=$public_app_url
if [ "$oidc_mode" = bundled ]; then
    export SYNVEDA_AUTH_HOST=$auth_host
    export SYNVEDA_PUBLIC_AUTH_URL=$public_auth_url
else
    unset SYNVEDA_AUTH_HOST SYNVEDA_PUBLIC_AUTH_URL
fi
export SYNVEDA_INSECURE_DEVELOPMENT_HTTP=$insecure_development_http
export SYNVEDA_OIDC_ISSUER=$oidc_issuer
export SYNVEDA_POSTGRES_BOOTSTRAP_URL=$postgres_bootstrap_url
export SYNVEDA_POSTGRES_BUNDLED_CLUSTER=$postgres_bundled_cluster
export SYNVEDA_DATABASE_EXPECTED_HOST=$database_expected_host
export SYNVEDA_DATABASE_EXPECTED_PORT=$database_expected_port
export SYNVEDA_DATABASE_EXPECTED_NAME=$database_expected_name
export SYNVEDA_DEV_HTTP_PORT=$public_port
export SYNVEDA_PROXY_HTTP_PORT=$proxy_http_port
export SYNVEDA_PROXY_HTTPS_PORT=$proxy_https_port
export SYNVEDA_RENDER_PUBLIC_EDGE_SUBNET=$public_edge_subnet
export SYNVEDA_RENDER_PUBLIC_EDGE_GATEWAY=$public_edge_gateway
export SYNVEDA_RENDER_APP_BACKEND_SUBNET=$app_backend_subnet
export SYNVEDA_RENDER_APP_BACKEND_GATEWAY=$app_backend_gateway
export SYNVEDA_RENDER_DATA_SUBNET=$synveda_data_subnet
export SYNVEDA_RENDER_DATA_GATEWAY=$synveda_data_gateway
export SYNVEDA_RENDER_KEYCLOAK_DATA_SUBNET=$keycloak_data_subnet
export SYNVEDA_RENDER_KEYCLOAK_DATA_GATEWAY=$keycloak_data_gateway
export SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_SUBNET=$keycloak_management_subnet
export SYNVEDA_RENDER_KEYCLOAK_MANAGEMENT_GATEWAY=$keycloak_management_gateway
export SYNVEDA_RENDER_IDENTITY_SUBNET=$identity_subnet
export SYNVEDA_RENDER_IDENTITY_GATEWAY=$identity_gateway
export SYNVEDA_RENDER_IDENTITY_DYNAMIC_RANGE=$identity_dynamic_range
export SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS=$proxy_identity_address
export SYNVEDA_RENDER_TELEMETRY_SUBNET=$telemetry_subnet
export SYNVEDA_RENDER_TELEMETRY_GATEWAY=$telemetry_gateway
export SYNVEDA_RENDER_APPLICATION_EGRESS_SUBNET=$application_egress_subnet
export SYNVEDA_RENDER_APPLICATION_EGRESS_GATEWAY=$application_egress_gateway
export SYNVEDA_RENDER_IDENTITY_EGRESS_SUBNET=$identity_egress_subnet
export SYNVEDA_RENDER_IDENTITY_EGRESS_GATEWAY=$identity_egress_gateway
export SYNVEDA_RENDER_TELEMETRY_EGRESS_SUBNET=$telemetry_egress_subnet
export SYNVEDA_RENDER_TELEMETRY_EGRESS_GATEWAY=$telemetry_egress_gateway
export SYNVEDA_CADDY_APP_CONFIG=$caddy_app_config
export SYNVEDA_CADDY_IDENTITY_CONFIG=$caddy_identity_config
export SYNVEDA_SECRETS_DIR=$secret_dir
export SYNVEDA_OIDC_DIRECTORY_SECRETS_DIR=$oidc_directory_secret_dir
export SYNVEDA_OIDC_ISSUERS_FILE=$issuer_file
export SYNVEDA_DATABASE_ROLES_FILE=$database_roles_file
export SYNVEDA_DATABASE_AUTHORITY_DIR=$database_authority_dir
export SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR=$keycloak_public_gate_dir
export SYNVEDA_PRODUCT_IMAGE=$product_image
export SYNVEDA_POSTGRES_IMAGE=$postgres_image
export SYNVEDA_KEYCLOAK_IMAGE=$keycloak_image
export SYNVEDA_KEYCLOAK_DATABASE_URL=$keycloak_database_url
export SYNVEDA_KEYCLOAK_SSL_REQUIRED=$keycloak_ssl_required
export SYNVEDA_CADDY_IMAGE=$caddy_image
export SYNVEDA_OTEL_COLLECTOR_IMAGE=$otel_image

set -- compose --project-directory "$compose_dir" \
    --env-file "$compose_dir/.env.example" -p "$project" \
    -f "$compose_dir/compose.yaml" -f "$compose_dir/compose.$runtime_overlay.yaml"
if [ "$runtime" = development ] && [ "$postgres_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.postgres.dev.yaml"
fi
if [ "$runtime" = development ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak.dev.yaml"
fi
if [ "$postgres_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.postgres.yaml"
fi
if [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak.yaml"
fi
if [ "$postgres_mode" = bundled ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak-postgres.yaml"
fi
if [ "$postgres_mode" = external ] && [ "$oidc_mode" = bundled ]; then
    set -- "$@" -f "$compose_dir/compose.keycloak-external-postgres.yaml"
fi
if [ "$postgres_mode" = external ]; then
    set -- "$@" -f "$compose_dir/compose.external-postgres.yaml"
fi
if [ "$postgres_mode" = external ] || [ "$oidc_mode" = external ]; then
    set -- "$@" -f "$compose_dir/compose.external.yaml"
fi
old_ifs=$IFS
IFS=,
for profile in $profiles; do
    [ -z "$profile" ] || set -- "$@" --profile "$profile"
done
IFS=$old_ifs

if [ -n "$output" ]; then
    set -- "$@" config --format json --output "$output"
else
    set -- "$@" config --quiet
fi
"$docker_bin" "$@"
echo "canonical Compose configuration valid for $project ($postgres_mode PostgreSQL, $oidc_mode OIDC)"
