/**
 * Trusted MCP Tools catalogue (CPR-26, ADR-0086).
 *
 * The console inspects and governs registry metadata through generated public
 * operations. It never launches stdio, connects to a remote server, resolves
 * a secret reference or treats a declared capability as permission.
 */

import { useState, type FormEvent } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link, navigate } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import {
  capabilityEntries,
  descriptorForDisplay,
  diffCount,
  diffSections,
  displayJson,
  mayWriteToolsAt,
  MCP_PROTOCOL_VERSION,
  parseJsonObject,
  READ_ONLY_METHODS,
  toolMutationMessage,
  versionStateLabel,
  type ToolCapabilityFamily,
} from "./tools.mjs";
import type {
  ProjectView,
  ToolBindingListView,
  ToolBindingView,
  ToolClientConfigurationView,
  ToolMutationView,
  ToolServerDescriptorBody,
  ToolServerListView,
  ToolServerVersionListView,
  ToolServerVersionView,
  ToolServerView,
  ToolTestRunListView,
  ToolTestRunView,
  ToolVersionDiffView,
} from "./generated/api.js";

type MutationNoticeValue = { result?: ToolMutationView; error?: string };
type TestNoticeValue = { result?: ToolTestRunView; error?: string };

const INITIAL_CAPABILITIES = JSON.stringify(
  {
    protocol_version: MCP_PROTOCOL_VERSION,
    server_info: { name: "pulseboard-tools", version: "1.0.0" },
    tools: [],
    resources: [],
    prompts: [],
  },
  null,
  2,
);

const INITIAL_DESCRIPTOR = JSON.stringify(
  {
    source_kind: "manifest",
    source_reference: "manifest:pulseboard-tools",
    transport: "streamable_http",
    endpoint: "https://mcp.example.test/mcp",
    authentication: "none",
    requested_permissions: [],
    metadata: {},
  },
  null,
  2,
);

const INITIAL_CLIENT_SERVER = JSON.stringify(
  { url: "https://mcp.example.test/mcp" },
  null,
  2,
);

export function Tools() {
  const { me, project } = useApp();
  const [cursor, setCursor] = useState<string | null>(null);
  const [seen, setSeen] = useState<ToolServerView[]>([]);
  const [importing, setImporting] = useState(false);
  const key = `tools/catalogue/${cursor ?? "first"}`;
  const entry = useQuery(key, () =>
    request("list_tool_servers", {
      query: { cursor: cursor ?? undefined, limit: "100" },
    }),
  );
  const retry = useRefresh(key);
  const canImport =
    project !== null && mayWriteToolsAt(me.anchors, project.scope_id);

  return (
    <>
      <PageHeading route="tools" />
      <p className="tools-intro">
        Inspect immutable MCP discovery evidence, approve changed versions through VedaFlow and
        bind one exact approved version to this project. The catalogue advertises metadata; it
        never grants execution authority.
      </p>
      <div className="banner warning">
        Synveda does not launch imported commands or proxy <code>tools/call</code>. Credentials
        stay behind secret references resolved by a trusted client adapter.
      </div>
      <div className="knowledge-toolbar">
        {canImport ? (
          <button type="button" onClick={() => setImporting((value) => !value)}>
            {importing ? "Close importer" : "Import MCP server"}
          </button>
        ) : null}
      </div>
      {project === null ? (
        <p className="muted">Select a project before importing or binding an MCP server.</p>
      ) : null}
      {project && !canImport ? (
        <p className="muted">
          This project does not forecast <code>tool.write</code>, so import and binding controls
          are not offered. Every read still meets its own gateway decision.
        </p>
      ) : null}
      {project && importing && canImport ? <ImportServer project={project} /> : null}
      <Loaded<ToolServerListView> entry={entry} what="the MCP Tools catalogue" onRetry={retry}>
        {(body) => {
          const servers = appendServers(seen, body.servers);
          return (
            <>
              <h2>Trusted catalogue</h2>
              {servers.length === 0 ? (
                <p className="muted">
                  No MCP server is visible under this policy. A denied aggregate is omitted, so
                  this does not disclose whether one exists elsewhere.
                </p>
              ) : (
                <ul className="tool-catalogue-list">
                  {servers.map((server) => (
                    <ToolServerRow key={server.id} server={server} />
                  ))}
                </ul>
              )}
              {body.next_cursor ? (
                <p>
                  <button
                    type="button"
                    onClick={() => {
                      setSeen(servers);
                      setCursor(body.next_cursor ?? null);
                    }}
                  >
                    Load more
                  </button>{" "}
                  <span className="muted">
                    The cursor advances over catalogue rows considered by the PDP, including rows
                    it did not disclose.
                  </span>
                </p>
              ) : null}
            </>
          );
        }}
      </Loaded>
      {project ? <ProjectConfiguration project={project} /> : null}
    </>
  );
}

