import { useState, type FormEvent } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import { Link } from "../Router.js";
import { whenOf } from "../people.mjs";
import { hrefOf } from "../routes.mjs";
import {
  capabilityEntries,
  descriptorForDisplay,
  diffCount,
  diffSections,
  displayJson,
  parseJsonObject,
  versionStateLabel,
  type ToolCapabilityFamily,
} from "../tools.mjs";
import type {
  ToolServerView,
  ToolServerVersionView,
  ToolVersionDiffView,
} from "../generated/api.js";
import { ToolConnectivityEvidence } from "./Connectivity.js";
import {
  INITIAL_CAPABILITIES,
  inputErrorMessage,
  mutationNotice,
  ToolMutationNotice,
  type ToolMutationNoticeValue,
} from "./forms.js";

export function ToolVersionWorkspace({
  server,
  versions,
  canChangeServer,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  canChangeServer: boolean;
}) {
  return (
    <>
      {canChangeServer && server.current_version_id ? <DiscoveryForm server={server} /> : null}
      {!canChangeServer ? (
        <p className="muted">
          This governing scope does not forecast <code>tool.write</code>, so discovery and
          test-report actions are not offered.
        </p>
      ) : null}
      <VersionExplorer server={server} versions={versions} canReportTest={canChangeServer} />
    </>
  );
}

function DiscoveryForm({ server }: { server: ToolServerView }) {
  const [open, setOpen] = useState(false);
  const [capabilities, setCapabilities] = useState(INITIAL_CAPABILITIES);
  const [notice, setNotice] = useState<ToolMutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let parsed: Record<string, unknown>;
    try {
      parsed = parseJsonObject(capabilities, "Discovery snapshot");
    } catch (cause) {
      setNotice({ kind: "error", message: inputErrorMessage(cause) });
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
    if (result.kind === "result") invalidate(`tools/server/${server.id}`, "tools/catalogue");
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
      <ToolMutationNotice notice={notice} />
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
      <VersionDetail server={server} version={version} canReportTest={canReportTest} />
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
      <ToolConnectivityEvidence server={server} version={version} canReport={canReportTest} />
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
