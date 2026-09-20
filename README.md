<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/synveda-lockup-dark.svg">
  <img src="assets/brand/synveda-lockup.svg" alt="Synveda" width="244" height="56">
</picture>

# Synveda

**Governed knowledge, context and skills for AI agents.** Synveda gives agents
reusable knowledge with clear ownership, evidence and review. Capture a finding
from a session, review the proposed change, then make the approved revision
available to a later task. Run it on your own infrastructure: PostgreSQL stores
the data, Cedar decides access, and the web console shows what changed and why.

![Synveda New Learnings in the fictional Northstar sample: source evidence and a proposed learning awaiting review](assets/product/review-learning.png)

The screenshot is from the locally tested installation candidate and fictional
sample. Synthetic replay is labelled; it is not a live-agent or human-review claim.

## Run with Docker

Download the versioned **prebuilt Docker bundle** and verify its checksum. It
uses images containing the console, CLI and preparation tools; the host needs Docker Compose,
curl, tar and a SHA-256 utility. Local evaluation uses loopback, generated private
credentials and bundled PostgreSQL/Keycloak. No source checkout, compiler, host
Node/OpenSSL, DNS changes or model subscription is required.

<!-- installation-version: 0.4.0; publication: published -->
**[Download v0.4.0](https://github.com/synveda/synveda/releases/tag/v0.4.0).**
The release includes checksums and native Linux AMD64/ARM64 installation reports.

```sh
# After downloading and checking synveda-reference-0.4.0.tar.gz:
tar -xzf synveda-reference-0.4.0.tar.gz
cd synveda-reference-0.4.0
./synveda-compose up
./synveda-compose credential
```

Open **http://localhost:8080/console/**. Create an empty workspace, or explicitly
run `./synveda-compose sample` for the fictional walkthrough.

[Docker download, configuration and lifecycle](deploy/compose/PREBUILT.md)

## Deploy to Kubernetes

Use the existing chart with independently bundled or existing PostgreSQL and
Keycloak/OIDC. Bundled PostgreSQL is one persistent, namespaced instance with
no operator prerequisite. CNPG remains an explicit operator-managed option.

```sh
# OCI and the downloadable .tgz contain the same chart:
helm pull oci://ghcr.io/synveda/charts/synveda --version 0.4.0 --untar
sh synveda/examples/prepare-local.sh "$HOME/.synveda-kubernetes"
```

Review the current Kubernetes context, create the documented evaluation
namespace and apply the private Secret file before installing. Follow the
[complete Kubernetes recipe](deploy/helm/synveda/examples/README.md) for the
bounded install and loopback port-forwards. Existing infrastructure uses
[reviewable values examples](deploy/helm/synveda/README.md#dependency-ownership).

## First useful result

The opt-in sample creates a fictional workspace, a source session and a proposed
learning using normal authenticated APIs. Sign in as the separate reviewer to
inspect the evidence and approve or reject it. Applied Knowledge becomes
available to subsequent Context selection. The
[sample walkthrough](deploy/compose/PREBUILT.md#first-workspace-and-sample)
explains the remaining explicit review and skill-binding steps.

## Support status

The published artifacts passed anonymous pulls and complete Docker installation,
authentication, recreation and paired recovery on native Linux AMD64/ARM64.
The same images and packaged chart passed all four dependency combinations in
Kind with Kubernetes 1.36.1. Local candidate evidence also covers macOS/OrbStack
on Apple Silicon. Docker Desktop, Windows/WSL2 and real OpenShift remain unqualified.

This is a self-hosted evaluation release: one gateway and worker, no HA claim,
no cross-epoch database upgrade, and no completed off-host disaster-recovery
qualification. Read the [readiness gaps](docs/PRODUCTION_READINESS.md) and
[installation choices](docs/INSTALL.md).

## Build from source

Contributor prerequisites and the existing source workflow are in
[CONTRIBUTING.md](CONTRIBUTING.md#local-deployment). Source builds are separate
from released installation.

<a id="run-with-prebuilt-docker-images"></a>
<a id="quick-start-from-a-source-checkout"></a>
The previous installation anchors now point to the Docker and contributor
instructions above.

## Agent setup

| Client              | Where to start                                                       |
| ------------------- | -------------------------------------------------------------------- |
| Claude Code         | [Plugin installation and connection](adapters/claude-code/README.md) |
| Codex CLI           | [Setup, tested version and limits](docs/integrations/codex.md)       |
| GitHub Copilot CLI  | [Setup, tested version and limits](docs/integrations/copilot-cli.md) |
| Other MCP clients   | [MCP connection guide](adapters/claude-code/README.md#the-mcp-tool)  |
| Python / TypeScript | [SDK guide and local package installation](sdks/README.md)           |

Codex and Copilot verification covers the named source builds on macOS arm64
with Keycloak. Cursor is experimental; other MCP clients have differing levels
of partial evidence. The SDKs provide an initial set of 15 operations and are
not yet published to npm or PyPI. Check the linked guide for your setup.

## Client support

Verified client lifecycles: Claude Code 2.1.241, GitHub Copilot CLI 1.0.83, Codex CLI 0.152.0.

See the [client support matrix](docs/CLIENT_SUPPORT.md) for the tested platforms,
setup and remaining limits. Other clients have partial checks or setup recipes;
those do not establish a working end-to-end lifecycle. This summary is checked
against [the adapter registry](adapters/registry.json).

## Known production gaps

Synveda is not yet ready for production. A current, fully verified release,
production key management, and off-host backup and restore procedures are still
needed. High availability and several operational checks also remain open.
The [readiness register](docs/PRODUCTION_READINESS.md) lists the gaps and the
checks needed to close them.

For deployment work, start with [Run with Docker](deploy/compose/PREBUILT.md).
There is also a [Kubernetes chart](deploy/helm/synveda/README.md), currently
limited to one gateway and one worker. Both have qualification limits; a local
demo passing does not establish a production deployment.

## How it works

```text
Agent clients / MCP / SDKs / web console
                  │
         Authenticated public API
                  │
       Gateway + Cedar policy checks
                  │
          PostgreSQL + pgvector
                  │
    Worker: capture, indexing, expiry
```

The core, gateway, worker and CLI are written in Rust. The React console and
adapters use the public API. Agents run in their own clients; Synveda manages
the knowledge, context, skills and tool definitions they can use.

Every read and write is checked by Cedar. PostgreSQL row-level security keeps
tenants isolated. Governed changes go through VedaFlow, Synveda's review and
publication workflow, and leave content-minimised audit evidence. Knowledge
revisions are immutable, so you can trace a change back to its source.

Read the [security model](docs/SECURITY.md) or
[technical plan](docs/SYNVEDA_TECH_PLAN.md) for the details.

## Working on Synveda

See [CONTRIBUTING.md](CONTRIBUTING.md) for source builds and tests and the
[website guide](website/README.md) for local Pages preview and brand maintenance.

## Documentation

- [Product guide](docs/INSTALL.md) — using the console, CLI and integrations.
- [Deployment overview](deploy/README.md) — packaging and deployment options.
- [Client support](docs/CLIENT_SUPPORT.md) — what has been tested with each client.
- [API reference](docs/api/openapi.json) — the generated OpenAPI contract.
- [Architecture decisions](docs/adr/README.md) — why the system works this way.
- [Feature inventory](docs/backlog/STATUS.md) — delivered and planned work.

## Licence

Synveda is [Apache-2.0](LICENSE). See [NOTICE](NOTICE) for notices and
[brand attributions](assets/brand/ATTRIBUTIONS.md) for the Inter font licence.
