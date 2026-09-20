# Authenticated public API clients (ADPT-4)

This initial, unpublished slice covers 15 operations from Synveda's checked
OpenAPI: Sessions, observations, Context, approved immutable Skills, Knowledge
proposals and audit pages. The generated contracts include the source SHA-256;
they target the 0.4.0 API in this checkout. Broader API coverage, published
packages and a public support policy remain in `docs/backlog/ADPT-4.md`.

TypeScript uses Node's maintained Fetch implementation; Python uses HTTPX.
Neither client implements retrieval, policy, OAuth, orchestration or automatic
task completion. The gateway owns Cedar, RLS, VedaFlow and audit.

## Install locally and check

Node 22+ and Python 3.11+ are required. Installed-archive validation passed all
nine tests per SDK in pinned Linux arm64 containers on Node 22.23.2/Python
3.11.16 and locally on macOS arm64 Node 24.18.0/Python 3.14.6. Each run built
twice from clean staging directories, installed offline into separate consumers
and checked exported types/resources. Both runs produced identical archives.
CI declares Node 22/Python 3.11 and Node 24/Python 3.14 on Linux amd64; those
remote matrix jobs were not executed locally. The full local CI and fresh
database suite passed at `8f40237`; neither was rerun for the compatibility
change. These package checks do not qualify live OIDC.
No public npm/PyPI package is published by this work.

The later live Keycloak run passed both examples through the canonical Docker
public proxy on a native Codex-created Session. It covered allowed context,
approved Skill bytes, pending/idempotent proposals, workspace denial and audit
correlation. Capture, explicit task-owner end, cross-session reuse and audit-
chain verification also passed. See the [Codex qualification](../docs/integrations/codex.md) and the
content-free `adapters/codex/fixtures/keycloak-qualification.json` result.

The completed Codex 0.152.0 qualification additionally exercised native
automatic compaction and outage/recovery, followed by both existing examples
on the same task. An unchanged Python client/bearer observed live grant,
revocation and re-authorisation through the public API. Temporary grants were
removed and audit correlation passed. The content-free result is
`adapters/codex/fixtures/live-qualification.json`; package release remains open.


```sh
pnpm install --frozen-lockfile
python3 -m venv sdks/python/.venv
. sdks/python/.venv/bin/activate
python -m pip install --require-hashes -r sdks/python/requirements-dev.lock
python -m pip install --no-deps -e sdks/python
make sdk-check
make interop-acceptance
```

The final command builds the existing CLI and Claude adapter, runs MCP boundary
tests, then uses `scripts/db-test.sh` for a fresh exact-role Docker PostgreSQL
fixture. Its real gateway uses ordinary tenant identities and maintained test
policy packs. It runs authentic Claude hook replay and both language examples
against one Session. This is not an OIDC or live vendor-client qualification.
Missing prerequisites fail this explicit target; the SDK test is explicitly
ignored in ordinary Rust runs that do not install language dependencies.

Regenerate after an intentional public contract change:

```sh
node scripts/generate-sdk-contract.mjs
make sdk-check
```

`SYNVEDA_PYTHON` and `SYNVEDA_DATAMODEL_CODEGEN` can name an existing environment's
executables. Python runtime and generator dependencies are hash-locked; the
TypeScript package uses the repository pnpm lock. The Python wheel includes
generated types, operation bindings, contract metadata and `py.typed`.

## Compatibility and release boundary

SDK version **0.1.0** targets API version **0.4.0** and checked OpenAPI SHA-256
`6e711eef8f63607cf007242dc70b3e5047a9abea405d9aa64302abaee46f8f25`.
That digest covers the whole API document; the SDK exposes only the 15 selected
operations. Another server version or contract has no compatibility claim from
these checks. Regenerate after reviewed contract or package-version changes,
then rerun the drift, installed-package and applicable gateway acceptance gates.

Both public imports expose build facts without a network request:

