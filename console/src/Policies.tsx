/**
 * Advanced ▸ Policies (CPR-8): what this tenant can assign, what is in
 * force by default, and what has been relaxed.
 *
 * Three reads, no writes. Assignment happens at a scope — that is what
 * `PUT /v1/admin/scopes/{id}/policy` is, and it is reached from Advanced ▸
 * Scopes where the scope in question is on screen — so this page is the
 * registry and the standing relaxations rather than a second place to
 * assign, which would be two surfaces for one act.
 *
 * All three calls use the generated public contract.
 */

import { request } from "./client.mjs";
import type { DefaultResponse, LapseListResponse, PacksResponse } from "./generated/api.js";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";
import { describeEnd } from "./explorer.mjs";
import { whenOf } from "./people.mjs";

export function Policies() {
  return (
    <>
      <PageHeading route="policies" />
      <Packs />
      <Default />
      <Lapses />
    </>
  );
}

function Packs() {
  const entry = useQuery("policy/packs", () => request("list_policy_packs", {}));
  const retry = useRefresh("policy/packs");
  return (
    <section>
      <h2>Assignable packs</h2>
      <Loaded<PacksResponse> entry={entry} what="the pack registry" onRetry={retry}>
        {(body) => (
          <ul className="packs">
            {body.packs.map((pack) => (
              <li key={`${pack.kind}:${pack.name}`}>
                <strong>
                  {pack.name}@{pack.version}
                </strong>{" "}
                <span className="tag">{pack.kind}</span>
                {pack.updated_at ? (
                  <div className="muted">updated {whenOf(pack.updated_at)}</div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </Loaded>
      <p className="muted">
        A pack is assigned at a scope and governs its subtree. Assign one under Advanced ▸ Scopes,
        where the scope you are deciding about is in front of you.
      </p>
    </section>
  );
}

function Default() {
  const entry = useQuery("policy/default", () => request("get_default_policy", {}));
  const retry = useRefresh("policy/default");
  return (
    <section>
      <h2>Tenant default</h2>
      <Loaded<DefaultResponse> entry={entry} what="the tenant default" onRetry={retry}>
        {(body) => (
          <p>
            <strong>{body.effective}</strong>{" "}
            <span className="muted">
              ({body.pack_name ? "tenant default" : "built-in default"})
            </span>
          </p>
        )}
      </Loaded>
    </section>
  );
}

function Lapses() {
  const entry = useQuery("lapses", () => request("list_lapses", { query: {} }));
  const retry = useRefresh("lapses");
  return (
    <section>
      <h2>Standing relaxations</h2>
      <p className="muted">
        Time-boxed grants across the tree. Both ends are shown, because a relaxation is as much a
        fact about the scope that received it as about the one that disclosed.
      </p>
      <Loaded<LapseListResponse> entry={entry} what="the standing grants" onRetry={retry}>
        {(body) =>
          (body.lapses ?? []).length === 0 ? (
            <p className="muted">Nothing is relaxed anywhere you can see.</p>
          ) : (
            <ul className="lapses">
              {body.lapses.map((lapse) => (
                <li key={lapse.id}>
                  <span className={`tag ${lapse.outcome}`}>{lapse.outcome}</span> {lapse.action}{" "}
                  {describeEnd(lapse.grantee_scope_path, lapse.grantee_scope_id)} →{" "}
                  {describeEnd(lapse.target_scope_path, lapse.target_scope_id)}
                  <div className="muted">
                    until {whenOf(lapse.expires_at)} — {lapse.reason}
                  </div>
                </li>
              ))}
            </ul>
          )
        }
      </Loaded>
    </section>
  );
}
