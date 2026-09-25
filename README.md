<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/synveda-lockup-dark.svg">
  <img src="assets/brand/synveda-lockup.svg" alt="Synveda" width="244" height="56">
</picture>

# Synveda

**Shared knowledge for AI agents, with clear access and review.** Synveda records
what an agent learns during a session, lets people review proposed learnings,
and supplies approved knowledge to later work. It runs on your infrastructure
and works alongside existing agent clients.

For a team, this means a finding does not have to stay in one conversation. A
project can reuse it, with its source and review history visible. For operators,
PostgreSQL stores the data, Cedar checks access on every read and write, and
the web console provides the day-to-day workflow.

![A fictional Northstar learning awaiting review](assets/product/review-learning.png)

*The screenshot uses fictional sample data.*

## Install for local evaluation

The prebuilt Docker bundle includes Synveda, PostgreSQL and Keycloak. You need
Docker Engine 28+, Compose 2.33.1+, `curl`, `tar`, GitHub CLI and a SHA-256
utility. Allow at least 6 GiB for Docker. No source build, cloud account or
model subscription is required. Start on Linux AMD64/ARM64; see
[platform limits](docs/PRODUCTION_READINESS.md) for other hosts.

<!-- installation-version: 0.4.3; publication: published -->
Download the [published v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
into a new directory and verify its publisher and checksum before extraction:

```sh
mkdir synveda-0.4.3 && cd synveda-0.4.3
release_url=https://github.com/synveda/synveda/releases/download/v0.4.3
for file in synveda-reference-0.4.3.tar.gz SHA256SUMS SHA256SUMS.sigstore.json; do
  curl -fLO "$release_url/$file"
done
gh attestation verify SHA256SUMS --bundle SHA256SUMS.sigstore.json \
  --repo synveda/synveda \
  --signer-workflow synveda/synveda/.github/workflows/release.yml \
  --source-ref refs/tags/v0.4.3 --deny-self-hosted-runners
awk '$2 == "synveda-reference-0.4.3.tar.gz" { n++; print } END { if (n != 1) exit 1 }' \
  SHA256SUMS > reference.sha256
if command -v shasum >/dev/null 2>&1; then
  shasum -a 256 --check reference.sha256
else
  sha256sum --check reference.sha256
fi
tar -xzf synveda-reference-0.4.3.tar.gz
cd synveda-reference-0.4.3
```

Before the first start, open `evaluation.json`. Its defaults use port `8080`,
a private Docker subnet and four sample accounts. Change `port` if 8080 is in
use, change `subnet` if it overlaps your network, or set `demoAccounts` to
`false` if you will supply your own identities. These settings become part of
the installation's private state; keep them with the matching data and keys.
The [Docker guide](deploy/compose/PREBUILT.md#state-and-ordinary-lifecycle)
explains the state and recovery rules.

```sh
./synveda-compose up
# With demoAccounts set to true:
./synveda-compose credential author
```

Open [http://localhost:8080/console/](http://localhost:8080/console/) (or your
configured port). Sign in as `synveda-demo-admin` with the password printed by
the credential command. **Getting started** walks you through creating a
workspace and project. With `demoAccounts:false`, use identities configured in
your OIDC provider instead.

## Try a complete example

The optional sample creates a fictional project, a source session and a
proposed learning. It uses the public API and needs no model key:

```sh
./synveda-compose sample
```

In the console, open **New Learnings** and inspect the ingestion retry finding.
Sign out, run `./synveda-compose credential reviewer`, and sign in as
`synveda-demo-member` to review it. Approve and apply the proposal in
**Advanced → Reviews**. Then request context for **Northstar Delivery →
Ingestion API** and inspect the approved revision and its source. The
[walkthrough](deploy/compose/PREBUILT.md#first-workspace-and-sample) gives the
exact review steps.

## Configure and connect

| Need | Start here |
| --- | --- |
| Local port, network and sample accounts | Edit `evaluation.json` before first start; see the [Docker guide](deploy/compose/PREBUILT.md#requirements). |
| Access and runtime behavior | Use **People** for grants and **Advanced → Configuration** for policy, capture and context settings; see the [product guide](docs/INSTALL.md#governed-runtime-configuration). |
| An existing database or identity provider | Follow the [Compose reference](deploy/compose/PREBUILT.md#use-existing-infrastructure) or [Kubernetes configuration](deploy/helm/synveda/CONFIGURATION.md). |
| An agent client | Install the [native client](docs/CONSUMER_CLI.md#release-downloads), then follow [Claude Code](adapters/claude-code/README.md), [Codex CLI](docs/integrations/codex.md) or [GitHub Copilot CLI](docs/integrations/copilot-cli.md). |

Python and TypeScript SDKs have an initial API slice; see the
[SDK guide](sdks/README.md) for their release status. The
[OpenAPI reference](docs/api/openapi.json) describes the public HTTP API.

## Client support

Client guides: Claude Code 2.1.241, GitHub Copilot CLI 1.0.83, Codex CLI 0.152.0.

See the [client support matrix](docs/CLIENT_SUPPORT.md) for platform coverage,
setup steps and capabilities. This list comes from the
[adapter registry](adapters/registry.json).

## Operating boundary

`./synveda-compose down` stops and removes containers while retaining the
databases and keys. `./synveda-compose up` resumes the same installation.
Backup, restore, updates and the separately confirmed destructive reset are
documented in the [Docker guide](deploy/compose/PREBUILT.md). The launcher
uses one fixed `synveda-evaluation` Compose project, so a different directory
does not create an independent installation.

This release is for self-hosted evaluation with one gateway and worker. High
availability, off-host recovery, production key custody and supported
cross-release upgrades remain open. The [readiness assessment](docs/PRODUCTION_READINESS.md)
lists current limits and future work. Kubernetes and external
infrastructure are available through the [deployment guide](deploy/README.md).

## Contribute and learn more

- [Contributing](CONTRIBUTING.md) and [source development](docs/DEVELOPMENT.md)
- [Product guide](docs/INSTALL.md) and [security model](docs/SECURITY.md)
- [Architecture](docs/SYNVEDA_TECH_PLAN.md), [current decisions](docs/adr/README.md) and [open work](docs/backlog/STATUS.md)

<a id="run-with-prebuilt-docker-images"></a>
<a id="quick-start-from-a-source-checkout"></a>
Previous installation links lead to the [prebuilt Docker guide](deploy/compose/PREBUILT.md)
or [source development guide](docs/DEVELOPMENT.md).

Synveda is [Apache-2.0 licensed](LICENSE). See [NOTICE](NOTICE) and
[asset attributions](assets/brand/ATTRIBUTIONS.md).
