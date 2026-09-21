#!/usr/bin/env bash
# Package the canonical CPR-45 Docker reference deployment.
#
# Usage:
#   scripts/package-release.sh VERSION OUTPUT SOURCE_SHA \
#     PRODUCT_DIGEST POSTGRES_DIGEST KEYCLOAK_DIGEST PROXY_DIGEST \
#     BROWSER_DIGEST HELM_POSTGRES_DIGEST
#
# Digests are bare `sha256:<64 lowercase hex>` values. The script constructs
# the selected registry references, writes the source/image environment manifest and
# copies only the runtime closure needed by the reference deployment. It never
# copies ignored local `.env`, runtime, secret or backup paths.
# SYNVEDA_PACKAGE_CONSUMER_CANDIDATE=1 additionally renders the unpublished
# OPS-12 plain-Compose candidate using the Compose CLI (no daemon required).
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -ne 9 ]; then
  echo "usage: package-release.sh VERSION OUTPUT SOURCE_SHA PRODUCT_DIGEST POSTGRES_DIGEST KEYCLOAK_DIGEST PROXY_DIGEST BROWSER_DIGEST HELM_POSTGRES_DIGEST" >&2
  exit 64
fi

version=$1
outdir=$2
source_sha=$3
product_digest=$4
postgres_digest=$5
keycloak_digest=$6
proxy_digest=$7
browser_digest=$8
helm_postgres_digest=$9
image_namespace=${SYNVEDA_IMAGE_NAMESPACE:-ghcr.io/synveda}
consumer_candidate=${SYNVEDA_PACKAGE_CONSUMER_CANDIDATE:-0}
case "$consumer_candidate" in
  0|1) ;;
  *) echo "package-release: SYNVEDA_PACKAGE_CONSUMER_CANDIDATE must be 0 or 1" >&2; exit 64 ;;
esac

# Validate every value before deriving or replacing a path.
sh scripts/release-version.sh "$version"
node --input-type=module - "$image_namespace" <<'JS'
import { imageNamespace } from "./scripts/release-registries.mjs";
imageNamespace(process.argv[2]);
JS
printf '%s\n' "$source_sha" | grep -Eq '^[0-9a-f]{40}$' || {
  echo "package-release: SOURCE_SHA must be a full lowercase 40-hex Git commit" >&2
  exit 64
}
validate_digest() {
  printf '%s\n' "$1" | grep -Eq '^sha256:[0-9a-f]{64}$' || {
    echo "package-release: image digests must be sha256 plus 64 lowercase hex characters" >&2
    exit 64
  }
}
for digest in "$product_digest" "$postgres_digest" "$keycloak_digest" \
  "$proxy_digest" "$browser_digest" "$helm_postgres_digest"; do
  validate_digest "$digest"
done

otel_image=$(awk -F= '$1 == "SYNVEDA_OTEL_COLLECTOR_IMAGE" { print $2; exit }' deploy/compose/.env.example)
prometheus_image=$(awk -F= '$1 == "SYNVEDA_PROMETHEUS_IMAGE" { print $2; exit }' deploy/compose/.env.example)
for external_image in "$otel_image" "$prometheus_image"; do
  printf '%s\n' "$external_image" | grep -Eq '^[A-Za-z0-9_./:+-]+@sha256:[0-9a-f]{64}$' || {
    echo "package-release: canonical external image is not digest pinned" >&2
    exit 1
  }
done

mkdir -p "$outdir"
stage="$outdir/synveda-reference-$version"
archive="$outdir/synveda-reference-$version.tar.gz"
rm -rf -- "$stage"
rm -f -- "$archive"
mkdir -p "$stage/deploy/compose"

copy_runtime_asset() {
  source_path=$1
  [ -f "$source_path" ] && [ ! -L "$source_path" ] || {
    echo "package-release: required runtime asset is unavailable: $source_path" >&2
    exit 1
  }
  mkdir -p "$stage/$(dirname "$source_path")"
  cp "$source_path" "$stage/$source_path"
}