function ToolServerRow({ server }: { server: ToolServerView }) {
  return (
    <li>
      <Link href={hrefOf("tool-server", { server_id: server.id })} className="row">
        <strong>{server.name}</strong>{" "}
        {server.current_version_id ? (
          <span className="tag done">approved head</span>
        ) : (
          <span className="tag quarantined">awaiting review</span>
        )}
        <p className="muted">
          Stable server {server.id} · governing scope {server.governing_scope_id}
        </p>
        <div className="muted">
          {server.current_version_id
            ? `Exact approved version ${server.current_version_id}`
            : "No version is approved or bindable yet"}
          {" · "}updated {whenOf(server.updated_at)}
        </div>
      </Link>
    </li>
  );
}

function ImportServer({ project }: { project: ProjectView }) {
  const [mode, setMode] = useState<"manifest" | "client_config">("manifest");
  const [name, setName] = useState("pulseboard-tools");
  const [descriptor, setDescriptor] = useState(INITIAL_DESCRIPTOR);
  const [client, setClient] = useState("claude_code");
  const [clientServer, setClientServer] = useState(INITIAL_CLIENT_SERVER);
  const [secretReference, setSecretReference] = useState("");
  const [capabilities, setCapabilities] = useState(INITIAL_CAPABILITIES);
  const [notice, setNotice] = useState<MutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let parsedCapabilities: Record<string, unknown>;
    let parsedDescriptor: Record<string, unknown> | null = null;
    let parsedClientServer: Record<string, unknown> | null = null;
    try {
      parsedCapabilities = parseJsonObject(capabilities, "Capability snapshot");
      if (mode === "manifest") {
        parsedDescriptor = parseJsonObject(descriptor, "Server descriptor");
      } else {
        parsedClientServer = parseJsonObject(clientServer, "Client server entry");
      }
    } catch (cause) {
      setNotice({ error: errorMessage(cause) });
      return;
    }
    setBusy(true);
    setNotice(null);
    const answer =
      mode === "manifest"
        ? await request("register_tool_server", {
            body: {
              governing_scope_id: project.scope_id,
              name,
              descriptor: parsedDescriptor as ToolServerDescriptorBody,
              capabilities: parsedCapabilities,
            },
            idempotencyKey: idempotencyKey(),
          })
        : await request("import_tool_client_config", {
            body: {
              governing_scope_id: project.scope_id,
              client,
              name,
              server: parsedClientServer as Record<string, unknown>,
              secret_reference: secretReference.trim() || undefined,
              capabilities: parsedCapabilities,
            },
            idempotencyKey: idempotencyKey(),
          });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.result) {
      invalidate("tools/catalogue");
      if (result.result.server_id) {
        navigate(hrefOf("tool-server", { server_id: result.result.server_id }));
      }
    }
  };

  return (
    <section className="tool-form">
      <h2>Import MCP server</h2>
      <p className="muted">
        Import creates a stable server and quarantined immutable version through VedaFlow. A
        supported client entry may contain a URL or one executable token plus literal arguments;
        embedded environment variables and headers are refused.
      </p>
      <form onSubmit={(event) => void submit(event)}>
        <div className="form-grid">
          <label>
            Import shape
            <select
              value={mode}
              onChange={(event) => setMode(event.target.value as typeof mode)}
            >
              <option value="manifest">Server manifest</option>
              <option value="client_config">Supported client configuration</option>
            </select>
          </label>
          <label>
            Catalogue name
            <input required value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <label>
            Governing project
            <input readOnly value={project.display_name} />
          </label>
          {mode === "client_config" ? (
            <label>
              Client grammar
              <select value={client} onChange={(event) => setClient(event.target.value)}>
                <option value="claude_code">Claude Code</option>
                <option value="cursor">Cursor</option>
                <option value="vscode">VS Code</option>
              </select>
            </label>
          ) : null}
        </div>
        {mode === "manifest" ? (
          <label className="full-field">
            Credential-free descriptor (JSON)
            <textarea
              rows={12}
              value={descriptor}
              onChange={(event) => setDescriptor(event.target.value)}
            />
          </label>
        ) : (
          <>
            <label className="full-field">
              One client server entry (JSON)
              <textarea
                rows={8}
                value={clientServer}
                onChange={(event) => setClientServer(event.target.value)}
              />
            </label>
            <label className="full-field">
              Secret reference, never a credential value
              <input
                placeholder="secret-ref://provider/name"
                value={secretReference}
                onChange={(event) => setSecretReference(event.target.value)}
              />
            </label>
          </>
        )}
        <label className="full-field">
          Complete stateless discovery snapshot (JSON)
          <textarea
            rows={14}
            value={capabilities}
            onChange={(event) => setCapabilities(event.target.value)}
          />
        </label>
        <p>
          <button type="submit" disabled={busy}>
            {busy ? "Submitting…" : "Propose import"}
          </button>
        </p>
      </form>
      <MutationNotice notice={notice} />
    </section>
  );
}

