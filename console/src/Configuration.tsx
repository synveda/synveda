/**
 * Advanced ▸ Configuration (CPR-30, ADR-0089).
 *
 * Templates are sources, artifacts are stable, versions are immutable and a
 * revisioned scope binding is the only runtime selector. Every button below
 * calls a generated public operation; every mutation becomes a typed
 * VedaFlow change at the gateway.
 */

import { useEffect, useMemo, useState, type FormEvent } from "react";

import { idempotencyKey, request } from "./client.mjs";
import {
  TRACE_RETENTION_OPTIONS,
  applyDemoConfigurationDraft,
  configurationSummary,
  configurationTarget,
  demoConfigurationDraft,
  mutationMessage,
  renderConfiguration,
  type DemoConfigurationDraft,
} from "./configuration.mjs";
import type {
  ConfigurationArtifactListView,
  ConfigurationArtifactView,
  ConfigurationBindingListView,
  ConfigurationBindingView,
  ConfigurationComparisonView,
  ConfigurationDocumentBody,
  ConfigurationMutationView,
  ConfigurationTemplateListView,
  ConfigurationTemplateView,
  ConfigurationVersionListView,
  ConfigurationVersionView,
  EffectiveConfigurationView,
  PacksResponse,
  RelaxationListView,
} from "./generated/api.js";
import { whenOf } from "./people.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { hrefOf } from "./routes.mjs";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";

type Notice = { result?: ConfigurationMutationView; error?: string };

function refreshConfiguration(): void {
  invalidate("configuration/", "policy/packs");
}

export function Configuration() {
  const { me, workspace, project } = useApp();
  const target = useMemo(
    () => configurationTarget(me, workspace, project),
    [me, workspace, project],
  );
  const templates = useQuery("configuration/templates", () =>
    request("list_configuration_templates", {}),
  );
  const artifacts = useQuery("configuration/artifacts", () =>
    request("list_configurations", { query: { limit: "100" } }),
  );
  const effective = useQuery(
    `configuration/effective/${target?.id ?? "none"}`,
    () =>
      target
        ? request("get_effective_configuration", { query: { scope_id: target.id } })
        : Promise.resolve({ kind: "invalid" as const, message: "no governed scope is selected" }),
  );

  return (
    <>
      <PageHeading route="configuration" />
      <p>
        One immutable document drives policy, capture, context, freshness, Skill and Tool
        advertisement, and provider boundaries. A profile name is provenance only; runtime code
        reads the exact selected version.
      </p>
      {target ? (
        <p className="muted">
          Editing selection for <strong>{target.label}</strong> · scope {target.id}
        </p>
      ) : (
        <div className="banner warning">Select a workspace or project to manage Configuration.</div>
      )}
      <Effective entry={effective} targetLabel={target?.label ?? "selected scope"} />
      <Loaded<ConfigurationTemplateListView>
        entry={templates}
        what="Configuration templates"
        onRetry={useRefresh("configuration/templates")}
      >
        {(body) => <Templates templates={body.templates} target={target} />}
      </Loaded>
      <Loaded<ConfigurationArtifactListView>
        entry={artifacts}
        what="Configuration artifacts"
        onRetry={useRefresh("configuration/artifacts")}
      >
        {(body) => <Artifacts artifacts={body.artifacts} targetScopeId={target?.id ?? null} />}
      </Loaded>
      <PolicySources />
      <GovernedRelaxations />
    </>
  );
}