# Reference runtime closure only: no source-development overlays, Docker build
# inputs, database-test fixtures or local state.
for asset in \
  deploy/compose/compose.yaml \
  deploy/compose/compose.reference.yaml \
  deploy/compose/compose.evaluation.yaml \
  deploy/compose/compose.evaluation-sample.yaml \
  deploy/compose/compose.postgres.yaml \
  deploy/compose/compose.keycloak.yaml \
  deploy/compose/compose.keycloak-postgres.yaml \
  deploy/compose/compose.keycloak-external-postgres.yaml \
  deploy/compose/compose.external-postgres.yaml \
  deploy/compose/compose.external.yaml \
  deploy/compose/compose.otlp-external.yaml \
  deploy/compose/compose.demo.yaml \
  deploy/compose/compose.browser-acceptance.yaml \
  deploy/compose/compose.observability.yaml \
  deploy/compose/compose.apalis.yaml \
  deploy/compose/compose.backup.yaml \
  deploy/compose/compose.restore.yaml \
  deploy/compose/configs/caddy/Caddyfile \
  deploy/compose/configs/caddy/app.reference.caddy \
  deploy/compose/configs/caddy/app.evaluation.caddy \
  deploy/compose/configs/caddy/identity.reference.caddy \
  deploy/compose/configs/caddy/identity.external.caddy \
  deploy/compose/configs/database/roles.reference.json \
  deploy/compose/configs/database/roles.external-oidc.json \
  deploy/compose/configs/otel/collector.yaml \
  deploy/compose/configs/otel/collector.observability.yaml \
  deploy/compose/configs/otel/collector.external.yaml \
  deploy/compose/configs/otel/collector.external.observability.yaml \
  deploy/compose/configs/prometheus/prometheus.yaml \
  deploy/compose/browser/seccomp_profile.json \
  deploy/compose/browser/seccomp_profile.NOTICE \
  deploy/compose/scripts/compose.sh \
  deploy/compose/scripts/evaluation.sh \
  deploy/compose/scripts/prepare-evaluation.mjs \
  deploy/compose/scripts/evaluation-recovery.mjs \
  deploy/compose/scripts/generate-secrets.sh \
  deploy/compose/scripts/generate-issuer.sh \
  deploy/compose/scripts/project-lock.sh \
  deploy/compose/scripts/run-node-closed \
  deploy/compose/scripts/monotonic-seconds.mjs \
  deploy/compose/scripts/run-with-deadline.mjs \
  deploy/compose/scripts/check-host-resolution.mjs \
  deploy/compose/scripts/check-network-preflight.mjs \
  deploy/compose/scripts/check-compose-assets.mjs \
  deploy/compose/scripts/check-runtime-smoke.mjs \
  deploy/compose/scripts/check-tls-inputs.mjs \
  deploy/compose/scripts/check-browser-seccomp.mjs \
  deploy/compose/scripts/manage-hosts-file.mjs \
  deploy/compose/scripts/recovery-set.mjs \
  deploy/compose/scripts/reset-runtime-state.mjs \
  deploy/compose/secrets.example/README.md; do
  copy_runtime_asset "$asset"
done

# The server-only guide travels with the archive; its further reading stays
# bound to the same source revision instead of the moving default branch.
node --input-type=module - "$source_sha" "$stage/INSTALL.md" "$version" <<'JS'
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
const [sha, output, version] = process.argv.slice(2);
const base = `https://github.com/synveda/synveda/blob/${sha}/`;
const source = readFileSync("deploy/compose/PREBUILT.md", "utf8");
const statusStart = source.indexOf("<!-- installation-version:");
const requirements = source.indexOf("## Requirements");
if (statusStart < 0 || requirements <= statusStart) throw new Error("packaged guide publication boundary missing");
// Immutable candidate bytes must also read correctly after promotion. Public
// main-branch status stays in installation.json; the archive points to evidence.
const qualifiedGuide = source.slice(0, statusStart)
  + `**Archive ${version}.** Verify SHA256SUMS and the matching image, Docker and\nKubernetes qualification reports from its approved release. A privately built\ncandidate does not establish published or platform support.\n\n`
  + source.slice(requirements);
