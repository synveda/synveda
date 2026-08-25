/**
 * Advanced ▸ Audit (CPR-8/33): query, export and verify the tenant chain.
 *
 * The verdict is above the events on purpose. "Here are 50 rows" is worth
 * nothing without "and the chain they are part of recomputes to its stored
 * hashes", which is the claim the whole audit design exists to support — so
 * the check is the first thing on the page rather than a button somebody
 * has to know to press.
 *
 * A broken chain is a **200 with `valid: false`**, not an error: the
 * verification succeeded; it is the chain that did not. So the page reads
 * the verdict rather than the status, which is exactly the mistake a
 * surface makes when it treats "did the call work" as "is the answer good".
 *
 * Both calls use the generated public contract.
 */

import { useState } from "react";

import { request } from "./client.mjs";
import type {
  AuditEventsResponse,
  AuditExportEvent,
  AuditExportPage,
  AuditVerifyResponse,
} from "./generated/api.js";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";
import { whenOf } from "./people.mjs";

/** How many rows the page asks for. A console is a window, not an export. */
const PAGE = 50;

export function Audit() {
  return (
    <>
      <PageHeading route="audit" />
      <Verify />
      <Recent />
      <Export />
    </>
  );
}

function Verify() {
  const entry = useQuery("audit/verify", () => request("verify_audit_chain", {}));
  const retry = useRefresh("audit/verify");
  return (
    <section>
      <h2>Chain</h2>
      <Loaded<AuditVerifyResponse> entry={entry} what="the chain verification" onRetry={retry}>
        {(body) => (
          <div className={body.valid ? "banner" : "banner error"} role="status">
            <p>
              <strong>{body.valid ? "The chain verifies." : "The chain does not verify."}</strong>{" "}
              {body.events} event{body.events === 1 ? "" : "s"} checked, head at seq{" "}
              {body.head_seq}.
            </p>
            <p className="mono breakable">{body.head_hash}</p>
            {body.valid ? null : (
              <p>
                First divergence at seq {body.broken_at ?? "unknown"}:{" "}
                {body.reason ?? "no reason given"}.
              </p>
            )}
          </div>
        )}
      </Loaded>
    </section>
  );
}

