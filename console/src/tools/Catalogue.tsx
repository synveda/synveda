import { useState, type FormEvent } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "../Query.js";
import { Link, navigate } from "../Router.js";
import { whenOf } from "../people.mjs";
import { hrefOf } from "../routes.mjs";
import { parseJsonObject } from "../tools.mjs";
import type {
  ProjectView,
  ToolServerDescriptorBody,
  ToolServerListView,
  ToolServerView,
} from "../generated/api.js";
import {
  INITIAL_CAPABILITIES,
  inputErrorMessage,
  mutationNotice,
  ToolMutationNotice,
  type ToolMutationNoticeValue,
} from "./forms.js";

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

export function ToolCatalogue({
  project,
  canImport,
}: {
  project: ProjectView | null;
  canImport: boolean;
}) {
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

  return (
    <>
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
  const [notice, setNotice] = useState<ToolMutationNoticeValue | null>(null);
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
      setNotice({ kind: "error", message: inputErrorMessage(cause) });
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
    if (result.kind === "result") {
      invalidate("tools/catalogue");
      if (result.value.server_id) {
        navigate(hrefOf("tool-server", { server_id: result.value.server_id }));
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
            <select value={mode} onChange={(event) => setMode(event.target.value as typeof mode)}>
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
      <ToolMutationNotice notice={notice} />
    </section>
  );
}

function appendServers(before: ToolServerView[], next: ToolServerView[]): ToolServerView[] {
  const byId = new Map(before.map((server) => [server.id, server]));
  for (const server of next) byId.set(server.id, server);
  return [...byId.values()];
}