const guide = qualifiedGuide
  .replace(/The following download is \*\*pending publication\*\*\. After [^\n]+ is approved:/, "For a new host, download only an approved release with its matching reports:")
  .replaceAll("https://github.com/synveda/synveda/blob/main/", base)
  .replace(/\]\(([^)#]+)(#[^)]*)?\)/g, (match, target, anchor = "") =>
    /^(https?:|#)/.test(target) ? match : `](${base}${path.posix.normalize(`deploy/compose/${target}`)}${anchor})`);
writeFileSync(output, guide);
JS
cp LICENSE NOTICE "$stage/"

product_image="$image_namespace/product@$product_digest"
postgres_image="$image_namespace/postgres@$postgres_digest"
keycloak_image="$image_namespace/keycloak@$keycloak_digest"
proxy_image="$image_namespace/proxy@$proxy_digest"
browser_image="$image_namespace/browser-acceptance@$browser_digest"
helm_postgres_image="$image_namespace/cnpg-postgres@$helm_postgres_digest"

# OPS-11: use the same immutable images in Helm. Separate overlays preserve
# the chart's refusal of unused CNPG settings in external-database mode.
cat > "$outdir/synveda-images-$version.yaml" <<EOF
# Generated from release $version, source $source_sha. Contains no credentials.
image:
  repository: $image_namespace/product
  tag: ""
  digest: $product_digest
postgres:
  bundled:
    image: $postgres_image
keycloak:
  image:
    repository: $image_namespace/keycloak
    tag: ""
    digest: $keycloak_digest
EOF
cat > "$outdir/synveda-cnpg-image-$version.yaml" <<EOF
# Apply only with postgres.mode=cnpg; keep the PostgreSQL version in the tag.
postgres:
  image: $image_namespace/cnpg-postgres:17.11-synveda-$version@$helm_postgres_digest
EOF

cat > "$stage/environment.json" <<EOF
{
  "schema_version": 1,
  "release_version": "$version",
  "source_sha": "$source_sha",
  "image_namespace": "$image_namespace",
  "deployment_contract": "CPR-45/ADR-0102",
  "images": {
    "product": "$product_image",
    "postgres": "$postgres_image",
    "keycloak": "$keycloak_image",
    "proxy": "$proxy_image",
    "browser_acceptance": "$browser_image",
    "helm_postgres": "$helm_postgres_image"
  },
  "external_images": {
    "otel_collector": "$otel_image",
    "prometheus": "$prometheus_image"
  }
}
EOF
printf '%s\n' "$version" > "$stage/version"
printf '%s\n' "$source_sha" > "$stage/source-sha"
printf '%s\n' "$product_image" > "$stage/product-image"
cat > "$stage/evaluation.json" <<EOF
{
  "port": 8080,
  "subnet": "10.231.60.0/24",
  "demoAccounts": true
}
EOF

cat > "$stage/deploy/compose/.env.example" <<EOF
# Generated non-secret defaults for Synveda reference release $version.
SYNVEDA_COMPOSE_RUNTIME=reference
SYNVEDA_POSTGRES_MODE=bundled
SYNVEDA_OIDC_MODE=bundled
SYNVEDA_OTLP_MODE=discard
SYNVEDA_OTLP_EXPORT_ENDPOINT=
SYNVEDA_COMPOSE_PROFILES=
SYNVEDA_APP_HOST=app.example.com
SYNVEDA_AUTH_HOST=auth.example.com
SYNVEDA_PUBLIC_SCHEME=https
SYNVEDA_TLS_MODE=files
SYNVEDA_RUNTIME_UID=65532
SYNVEDA_RUNTIME_GID=65532
SYNVEDA_COMPOSE_IPV4_POOL=172.30.240.0/24
SYNVEDA_PRODUCT_IMAGE=$product_image
SYNVEDA_POSTGRES_IMAGE=$postgres_image
SYNVEDA_KEYCLOAK_IMAGE=$keycloak_image
SYNVEDA_CADDY_IMAGE=$proxy_image
SYNVEDA_BROWSER_IMAGE=$browser_image
SYNVEDA_OTEL_COLLECTOR_IMAGE=$otel_image
SYNVEDA_PROMETHEUS_IMAGE=$prometheus_image
SYNVEDA_PROMETHEUS_PORT=9090
RUST_LOG=info
EOF

cat > "$stage/synveda-compose" <<EOF
#!/bin/sh
# Generated reference launcher. Image identities are immutable release inputs;
# hostnames, dependency modes and operator-owned paths remain runtime inputs.
set -eu
bundle_dir=\$(CDPATH= cd "\$(dirname "\$0")" && pwd -P)
if [ "\${SYNVEDA_COMPOSE_RUNTIME:-evaluation}" = evaluation ]; then
  exec sh "\$bundle_dir/deploy/compose/scripts/evaluation.sh" "\$@"
fi
case "\$(basename "\$(dirname "\$bundle_dir")")" in
  releases) install_home=\$(dirname "\$(dirname "\$(dirname "\$bundle_dir")")") ;;
  *) install_home=\${SYNVEDA_HOME:-\${HOME:?HOME is required}/.synveda} ;;
esac
project=synveda-reference
if [ -n "\${SYNVEDA_COMPOSE_PROJECT_SUFFIX:-}" ]; then
  project=\$project-\$SYNVEDA_COMPOSE_PROJECT_SUFFIX
fi
state_root=\${SYNVEDA_REFERENCE_STATE_ROOT:-\$install_home/state}
backup_root=\${SYNVEDA_REFERENCE_BACKUP_ROOT:-\$install_home/backups}
: "\${SYNVEDA_APP_HOST:?set SYNVEDA_APP_HOST to the application DNS name}"
SYNVEDA_POSTGRES_MODE=\${SYNVEDA_POSTGRES_MODE:-bundled}
SYNVEDA_OIDC_MODE=\${SYNVEDA_OIDC_MODE:-bundled}
SYNVEDA_COMPOSE_IPV4_POOL=\${SYNVEDA_COMPOSE_IPV4_POOL:-172.30.240.0/24}
if [ "\$SYNVEDA_OIDC_MODE" = bundled ]; then
  : "\${SYNVEDA_AUTH_HOST:?set SYNVEDA_AUTH_HOST to the identity DNS name}"
fi
SYNVEDA_COMPOSE_RUNTIME=reference
SYNVEDA_PUBLIC_SCHEME=https
SYNVEDA_PRODUCT_IMAGE='$product_image'
SYNVEDA_POSTGRES_IMAGE='$postgres_image'
SYNVEDA_KEYCLOAK_IMAGE='$keycloak_image'
SYNVEDA_CADDY_IMAGE='$proxy_image'
SYNVEDA_BROWSER_IMAGE='$browser_image'
SYNVEDA_OTEL_COLLECTOR_IMAGE='$otel_image'
SYNVEDA_PROMETHEUS_IMAGE='$prometheus_image'
SYNVEDA_SECRETS_DIR=\${SYNVEDA_SECRETS_DIR:-\$state_root/\$project/secrets}
SYNVEDA_OIDC_ISSUERS_FILE=\${SYNVEDA_OIDC_ISSUERS_FILE:-\$state_root/\$project/issuers.json}
SYNVEDA_DATABASE_AUTHORITY_DIR=\${SYNVEDA_DATABASE_AUTHORITY_DIR:-\$state_root/\$project/database-authority}
SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR=\${SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR:-\$state_root/\$project/keycloak-public-gate}
SYNVEDA_DATABASE_BACKUP_ROOT=\${SYNVEDA_DATABASE_BACKUP_ROOT:-\$backup_root/database/\$project}
SYNVEDA_RECOVERY_SECRETS_ROOT=\${SYNVEDA_RECOVERY_SECRETS_ROOT:-\$backup_root/secrets/\$project}
export SYNVEDA_COMPOSE_RUNTIME SYNVEDA_PUBLIC_SCHEME SYNVEDA_PRODUCT_IMAGE \
  SYNVEDA_POSTGRES_MODE SYNVEDA_OIDC_MODE SYNVEDA_POSTGRES_IMAGE \
  SYNVEDA_COMPOSE_IPV4_POOL \
  SYNVEDA_KEYCLOAK_IMAGE SYNVEDA_CADDY_IMAGE SYNVEDA_BROWSER_IMAGE \
  SYNVEDA_OTEL_COLLECTOR_IMAGE SYNVEDA_PROMETHEUS_IMAGE \
  SYNVEDA_SECRETS_DIR SYNVEDA_OIDC_ISSUERS_FILE \
  SYNVEDA_DATABASE_AUTHORITY_DIR SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR \
  SYNVEDA_DATABASE_BACKUP_ROOT SYNVEDA_RECOVERY_SECRETS_ROOT
case "\${1:-}" in
  secrets|issuer)
    [ "\$#" -eq 1 ] || { echo "usage: synveda-compose {secrets|issuer}" >&2; exit 64; }
    exec "\$bundle_dir/deploy/compose/scripts/generate-\$1.sh" --if-missing
    ;;