function Recent() {
  const [draft, setDraft] = useState<AuditFilters>(EMPTY_FILTERS);
  const [filters, setFilters] = useState<AuditFilters>(EMPTY_FILTERS);
  const key = `audit/events/${JSON.stringify(filters)}`;
  const entry = useQuery(key, async () => {
    const query = auditQuery(filters);
    if (Object.values(query).some((value) => value !== undefined)) {
      return request("list_audit_events", {
        query: { ...query, after: "0", limit: String(PAGE) },
      });
    }
    // The audit collection is deliberately forward-keyset paginated. Read
    // the authorised head first so "recent" means the tail of that same
    // public chain instead of accidentally rendering its oldest page.
    const verification = await request("verify_audit_chain", {});
    if (verification.kind !== "ok") return verification;
    const after = Math.max(0, verification.body.head_seq - PAGE);
    return request("list_audit_events", {
      query: { after: String(after), limit: String(PAGE) },
    });
  });
  const retry = useRefresh(key);
  return (
    <section>
      <h2>Recent events</h2>
      <form
        className="filters audit-filters"
        onSubmit={(event) => {
          event.preventDefault();
          setFilters(draft);
        }}
      >
        <AuditField label="Actor" value={draft.actor} onChange={(actor) => setDraft({ ...draft, actor })} />
        <AuditField label="Action" value={draft.action} onChange={(action) => setDraft({ ...draft, action })} />
        <label>
          Outcome
          <select value={draft.outcome} onChange={(event) => setDraft({ ...draft, outcome: event.target.value })}>
            <option value="">Any</option>
            {['allow', 'deny', 'success', 'failure'].map((value) => <option key={value}>{value}</option>)}
          </select>
        </label>
        <label>
          Artifact family
          <select value={draft.artifactFamily} onChange={(event) => setDraft({ ...draft, artifactFamily: event.target.value })}>
            <option value="">Any</option>
            {['knowledge', 'skill', 'tool_server', 'tool_binding', 'configuration', 'policy_relaxation', 'okf_import', 'prompt', 'context_pack'].map((value) => <option key={value}>{value}</option>)}
          </select>
        </label>
        <AuditField label="Artifact ID" value={draft.artifactId} disabled={!draft.artifactFamily} onChange={(artifactId) => setDraft({ ...draft, artifactId })} />
        <AuditField label="Artifact version" value={draft.artifactVersion} disabled={!draft.artifactFamily} onChange={(artifactVersion) => setDraft({ ...draft, artifactVersion })} />
        <AuditField label="Session ID" value={draft.sessionId} onChange={(sessionId) => setDraft({ ...draft, sessionId })} />
        <AuditField label="Context run ID" value={draft.contextRunId} onChange={(contextRunId) => setDraft({ ...draft, contextRunId })} />
        <AuditField label="Resource" value={draft.resource} onChange={(resource) => setDraft({ ...draft, resource })} />
        <label>
          From
          <input type="datetime-local" value={draft.from} onChange={(event) => setDraft({ ...draft, from: event.target.value })} />
        </label>
        <label>
          Until
          <input type="datetime-local" value={draft.until} onChange={(event) => setDraft({ ...draft, until: event.target.value })} />
        </label>
        <button type="submit">Apply filters</button>
        {isFiltered(draft) ? <button type="button" className="secondary" onClick={() => { setDraft(EMPTY_FILTERS); setFilters(EMPTY_FILTERS); }}>Clear</button> : null}
      </form>
      <Loaded<AuditEventsResponse> entry={entry} what="the audit log" onRetry={retry}>
        {(body) =>
          body.events.length === 0 ? (
            <p className="muted">Nothing recorded yet.</p>
          ) : (
            <table className="audit">
              <thead>
                <tr>
                  <th>Seq</th>
                  <th>When</th>
                  <th>Actor</th>
                  <th>Action</th>
                  <th>Resource</th>
                  <th>Outcome</th>
                </tr>
              </thead>
              <tbody>
                {body.events.map((event) => (
                  <tr key={event.seq}>
                    <td className="mono">{event.seq}</td>
                    <td>{whenOf(event.occurred_at)}</td>
                    <td>
                      {event.actor_subject}
                      <div className="muted">{event.actor_kind}</div>
                    </td>
                    <td className="mono">{event.action}</td>
                    <td className="mono breakable">{event.resource}</td>
                    <td>
                      <span className={`tag ${event.outcome}`}>{event.outcome}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        }
      </Loaded>
      <p className="muted">
        {isFiltered(filters) ? `The first ${PAGE} matches.` : `The most recent ${PAGE}.`} Every
        filter is evaluated by the governed public audit API; a page is a window on the chain,
        not a historical authorisation replay.
      </p>
    </section>
  );
}

interface AuditFilters {
  actor: string;
  action: string;
  outcome: string;
  resource: string;
  from: string;
  until: string;
  artifactFamily: string;
  artifactId: string;
  artifactVersion: string;
  sessionId: string;
  contextRunId: string;
}

const EMPTY_FILTERS: AuditFilters = {
  actor: "", action: "", outcome: "", resource: "", from: "", until: "",
  artifactFamily: "", artifactId: "", artifactVersion: "", sessionId: "", contextRunId: "",
};

function AuditField({ label, value, disabled = false, onChange }: { label: string; value: string; disabled?: boolean; onChange: (value: string) => void }) {
  return <label>{label}<input value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} /></label>;
}

function auditQuery(filters: AuditFilters): Record<string, string | undefined> {
  const artifactFamily = value(filters.artifactFamily);
  return {
    actor: value(filters.actor),
    action: value(filters.action),
    outcome: value(filters.outcome),
    resource: value(filters.resource),
    from: instant(filters.from),
    until: instant(filters.until),
    artifact_family: artifactFamily,
    artifact_id: artifactFamily ? value(filters.artifactId) : undefined,
    artifact_version: artifactFamily ? value(filters.artifactVersion) : undefined,
    session_id: value(filters.sessionId),
    context_run_id: value(filters.contextRunId),
  };
}

function value(raw: string): string | undefined {
  return raw.trim() || undefined;
}

function instant(raw: string): string | undefined {
  return raw ? new Date(raw).toISOString() : undefined;
}

function isFiltered(filters: AuditFilters): boolean {
  return Object.values(filters).some((entry) => entry.length > 0);
}

type AuditExportDocument = Pick<
  AuditExportPage,
  "format" | "hash_algorithm" | "canonicalization" | "tenant_id" | "genesis_hash" | "snapshot_seq" | "snapshot_hash"
> & { events: AuditExportEvent[] };

/** Assemble one frozen export through generated cursor pages. */
export async function assembleAuditExport(
  fetchPage: (after: number, through?: number) => Promise<AuditExportPage>,
): Promise<AuditExportDocument> {
  let after = 0;
  let first: AuditExportPage | undefined;
  const events: AuditExportEvent[] = [];
  while (true) {
    const page = await fetchPage(after, first?.snapshot_seq);
    if (first === undefined) {
      first = page;
    } else {
      for (const field of ["format", "hash_algorithm", "canonicalization", "tenant_id", "genesis_hash", "snapshot_seq", "snapshot_hash"] as const) {
        if (page[field] !== first[field]) throw new Error(`The frozen audit export changed ${field} between pages.`);
      }
    }
    events.push(...page.events);
    if (page.next_cursor == null) break;
    if (page.next_cursor <= after) throw new Error("The audit export cursor did not advance.");
    after = page.next_cursor;
  }
  if (first === undefined) throw new Error("The audit API returned no export snapshot.");
  return {
    format: first.format,
    hash_algorithm: first.hash_algorithm,
    canonicalization: first.canonicalization,
    tenant_id: first.tenant_id,
    genesis_hash: first.genesis_hash,
    snapshot_seq: first.snapshot_seq,
    snapshot_hash: first.snapshot_hash,
    events,
  };
}

function Export() {
  const [status, setStatus] = useState<string>();
  async function run() {
    setStatus("Assembling one frozen chain prefix…");
    try {
      const document = await assembleAuditExport(async (after, through) => {
        const answer = await request("export_audit_chain", {
          query: { after: String(after), through: through == null ? undefined : String(through), limit: "1000" },
        });
        if (answer.kind !== "ok") throw new Error("message" in answer ? answer.message : "The audit export was refused.");
        return answer.body;
      });
      const blob = new Blob([`${JSON.stringify(document, null, 2)}\n`], { type: "application/json" });
      const href = URL.createObjectURL(blob);
      const link = window.document.createElement("a");
      link.href = href;
      link.download = `synveda-audit-${document.tenant_id}-${document.snapshot_seq}.json`;
      link.click();
      URL.revokeObjectURL(href);
      setStatus(`Downloaded ${document.events.length} events at frozen head ${document.snapshot_seq}. Verify offline with synveda audit verify-export.`);
    } catch (cause) {
      setStatus(cause instanceof Error ? cause.message : "The audit export failed.");
    }
  }
  return <section><h2>Offline evidence</h2><p>Download every canonical hash input from one frozen tenant-bound prefix. The export contains metadata and hashes, never Knowledge content or plaintext secrets.</p><button type="button" onClick={() => void run()}>Download audit export</button>{status ? <p className="muted" role="status">{status}</p> : null}</section>;
}
