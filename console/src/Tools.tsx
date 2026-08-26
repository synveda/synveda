/**
 * Trusted MCP catalogue routes. Displayed capabilities are evidence, never
 * authority, and every read or mutation uses the generated public API.
 */

import { request } from "./client.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import { mayWriteToolsAt } from "./tools.mjs";
import { ToolCatalogue } from "./tools/Catalogue.js";
import { ProjectBindingPanel, ProjectConfiguration } from "./tools/ProjectBindings.js";
import { ToolVersionWorkspace } from "./tools/Versions.js";
import type { ToolServerVersionListView, ToolServerView } from "./generated/api.js";

export function Tools() {
  const { me, project } = useApp();
  const canImport = project !== null && mayWriteToolsAt(me.anchors, project.scope_id);

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
      <ToolCatalogue project={project} canImport={canImport} />
      {project ? <ProjectConfiguration project={project} /> : null}
    </>
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
        {(server) => (
          <ToolServerDetail key={server.current_version_id ?? "quarantined"} server={server} />
        )}
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
              <ToolVersionWorkspace
                server={server}
                versions={body.versions}
                canChangeServer={canChangeServer}
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