esac
exec "\$bundle_dir/deploy/compose/scripts/compose.sh" "\$@"
EOF
chmod 755 "$stage/synveda-compose"

cat > "$stage/README.md" <<EOF
# Synveda Docker bundle $version

Read [INSTALL.md](INSTALL.md) for checksum verification, loopback first sign-in,
optional sample, lifecycle and reference HTTPS. Run \`./synveda-compose up\` to
prepare private state in a short-lived container and start the pinned services.
No source checkout, host Node/OpenSSL, Rust toolchain or image build is required
for evaluation. Only the proxy's selected loopback port is exposed.

\`environment.json\` records source and immutable image identities. Private
keys/state and paired backups live below \`SYNVEDA_HOME/state\`, outside the
release. Preserve them with the databases. An archive alone is not runtime
qualification: read the release's architecture-specific image, Docker and
Kubernetes reports. No HA or off-host disaster-recovery claim is implied.
EOF

if find "$stage" -type f \( -name '.env' -o -iname '*rauthy*' -o \
  -iname '*temporal*' -o -name '*.dev.yaml' -o -name 'Dockerfile' \) \
  -print -quit | grep -q .; then
  echo "package-release: retired, development or local-only input entered the reference archive" >&2
  exit 1
fi
if find "$stage" -type d \( -name runtime -o -name backups -o -name secrets \) \
  -print -quit | grep -q .; then
  echo "package-release: mutable state entered the reference archive" >&2
  exit 1
fi
if grep -R -i -E '\brauthy\b|\btemporal(io|[-_][a-z0-9_]+)?\b' \
  "$stage/deploy/compose" >/dev/null; then
  echo "package-release: retired runtime marker entered the reference archive" >&2
  exit 1
fi

if [ "$consumer_candidate" = 1 ]; then
  # Local qualification only until OPS-12 adds paired named-volume recovery
  # and release acceptance. The publishing workflow does not select this flag.
  copy_runtime_asset deploy/compose/scripts/initialize-consumer.mjs
  cp deploy/compose/CONSUMER.md "$stage/CONSUMER.md"
  node scripts/package-consumer-compose.mjs "$stage"
fi

tar -czf "$archive" -C "$outdir" "synveda-reference-$version"
rm -rf -- "$stage"
echo "packaged $archive"