function Effective({
  entry,
  targetLabel,
}: {
  entry: ReturnType<typeof useQuery>;
  targetLabel: string;
}) {
  return (
    <section>
      <h2>Effective now at {targetLabel}</h2>
      <Loaded<EffectiveConfigurationView> entry={entry} what="effective Configuration">
        {(body) => (
          <>
            <div className={`banner ${body.fail_safe ? "warning" : "success"}`} role="status">
              <strong>{body.fail_safe ? "Built-in fail-safe is effective" : "A governed version is effective"}</strong>
              <p>{configurationSummary(body.document)}</p>
              <p className="muted">
                {body.fail_safe
                  ? "No enabled binding applies. This is conservative fallback behavior, not a configured or reviewed version."
                  : "A proposed version or binding does not replace this evidence until VedaFlow applies it."}
              </p>
            </div>
            <details className="technical-details">
              <summary>Effective version and selector evidence</summary>
              <dl>
                <dt>Requested scope</dt><dd className="mono breakable">{body.scope_id}</dd>
                <dt>Artifact</dt><dd className="mono breakable">{body.artifact_id ?? "unavailable — fail-safe"}</dd>
                <dt>Version</dt><dd className="mono breakable">{body.version_id ?? "unavailable — fail-safe"}</dd>
                <dt>Binding</dt><dd className="mono breakable">{body.binding_id ?? "unavailable — fail-safe"}</dd>
                <dt>Binding scope</dt><dd className="mono breakable">{body.binding_scope_id ?? "unavailable — fail-safe"}</dd>
                <dt>Content hash</dt><dd className="mono breakable">{body.content_hash}</dd>
              </dl>
            </details>
          </>
        )}
      </Loaded>
    </section>
  );
}

function Templates({
  templates,
  target,
}: {
  templates: ConfigurationTemplateView[];
  target: { id: string; label: string } | null;
}) {
  const [notice, setNotice] = useState<Notice>({});
  const [busy, setBusy] = useState<string | null>(null);

  const create = async (template: ConfigurationTemplateView) => {
    if (!target) return;
    setBusy(template.name);
    setNotice({});
    const created = await request("create_configuration", {
      idempotencyKey: idempotencyKey(),
      body: {
        governing_scope_id: target.id,
        name: `${template.name}-runtime`,
        source_template: template.name,
        document: template.document,
      },
    });
    if (created.kind !== "ok") {
      setNotice({ error: created.kind === "unauthenticated" ? "Your session expired." : created.message });
      setBusy(null);
      return;
    }
    setNotice({ result: created.body });
    if (created.body.outcome === "applied" && created.body.artifact_id) {
      const bound = await request("create_configuration_binding", {
        idempotencyKey: idempotencyKey(),
        body: {
          scope_id: target.id,
          artifact_id: created.body.artifact_id,
          enabled: true,
        },
      });
      if (bound.kind === "ok") setNotice({ result: bound.body });
      else {
        setNotice({
          error: `${mutationMessage(created.body)} Binding failed: ${bound.kind === "unauthenticated" ? "session expired" : bound.message}`,
        });
      }
    }
    setBusy(null);
    refreshConfiguration();
  };

  return (
    <section>
      <h2>Seeded templates</h2>
      <p className="muted">
        Personal, team and enterprise are complete source documents—not editions. Creating one
        proposes an ordinary immutable version. Only when that applies does this page submit the
        separate governed binding; either change may require review.
      </p>
      <ul className="packs">
        {templates.map((template) => (
          <li key={template.name}>
            <strong>{template.name}</strong> <span className="mono">{template.content_hash}</span>
            <div className="muted">{configurationSummary(template.document)}</div>
            <button
              type="button"
              disabled={!target || busy !== null}
              onClick={() => void create(template)}
            >
              {busy === template.name ? "Opening governed change…" : `Propose ${template.name} at ${target?.label ?? "scope"}`}
            </button>
          </li>
        ))}
      </ul>
      <NoticeView notice={notice} />
    </section>
  );
}

function Artifacts({
  artifacts,
  targetScopeId,
}: {
  artifacts: ConfigurationArtifactView[];
  targetScopeId: string | null;
}) {
  const [selected, setSelected] = useState(artifacts[0]?.id ?? "");
  useEffect(() => {
    if (!artifacts.some((artifact) => artifact.id === selected)) setSelected(artifacts[0]?.id ?? "");
  }, [artifacts, selected]);
  return (
    <section>
      <h2>Versioned configurations</h2>
      {artifacts.length === 0 ? (
        <p className="muted">No governed Configuration artifact is visible.</p>
      ) : (
        <>
          <label>
            <span className="switcher-label">Artifact</span>
            <select value={selected} onChange={(event) => setSelected(event.target.value)}>
              {artifacts.map((artifact) => (
                <option value={artifact.id} key={artifact.id}>
                  {artifact.name}
                </option>
              ))}
            </select>
          </label>
          {selected ? <Artifact artifact={artifacts.find((item) => item.id === selected)!} targetScopeId={targetScopeId} /> : null}
        </>
      )}
    </section>
  );
}

