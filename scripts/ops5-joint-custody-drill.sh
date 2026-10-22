#!/bin/sh
# OPS-5 local two-point rehearsal of the PITR custody and restore tooling.
# It uses the opt-in pgBackRest image, a pinned local TLS S3 server, the real
# product schema, tenant keys and audit chain through the CLI image, a stand-in
# identity database in the same cluster, and the lifecycle's own planner and
# restore wrapper. It does not start the gateway, Keycloak or browser, so it is
# not the public-API application drill.
#
#   OPS5_PITR_IMAGE   image built from deploy/compose/postgres/Dockerfile --target pitr
#   OPS5_CLI_IMAGE    image containing /usr/local/bin/synveda from this source
#   OPS5_DRILL_ROOT   new directory the Docker engine can bind-mount (default mktemp)
#   OPS5_DRILL_REPORT path for the JSON report (default $OPS5_DRILL_ROOT/report.json)
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
planner=$repo_root/deploy/compose/scripts/pitr-recovery.mjs
pitr_image=${OPS5_PITR_IMAGE:?set OPS5_PITR_IMAGE}
cli_image=${OPS5_CLI_IMAGE:?set OPS5_CLI_IMAGE}
s3_image=chrislusf/seaweedfs@sha256:ce9e796f1fe6f06968f4c04bdaf8f678dad9c8acdfef3d244133d71bfa6bf882
root=${OPS5_DRILL_ROOT:-$(mktemp -d /tmp/synveda-ops5-custody.XXXXXX)}
[ ! -e "$root/.started" ] || { echo "drill root was already used" >&2; exit 73; }
mkdir -p "$root" && chmod 700 "$root" && : > "$root/.started"
report=${OPS5_DRILL_REPORT:-$root/report.json}
id=$(basename "$root" | tr -cd 'a-z0-9' | tail -c 12)
net=ops5-custody-$id
s3=ops5-custody-s3-$id
source_pg=ops5-custody-source-$id
source_volume=ops5-custody-source-data-$id
targets=""
volumes="$source_volume"
cleanup() {
    for container in $s3 $source_pg $targets; do docker rm -f "$container" >/dev/null 2>&1 || true; done
    for volume in $volumes; do docker volume rm "$volume" >/dev/null 2>&1 || true; done
    docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

log() { printf '%s %s\n' "$(date -u '+%H:%M:%S')" "$*"; }
now() { date -u '+%s'; }
pgb_options="--config=/var/run/postgresql/pgbackrest.conf --stanza=synveda --repo1-type=s3 --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y --log-level-file=off"

mkdir -m 700 "$root/s3data" "$root/secrets" "$root/marks" "$root/custody" "$root/work"
cd "$root"
openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 1 -subj /CN=ops5-custody-ca \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
    -keyout ca.key -out ca.crt >/dev/null 2>&1
openssl req -newkey rsa:2048 -sha256 -nodes -subj /CN=s3-ops5 -keyout server.key -out server.csr >/dev/null 2>&1
printf 'subjectAltName=DNS:s3-ops5\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' > server.ext
openssl x509 -req -sha256 -days 1 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
    -extfile server.ext -out server.crt >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=other -keyout other.key -out other.crt >/dev/null 2>&1
chmod 644 ca.crt server.crt server.key other.crt
s3_secret=$(openssl rand -hex 16)
cipher=$(openssl rand -hex 32)
printf '{"identities":[{"name":"drill","credentials":[{"accessKey":"ops5custody","secretKey":"%s"}],"actions":["Admin","Read","List","Tagging","Write"]}]}\n' "$s3_secret" > s3.json
chmod 644 s3.json
write_config() {
    printf '[global]\nrepo1-type=s3\nrepo1-path=/synveda/custody-drill\nrepo1-s3-bucket=synveda-drill\nrepo1-s3-endpoint=s3-ops5\nrepo1-s3-region=us-east-1\nrepo1-s3-key=ops5custody\nrepo1-s3-key-secret=%s\nrepo1-s3-uri-style=path\nrepo1-storage-port=8334\nrepo1-storage-ca-file=/var/run/postgresql/pgbackrest-ca.pem\nrepo1-cipher-type=aes-256-cbc\nrepo1-cipher-pass=%s\nrepo1-storage-verify-tls=y\n[synveda]\npg1-path=/var/lib/postgresql/data\n' "$1" "$2" > "$3"
    chmod 600 "$3"
}
write_config "$s3_secret" "$cipher" pgbackrest.conf
write_config "expired-$s3_secret" "$cipher" expired.conf
write_config "$s3_secret" "$(openssl rand -hex 32)" wrongpass.conf
for file in pgbackrest.conf expired.conf wrongpass.conf; do chmod 644 "$file"; done

docker network create --internal "$net" >/dev/null
docker run --detach --name "$s3" --network "$net" --network-alias s3-ops5 \
    -v "$root/s3data:/data" -v "$root/server.crt:/certs/server.crt:ro" \
    -v "$root/server.key:/certs/server.key:ro" -v "$root/s3.json:/certs/s3.json:ro" \
    --entrypoint weed "$s3_image" mini -dir=/data -master.telemetry=false -s3 \
    -s3.config=/certs/s3.json -s3.port.https=8334 -s3.cert.file=/certs/server.crt \
    -s3.key.file=/certs/server.key >/dev/null
attempt=0
until docker logs "$s3" 2>&1 | grep -Eq 'S3[[:space:]]+ready'; do
    attempt=$((attempt + 1)); [ "$attempt" -lt 60 ] || { echo "S3 did not start" >&2; exit 69; }; sleep 1
done
printf 's3.bucket.create -name synveda-drill\n' | docker exec -i "$s3" weed shell -master=localhost:9333 -filer=localhost:8888 >/dev/null

owner_password=$(openssl rand -hex 32)
start_cluster() {
    name=$1; volume=$2; shift 2
    docker run --detach --name "$name" --network "$net" --network-alias postgres \
        --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
        -v "$volume:/var/lib/postgresql/data" -v "$root/pgbackrest.conf:/run/secrets/pgbackrest_conf:ro" \
        -v "$root/ca.crt:/run/secrets/pgbackrest_ca:ro" -e POSTGRES_USER=synveda_owner \
        -e POSTGRES_DB=postgres -e "POSTGRES_PASSWORD=$owner_password" \
        --entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint "$pitr_image" postgres "$@" >/dev/null
}
psql_in() { docker exec -u postgres "$1" psql -X -qAt -v ON_ERROR_STOP=1 -U synveda_owner -d "$2" -c "$3"; }
wait_primary() {
    attempt=0
    while [ "$attempt" -lt 180 ]; do
        [ "$(psql_in "$1" postgres 'select pg_is_in_recovery()' 2>/dev/null || true)" = f ] && return 0
        attempt=$((attempt + 1)); sleep 1
    done
    docker logs "$1" 2>&1 | tail -20 >&2
    return 69
}
docker volume create "$source_volume" >/dev/null
start_cluster "$source_pg" "$source_volume" -c archive_mode=on -c archive_timeout=10s \
    -c "archive_command=pgbackrest $pgb_options --pg1-path=/var/lib/postgresql/data archive-push %p"
wait_primary "$source_pg"
log "source cluster primary"

roles='{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["synveda_owner"],"administrative_memberships":[],"forbidden_databases":["keycloak","postgres","template1"],"isolated_peer_roles":["keycloak"]}'
printf '%s\n' "$roles" > roles.json
chmod 644 roles.json
for role in migrator gateway worker keycloak; do openssl rand -hex 32 > "secrets/${role}_password"; done
psql_in "$source_pg" postgres "create role keycloak login password '$(cat secrets/keycloak_password)'"
psql_in "$source_pg" postgres 'create database keycloak owner keycloak'
docker run --rm --network "$net" -e SYNVEDA_DATABASE_ROLES_FILE=/roles.json -v "$root/roles.json:/roles.json:ro" \
    --entrypoint /usr/local/bin/synveda "$cli_image" db setup-sql > setup.sql
docker exec -i -u postgres "$source_pg" psql -X -q -v ON_ERROR_STOP=1 -U synveda_owner -d postgres < setup.sql >/dev/null
for role in migrator gateway worker; do
    psql_in "$source_pg" postgres "alter role synveda_$role password '$(cat "secrets/${role}_password")'"
    printf 'postgres://synveda_%s:%s@postgres:5432/synveda\n' "$role" "$(cat "secrets/${role}_password")" > "secrets/${role}_url"
done
chmod 644 secrets/*
docker run --rm --entrypoint /usr/local/bin/synveda "$cli_image" kms keygen 2>/dev/null > secrets/kms_key
docker run --rm --entrypoint /usr/local/bin/synveda "$cli_image" kms keygen 2>/dev/null > secrets/kms_key_new
docker run --rm --entrypoint /usr/local/bin/synveda "$cli_image" kms keygen 2>/dev/null > secrets/wrong_kms_key
docker run --rm --entrypoint /usr/local/bin/synveda "$cli_image" kms keygen 2>/dev/null > secrets/custodian_key
docker run --rm --entrypoint /usr/local/bin/synveda "$cli_image" kms keygen 2>/dev/null > secrets/wrong_custodian_key
printf 'local:ops5-drill-1\n' > secrets/kms_key_ref
printf 'local:ops5-drill-2\n' > secrets/kms_key_new_ref
chmod 644 secrets/*

# role, key file, key ref file, then CLI arguments
cli() {
    cli_role=$1; cli_key=$2; cli_ref=$3; shift 3
    docker run --rm --network "$net" -e DATABASE_URL_FILE=/secrets/${cli_role}_url \
        -e SYNVEDA_DATABASE_ROLES_FILE=/roles.json -e SYNVEDA_DATABASE_REQUIRED_PEER=keycloak \
        -e SYNVEDA_KMS_KEY_FILE=/secrets/$cli_key -e SYNVEDA_KMS_KEY_REF_FILE=/secrets/$cli_ref \
        -e SYNVEDA_KMS_PREVIOUS_KEY_FILE=/secrets/kms_key -e SYNVEDA_KMS_PREVIOUS_KEY_REF_FILE=/secrets/kms_key_ref \
        -v "$root/secrets:/secrets:ro" -v "$root/roles.json:/roles.json:ro" \
        --entrypoint /usr/local/bin/synveda "$cli_image" "$@"
}
cli migrator kms_key kms_key_ref db migrate >/dev/null
tenant_json=$(cli migrator kms_key kms_key_ref tenant create --slug ops5-a --name "OPS-5 A")
tenant_a=$(printf '%s' "$tenant_json" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).id))')
cli migrator kms_key kms_key_ref tenant key rotate --tenant "$tenant_a" >/dev/null
psql_in "$source_pg" keycloak 'create table identity_marks (id integer primary key, written_at timestamptz not null default now())'
docker exec -u postgres "$source_pg" psql -X -q -U synveda_owner -d keycloak -c 'alter table identity_marks owner to keycloak' >/dev/null
log "product schema, tenant A and identity database written"

for step in stanza-create check "--no-expire-auto --type=full backup"; do
    # shellcheck disable=SC2086
    docker exec -u postgres "$source_pg" pgbackrest $pgb_options --pg1-path=/var/lib/postgresql/data \
        --pg1-user=synveda_owner $step \
        > work/pgbackrest-step.log 2>&1 || { cat work/pgbackrest-step.log >&2; exit 69; }
done
log "encrypted full base backup taken"

metadata_sql=$(awk '/^pitr_metadata_sql="/{f=1; sub(/^pitr_metadata_sql="/, "")} f{if ($0 ~ /"$/) {sub(/"$/, ""); print; exit} print}' \
    "$repo_root/deploy/compose/scripts/compose.sh")
docker exec -u postgres "$source_pg" pgbackrest $pgb_options --log-level-console=off --output=json info > work/info.json
psql_in "$source_pg" synveda "$metadata_sql" > work/metadata
mkdir -m 700 header entries
cp pgbackrest.conf work/source.conf && chmod 600 work/source.conf
node "$planner" binding --info work/info.json --metadata work/metadata --config work/source.conf --ca true \
    --project synveda-reference-acceptance-drillsrc --backup-id gen-1 --tenant-id "$tenant_a" \
    --postgres-image "$pitr_image" --product-image "$cli_image" --custodian ops5-drill-custodian \
    --created-at "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" --output header/header.json >/dev/null
install -m 600 secrets/kms_key entries/synveda_kms_key
install -m 600 secrets/kms_key_ref entries/synveda_kms_key_ref
openssl rand -hex 32 > entries/keycloak_convergence_admin_password
install -m 600 secrets/keycloak_password entries/keycloak_database_password
printf '%s\n' "$owner_password" > entries/postgres_owner_password
for role in migrator gateway worker; do
    install -m 600 "secrets/${role}_password" "entries/synveda_${role}_password"
    install -m 600 "secrets/${role}_url" "entries/synveda_${role}_database_url"
done
printf '%s\n' "$cipher" > entries/pgbackrest_cipher_pass
chmod 600 entries/*
seal_started=$(now)
docker run --rm --user "$(id -u):$(id -g)" -e SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/key -v "$root/secrets/custodian_key:/key:ro" \
    -v "$root/header:/header:ro" -v "$root/entries:/entries:ro" -v "$root/custody:/output" \
    --entrypoint /usr/local/bin/synveda "$cli_image" kms custody seal --header /header/header.json \
    --input-dir /entries --output /output/custody.svrc > work/seal.json
rm -f entries/*
log "custody bundle sealed ($(wc -c < custody/custody.svrc) bytes); plaintext entries removed"

wait_archived() {
    attempt=0
    while [ "$attempt" -lt 120 ]; do
        archived=$(psql_in "$source_pg" postgres "select coalesce(last_archived_wal, '') from pg_stat_archiver")
        if [ -n "$archived" ] && [ "$(printf '%s\n%s\n' "$1" "$archived" | LC_ALL=C sort | tail -n 1)" = "$archived" ]; then
            return 0
        fi
        attempt=$((attempt + 1)); sleep 1
    done
    echo "WAL $1 was not archived" >&2
    exit 69
}
mark() {
    mark_row=$(psql_in "$source_pg" postgres "select lsn::text || '|' || pg_walfile_name(lsn) from (select pg_create_restore_point('$1') as lsn) m")
    mark_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
    psql_in "$source_pg" postgres 'select pg_switch_wal()' >/dev/null
    wait_archived "${mark_row#*|}"
    node "$planner" mark --project synveda-reference-acceptance-drillsrc --name "$1" --lsn "${mark_row%%|*}" \
        --wal "${mark_row#*|}" --created-at "$mark_at" --output "marks/$1.json" >/dev/null
}
audit_digest() { psql_in "$1" synveda "select count(*) || ':' || encode(sha256(string_agg(row_to_json(a)::text, chr(10) order by seq)::bytea), 'hex') from (select * from audit_log where tenant_id = '$tenant_a' and seq <= $2) a"; }

psql_in "$source_pg" keycloak 'insert into identity_marks (id) values (1)'
head_1=$(psql_in "$source_pg" synveda "select max(seq) from audit_log where tenant_id = '$tenant_a'")
prefix_1=$(audit_digest "$source_pg" "$head_1")
mark point-1
log "point-1 recorded: tenant A audit head $head_1"
tenant_b=$(cli migrator kms_key kms_key_ref tenant create --slug ops5-b --name "OPS-5 B" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).id))')
cli migrator kms_key kms_key_ref tenant key rotate --tenant "$tenant_a" >/dev/null
psql_in "$source_pg" keycloak 'insert into identity_marks (id) values (2)'
head_2=$(psql_in "$source_pg" synveda "select max(seq) from audit_log where tenant_id = '$tenant_a'")
mark point-2
log "point-2 recorded: tenant B admitted, tenant A audit head $head_2"
psql_in "$source_pg" keycloak 'insert into identity_marks (id) values (3)'
row_3_wal=$(psql_in "$source_pg" postgres 'select pg_walfile_name(pg_current_wal_lsn())')
psql_in "$source_pg" postgres 'select pg_switch_wal()' >/dev/null
wait_archived "$row_3_wal"
archived_through=$(psql_in "$source_pg" postgres "select last_archived_wal || '|' || to_char(last_archived_time at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') from pg_stat_archiver")
psql_in "$source_pg" keycloak 'insert into identity_marks (id) values (4)'
lost_write_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
docker kill "$source_pg" >/dev/null
docker rm "$source_pg" >/dev/null
log "source storage removed after an unarchived write (archive through ${archived_through%%|*})"

repository() {
    repository_config=$1; repository_ca=$2; shift 2
    docker run --rm --network "$net" --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
        -v "$root/$repository_config:/run/secrets/pgbackrest_conf:ro" -v "$root/$repository_ca:/run/secrets/pgbackrest_ca:ro" \
        --entrypoint /usr/local/bin/synveda-pgbackrest-repository "$pitr_image" "$@"
}
open_custody() {
    rm -rf "$root/opened" && mkdir -m 700 "$root/opened"
    docker run --rm --user "$(id -u):$(id -g)" -e SYNVEDA_RECOVERY_CUSTODY_KEY_FILE=/key \
        -v "$root/secrets/$1:/key:ro" -v "$root/custody:/bundle:ro" -v "$root/opened:/opened" \
        --entrypoint /usr/local/bin/synveda "$cli_image" kms custody open --input /bundle/custody.svrc --output-dir /opened >/dev/null
}
plan_restore() {
    node "$planner" plan --opened-dir opened --info "work/$1-info.json" --verify "work/$1-verify.txt" \
        --listing "work/$1-listing.json" --config work/source.conf --ca true --target "$2" --marks-dir marks \
        --project synveda-reference-acceptance-drillsrc --backup-id gen-1 --tenant-id "$tenant_a" \
        --postgres-image "$pitr_image" --product-image "$cli_image"
}
read_repository() {
    repository "$2" ca.crt info > "work/$1-info.json"
    archive_id=$(node "$planner" repository-check --info "work/$1-info.json")
    repository "$2" ca.crt verify > "work/$1-verify.txt" 2> "work/$1-verify.err" || \
        echo "verify exit $?" >> "work/$1-verify.txt"
    repository "$2" ca.crt archive-list "$archive_id" > "work/$1-listing.json"
}
restore_target() {
    target_label=$1; recovery_target=$2
    target=ops5-custody-$target_label-$id; volume=ops5-custody-$target_label-data-$id
    targets="$targets $target"; volumes="$volumes $volume"
    started=$(now)
    open_custody custodian_key
    read_repository "$target_label" pgbackrest.conf
    decided=$(plan_restore "$target_label" "$recovery_target")
    set -- $(printf '%s' "$decided" | tr '|' ' ')
    docker volume create "$volume" >/dev/null
    if [ "$2" = name ]; then
        docker run --rm --network "$net" --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
            -v "$volume:/var/lib/postgresql/data" -v "$root/pgbackrest.conf:/run/secrets/pgbackrest_conf:ro" \
            -v "$root/ca.crt:/run/secrets/pgbackrest_ca:ro" --entrypoint /usr/local/bin/synveda-pgbackrest-repository \
            "$pitr_image" restore "$1" name "$3" >/dev/null 2>&1
    else
        docker run --rm --network "$net" --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
            -v "$volume:/var/lib/postgresql/data" -v "$root/pgbackrest.conf:/run/secrets/pgbackrest_conf:ro" \
            -v "$root/ca.crt:/run/secrets/pgbackrest_ca:ro" --entrypoint /usr/local/bin/synveda-pgbackrest-repository \
            "$pitr_image" restore "$1" default >/dev/null 2>&1
    fi
    start_cluster "$target" "$volume" -c archive_mode=off
    wait_primary "$target"
    promoted=$(now)
    cli migrator kms_key kms_key_ref db migrate --check >/dev/null
    verified=$(cli gateway kms_key kms_key_ref db recovery-verify --tenant "$tenant_a")
    cli gateway wrong_kms_key kms_key_ref db recovery-verify --tenant "$tenant_a" --expect-key-refusal >/dev/null
    finished=$(now)
    rto_database=$((promoted - started)); rto_verified=$((finished - started))
}

restore_target point1 name:point-1
tenants_1=$(psql_in "$target" synveda 'select count(*) from tenants')
identity_1=$(psql_in "$target" keycloak "select string_agg(id::text, ',' order by id) from identity_marks")
restored_prefix_1=$(audit_digest "$target" "$head_1")
[ "$tenants_1" = 1 ] && [ "$identity_1" = 1 ] && [ "$restored_prefix_1" = "$prefix_1" ] || {
    echo "point-1 restore did not reproduce point-1 state: tenants=$tenants_1 identity=$identity_1" >&2; exit 1; }
cli migrator kms_key kms_key_ref tenant key rotate --tenant "$tenant_a" >/dev/null
appended=$(cli gateway kms_key kms_key_ref db recovery-verify --tenant "$tenant_a")
point1_rto=$rto_verified; point1_db=$rto_database; point1_verify=$verified; point1_append=$appended
docker rm -f "$target" >/dev/null
log "point-1 restored: 1 tenant, identity row 1, frozen audit prefix equal, new audit append verified"

restore_target point2 name:point-2
tenants_2=$(psql_in "$target" synveda 'select count(*) from tenants')
identity_2=$(psql_in "$target" keycloak "select string_agg(id::text, ',' order by id) from identity_marks")
[ "$tenants_2" = 2 ] && [ "$identity_2" = "1,2" ] && [ "$(audit_digest "$target" "$head_1")" = "$prefix_1" ] || {
    echo "point-2 restore did not reproduce point-2 state" >&2; exit 1; }
point2_rto=$rto_verified; point2_verify=$verified
docker rm -f "$target" >/dev/null
log "point-2 restored: 2 tenants, identity rows 1,2"

restore_target latest latest
tenants_l=$(psql_in "$target" synveda 'select count(*) from tenants')
identity_l=$(psql_in "$target" keycloak "select string_agg(id::text, ',' order by id) from identity_marks")
[ "$tenants_l" = 2 ] && [ "$identity_l" = "1,2,3" ] || {
    echo "latest restore did not reproduce the archived state: tenants=$tenants_l identity=$identity_l" >&2; exit 1; }
latest_rto=$rto_verified
cli gateway kms_key_new kms_key_new_ref kms rewrap > work/rewrap.json
cli gateway kms_key_new kms_key_new_ref db recovery-verify --tenant "$tenant_a" > work/rewrap-verify.json
if cli gateway kms_key kms_key_ref db recovery-verify --tenant "$tenant_a" > work/stale.txt 2>&1; then
    echo "the retiring KEK still opened a rewrapped generation" >&2; exit 1
fi
docker rm -f "$target" >/dev/null
log "latest restored: identity row 4 (unarchived) absent; KEK replaced on the restored copy; retiring KEK refused"

# Fault matrix: each refusal happens before any target volume exists.
faults=""
expect_refusal() {
    fault=$1; pattern=$2; shift 2
    if "$@" > "work/fault-$fault.out" 2> "work/fault-$fault.err"; then
        echo "fault $fault was not refused" >&2; exit 1
    fi
    grep -Eq "$pattern" "work/fault-$fault.err" || { echo "fault $fault refused for an unexpected reason" >&2; cat "work/fault-$fault.err" >&2; exit 1; }
    if grep -Eq "$s3_secret|$cipher" "work/fault-$fault.out" "work/fault-$fault.err"; then
        echo "fault $fault output exposed a secret" >&2; exit 1
    fi
    faults="$faults\"$fault\","
}
open_custody custodian_key
expect_refusal wrong-custodian-key 'did not open under this custodian key' open_custody wrong_custodian_key
open_custody custodian_key
repository wrongpass.conf ca.crt info > work/wrongpass-info.json
expect_refusal wrong-passphrase 'cipher passphrase is wrong' node "$planner" repository-check --info work/wrongpass-info.json
repository expired.conf ca.crt info > work/expired-info.json
expect_refusal expired-credential 'renew the expired or revoked' node "$planner" repository-check --info work/expired-info.json
repository pgbackrest.conf other.crt info > work/untrusted-info.json
expect_refusal untrusted-ca 'TLS certificate was not trusted' node "$planner" repository-check --info work/untrusted-info.json
expect_refusal unrecorded-target 'is not recorded' plan_restore latest name:never-marked

segment_path=$(node -e '
const l = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
const m = JSON.parse(require("fs").readFileSync(process.argv[2], "utf8"));
const names = Object.keys(l).filter((k) => /\/[0-9A-F]{24}-/.test(k)).sort();
const after = names.filter((k) => k.split("/")[1].slice(0, 24) > m.wal);
process.stdout.write(after[0]);' work/latest-listing.json marks/point-1.json)
printf 'fs.rm /buckets/synveda-drill/synveda/custody-drill/archive/synveda/%s/%s\n' "$archive_id" "$segment_path" | \
    docker exec -i "$s3" weed shell -master=localhost:9333 -filer=localhost:8888 >/dev/null
read_repository gap pgbackrest.conf
expect_refusal missing-wal 'archived WAL is incomplete' plan_restore gap latest
plan_restore gap name:point-1 > work/gap-point1.txt
corrupt_path=$(node -e '
const l = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
process.stdout.write(Object.keys(l).filter((k) => /\/[0-9A-F]{24}-/.test(k)).sort()[0]);' work/gap-listing.json)
docker exec "$s3" sh -c "printf corrupt > /tmp/corrupt && curl -s -o /dev/null -X PUT --data-binary @/tmp/corrupt http://localhost:8888/buckets/synveda-drill/synveda/custody-drill/archive/synveda/$archive_id/$corrupt_path"
read_repository corrupt pgbackrest.conf
expect_refusal corrupt-object 'corrupt or unreadable' plan_restore corrupt name:point-1

interrupted=ops5-custody-interrupted-data-$id
volumes="$volumes $interrupted"
docker volume create "$interrupted" >/dev/null
docker run --detach --name "ops5-custody-interrupt-$id" --network "$net" \
    --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m -v "$interrupted:/var/lib/postgresql/data" \
    -v "$root/pgbackrest.conf:/run/secrets/pgbackrest_conf:ro" -v "$root/ca.crt:/run/secrets/pgbackrest_ca:ro" \
    --entrypoint /usr/local/bin/synveda-pgbackrest-repository "$pitr_image" restore \
    "$(node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync("opened/header.json","utf8")).backup.label)')" default >/dev/null
targets="$targets ops5-custody-interrupt-$id"
sleep 3
docker kill "ops5-custody-interrupt-$id" >/dev/null 2>&1 || true
docker rm -f "ops5-custody-interrupt-$id" >/dev/null 2>&1 || true
expect_refusal interrupted-restore-reuse 'not empty; restore never overwrites' docker run --rm --network "$net" \
    --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m -v "$interrupted:/var/lib/postgresql/data" \
    -v "$root/pgbackrest.conf:/run/secrets/pgbackrest_conf:ro" -v "$root/ca.crt:/run/secrets/pgbackrest_ca:ro" \
    --entrypoint /usr/local/bin/synveda-pgbackrest-repository "$pitr_image" restore 20000101-000000F default
log "fault matrix refused: ${faults%,}"

timeline_two=$(node -e '
const l = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
process.stdout.write(String(Object.keys(l).filter((k) => /^00000002/.test(k.split("/").pop())).length));' work/corrupt-listing.json)
[ "$timeline_two" = 0 ] || { echo "a restored target archived into the source repository" >&2; exit 1; }

printf '%s' "$point1_verify" > work/point1-verify.json
printf '%s' "$point1_append" > work/point1-append.json
printf '%s' "$point2_verify" > work/point2-verify.json
D_PITR_IMAGE=$pitr_image D_CLI_IMAGE=$cli_image D_BUNDLE_BYTES=$(wc -c < custody/custody.svrc) \
D_RTO1=$point1_rto D_DB1=$point1_db D_RTO2=$point2_rto D_RTOL=$latest_rto \
D_ARCHIVED=$archived_through D_LOST=$lost_write_at D_FAULTS=${faults%,} \
node -e '
const [report, verify1, append1, verify2, rewrap, rewrapVerify, stale, seal] = process.argv.slice(1);
const fs = require("fs");
const json = (path) => JSON.parse(fs.readFileSync(path, "utf8"));
const value = (name) => process.env[name];
const result = {
  format: "synveda-ops5-custody-drill-v1",
  scope: "local two-point PITR custody and restore tooling; no gateway, Keycloak or public API",
  postgresql_image: value("D_PITR_IMAGE"),
  cli_image: value("D_CLI_IMAGE"),
  repository: "local seaweedfs over verified private-CA TLS on an isolated Docker network",
  custody: { bundle_bytes: Number(value("D_BUNDLE_BYTES")), sealed: json(seal) },
  points: {
    "point-1": { tenants: 1, identity_rows: "1", audit_prefix_equal: true, recovery_verify: json(verify1), after_new_append: json(append1), seconds_to_verified: Number(value("D_RTO1")), seconds_to_promoted: Number(value("D_DB1")) },
    "point-2": { tenants: 2, identity_rows: "1,2", audit_prefix_equal: true, recovery_verify: json(verify2), seconds_to_verified: Number(value("D_RTO2")) },
    latest: { tenants: 2, identity_rows: "1,2,3", unarchived_identity_row_4_absent: true, seconds_to_verified: Number(value("D_RTOL")) },
  },
  rpo: { archived_through: value("D_ARCHIVED"), unarchived_write_at: value("D_LOST"), definition: "loss window = commits after the last archived WAL segment at the loss instant; archive_timeout bounds it" },
  rto_definition: "first restore command (custody open) to restored cluster promoted and recovery-verify passed; excludes fetching custody, application start and browser login",
  kek_replacement: { rewrap: json(rewrap), verify_with_replacement: json(rewrapVerify), retiring_kek_refused: fs.readFileSync(stale, "utf8").trim() },
  faults_refused_before_target_state: value("D_FAULTS").split(",").map((name) => name.replaceAll(String.fromCharCode(34), "")),
  restored_targets_archived_to_source: 0,
};
fs.writeFileSync(report, `${JSON.stringify(result, null, 2)}\n`);
' "$report" work/point1-verify.json work/point1-append.json work/point2-verify.json work/rewrap.json \
    work/rewrap-verify.json work/stale.txt work/seal.json
log "report written to $report"