export function ToolServerItem({ serverId }: { serverId: string }) {
  const key = `tools/server/${serverId}`;
  const entry = useQuery(key, () => request("get_tool_server", { path: { id: serverId } }));
  const retry = useRefresh(key);
  return (
    <>
      <PageHeading route="tool-server" />
      <p>
        <Link href={hrefOf("tools")}>← MCP Tools catalogue</Link>
      </p>
      <Loaded<ToolServerView> entry={entry} what="this MCP server" onRetry={retry}>
        {(server) => <ToolServerDetail key={server.current_version_id ?? "quarantined"} server={server} />}
      </Loaded>
    </>
  );
}

function ToolServerDetail({ server }: { server: ToolServerView }) {
  const { me, project } = useApp();
  const key = `tools/server/${server.id}/versions`;
  const versions = useQuery(key, () =>
    request("list_tool_server_versions", {
      path: { id: server.id },
      query: { limit: "200" },
    }),
  );
  const canChangeServer = mayWriteToolsAt(me.anchors, server.governing_scope_id);
  const canBind = project !== null && mayWriteToolsAt(me.anchors, project.scope_id);
  return (
    <article className="tool-detail">
      <header>
        <h2>{server.name}</h2>
        <p>
          {server.current_version_id ? (
            <span className="tag done">approved head</span>
          ) : (
            <span className="tag quarantined">no approved version</span>
          )}
        </p>
        <p className="muted">
          Stable server {server.id} · governing scope {server.governing_scope_id} · created{" "}
          {whenOf(server.created_at)}
        </p>
      </header>
      <Loaded<ToolServerVersionListView> entry={versions} what="immutable MCP versions">
        {(body) =>
          body.versions.length === 0 ? (
            <p className="muted">No policy-visible immutable version exists.</p>
          ) : (
            <>
              {canChangeServer && server.current_version_id ? (
                <DiscoveryForm server={server} />
              ) : null}
              {!canChangeServer ? (
                <p className="muted">
                  This governing scope does not forecast <code>tool.write</code>, so discovery and
                  test-report actions are not offered.
                </p>
              ) : null}
              <VersionExplorer
                server={server}
                versions={body.versions}
                canReportTest={canChangeServer}
              />
              <ProjectBindingPanel
                server={server}
                versions={body.versions}
                project={project}
                canWrite={canBind}
              />
            </>
          )
        }
      </Loaded>
    </article>
  );
}