```typescript
import { SDK_VERSION, API_VERSION, OPENAPI_SHA256 } from "@synveda/sdk";
```

```python
from synveda import SDK_VERSION, API_VERSION, OPENAPI_SHA256
```

`API_VERSION` is the build's target, not an observation of the connected server.
The generator reads each package manifest and checked OpenAPI; request headers
use that generated SDK version. Installed-package checks compare all three
values with their sources and verify the actual client-identification header.
They print the OS, architecture and runtime with content-free archive evidence.

The 2026-09-20 product version bump regenerates the API version and document
digest; all 15 operation bindings and 52 SDK schemas remain unchanged. Local
macOS arm64 checks with Node 24.18.0/Python 3.14.6 pass nine source tests and nine
installed-package tests per SDK, including identical clean builds, installed
types/resources, licence text and 0.4.0 target metadata. Authenticated gateway
acceptance and the Linux runtime matrix were not rerun for this metadata change;
the earlier evidence below remains tied to its named source.

The 2026-09-19 Apache-2.0 increment starting at `9365c92` repeats the two
native arm64 rows below, including exact installed licence/notice checks.
It changes only OpenAPI licence metadata and therefore the document digest;
operation/schema types and the generated console client are unchanged. The
earlier emulated amd64 result remains tied to its original commit.

| Environment | Node / Python | Evidence |
| --- | --- | --- |
| Pinned offline Docker, Linux arm64 | 22.23.2 / 3.11.16 | Nine tests per installed SDK pass; metadata, exports/types/resources and two identical clean builds pass; zero skips. |
| Local macOS arm64 | 24.18.0 / 3.14.6 | The same package checks pass; all archive hashes equal the Docker builds; zero skips. |
| Pinned offline Docker, emulated Linux amd64, at `09ff50a` | 22.23.2 / 3.11.16 | Nine tests per installed SDK pass with zero skips and matching archive hashes; emulation on the arm64 Docker host is not native CI evidence. |
| Declared CI, Linux amd64 | 22 / 3.11 and 24 / 3.14 | Both rows run SDK drift and archive checks; only the minimum pair runs gateway acceptance. Remote execution remains unverified. |

The package manifests' Node 22+/Python 3.11+ requirements are minimums, not
proof for every higher runtime or platform. Public support windows and
deprecation commitments remain owner decisions. The existing authenticated
workflow evidence below belongs to its named source/deployment; archive replay
does not extend it to another server.

| Publishing prerequisite | Current state / required decision |
| --- | --- |
| First-party licence | Apache-2.0, matching the repository. Root `LICENSE` and `NOTICE` are canonical; the existing generator copies them into both packages. Archive checks verify installed licence metadata and byte-identical text. Third-party terms and the dependency allowlist remain unchanged. |
| Package namespaces | Both packages remain Synveda-managed: `@synveda/sdk` on npm and `synveda-sdk` on PyPI. GitHub confirms repository ownership by the `synveda` organization; registry publishing rights are separate and remain unverified. The npm package remains private and the Python name remains a local build name. |
| Publisher and provenance | Confirm release owner, trusted publisher identities and signing/provenance custody before adding publication automation. |
| Support policy | Adopt or revise the measured runtime/server window and define compatibility/deprecation policy; no broader range is inferred here. |

