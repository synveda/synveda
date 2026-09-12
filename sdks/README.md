# Authenticated public API clients (ADPT-4)

This initial, unpublished slice covers 15 operations from Synveda's checked
OpenAPI: Sessions, observations, Context, approved immutable Skills, Knowledge
proposals and audit pages. The generated contracts include the source SHA-256;
they target the 0.2.0 API in this checkout. Broader API coverage, published
packages and a supported server/runtime matrix remain in `docs/backlog/ADPT-4.md`.

TypeScript uses Node's maintained Fetch implementation; Python uses HTTPX.
Neither client implements retrieval, policy, OAuth, orchestration or automatic
task completion. The gateway owns Cedar, RLS, VedaFlow and audit.

## Install locally and check

Node 22+ and Python 3.11+ are required. Installed-archive validation passed all
nine tests per SDK in pinned Linux arm64 containers on Node 22.23.2/Python
3.11.16 and locally on macOS arm64 Node 24.18.0/Python 3.14.6. Each run built
twice from clean staging directories, installed offline into separate consumers
and checked exported types/resources. Both runs produced identical archives.
CI declares Node 22/Python 3.11 on Linux amd64; that remote job was not executed
locally. The full local CI and fresh database suite passed at `8f40237`;
neither was rerun for the packaging change. These checks do not qualify live OIDC.
No public npm/PyPI package is published by this work.

The later live Keycloak run passed both examples through the canonical Docker
public proxy on a native Codex-created Session. It covered allowed context,
approved Skill bytes, pending/idempotent proposals, workspace denial and audit
correlation. Capture, explicit task-owner end, cross-session reuse and audit-
chain verification also passed. See `docs/INTEROPERABILITY_PLAN.md` and the
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

The scripts print archive SHA-256 values and runtime versions. Python builds
use a fixed `SOURCE_DATE_EPOCH`; matching clean builds are reproducibility
evidence for those inputs, not signatures or a release support commitment.
Public namespace, licence and signing/provenance ownership remain open in
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
acceptance; the execution results are recorded in `docs/INTEROPERABILITY_PLAN.md`.
