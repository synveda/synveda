import { useState, type FormEvent } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import { whenOf } from "../people.mjs";
import { displayJson, parseJsonObject, READ_ONLY_METHODS } from "../tools.mjs";
import type {
  ToolServerView,
  ToolServerVersionView,
  ToolTestRunListView,
  ToolTestRunView,
} from "../generated/api.js";
import {
  failedAnswerMessage,
  inputErrorMessage,
  ToolTestNotice,
  type ToolTestNoticeValue,
} from "./forms.js";

export function ToolConnectivityEvidence({
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
  const [notice, setNotice] = useState<ToolTestNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let parsed: Record<string, unknown>;
    try {
      parsed = parseJsonObject(evidence, "Test evidence");
    } catch (cause) {
      setNotice({ kind: "error", message: inputErrorMessage(cause) });
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
      setNotice({ kind: "result", value: answer.body });
      invalidate(`tools/server/${server.id}/versions/${version.id}/tests`);
    } else {
      setNotice({ kind: "error", message: failedAnswerMessage(answer) });
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
            <textarea
              rows={6}
              value={evidence}
              onChange={(event) => setEvidence(event.target.value)}
            />
          </label>
          <p>
            <button type="submit" disabled={busy}>
              {busy ? "Recording…" : "Record report"}
            </button>
          </p>
        </form>
      ) : null}
      <ToolTestNotice notice={notice} />
    </div>
  );
}