function Artifact({
  artifact,
  targetScopeId,
}: {
  artifact: ConfigurationArtifactView;
  targetScopeId: string | null;
}) {
  const versionsKey = `configuration/versions/${artifact.id}`;
  const bindingsKey = `configuration/bindings/${targetScopeId ?? "none"}`;
  const versions = useQuery(versionsKey, () =>
    request("list_configuration_versions", {
      path: { id: artifact.id },
      query: { limit: "100" },
    }),
  );
  const bindings = useQuery(bindingsKey, () =>
    targetScopeId
      ? request("list_configuration_bindings", {
          query: { scope_id: targetScopeId, limit: "100" },
        })
      : Promise.resolve({ kind: "invalid" as const, message: "no governed scope is selected" }),
  );
  return (
    <article className="node-detail">
      <h3>{artifact.name}</h3>
      <p className="muted">
        Stable id {artifact.id} · governed at {artifact.governing_scope_id} · artifact current {artifact.current_version_id} · updated {whenOf(artifact.updated_at)}
      </p>
      <p className="muted">
        “Artifact current” is version history, not proof this artifact is effective at the selected
        scope. The binding and Effective now panels answer that separately.
      </p>
      <Loaded<ConfigurationVersionListView>
        entry={versions}
        what="immutable Configuration versions"
        onRetry={useRefresh(versionsKey)}
      >
        {(versionPage) => (
          <Loaded<ConfigurationBindingListView>
            entry={bindings}
            what="scope Configuration bindings"
            onRetry={useRefresh(bindingsKey)}
          >
            {(bindingPage) => (
              <ArtifactControls
                artifact={artifact}
                versions={versionPage.versions}
                binding={bindingPage.bindings.find((entry) => entry.artifact_id === artifact.id) ?? null}
                targetScopeId={targetScopeId}
              />
            )}
          </Loaded>
        )}
      </Loaded>
    </article>
  );
}

