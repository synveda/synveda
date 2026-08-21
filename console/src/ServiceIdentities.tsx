/**
 * Advanced ▸ Service identities (CPR-8): the agents registered to act here.
 *
 * A read-only listing. Registering one mints a credential, and a credential
 * belongs in a terminal rather than in a browser tab somebody might screen
 * share — `synveda service register` is where that happens, and this page
 * says so instead of growing a form whose response would be a secret.
 *
 * Hand-written: the service-identity plane is not on the OpenAPI contract
 * yet.
 */

import { serviceIdentities } from "./api.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";

interface IdentityListing {
  identities: {
    id: string;
    subject?: string | null;
    kind: string;
    display_name?: string | null;
    scope_id: string;
    status: string;
  }[];
}

export function ServiceIdentities() {
  const entry = useQuery("service-identities", () => serviceIdentities());
  const retry = useRefresh("service-identities");
  return (
    <>
      <PageHeading route="service-identities" />
      <Loaded<IdentityListing> entry={entry} what="the service identities" onRetry={retry}>
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
        Register one with <code>synveda service register</code>. It is not offered here because
        the response carries a credential, and a credential belongs in a terminal rather than in
        a browser tab.
      </p>
    </>
  );
}
