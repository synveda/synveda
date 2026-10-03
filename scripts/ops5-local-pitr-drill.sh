#!/bin/sh
# OPS-5 local mechanics only. This isolated S3 server and disposable database
# exercise pgBackRest archive, restore points and wrong-key refusal; they do
# not qualify off-host custody or the Synveda application recovery contract.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
drill_image=synveda/postgres-pitr:ops5-local-drill
drill_s3_image=chrislusf/seaweedfs@sha256:ce9e796f1fe6f06968f4c04bdaf8f678dad9c8acdfef3d244133d71bfa6bf882
docker build --quiet --target pitr -f "$repo_root/deploy/compose/postgres/Dockerfile" \
    -t "$drill_image" "$repo_root" >/dev/null

drill_root=$(mktemp -d /tmp/synveda-ops5-drill.XXXXXX)
drill_id=${drill_root##*.}
drill_network=synveda-ops5-drill-$drill_id
drill_s3=synveda-ops5-s3-$drill_id
drill_pg=synveda-ops5-pg-$drill_id
drill_pg_volume=synveda-ops5-pgdata-$drill_id
drill_before=synveda-ops5-before-$drill_id
drill_after=synveda-ops5-after-$drill_id
drill_before_volume=synveda-ops5-before-data-$drill_id
drill_after_volume=synveda-ops5-after-data-$drill_id
drill_wrong_volume=synveda-ops5-wrong-data-$drill_id
cleanup() {
    docker rm -f "$drill_pg" "$drill_s3" "$drill_before" "$drill_after" >/dev/null 2>&1 || true
    docker volume rm "$drill_pg_volume" "$drill_before_volume" "$drill_after_volume" "$drill_wrong_volume" >/dev/null 2>&1 || true
    docker network rm "$drill_network" >/dev/null 2>&1 || true
    rm -rf -- "$drill_root"
}
trap cleanup EXIT HUP INT TERM

mkdir -m 0700 "$drill_root/s3data"
openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 1 \
    -subj /CN=synveda-ops5-test-ca \
    -addext basicConstraints=critical,CA:TRUE \
    -addext keyUsage=critical,keyCertSign,cRLSign \
    -keyout "$drill_root/ca.key" -out "$drill_root/ca.crt" \
    >/dev/null 2>&1
openssl req -newkey rsa:2048 -sha256 -nodes \
    -subj /CN=s3-ops5 -keyout "$drill_root/server.key" \
    -out "$drill_root/server.csr" >/dev/null 2>&1
cat > "$drill_root/server.ext" <<'EOF'
subjectAltName=DNS:s3-ops5
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
EOF
openssl x509 -req -sha256 -days 1 -in "$drill_root/server.csr" \
    -CA "$drill_root/ca.crt" -CAkey "$drill_root/ca.key" \
    -CAcreateserial -extfile "$drill_root/server.ext" \
    -out "$drill_root/server.crt" >/dev/null 2>&1
chmod 0600 "$drill_root/ca.key" "$drill_root/ca.crt" \
    "$drill_root/server.key" "$drill_root/server.crt"
cat > "$drill_root/s3.json" <<'EOF'
{"identities":[{"name":"drill","credentials":[{"accessKey":"ops5drill","secretKey":"ops5drillsecret"}],"actions":["Admin","Read","List","Tagging","Write"]}]}
EOF
chmod 0600 "$drill_root/s3.json"

docker network create --internal "$drill_network" >/dev/null
docker run --detach --name "$drill_s3" --network "$drill_network" \
    --network-alias s3-ops5 \
    --mount "type=bind,src=$drill_root/s3data,dst=/data" \
    --mount "type=bind,src=$drill_root/server.crt,dst=/certs/server.crt,readonly" \
    --mount "type=bind,src=$drill_root/server.key,dst=/certs/server.key,readonly" \
    --mount "type=bind,src=$drill_root/s3.json,dst=/certs/s3.json,readonly" \
    --entrypoint weed "$drill_s3_image" \
    mini -dir=/data -master.telemetry=false -s3 \
    -s3.config=/certs/s3.json -s3.port.https=8334 \
    -s3.cert.file=/certs/server.crt -s3.key.file=/certs/server.key \
    >/dev/null

ready=false
attempt=0
while [ "$attempt" -lt 40 ]; do
    if docker logs "$drill_s3" 2>&1 | rg -q 'S3[[:space:]]+ready'; then
        ready=true
        break
    fi
    attempt=$((attempt + 1))
    sleep 1
done
if [ "$ready" != true ]; then
    docker logs "$drill_s3" >&2
    exit 1
fi
docker run --rm --network "$drill_network" \
    --mount "type=bind,src=$drill_root/ca.crt,dst=/ca.crt,readonly" \
    --entrypoint sh "$drill_image" -c \
    'printf "" | openssl s_client -connect s3-ops5:8334 -servername s3-ops5 -CAfile /ca.crt -verify_return_error 2>&1 | grep -q "Verify return code: 0 (ok)"'
printf 's3.bucket.create -name synveda-drill\n' |
    docker exec -i "$drill_s3" weed shell -master=localhost:9333 -filer=localhost:8888
echo 'local S3 bucket created over isolated network'

cat > "$drill_root/pgbackrest.conf" <<'EOF'
[global]
repo1-type=s3
repo1-path=/synveda/local-drill
repo1-s3-bucket=synveda-drill
repo1-s3-endpoint=s3-ops5
repo1-s3-region=us-east-1
repo1-s3-key=ops5drill
repo1-s3-key-secret=ops5drillsecret
repo1-s3-uri-style=path
repo1-storage-port=8334
repo1-storage-ca-file=/var/run/postgresql/pgbackrest-ca.pem
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
repo1-storage-verify-tls=y
[synveda]
pg1-path=/var/lib/postgresql/data
EOF
chmod 0600 "$drill_root/pgbackrest.conf"
docker volume create "$drill_pg_volume" >/dev/null
docker run --detach --name "$drill_pg" --network "$drill_network" \
    --read-only --cap-drop=ALL --cap-add=CHOWN --cap-add=DAC_OVERRIDE \
    --cap-add=FOWNER --cap-add=KILL --cap-add=SETGID --cap-add=SETUID \
    --tmpfs /tmp:rw,noexec,nosuid,nodev,mode=1777,size=64m \
    --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
    --mount "type=volume,src=$drill_pg_volume,dst=/var/lib/postgresql/data" \
    --mount "type=bind,src=$drill_root/pgbackrest.conf,dst=/run/secrets/pgbackrest_conf,readonly" \
    --mount "type=bind,src=$drill_root/ca.crt,dst=/run/secrets/pgbackrest_ca,readonly" \
    -e POSTGRES_PASSWORD=drill-only-password \
    --entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint \
    "$drill_image" postgres \
    -c archive_mode=on -c archive_timeout=10s \
    -c "archive_command=pgbackrest --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda --pg1-path=/var/lib/postgresql/data --repo1-type=s3 --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y --log-level-file=off archive-push %p" \
    >/dev/null
ready=false
attempt=0
while [ "$attempt" -lt 40 ]; do
    if docker logs "$drill_pg" 2>&1 | rg -q 'PostgreSQL init process complete' &&
        docker exec "$drill_pg" pg_isready -U postgres >/dev/null 2>&1; then
        ready=true
        break
    fi
    attempt=$((attempt + 1))
    sleep 1
done
if [ "$ready" != true ]; then
    docker logs "$drill_pg" >&2
    exit 1
fi
docker exec -u postgres "$drill_pg" sh -c \
    'test -r /var/run/postgresql/pgbackrest-ca.pem && openssl x509 -in /var/run/postgresql/pgbackrest-ca.pem -noout && echo postgres-ca-readable'
docker exec -u postgres "$drill_pg" pgbackrest \
    --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
    --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
    --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y \
    --log-level-file=off stanza-create
docker exec -u postgres "$drill_pg" pgbackrest \
    --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
    --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
    --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y \
    --log-level-file=off check
echo 'local S3 stanza and WAL check passed'
docker exec -u postgres "$drill_pg" psql -v ON_ERROR_STOP=1 -U postgres \
    -d postgres -q -c 'CREATE TABLE ops5_probe (id integer PRIMARY KEY)'
docker exec -u postgres "$drill_pg" psql -v ON_ERROR_STOP=1 -U postgres \
    -d postgres -q -c 'INSERT INTO ops5_probe VALUES (1)'
docker exec -u postgres "$drill_pg" pgbackrest \
    --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
    --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
    --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y \
    --log-level-file=off --no-expire-auto --type=full backup
docker exec -u postgres "$drill_pg" psql -v ON_ERROR_STOP=1 -U postgres \
    -d postgres -q -c "SELECT pg_create_restore_point('ops5_before_second')"
docker exec -u postgres "$drill_pg" psql -v ON_ERROR_STOP=1 -U postgres \
    -d postgres -q -c 'INSERT INTO ops5_probe VALUES (2)'
docker exec -u postgres "$drill_pg" psql -v ON_ERROR_STOP=1 -U postgres \
    -d postgres -q -c 'SELECT pg_switch_wal()'
docker exec -u postgres "$drill_pg" pgbackrest \
    --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
    --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
    --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y \
    --log-level-file=off check
echo 'local encrypted full backup and two write points passed'

restore_volume() {
    restore_volume_name=$1
    shift
    docker volume create "$restore_volume_name" >/dev/null
    docker run --rm --network "$drill_network" \
        --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
        --mount "type=volume,src=$restore_volume_name,dst=/var/lib/postgresql/data" \
        --mount "type=bind,src=$drill_root/pgbackrest.conf,dst=/run/secrets/pgbackrest_conf,readonly" \
        --mount "type=bind,src=$drill_root/ca.crt,dst=/run/secrets/pgbackrest_ca,readonly" \
        --entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint \
        "$drill_image" \
        pgbackrest --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
        --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
        --repo1-cipher-type=aes-256-cbc --repo1-storage-verify-tls=y \
        --log-level-file=off "$@" restore
}

start_restored() {
    restore_name=$1
    restore_volume_name=$2
    docker run --detach --name "$restore_name" --network "$drill_network" \
        --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
        --mount "type=volume,src=$restore_volume_name,dst=/var/lib/postgresql/data" \
        --mount "type=bind,src=$drill_root/pgbackrest.conf,dst=/run/secrets/pgbackrest_conf,readonly" \
        --mount "type=bind,src=$drill_root/ca.crt,dst=/run/secrets/pgbackrest_ca,readonly" \
        -e POSTGRES_PASSWORD=drill-only-password \
        --entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint \
        "$drill_image" postgres -c archive_mode=off >/dev/null
    restore_ready=false
    restore_attempt=0
    while [ "$restore_attempt" -lt 40 ]; do
        if docker exec "$restore_name" pg_isready -U postgres >/dev/null 2>&1; then
            restore_ready=true
            break
        fi
        restore_attempt=$((restore_attempt + 1))
        sleep 1
    done
    if [ "$restore_ready" != true ]; then
        docker logs "$restore_name" >&2
        exit 1
    fi
}

restore_volume "$drill_before_volume" --type=name --target=ops5_before_second --target-action=promote
start_restored "$drill_before" "$drill_before_volume"
before_count=$(docker exec "$drill_before" psql -A -t -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'SELECT count(*) FROM ops5_probe')
[ "$before_count" = 1 ] || { echo "before restore count: $before_count" >&2; exit 1; }
echo 'named-point restore verified one row'

restore_volume "$drill_after_volume" --type=default
start_restored "$drill_after" "$drill_after_volume"
after_count=$(docker exec "$drill_after" psql -A -t -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'SELECT count(*) FROM ops5_probe')
[ "$after_count" = 2 ] || { echo "end-of-archive restore count: $after_count" >&2; exit 1; }
echo 'end-of-archive restore verified two rows'

sed 's/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/' \
    "$drill_root/pgbackrest.conf" > "$drill_root/wrong-key.conf"
chmod 0600 "$drill_root/wrong-key.conf"
docker volume create "$drill_wrong_volume" >/dev/null
if docker run --rm --network "$drill_network" \
    --tmpfs /var/run/postgresql:rw,nosuid,nodev,mode=3775,size=16m \
    --mount "type=volume,src=$drill_wrong_volume,dst=/var/lib/postgresql/data" \
    --mount "type=bind,src=$drill_root/wrong-key.conf,dst=/run/secrets/pgbackrest_conf,readonly" \
    --mount "type=bind,src=$drill_root/ca.crt,dst=/run/secrets/pgbackrest_ca,readonly" \
    --entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint \
    "$drill_image" \
    pgbackrest --config=/var/run/postgresql/pgbackrest.conf --stanza=synveda \
    --pg1-path=/var/lib/postgresql/data --repo1-type=s3 \
    --repo1-cipher-type=aes-256-cbc \
    --repo1-storage-verify-tls=y --log-level-file=off --type=default restore \
    > "$drill_root/wrong-key.log" 2>&1; then
    echo 'wrong repository passphrase unexpectedly opened backup metadata' >&2
    exit 1
fi
if ! rg -q 'unable to load info file' "$drill_root/wrong-key.log" ||
    ! rg -q 'no backup set found to restore' "$drill_root/wrong-key.log"; then
    echo 'wrong-key restore failed for an unexpected reason' >&2
    exit 1
fi
echo 'wrong repository passphrase refused'