function ArtifactControls({
  artifact,
  versions,
  binding,
  targetScopeId,
}: {
  artifact: ConfigurationArtifactView;
  versions: ConfigurationVersionView[];
  binding: ConfigurationBindingView | null;
  targetScopeId: string | null;
}) {
  const current = versions.find((version) => version.id === artifact.current_version_id) ?? versions[0];
  const [draft, setDraft] = useState<DemoConfigurationDraft | null>(
    current ? demoConfigurationDraft(current.document) : null,
  );
  const [from, setFrom] = useState(versions[1]?.id ?? versions[0]?.id ?? "");
  const [to, setTo] = useState(versions[0]?.id ?? "");
  const [comparison, setComparison] = useState<ConfigurationComparisonView | null>(null);
  const [rollback, setRollback] = useState(versions[1]?.id ?? "");
  const [notice, setNotice] = useState<Notice>({});
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setDraft(current ? demoConfigurationDraft(current.document) : null);
  }, [current?.id]);

  const changed = async () => {
    if (!from || !to) return;
    const outcome = await request("compare_configuration_versions", {
      path: { id: artifact.id },
      query: { from, to },
    });
    if (outcome.kind === "ok") setComparison(outcome.body);
    else setNotice({ error: outcome.kind === "unauthenticated" ? "Your session expired." : outcome.message });
  };

  const publish = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setNotice({});
    if (!current || !draft) {
      setNotice({ error: "No immutable version is available to edit." });
      setBusy(false);
      return;
    }
    let document: ConfigurationDocumentBody;
    try {
      document = applyDemoConfigurationDraft(current.document, draft);
    } catch (error) {
      setNotice({ error: error instanceof Error ? error.message : "The settings are invalid." });
      setBusy(false);
      return;
    }
    const outcome = await request("publish_configuration_version", {
      path: { id: artifact.id },
      idempotencyKey: idempotencyKey(),
      body: { expected_current_version_id: artifact.current_version_id, document },
    });
    setBusy(false);
    if (outcome.kind === "ok") setNotice({ result: outcome.body });
    else setNotice({ error: outcome.kind === "unauthenticated" ? "Your session expired." : outcome.message });
    refreshConfiguration();
  };

  const updateDraft = (next: Partial<DemoConfigurationDraft>) => {
    setDraft((value) => (value ? { ...value, ...next } : value));
  };

  let preview: ConfigurationDocumentBody | null = null;
  if (current && draft) {
    try {
      preview = applyDemoConfigurationDraft(current.document, draft);
    } catch {
      // The submit path renders the contract-aligned error. Until then the
      // disclosure stays explicit rather than showing a stale preview.
    }
  }

  const bind = async () => {
    if (!targetScopeId) return;
    setBusy(true);
    const outcome = await request("create_configuration_binding", {
      idempotencyKey: idempotencyKey(),
      body: { scope_id: targetScopeId, artifact_id: artifact.id, enabled: true },
    });
    setBusy(false);
    if (outcome.kind === "ok") setNotice({ result: outcome.body });
    else setNotice({ error: outcome.kind === "unauthenticated" ? "Your session expired." : outcome.message });
    refreshConfiguration();
  };

  const updateBinding = async (enabled: boolean, pinnedVersionId: string | null, reason: string) => {
    if (!binding) return;
    setBusy(true);
    const outcome = await request("update_configuration_binding", {
      path: { id: binding.id },
      idempotencyKey: idempotencyKey(),
      body: {
        expected_revision: binding.revision,
        artifact_id: artifact.id,
        pinned_version_id: pinnedVersionId,
        enabled,
        reason,
      },
    });
    setBusy(false);
    if (outcome.kind === "ok") setNotice({ result: outcome.body });
    else setNotice({ error: outcome.kind === "unauthenticated" ? "Your session expired." : outcome.message });
    refreshConfiguration();
  };

  const rollBack = async () => {
    if (!binding || !rollback) return;
    setBusy(true);
    const outcome = await request("rollback_configuration_binding", {
      path: { id: binding.id },
      idempotencyKey: idempotencyKey(),
      body: { expected_revision: binding.revision, version_id: rollback },
    });
    setBusy(false);
    if (outcome.kind === "ok") setNotice({ result: outcome.body });
    else setNotice({ error: outcome.kind === "unauthenticated" ? "Your session expired." : outcome.message });
    refreshConfiguration();
  };

  return (
    <>
      <h4>Immutable history</h4>
      <ul>
        {versions.map((version) => (
          <li key={version.id}>
            <strong>v{version.ordinal}</strong> {version.source_template ? <span className="tag">{version.source_template}</span> : null}{" "}
            <span className="mono">{version.content_hash}</span>
            <div className="muted">{configurationSummary(version.document)} · {whenOf(version.created_at)}</div>
          </li>
        ))}
      </ul>
      {versions.length > 1 ? (
        <div className="stacked-form">
          <h4>Compare versions</h4>
          <label>From <select value={from} onChange={(event) => setFrom(event.target.value)}>{versions.map((version) => <option key={version.id} value={version.id}>v{version.ordinal}</option>)}</select></label>
          <label>To <select value={to} onChange={(event) => setTo(event.target.value)}>{versions.map((version) => <option key={version.id} value={version.id}>v{version.ordinal}</option>)}</select></label>
          <button type="button" onClick={() => void changed()}>Compare exact versions</button>
          {comparison ? <p>{comparison.changed_fields.length === 0 ? "Documents are identical." : `Changed: ${comparison.changed_fields.join(", ")}`}</p> : null}
        </div>
      ) : null}
      {draft && current ? (
        <form className="stacked-form configuration-editor" noValidate onSubmit={(event) => void publish(event)}>
          <h4>Propose Capture and Context settings</h4>
          <p className="muted">
            These are the demo's supported everyday settings. Every other field is copied
            unchanged from v{current.ordinal}; the gateway validates the complete document again.
          </p>

          <fieldset>
            <legend>Capture</legend>
            <label className="choice">
              <input
                type="checkbox"
                checked={draft.captureEnabled}
                onChange={(event) => {
                  const enabled = event.target.checked;
                  updateDraft({
                    captureEnabled: enabled,
                    ...(enabled
                      ? {}
                      : { captureOnSessionEnd: false, captureExplicitRequest: false }),
                  });
                }}
              />
              <span>
                <strong>Enable Capture</strong>
                <span className="muted"> Allow configured extraction from Session evidence.</span>
              </span>
            </label>
            <label className="choice">
              <input
                type="checkbox"
                disabled={!draft.captureEnabled}
                checked={draft.captureOnSessionEnd}
                onChange={(event) => updateDraft({ captureOnSessionEnd: event.target.checked })}
              />
              <span>
                <strong>On Session end</strong>
                <span className="muted"> Freeze a batch when a Session reaches a terminal state.</span>
              </span>
            </label>
            <label className="choice">
              <input
                type="checkbox"
                disabled={!draft.captureEnabled}
                checked={draft.captureExplicitRequest}
                onChange={(event) => updateDraft({ captureExplicitRequest: event.target.checked })}
              />
              <span>
                <strong>On explicit request</strong>
                <span className="muted"> Permit the authenticated Capture endpoint to freeze a batch.</span>
              </span>
            </label>
            <label>
              Minimum confidence (0–1000)
              <input
                required
                type="number"
                min="0"
                max="1000"
                step="1"
                value={draft.captureMinimumConfidencePermille}
                onChange={(event) => updateDraft({ captureMinimumConfidencePermille: event.target.value })}
              />
              <span className="muted">Candidates below this integer score are not retained.</span>
            </label>
            <label>
              Maximum candidates per batch (1–256)
              <input
                required
                type="number"
                min="1"
                max="256"
                step="1"
                value={draft.captureMaximumCandidatesPerBatch}
                onChange={(event) => updateDraft({ captureMaximumCandidatesPerBatch: event.target.value })}
              />
              <span className="muted">A hard bound after extraction and validation.</span>
            </label>
          </fieldset>

          <fieldset>
            <legend>Context</legend>
            <label>
              Maximum token budget (1–100000)
              <input
                required
                type="number"
                min="1"
                max="100000"
                step="1"
                value={draft.contextTokenBudget}
                onChange={(event) => updateDraft({ contextTokenBudget: event.target.value })}
              />
              <span className="muted">Requests may narrow this delivery ceiling, never widen it.</span>
            </label>
            <label>
              Trace retention
              <select
                value={draft.contextTraceRetention}
                onChange={(event) =>
                  updateDraft({
                    contextTraceRetention: event.target.value as DemoConfigurationDraft["contextTraceRetention"],
                  })
                }
              >
                {TRACE_RETENTION_OPTIONS.map((mode) => (
                  <option key={mode} value={mode}>{mode.replaceAll("_", " ")}</option>
                ))}
              </select>
              <span className="muted">
                Full retains visible references and scores; redacted omits sensitive diagnostics;
                hashes only removes Knowledge addresses; disabled keeps only the minimal run envelope.
              </span>
            </label>
            <label className="choice">
              <input
                type="checkbox"
                checked={draft.contextIncludeUnreviewedCandidates}
                onChange={(event) =>
                  updateDraft({ contextIncludeUnreviewedCandidates: event.target.checked })
                }
              />
              <span>
                <strong>Include visibly unreviewed candidates</strong>
                <span className="muted">
                  {" "}Adds the supported unreviewed channel; it does not publish candidates or bypass PDP reads.
                </span>
              </span>
            </label>
          </fieldset>

          <details className="technical-details">
            <summary>Complete immutable document to propose</summary>
            {preview ? (
              <pre className="json-value">{renderConfiguration(preview)}</pre>
            ) : (
              <p className="muted">Unavailable until every visible field passes validation.</p>
            )}
          </details>
          <p className="muted">
            Submitting names expected current version {artifact.current_version_id} and opens a
            VedaFlow change. Review policy may leave it pending; pending is not effective.
          </p>
          <button type="submit" disabled={busy}>
            {busy ? "Submitting proposal…" : "Propose immutable version through VedaFlow"}
          </button>
        </form>
      ) : (
        <p className="muted">No immutable version is available to edit.</p>
      )}
      <div className="stacked-form">
        <h4>Binding at selected scope</h4>
        {binding ? (
          <>
            <p>
              revision {binding.revision} · {binding.enabled ? "enabled" : "disabled"} · {binding.pinned_version_id ? `pinned ${binding.pinned_version_id}` : "following current"}
            </p>
            <div>
              <button type="button" disabled={busy} onClick={() => void updateBinding(!binding.enabled, binding.pinned_version_id ?? null, binding.enabled ? "disable from Configuration console" : "enable from Configuration console")}>{binding.enabled ? "Disable" : "Enable"}</button>{" "}
              {binding.pinned_version_id ? <button type="button" disabled={busy} onClick={() => void updateBinding(binding.enabled, null, "follow current version")}>Follow current</button> : null}
            </div>
            {versions.length > 1 ? (
              <div>
                <select value={rollback} onChange={(event) => setRollback(event.target.value)}>
                  <option value="">Choose historical version</option>
                  {versions.filter((version) => version.id !== artifact.current_version_id).map((version) => <option key={version.id} value={version.id}>v{version.ordinal} · {version.content_hash.slice(0, 12)}</option>)}
                </select>{" "}
                <button type="button" disabled={busy || !rollback} onClick={() => void rollBack()}>Roll binding back</button>
              </div>
            ) : null}
          </>
        ) : (
          <button type="button" disabled={busy || !targetScopeId} onClick={() => void bind()}>
            Propose binding this artifact here
          </button>
        )}
      </div>
      <NoticeView notice={notice} />
    </>
  );
}

