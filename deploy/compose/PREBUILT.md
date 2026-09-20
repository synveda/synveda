# Run Synveda from prebuilt Docker images

A small team can run the server without compiling Synveda. Download the
reference deployment archive from a matching release, then use its launcher
to pull the images and start PostgreSQL, Keycloak, the gateway, worker, proxy
and private telemetry collector. The web console is already in the product
image. There is no Git checkout, Rust toolchain, pnpm install or local image
build on this path.

**Availability:** as checked on 2026-09-20, the latest public release is
v0.2.0. It predates this deployment and has no current reference archive.
The commands below are for the next compatible release that includes
`synveda-reference-<version>.tar.gz` and both `release-images-*.json` reports.
Do not substitute v0.2.0 or mix its files with the current installer.

This remains a controlled single-host evaluation. Production key custody,
off-host recovery and deployment qualification are still open in the
[readiness register](https://github.com/synveda/synveda/blob/main/docs/PRODUCTION_READINESS.md).

## What you need

- A macOS or Linux host and a regular, non-root account with Docker access.
  Images target **Linux AMD64 and ARM64**. Docker supplies the Linux runtime
  on a Mac. Image availability is separate from deployment qualification:
  the recorded full development run used macOS/OrbStack; Linux and Docker
  Desktop reference acceptance remain pending. Native Windows and WSL2 setup
  are not documented or tested.
- Docker Engine **28+** on a local Unix socket, Compose **2.33.1+**, Node.js
  **22+**, OpenSSL, curl, tar and a SHA-256 utility. The launcher uses Node
  for preflight checks; it needs neither npm packages nor a compiler.
- Two DNS names you control, such as `app.example.com` and `auth.example.com`,
  pointing to this host. Browsers and containers must reach the same names.
- A trusted TLS certificate covering both names and its unencrypted PEM
  private key. Certificate renewal is operator-managed; the launcher does
  not request ACME certificates. Private-CA reference deployments are not
  qualified by this guide.
- Ports **80 and 443** available, and a private `/24` subnet that does not
  overlap your LAN, VPN or other Docker networks.

You do not need a GitHub account or registry token for public release images.
An authentication error may mean a package has not been made public; report
the release and image name rather than adding publishing credentials.

## 1. Download one complete release

Open [GitHub Releases](https://github.com/synveda/synveda/releases). Choose a
release that explicitly includes the current Docker reference, and copy its
tag into `SYNVEDA_VERSION`. The placeholder below must be replaced.

```sh
export SYNVEDA_VERSION='vMAJOR.MINOR.PATCH'
version="${SYNVEDA_VERSION#v}"
mkdir "synveda-$version"
cd "synveda-$version"
release_url="https://github.com/synveda/synveda/releases/download/$SYNVEDA_VERSION"
curl -fLO "$release_url/synveda-reference-$version.tar.gz"
curl -fLO "$release_url/SHA256SUMS"
awk -v file="synveda-reference-$version.tar.gz" \
  '$2 == file { count++; print } END { if (count != 1) exit 1 }' \
  SHA256SUMS > reference.sha256
```

On Linux, verify and extract:

```sh
sha256sum --check reference.sha256 && tar -xzf "synveda-reference-$version.tar.gz"
```

On macOS, use `shasum -a 256 --check reference.sha256` for the checksum command.
Stop if verification fails. Checksums detect a damaged download; these
artifacts are not yet signed.

```sh
cd "synveda-reference-$version"
cat environment.json
```

The manifest records the version, source commit and exact image digests. Keep
the archive, manifest and release reports with your deployment records. The
launcher uses these pinned images; it does not follow a moving `latest` tag.

## 2. Set the hostnames and prepare TLS

Use the same settings whenever you operate this deployment. Save them in an
operator-owned shell file outside the extracted release; the launcher does
not automatically load a `.env` file. Replace these example names and subnet:

```sh
export SYNVEDA_HOME="$HOME/.synveda"
export SYNVEDA_APP_HOST=app.example.com
export SYNVEDA_AUTH_HOST=auth.example.com
export SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24
install -d -m 700 "$SYNVEDA_HOME" "$SYNVEDA_HOME/state" \
  "$SYNVEDA_HOME/state/synveda-reference"
./synveda-compose secrets
./synveda-compose issuer
```

These commands create the bundled issuer configuration, private keys and database credentials in
`$SYNVEDA_HOME/state/synveda-reference/secrets`. Keep that directory: losing the
encryption key can make stored data unreadable. It is separate from the
downloaded release and survives replacing it.

Repeated preparation preserves existing keys and issuer settings. External
providers require operator-supplied credentials and issuer configuration;
the corresponding bundled helper refuses that mode.

Copy your leaf-first certificate chain and matching key into the generated
directory, as the same regular user:

```sh
install -m 600 /absolute/path/to/fullchain.pem \
  "$SYNVEDA_HOME/state/synveda-reference/secrets/tls_cert"
install -m 600 /absolute/path/to/privkey.pem \
  "$SYNVEDA_HOME/state/synveda-reference/secrets/tls_key"
```

The default runtime UID/GID follows the operator. Do not change ownership or
relax secret-file permissions to fix a failed preflight.

## 3. Start the server and sign in

For a disposable evaluation, enable the existing demo accounts **before**
startup:

```sh
export SYNVEDA_COMPOSE_PROFILES=demo
```

For real team identities, leave the demo profile unset and have your identity
administrator provision the users. Bundled Keycloak creates the realm and
clients, but no ordinary users by default; its admin interface is private.
Alternatively, use the existing
[external OIDC settings](https://github.com/synveda/synveda/blob/main/deploy/compose/README.md#external-oidc)
with this launcher, replacing the source guide's `make compose-*` commands
with `./synveda-compose <action>`. Configure the provider before the first
start; changing an existing tenant's issuer is not a supported migration.

```sh
./synveda-compose config
./synveda-compose up
./synveda-compose smoke
```

`up` pulls the release images, prepares the databases and starts services with
`--no-build`. Only the proxy exposes public ports. The first download can be
large; subsequent starts reuse the images and preserve data.

Open `https://<your-application-host>/console/`. For the demo, sign in as
**synveda-demo-admin**, using the password in
`$SYNVEDA_HOME/state/synveda-reference/secrets/keycloak_demo_admin_password`.
For a team, the first qualifying member of the IdP's `synveda-admins` group
receives the initial Synveda administrator grant. Later access is managed
through Synveda's governed grants.

Follow **Getting started** to create a workspace and project. Agent users can
then connect to this server from their own machines. The native CLI/client
installer is separate and currently targets macOS ARM64 and Linux x86_64;
it is not required on this Docker host. See the
[agent setup guide](https://github.com/synveda/synveda/blob/main/README.md#agent-setup).

## Stop, restart and update

From the extracted release directory, with the same environment settings:

```sh
./synveda-compose down
./synveda-compose up
./synveda-compose smoke
```

`down` keeps the database volume and private state. Keep the previous release
directory when preparing an update. Download and verify the new complete
bundle, review its compatibility notes, and back up both databases **and**
their separate key/identity material before switching. Replacing a bundle is
not a database migration or a proven rollback. Older schema epochs are refused
and must never be reset as a team upgrade.

The [operations guide](https://github.com/synveda/synveda/blob/main/docs/INSTALL.md#backing-up-and-restoring-the-compose-reference)
covers the current recovery boundary. For an existing Kubernetes environment,
use the [Helm guide](https://github.com/synveda/synveda/blob/main/deploy/helm/synveda/README.md)
and the same release's digest overlays.
