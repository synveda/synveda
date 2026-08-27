/**
 * Advanced ▸ Service identities (CPR-8): the agents registered to act here.
 *
 * A read-only listing through the generated public contract. Registration
 * binds an external IdP subject at a governed scope; the IdP remains the
 * credential issuer and Synveda never returns a client secret here.
 */

import { request } from "./client.mjs";
import type { ServiceIdentitiesResponse } from "./generated/api.js";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";

export function ServiceIdentities() {
  const entry = useQuery("service-identities", () => request("list_service_identities", {}));
  const retry = useRefresh("service-identities");
  return (
    <>
      <PageHeading route="service-identities" />
      <Loaded<ServiceIdentitiesResponse>
        entry={entry}
        what="the service identities"
        onRetry={retry}
      >
        {(body) =>
          body.identities.length === 0 ? (
            <p className="muted">No agents registered in this tenant.</p>
          ) : (
            <table className="members">
              <thead>
                <tr>
                  <th>Agent</th>
                  <th>Subject</th>
                  <th>Scope</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {body.identities.map((identity) => (
                  <tr key={identity.id}>
                    <td>{identity.display_name ?? identity.id}</td>
                    <td className="mono">{identity.subject ?? "—"}</td>
                    <td className="mono">{identity.scope_id.slice(0, 8)}</td>
                    <td>
                      <span className="tag">{identity.status}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        }
      </Loaded>
      <p className="muted">
        Register one with <code>synveda service register</code>. The subject must match the
        external identity provider client that will authenticate the agent.
      </p>
    </>
  );
}
