import { useState } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import { whenOf } from "../people.mjs";
import { activationEvidence, evidenceLabel } from "../skills.mjs";
import { failedAnswerMessage, JsonValue } from "./ui.js";
import type {
  SkillTestRunListView,
  SkillUsageListView,
  SkillVersionView,
  SkillView,
} from "../generated/api.js";

export function FixtureTests({
  skill,
  version,
  canRun,
}: {
  skill: SkillView;
  version: SkillVersionView;
  canRun: boolean;
}) {
  const key = `skills/item/${skill.id}/versions/${version.id}/tests`;
  const entry = useQuery(key, () =>
    request("list_skill_tests", {
      path: { id: skill.id, version_id: version.id },
      query: { limit: "50" },
    }),
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    setNotice(null);
    const answer = await request("run_skill_test", {
      path: { id: skill.id, version_id: version.id },
      body: { harness: "validation_sandbox" },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    if (answer.kind === "ok") {
      setNotice("Validation sandbox passed and recorded immutable evidence.");
      invalidate(key);
    } else {
      setNotice(failedAnswerMessage(answer));
    }
  };
  return (
    <section>
      <h4>Fixture testing</h4>
      <div className="banner warning">
        The built-in <strong>validation sandbox</strong> parses, scans and scores this exact bundle.
        It executes no Skill scripts. Controlled-client results are labelled separately by the
        adapter harness that observed them.
      </div>
      {canRun ? (
        <p>
          <button type="button" disabled={busy} onClick={() => void run()}>
            {busy ? "Running…" : "Run validation sandbox"}
          </button>
        </p>
      ) : null}
      {notice ? (
        <div className="banner" role="status">
          {notice}
        </div>
      ) : null}
      <Loaded<SkillTestRunListView> entry={entry} what="Skill test history">
        {(body) =>
          body.runs.length === 0 ? (
            <p className="muted">No controlled test evidence has been recorded.</p>
          ) : (
            <ul className="evidence-list">
              {body.runs.map((run) => (
                <li key={run.id}>
                  <strong>{run.outcome}</strong> · {run.harness.replaceAll("_", " ")} ·{" "}
                  {run.harness_version} · {whenOf(run.created_at)}
                  <JsonValue value={run.evidence} empty="No evidence fields." />
                </li>
              ))}
            </ul>
          )
        }
      </Loaded>
    </section>
  );
}

export function UsageEvidence({ skill, version }: { skill: SkillView; version: SkillVersionView }) {
  const key = `skills/item/${skill.id}/versions/${version.id}/usage`;
  const entry = useQuery(key, () =>
    request("list_skill_usage", {
      path: { id: skill.id, version_id: version.id },
      query: { limit: "100" },
    }),
  );
  return (
    <section>
      <h4>Recent activation evidence</h4>
      <Loaded<SkillUsageListView> entry={entry} what="Skill usage evidence">
        {(body) => {
          const counts = activationEvidence(body.events);
          return body.events.length === 0 ? (
            <p className="muted">
              No usage evidence has been recorded. Absence of evidence is not proof that a model
              did not mention the Skill.
            </p>
          ) : (
            <>
              <p className="muted">
                {counts.activated} activations · {counts.hostObserved} host-observed events ·{" "}
                {counts.modelReported} model-reported events
              </p>
              <ul className="evidence-list">
                {body.events.map((event) => (
                  <li key={event.id}>
                    <strong>{event.stage.replaceAll("_", " ")}</strong>{" "}
                    <span className={`evidence ${event.evidence}`}>
                      {evidenceLabel(event.evidence)}
                    </span>
                    {event.resource_path ? ` · ${event.resource_path}` : ""} ·{" "}
                    {whenOf(event.occurred_at)}
                    {event.session_id ? (
                      <div className="muted">session {event.session_id}</div>
                    ) : null}
                  </li>
                ))}
              </ul>
            </>
          );
        }}
      </Loaded>
    </section>
  );
}