function PolicySources() {
  const entry = useQuery("policy/packs", () => request("list_policy_packs", {}));
  return (
    <section>
      <h2>Policy sources</h2>
      <p className="muted">A Configuration version selects one of these Cedar sources. A declaration never assigns itself.</p>
      <Loaded<PacksResponse> entry={entry} what="the policy source catalogue">
        {(body) => <ul className="packs">{body.packs.map((pack) => <li key={`${pack.kind}:${pack.name}`}><strong>{pack.name}@{pack.version}</strong> <span className="tag">{pack.kind}</span></li>)}</ul>}
      </Loaded>
    </section>
  );
}

function GovernedRelaxations() {
  const entry = useQuery("relaxations", () =>
    request("list_relaxations", { query: { limit: "200" } }),
  );
  return (
    <section>
      <h2>Governed policy relaxations</h2>
      <p className="muted">
        Each entry is an immutable Policy/apply version bound to the exact Configuration version
        that permitted it. Database time enforces its hard expiry.
      </p>
      <Loaded<RelaxationListView> entry={entry} what="governed relaxations">
        {(body) => (
          <p>
            {body.relaxations.length === 0
              ? "Nothing is relaxed."
              : `${body.relaxations.length} policy-visible relaxation(s).`}
          </p>
        )}
      </Loaded>
    </section>
  );
}

export function NoticeView({ notice }: { notice: Notice }) {
  if (notice.error) return <div className="banner error" role="alert">{notice.error}</div>;
  if (notice.result) {
    return (
      <div
        className={`banner ${notice.result.outcome === "applied" ? "success" : notice.result.outcome === "rejected" ? "error" : "warning"}`}
        role={notice.result.outcome === "rejected" ? "alert" : "status"}
      >
        {mutationMessage(notice.result)}{" "}
        {notice.result.outcome === "pending_review" ? (
          <Link href={hrefOf("review", { proposal_id: notice.result.change_id })}>
            Open this governed change
          </Link>
        ) : null}
      </div>
    );
  }
  return null;
}