The [release decision proposal](../docs/backlog/ADPT-4.md#release-decision-proposal)
names a first candidate, an SDK-only OIDC publishing path, a narrow compatibility
policy and three implementation batches. The owner has confirmed repository
licence inheritance and Synveda stewardship; the release mechanics and support
policy remain proposed choices. The older product release `v0.2.0` predates this
checked contract, so a release must name
the exact tested server source/image and digest, not just an API version string.

## Check local package archives

After installing the locked development dependencies above, prepare a wheelhouse
for the Python runtime/platform being tested. The explicit check requires it;
missing prerequisites fail. It builds the existing Hatchling sdist and then a
wheel from that sdist, uses npm's `prepack` build hook, and reuses the existing
wire tests through the installed packages. Test/workflow files stay outside the
npm archive. It also checks TypeScript declarations and Python's packaged
OpenAPI digest, generated imports and `py.typed` without development packages
in either consumer. Scratch consumers are removed when the check finishes.

```sh
export SYNVEDA_SDK_WHEELHOUSE="$(mktemp -d)"
python -m pip download --require-hashes --only-binary=:all: \
  -r sdks/python/requirements-dev.lock --dest "$SYNVEDA_SDK_WHEELHOUSE"
make sdk-package-check
```

For Docker-first minimum-runtime validation, use the pinned images below.
The checkout needs its frozen pnpm dependencies; the Python download and build
both run in the target image so native development wheels match its platform.
Only dependency preparation needs network access. These disposable containers
do not require a running gateway or credentials.

```sh
SYNVEDA_SDK_NODE_IMAGE=node@sha256:7725a5c2c83eed1d36258c66efae14b1ceccd021db9ed1d9559d3335ed3d68ed
SYNVEDA_SDK_PYTHON_IMAGE=python@sha256:9534e5a8e315485d4061ed659af0fd78a284c015f9b73661b41d6bab25604534
export SYNVEDA_SDK_WHEELHOUSE="$(mktemp -d)"
docker run --rm -v "$PWD:/source:ro" -v "$SYNVEDA_SDK_WHEELHOUSE:/wheels" \
  "$SYNVEDA_SDK_PYTHON_IMAGE" python -m pip download --no-cache-dir \
  --require-hashes --only-binary=:all: \
  -r /source/sdks/python/requirements-dev.lock --dest /wheels
docker run --rm --network none -v "$PWD:/source:ro" -w /source \
  "$SYNVEDA_SDK_NODE_IMAGE" node scripts/check-sdk-package.mjs
docker run --rm --network none -v "$PWD:/source:ro" \
  -v "$SYNVEDA_SDK_WHEELHOUSE:/wheels:ro" -w /source \
  -e SYNVEDA_SDK_WHEELHOUSE=/wheels "$SYNVEDA_SDK_PYTHON_IMAGE" sh -ec '
    python -m venv /tmp/builder
    /tmp/builder/bin/python -m pip install --no-index --no-cache-dir \
      --only-binary=:all: --find-links /wheels --require-hashes \
      -r sdks/python/requirements-dev.lock
    /tmp/builder/bin/python scripts/check-sdk-package.py'
```

To repeat the emulated Linux amd64 minimum-runtime check on an arm64 Docker
host, pass `--platform linux/amd64` to all three containers and prepare a
separate wheelhouse for that architecture. The measured child images were
`node@sha256:b6ca9eabea5fc816699178b5dd4842270e1c42f90739afc93bbcfdf91a13512e`
and `python@sha256:d1053354624536b044162aaab1e418bd000ea35184fb1ae098ab3166b1072e72`.
The actual checks ran as UID/GID 1000 with a read-only root, dropped capabilities,
`no-new-privileges` and an executable `/tmp` tmpfs (256 MiB for Node, 512 MiB
for Python). The tmpfs must permit execution of the isolated Python venv.

The root [licence](../LICENSE) and [notice](../NOTICE) are copied by
`scripts/generate-sdk-contract.mjs`; do not edit the SDK copies independently.
`sdk-check` rejects copy or SPDX metadata drift. `sdk-package-check` verifies
the licence/notice and standard metadata in the extracted npm archive and
Python sdist/wheel, including their installed distributions.

The scripts print archive SHA-256 values and runtime versions. Python builds
use a fixed `SOURCE_DATE_EPOCH`; matching clean builds are reproducibility
evidence for those inputs, not signatures or a release support commitment.
The results also identify the SDK/API versions and source OpenAPI digest.
Public namespace control and signing/provenance ownership remain open in
[ADPT-4](../docs/backlog/ADPT-4.md).

## Application boundary

Create or obtain a Synveda Session once for an application task. Save its ID
with the application's task state and pass that same ID to subsequent SDK and
MCP calls. Reconnects do not create or end tasks. The task owner explicitly
requests Capture and ends the Session when appropriate.

Python imports `Client`, `ApiError` and `TransportError` from `synveda`, with
typed operations in `synveda.operations`. TypeScript imports them from the
local `@synveda/sdk` workspace package and calls `client.request(operationId,
options)`. Request/response types and operation paths are generated, while
path and query argument names are checked against generated metadata at runtime.
Only the allowlisted contract slice can be called.

Supply an async bearer provider: Python receives `refresh: bool`; TypeScript
receives `refresh: boolean` and an `AbortSignal`. Reuse the application's
maintained OIDC client or the existing `synveda auth token --json` command
after `synveda login`. When using CLI credentials, bind the client to the
returned `gateway_url`; a repository setting must not redirect that bearer.
Keycloak remains the Compose reference IdP through generic OIDC/PKCE.

The default total deadline is 30 seconds (maximum 120); request and response
bodies default to 8 MiB (maximum 64 MiB). Python cancellation propagates as
`asyncio.CancelledError`; TypeScript accepts a caller signal. Credential
providers must cooperate with cancellation. Responses expose status, trace ID
and Retry-After; typed API errors retain the public error taxonomy. Correlation
prefers the gateway's `X-Synveda-Trace-Id` response header because the public
Compose proxy removes incoming trace context. Missing or invalid response IDs
fall back to the sent trace ID, which may differ from the server's trace on an
older deployment. Session and artifact IDs remain the durable audit anchors. The SDK
does not log requests, credentials or returned content and refuses redirects.

After a 401, only GET or an operation with a required public idempotency key
may refresh and retry once, using changed credentials and identical request
bytes/key/trace. Rate limits, transient failures and ambiguous writes return
to the application. Event append uses stable `client_event_id` values and is
never automatically replayed. Pagination is explicit: supply the returned
`after`/cursor value to the next bounded request.

## Shared workflow example

`typescript/src/workflow.mts` and `python/workflow.py` execute the same scenario:
allowed context, exact approved Skill file, observation, pending proposal,
idempotent proposal replay, cross-workspace denial and content-free audit
correlation. They are acceptance examples for synthetic data, not an agent loop.
The fixture in `crates/synveda-gateway/tests/support/sdk_interop.rs` provisions
their inputs through existing tenant fixtures and public APIs.

To run them against a separately prepared deployment, supply one scenario JSON
with `gateway`, `session_id`, `scope_id`, `project_id`, `workspace_id`, `skill_id`,
`version_id`, `knowledge_id`, `denied_session_id`, `allowed_marker`, `skill_marker`,
`proposal_marker`, `query` and a unique `run_key`. The named Session and
Knowledge must be readable by the ordinary member; the Skill must have an
approved enabled binding and its exact `SKILL.md` must contain the nonempty
`skill_marker`; the policy must require Knowledge review; the
unrelated Session must actually exist in another workspace. An auditor is a
separate principal with the tenant audit grant.

Point `SYNVEDA_TOKEN_FILE` and `SYNVEDA_AUDITOR_TOKEN_FILE` at private token files
obtained through the deployment's ordinary authentication. Keep token files
outside the checkout, with owner-only permissions. The examples re-read them
through the bearer callback; the application remains responsible for rotation.

```sh
node sdks/typescript/dist/workflow.mjs /path/to/scenario.json
python sdks/python/workflow.py /path/to/scenario.json
```

Their output contains only client, Session, ContextRun, proposal and trace IDs.
Follow `deploy/compose/README.md` for the canonical Keycloak/issuer/hosts/secret
setup. Reusing an old deployment is not evidence of passing fresh Compose
acceptance; the named live results are recorded in the
[Codex](../docs/integrations/codex.md) and
[Copilot](../docs/integrations/copilot-cli.md) guides and their linked fixtures.