function DiscoveryForm({ server }: { server: ToolServerView }) {
  const [open, setOpen] = useState(false);
  const [capabilities, setCapabilities] = useState(INITIAL_CAPABILITIES);
  const [notice, setNotice] = useState<MutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let parsed: Record<string, unknown>;
    try {
      parsed = parseJsonObject(capabilities, "Discovery snapshot");
    } catch (cause) {
      setNotice({ error: errorMessage(cause) });
      return;
    }
    if (!server.current_version_id) return;
    setBusy(true);
    setNotice(null);
    const answer = await request("discover_tool_server", {
      path: { id: server.id },
      body: {
        expected_current_version_id: server.current_version_id,
        capabilities: parsed,
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.result) invalidate(`tools/server/${server.id}`, "tools/catalogue");
  };
  return (
    <section className="tool-form">
      <h3>Discovery</h3>
      <button type="button" onClick={() => setOpen((value) => !value)}>
        {open ? "Close discovery reporter" : "Report stateless discovery"}
      </button>
      {open ? (
        <form onSubmit={(event) => void submit(event)}>
          <p className="muted">
            A trusted adapter reports a complete snapshot against exact approved version{" "}
            <span className="mono breakable">{server.current_version_id}</span>. Any capability
            drift mints a quarantined version; the approved pointer and project bindings do not
            move.
          </p>
          <label className="full-field">
            Complete discovery snapshot (JSON)
            <textarea
              rows={14}
              value={capabilities}
              onChange={(event) => setCapabilities(event.target.value)}
            />
          </label>
          <p>
            <button type="submit" disabled={busy}>
              {busy ? "Submitting…" : "Submit discovery evidence"}
            </button>
          </p>
        </form>
      ) : null}
      <MutationNotice notice={notice} />
    </section>
  );
}

function VersionExplorer({
  server,
  versions,
  canReportTest,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  canReportTest: boolean;
}) {
  const [versionId, setVersionId] = useState(versions[0]?.id ?? "");
  const version = versions.find((candidate) => candidate.id === versionId) ?? versions[0];
  if (!version) return null;
  return (
    <section>
      <h3>Immutable versions</h3>
      <label>
        Inspect version{" "}
        <select value={version.id} onChange={(event) => setVersionId(event.target.value)}>
          {versions.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>
              v{candidate.ordinal} · {candidate.state}
              {candidate.id === server.current_version_id ? " · approved head" : ""} ·{" "}
              {whenOf(candidate.created_at)}
            </option>
          ))}
        </select>
      </label>
      <VersionDetail
        server={server}
        version={version}
        canReportTest={canReportTest}
      />
    </section>
  );
}

function VersionDetail({
  server,
  version,
  canReportTest,
}: {
  server: ToolServerView;
  version: ToolServerVersionView;
  canReportTest: boolean;
}) {
  const descriptor = version.descriptor;
  return (
    <div className="tool-version">
      {version.state === "quarantined" ? (
        <div className="banner quarantine" role="status">
          <strong>Quarantined changed version.</strong> It cannot be bound or generated into a
          client configuration until its VedaFlow change applies. Change {version.change_id}.{" "}
          <Link href={hrefOf("reviews")}>Review in Advanced</Link>
        </div>
      ) : null}
      <section>
        <h4>Source, validation and trust</h4>
        <dl className="facts">
          <dt>Version</dt>
          <dd>
            v{version.ordinal} · <span className="mono breakable">{version.id}</span>
          </dd>
          <dt>Trust</dt>
          <dd>{versionStateLabel(version.state)}</dd>
          <dt>Digest</dt>
          <dd className="mono breakable">{version.digest}</dd>
          <dt>Capability digest</dt>
          <dd className="mono breakable">{version.capability_digest}</dd>
          <dt>Protocol</dt>
          <dd>MCP {version.protocol_version} · validated exact stable contract</dd>
          <dt>Source</dt>
          <dd>
            {descriptor.source_kind} · {descriptor.source_reference}
          </dd>
          <dt>Transport</dt>
          <dd>{descriptor.transport}</dd>
          <dt>Endpoint / command</dt>
          <dd>{descriptor.endpoint ?? descriptor.command ?? "Not declared"}</dd>
          <dt>Authentication</dt>
          <dd>{descriptor.authentication}</dd>
          <dt>Secret reference</dt>
          <dd>{version.secret_reference_present ? "configured" : "not configured"}</dd>
          <dt>Metadata validation</dt>
          <dd>Passed bounded, credential-free import validation</dd>
          <dt>Executable scan</dt>
          <dd>Not performed; Synveda stores metadata and never launches the server</dd>
          <dt>Last discovery</dt>
          <dd>{whenOf(version.discovered_at)}</dd>
        </dl>
        <details>
          <summary>Credential-safe descriptor metadata</summary>
          <pre className="json-value">{displayJson(descriptorForDisplay(descriptor))}</pre>
        </details>
        <h5>Requested permissions</h5>
        <div className="banner warning">
          Requested permissions, capability names, descriptions and JSON schemas are evidence for
          review. They grant no authorisation.
        </div>
        {descriptor.requested_permissions && descriptor.requested_permissions.length > 0 ? (
          <ul className="inline-list">
            {descriptor.requested_permissions.map((permission) => (
              <li key={permission}>{permission}</li>
            ))}
          </ul>
        ) : (
          <p className="muted">No permission labels requested.</p>
        )}
      </section>
      <VersionDiff
        serverId={server.id}
        version={version}
        against={server.current_version_id ?? null}
      />
      <section>
        <h4>Discovered capabilities</h4>
        {(["tools", "resources", "prompts"] as const).map((family) => (
          <CapabilityFamily key={family} family={family} version={version} />
        ))}
      </section>
      <TestEvidence server={server} version={version} canReport={canReportTest} />
    </div>
  );
}

