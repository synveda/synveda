/**
 * Advanced ▸ Audit (CPR-8): the chain, and whether it still verifies.
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
 * Both calls are hand-written: the audit plane is not on the OpenAPI
 * contract yet.
 */

import { auditEvents, auditVerify } from "./api.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";
import { whenOf } from "./people.mjs";

/** How many rows the page asks for. A console is a window, not an export. */
const PAGE = 50;

interface Verification {
  valid: boolean;
  events: number;
  head_seq: number;
  head_hash: string;
  broken_at?: number;
  reason?: string;
}

interface Events {
  events: {
    seq: number;
    occurred_at: string;
    actor_kind: string;
    actor_subject: string;
    action: string;
    resource: string;
    outcome: string;
    hash: string;
  }[];
}

export function Audit() {
  return (
    <>
      <PageHeading route="audit" />
      <Verify />
      <Recent />
    </>
  );
}

function Verify() {
  const entry = useQuery("audit/verify", () => auditVerify());
  const retry = useRefresh("audit/verify");
  return (
    <section>
      <h2>Chain</h2>
      <Loaded<Verification> entry={entry} what="the chain verification" onRetry={retry}>
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
  const entry = useQuery("audit/events", () => auditEvents(PAGE));
  const retry = useRefresh("audit/events");
  return (
    <section>
      <h2>Recent events</h2>
      <Loaded<Events> entry={entry} what="the audit log" onRetry={retry}>
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
        The most recent {PAGE}. Filtering and export are the CLI's (`synveda audit`) — this page
        is a window on the chain, not a substitute for it.
      </p>
    </section>
  );
}
