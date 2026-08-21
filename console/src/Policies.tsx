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
 * All three are hand-written calls: the policy plane is not on the OpenAPI
 * contract yet.
 */

import { defaultPolicy, policyPacks, standingLapses } from "./api.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading } from "./Shell.js";
import { describeEnd, type LapseListing } from "./explorer.mjs";
import { whenOf } from "./people.mjs";

interface PackListing {
  packs: { name: string; version: number; kind: string; updated_at?: string }[];
}

interface DefaultPack {
  name: string;
  version: number;
  origin: { kind: string };
}

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
  const entry = useQuery("policy/packs", () => policyPacks());
  const retry = useRefresh("policy/packs");
  return (
    <section>
      <h2>Assignable packs</h2>
      <Loaded<PackListing> entry={entry} what="the pack registry" onRetry={retry}>
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
  const entry = useQuery("policy/default", () => defaultPolicy());
  const retry = useRefresh("policy/default");
  return (
    <section>
      <h2>Tenant default</h2>
      <Loaded<DefaultPack> entry={entry} what="the tenant default" onRetry={retry}>
        {(body) => (
          <p>
            <strong>
              {body.name}@{body.version}
            </strong>{" "}
            <span className="muted">({body.origin.kind})</span>
          </p>
        )}
      </Loaded>
    </section>
  );
}

function Lapses() {
  const entry = useQuery("lapses", () => standingLapses());
  const retry = useRefresh("lapses");
  return (
    <section>
      <h2>Standing relaxations</h2>
      <p className="muted">
        Time-boxed grants across the tree. Both ends are shown, because a relaxation is as much a
        fact about the scope that received it as about the one that disclosed.
      </p>
      <Loaded<LapseListing> entry={entry} what="the standing grants" onRetry={retry}>
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
