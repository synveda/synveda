<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/synveda-lockup-dark.svg">
  <img src="assets/brand/synveda-lockup.svg" alt="Synveda" width="244" height="56">
</picture>

# Synveda

**Shared memory and context for AI agents.**

Synveda keeps useful knowledge from your agents' work so you can use it again.
Connect an agent, review the findings from its sessions, and keep the decisions,
conventions and procedures that matter to your project. Future tasks can
retrieve that knowledge without needing the original conversation.

You choose what gets published, who can access it, and which skills and tool
definitions a project can use. Synveda runs on your infrastructure, with
PostgreSQL storing the data and a web console for managing it.

[Prebuilt Docker deployment](#run-with-prebuilt-docker-images) ·
[Try it locally](#quick-start-from-a-source-checkout) ·
[Connect an agent](#agent-setup) ·
[Product guide](docs/INSTALL.md) ·
[Contribute](CONTRIBUTING.md)

> **Current status: local evaluation.** The source demo is available today.
> Prebuilt deployment needs a new release: public v0.2.0 is older than the
> current code and database schema. Production deployment still has
> [open requirements](#known-production-gaps).

## What you can do

| In the console        | Use it to                                                                  |
| --------------------- | -------------------------------------------------------------------------- |
| **Sessions**          | See what your agents worked on and inspect their activity.                 |
| **New Learnings**     | Review suggestions captured from sessions before adding them to Knowledge. |
| **Knowledge**         | Find project knowledge, edit it, and see its sources and revision history. |
| **Context**           | Request information for a task and inspect what was selected and why.      |
| **Skills and Tools**  | Manage reusable skill versions and approved MCP server definitions.        |
| **Reviews and Audit** | Review proposed changes and inspect the recorded decisions.                |

For example, an agent might discover why a service retries failed requests. You
can review that finding, publish it with its source, and make it available to
later sessions. Capture proposes the finding; your policy decides how it can be
published.

## Run with prebuilt Docker images

A small team should not need to compile the server. The release workflow
builds **Linux AMD64 and ARM64** images and packages a launcher that downloads
the matching versions. The console, gateway and worker are already compiled.

Use the [prebuilt Docker guide](deploy/compose/PREBUILT.md) when a compatible
release is available. You download one deployment archive, verify its checksum,
configure DNS and TLS, then run `synveda-compose up` and `synveda-compose smoke`.
The host needs Docker, Compose, Node.js and OpenSSL; it needs no Git checkout,
Rust compiler or pnpm. Each agent user connects to that shared server.

The current public v0.2.0 release does **not** include this archive. The next
release must pass anonymous image pulls and executable checks on both
architectures before it is announced. This does not establish production
readiness or Windows/WSL2 support. For a local evaluation now, use the source
demo below; for an existing Kubernetes cluster, see the
[Helm guide](deploy/helm/synveda/README.md).

## Quick start from a source checkout

The local demo builds Synveda and starts it with PostgreSQL and Keycloak. You
do not need a model API key or paid agent account to explore the console.

### Before you start

- **macOS or Linux**, using a regular user account. Windows and WSL2 setup are
  not yet documented or tested. The completed deployment run used macOS with
  OrbStack; Linux and Docker Desktop acceptance are still pending.
- **Docker Engine 28+**, **Compose 2.33.1+**, and a running **default Buildx
  builder** using the local **docker** driver. Remote Docker contexts are not
  supported by these scripts.
- **Git, Node.js 22+, OpenSSL and GNU Make**. Rust is only needed if you also
  want to build the CLI or work on the backend.
- Permission to add two local hostname entries. The hosts helper needs a
  root-owned, non-writable, ACL-free Node installation at /usr/bin/node or
  /usr/local/bin/node. Linux also needs getfacl from the acl package. See the
  [hostname setup guide](deploy/compose/README.md#development-hostname-setup).

### 1. Prepare the local addresses

```sh
git clone https://github.com/synveda/synveda.git
cd synveda
make compose-hosts-plan
make compose-hosts-status
```

Review the proposed hostname entries. With the default settings, install them:

```sh
SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
  make compose-hosts-install
```

Only this hosts helper needs elevated privileges. Run the other commands as
your regular user. If another Synveda project already owns these names or
port 8080, follow the hostname guide before continuing.

<details>
<summary>Refresh your DNS cache, then check the addresses</summary>

On macOS:

```sh
sudo dscacheutil -flushcache
sudo killall -HUP mDNSResponder
```

On Linux with systemd-resolved:

```sh
sudo resolvectl flush-caches
```

For other Linux resolvers, follow their cache-flush procedure. Then run:

```sh
make compose-hosts-status
make compose-resolver-check
```

</details>

### 2. Start Synveda

```sh
export SYNVEDA_COMPOSE_PROFILES=demo
make compose-config
make compose-up
make compose-smoke
```

There is no .env file to copy. Startup reads the checked-in defaults, generates
private secrets locally, prepares the databases and starts the services.
Rerunning it keeps the existing data and secrets. The smoke check verifies the
services and endpoints; browser sign-in is the next step.

### 3. Open the console

Visit **[app.synveda.test:8080/console/](http://app.synveda.test:8080/console/)**,
or the exact address printed by startup. Use that hostname rather than localhost
so sign-in returns to the correct place.

Sign in as **synveda-demo-admin**. Its display name is **Avery Author**. Use the
generated password stored in this local file, and keep the file private:

```text
deploy/compose/runtime/synveda-development/secrets/keycloak_demo_admin_password
```

On a fresh database, **Getting started** walks you through creating a workspace,
adding a project and connecting a client. Once set up, **Home** links to sessions,
new learnings, knowledge and context requests.

Want to explore populated data? Follow the
[ingestion-retry walkthrough](deploy/compose/README.md#governed-ingestion-retry-walkthrough).
It uses fictional demo content to walk through capture, review, publication and
retrieval. It needs the source CLI: build it with
`cargo build --locked -p synveda-cli`, then use `./target/debug/synveda` wherever
the guide says `synveda`, unless that binary is already on your PATH.

### Stop and return later

From the same shell, with the same profile and any project selectors:

```sh
make compose-down
# When you want to return:
make compose-up
```

Stopping preserves your data and generated secrets. Data reset and hostname
removal are separate operations in the [Compose guide](deploy/compose/README.md).
If startup fails, begin with `make compose-hosts-status` and
`make compose-resolver-check`, then use the log commands printed by startup.

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

For deployment work, start with [Docker Compose](deploy/compose/README.md).
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

Use Rust **1.96.0**, Node.js **22+** and pnpm **11.13.1**.

```sh
pnpm install --frozen-lockfile
pnpm -r test
pnpm -r build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Rust builds use the committed SQLx offline metadata. `make db-test` runs the
fresh-database suite; `make ci` runs the full PR checks and also needs the
[Python development dependencies](sdks/README.md#install-locally-and-check).
See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) before making changes.

To work only on the public site:

```sh
pnpm install --frozen-lockfile --filter @synveda/website
pnpm --filter @synveda/website check
pnpm --filter @synveda/website preview
```

Open [127.0.0.1:4173/synveda/](http://127.0.0.1:4173/synveda/).
The [website guide](website/README.md) covers branding, GitHub Pages and uploads.

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