function VersionDiff({
  serverId,
  version,
  against,
}: {
  serverId: string;
  version: ToolServerVersionView;
  against: string | null;
}) {
  if (!against || against === version.id) {
    return (
      <section>
        <h4>Approved-version comparison</h4>
        <p className="muted">
          {against
            ? "This is the approved head; select another version to compare it."
            : "No approved baseline exists yet. The first version remains quarantined until review."}
        </p>
      </section>
    );
  }
  return <VersionDiffQuery serverId={serverId} version={version} against={against} />;
}

function VersionDiffQuery({
  serverId,
  version,
  against,
}: {
  serverId: string;
  version: ToolServerVersionView;
  against: string;
}) {
  const key = `tools/server/${serverId}/versions/${version.id}/diff/${against}`;
  const entry = useQuery(key, () =>
    request("diff_tool_server_version", {
      path: { id: serverId, version_id: version.id },
      query: { against },
    }),
  );
  return (
    <section>
      <h4>Approved-version comparison</h4>
      <Loaded<ToolVersionDiffView> entry={entry} what="the immutable version comparison">
        {(diff) => (
          <>
            <p>
              <strong>{diffCount(diff)} visible changes</strong> against exact approved version{" "}
              <span className="mono breakable">{diff.from_version_id}</span>.
            </p>
            {diff.descriptor_changed.length > 0 ? (
              <p>Descriptor changed: {diff.descriptor_changed.join(", ")}</p>
            ) : (
              <p className="muted">No descriptor fields changed.</p>
            )}
            <div className="tool-diff-grid">
              {diffSections(diff).map((section) => (
                <div key={section.label}>
                  <h5>{section.label}</h5>
                  <DiffList label="Added" values={section.added} />
                  <DiffList label="Changed" values={section.changed} />
                  <DiffList label="Removed" values={section.removed} />
                </div>
              ))}
            </div>
          </>
        )}
      </Loaded>
    </section>
  );
}

function DiffList({ label, values }: { label: string; values: string[] }) {
  return (
    <p className={values.length === 0 ? "muted" : ""}>
      {label}: {values.length === 0 ? "none" : values.join(", ")}
    </p>
  );
}

function CapabilityFamily({
  family,
  version,
}: {
  family: ToolCapabilityFamily;
  version: ToolServerVersionView;
}) {
  const entries = capabilityEntries(version, family);
  return (
    <section className="tool-capability-family">
      <h5>
        {family[0]?.toUpperCase()}
        {family.slice(1)} · {entries.length}
      </h5>
      {entries.length === 0 ? (
        <p className="muted">None discovered in this immutable snapshot.</p>
      ) : (
        <ul className="tool-capabilities">
          {entries.map((entry) => (
            <li key={entry.identity}>
              <details>
                <summary>
                  <code>{entry.identity}</code>
                </summary>
                <p>{entry.description ?? "No description supplied."}</p>
                <h6>JSON schema / arguments</h6>
                {entry.schema === null ? (
                  <p className="muted">No schema declared.</p>
                ) : (
                  <pre className="json-value">{displayJson(entry.schema)}</pre>
                )}
                <h6>Normalised metadata</h6>
                <pre className="json-value">{displayJson(entry.details)}</pre>
              </details>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function TestEvidence({
  server,
  version,
  canReport,
}: {
  server: ToolServerView;
  version: ToolServerVersionView;
  canReport: boolean;
}) {
  const key = `tools/server/${server.id}/versions/${version.id}/tests`;
  const entry = useQuery(key, () =>
    request("list_tool_server_tests", {
      path: { id: server.id, version_id: version.id },
      query: { limit: "100" },
    }),
  );
  return (
    <section>
      <h4>Read-only connectivity evidence</h4>
      <div className="banner warning">
        A named trusted adapter performs discovery/list checks and reports the result. This page
        records that evidence; the gateway does not connect to the server or execute a tool.
      </div>
      <Loaded<ToolTestRunListView> entry={entry} what="read-only test evidence">
        {(body) => (
          <>
            {body.runs.length === 0 ? (
              <p className="muted">Health: not tested for this exact version.</p>
            ) : (
              <ul className="tool-test-runs">
                {body.runs.map((run, index) => (
                  <li key={run.id}>
                    <strong>{index === 0 ? "Latest health" : "Historical result"}: </strong>
                    <span className={`tag ${run.outcome === "passed" ? "done" : "quarantined"}`}>
                      {run.outcome}
                    </span>{" "}
                    {run.harness} {run.harness_version} · {whenOf(run.created_at)}
                    {run.latency_ms === null || run.latency_ms === undefined
                      ? ""
                      : ` · ${run.latency_ms}ms`}
                    <p>Read-only methods: {run.methods.join(", ")}</p>
                    <pre className="json-value">{displayJson(run.evidence)}</pre>
                  </li>
                ))}
              </ul>
            )}
            {canReport ? <TestReportForm server={server} version={version} /> : null}
          </>
        )}
      </Loaded>
    </section>
  );
}

function TestReportForm({
  server,
  version,
}: {
  server: ToolServerView;
  version: ToolServerVersionView;
}) {
  const [open, setOpen] = useState(false);
  const [harness, setHarness] = useState<ToolTestRunView["harness"]>(
    version.descriptor.transport === "stdio" ? "trusted_local_adapter" : "remote_http_adapter",
  );
  const [harnessVersion, setHarnessVersion] = useState("trusted-adapter/1");
  const [outcome, setOutcome] = useState<ToolTestRunView["outcome"]>("passed");
  const [latency, setLatency] = useState("");
  const [evidence, setEvidence] = useState('{"executes_tools": false}');
  const [notice, setNotice] = useState<TestNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let parsed: Record<string, unknown>;
    try {
      parsed = parseJsonObject(evidence, "Test evidence");
    } catch (cause) {
      setNotice({ error: errorMessage(cause) });
      return;
    }
    setBusy(true);
    setNotice(null);
    const answer = await request("run_tool_server_test", {
      path: { id: server.id, version_id: version.id },
      body: {
        harness,
        harness_version: harnessVersion,
        outcome,
        methods: [...READ_ONLY_METHODS],
        latency_ms: latency.length > 0 ? Number(latency) : undefined,
        evidence: parsed,
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    if (answer.kind === "ok") {
      setNotice({ result: answer.body });
      invalidate(`tools/server/${server.id}/versions/${version.id}/tests`);
    } else {
      setNotice({ error: failedAnswerMessage(answer) });
    }
  };
  return (
    <div className="tool-test-form">
      <button type="button" onClick={() => setOpen((value) => !value)}>
        {open ? "Close reporter" : "Record trusted adapter test"}
      </button>
      {open ? (
        <form onSubmit={(event) => void submit(event)}>
          <div className="form-grid">
            <label>
              Reporter boundary
              <select
                value={harness}
                onChange={(event) => setHarness(event.target.value as typeof harness)}
              >
                <option value="trusted_local_adapter">Trusted local adapter</option>
                <option value="remote_http_adapter">Remote HTTP adapter</option>
              </select>
            </label>
            <label>
              Reporter version
              <input
                required
                value={harnessVersion}
                onChange={(event) => setHarnessVersion(event.target.value)}
              />
            </label>
            <label>
              Outcome
              <select
                value={outcome}
                onChange={(event) => setOutcome(event.target.value as typeof outcome)}
              >
                <option value="passed">Passed</option>
                <option value="failed">Failed</option>
                <option value="error">Error</option>
              </select>
            </label>
            <label>
              Latency (ms)
              <input
                type="number"
                min="0"
                value={latency}
                onChange={(event) => setLatency(event.target.value)}
              />
            </label>
          </div>
          <p className="muted">Closed read-only methods: {READ_ONLY_METHODS.join(", ")}.</p>
          <label className="full-field">
            Credential-free reporter evidence (JSON)
            <textarea rows={6} value={evidence} onChange={(event) => setEvidence(event.target.value)} />
          </label>
          <p>
            <button type="submit" disabled={busy}>
              {busy ? "Recording…" : "Record report"}
            </button>
          </p>
        </form>
      ) : null}
      <TestNotice notice={notice} />
    </div>
  );
}

function ProjectBindingPanel({
  server,
  versions,
  project,
  canWrite,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  project: ProjectView | null;
  canWrite: boolean;
}) {
  if (!project) {
    return (
      <section>
        <h3>Project binding</h3>
        <p className="muted">Select a project to inspect or change its exact binding.</p>
      </section>
    );
  }
  return (
    <ProjectBindingQuery
      server={server}
      versions={versions}
      project={project}
      canWrite={canWrite}
    />
  );
}

function ProjectBindingQuery({
  server,
  versions,
  project,
  canWrite,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  project: ProjectView;
  canWrite: boolean;
}) {
  const key = `tools/bindings/${project.id}`;
  const entry = useQuery(key, () =>
    request("list_tool_bindings", {
      query: { project_id: project.id, include_removed: "true", limit: "200" },
    }),
  );
  const approved = versions.filter((version) => version.state === "approved");
  return (
    <section>
      <h3>Project binding · {project.display_name}</h3>
      <p className="muted">
        A binding names one exact approved digest. Approving a newer version never changes what
        this project advertises.
      </p>
      <Loaded<ToolBindingListView> entry={entry} what="this project's MCP bindings">
        {(body) => {
          const binding = body.bindings.find((candidate) => candidate.server_id === server.id);
          return binding ? (
            <ExistingBinding
              binding={binding}
              approved={approved}
              canWrite={canWrite}
            />
          ) : (
            <CreateBinding
              server={server}
              project={project}
              approved={approved}
              canWrite={canWrite}
            />
          );
        }}
      </Loaded>
      {versions.some((version) => version.state === "quarantined") ? (
        <p className="muted">
          Quarantined versions are deliberately absent from the binding picker. Review their
          VedaFlow change first.
        </p>
      ) : null}
    </section>
  );
}

function CreateBinding({
  server,
  project,
  approved,
  canWrite,
}: {
  server: ToolServerView;
  project: ProjectView;
  approved: ToolServerVersionView[];
  canWrite: boolean;
}) {
  const [selected, setSelected] = useState(
    approved.find((version) => version.id === server.current_version_id)?.id ?? approved[0]?.id ?? "",
  );
  const [notice, setNotice] = useState<MutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  if (approved.length === 0) {
    return <p className="muted">No approved version exists, so this server cannot be bound.</p>;
  }
  if (!canWrite) {
    return <p className="muted">No binding exists, and policy does not offer creation here.</p>;
  }
  const create = async () => {
    setBusy(true);
    setNotice(null);
    const answer = await request("create_tool_binding", {
      body: {
        project_id: project.id,
        server_id: server.id,
        version_id: selected,
        state: "enabled",
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.result?.outcome === "applied") {
      invalidate("tools/bindings", `tools/config/${project.id}`);
    }
  };
  return (
    <div className="binding-actions">
      <label>
        Exact approved version
        <select value={selected} onChange={(event) => setSelected(event.target.value)}>
          {approved.map((version) => (
            <option key={version.id} value={version.id}>
              v{version.ordinal} · {version.digest.slice(0, 12)}
            </option>
          ))}
        </select>
      </label>
      <button type="button" disabled={busy || !selected} onClick={() => void create()}>
        {busy ? "Submitting…" : "Bind exact version"}
      </button>
      <MutationNotice notice={notice} />
    </div>
  );
}

function ExistingBinding({
  binding,
  approved,
  canWrite,
}: {
  binding: ToolBindingView;
  approved: ToolServerVersionView[];
  canWrite: boolean;
}) {
  const [selected, setSelected] = useState(binding.version_id);
  const [notice, setNotice] = useState<MutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  const change = async (
    state: ToolBindingView["state"],
    reason: "disable" | "enable" | "repin" | "remove",
    versionId = binding.version_id,
  ) => {
    setBusy(true);
    setNotice(null);
    const answer = await request("update_tool_binding", {
      path: { id: binding.id },
      body: {
        expected_revision: binding.revision,
        version_id: versionId,
        state,
        reason,
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.result?.outcome === "applied") {
      invalidate("tools/bindings", `tools/config/${binding.project_id}`);
    }
  };
  return (
    <div className="tool-binding-card">
      <dl className="facts compact">
        <dt>Status</dt>
        <dd>{binding.state}</dd>
        <dt>Exact version</dt>
        <dd className="mono breakable">{binding.version_id}</dd>
        <dt>Revision</dt>
        <dd>{binding.revision}</dd>
        <dt>Updated</dt>
        <dd>{whenOf(binding.updated_at)}</dd>
      </dl>
      {canWrite ? (
        <div className="binding-actions">
          {binding.state === "enabled" ? (
            <button type="button" disabled={busy} onClick={() => void change("disabled", "disable")}>
              Disable
            </button>
          ) : (
            <button type="button" disabled={busy} onClick={() => void change("enabled", "enable")}>
              {binding.state === "removed" ? "Restore binding" : "Enable"}
            </button>
          )}
          <label>
            Exact approved version
            <select value={selected} onChange={(event) => setSelected(event.target.value)}>
              {approved.map((version) => (
                <option key={version.id} value={version.id}>
                  v{version.ordinal} · {version.digest.slice(0, 12)}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy || selected === binding.version_id}
            onClick={() => void change(binding.state, "repin", selected)}
          >
            Repin exact version
          </button>
          {binding.state !== "removed" ? (
            <button type="button" disabled={busy} onClick={() => void change("removed", "remove")}>
              Remove binding
            </button>
          ) : null}
        </div>
      ) : (
        <p className="muted">Policy does not offer binding changes at this project scope.</p>
      )}
      <MutationNotice notice={notice} />
    </div>
  );
}

function ProjectConfiguration({ project }: { project: ProjectView }) {
  const key = `tools/config/${project.id}`;
  const entry = useQuery(key, () =>
    request("generate_tool_client_config", { path: { project_id: project.id } }),
  );
  return (
    <section className="tool-configuration">
      <h2>Generated client configuration · {project.display_name}</h2>
      <p className="muted">
        Only enabled exact approved bindings appear. The browser masks opaque secret-reference
        identifiers; a trusted adapter resolves them outside the gateway.
      </p>
      <Loaded<ToolClientConfigurationView> entry={entry} what="secret-safe client configuration">
        {(body) => (
          <>
            <p>
              {body.bindings.length} exact binding{body.bindings.length === 1 ? "" : "s"}.
            </p>
            {body.bindings.length > 0 ? (
              <ul className="inline-list">
                {body.bindings.map((binding) => (
                  <li key={binding.binding_id}>
                    <span className="mono">{binding.digest.slice(0, 12)}</span> · exact{" "}
                    {binding.version_id}
                  </li>
                ))}
              </ul>
            ) : null}
            <pre className="json-value">{displayJson(body.configuration)}</pre>
          </>
        )}
      </Loaded>
    </section>
  );
}

function MutationNotice({ notice }: { notice: MutationNoticeValue | null }) {
  if (!notice) return null;
  if (notice.error) {
    return (
      <div className="banner error" role="alert">
        {notice.error}
      </div>
    );
  }
  if (!notice.result) return null;
  const tone =
    notice.result.outcome === "rejected"
      ? "error"
      : notice.result.outcome === "pending_review"
        ? "warning"
        : "success";
  return (
    <div className={`banner ${tone}`} role="status">
      {toolMutationMessage(notice.result)} Change {notice.result.change_id}.{" "}
      {notice.result.outcome === "pending_review" ? (
        <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
      ) : null}
    </div>
  );
}

function TestNotice({ notice }: { notice: TestNoticeValue | null }) {
  if (!notice) return null;
  if (notice.error) {
    return (
      <div className="banner error" role="alert">
        {notice.error}
      </div>
    );
  }
  if (!notice.result) return null;
  return (
    <div className="banner success" role="status">
      Recorded immutable {notice.result.outcome} report {notice.result.id} for exact version{" "}
      {notice.result.version_id}.
    </div>
  );
}

function mutationNotice(answer: Answer<ToolMutationView>): MutationNoticeValue {
  return answer.kind === "ok" ? { result: answer.body } : { error: failedAnswerMessage(answer) };
}

function failedAnswerMessage(answer: Exclude<Answer<unknown>, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "The JSON input could not be read.";
}

function appendServers(before: ToolServerView[], next: ToolServerView[]): ToolServerView[] {
  const byId = new Map(before.map((server) => [server.id, server]));
  for (const server of next) byId.set(server.id, server);
  return [...byId.values()];
}
