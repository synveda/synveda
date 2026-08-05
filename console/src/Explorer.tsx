/**
 * The hierarchy & policy explorer (CNSL-2) — scopes, packs, roles and
 * standing lapses on one page.
 *
 * The screen exists because those four facts are one question. "How is this
 * team governed" is answered by a pack, the roles in force over it, the
 * grants standing across it, and where each of those came from — and before
 * this feature answering it took four calls, two of which did not exist and
 * one of which required already knowing the answer.
 *
 * # The tree is lazy, and that is a correctness property
 *
 * Children on expand, never `descendants` from the root (ADR-0058 decision
 * 5). HIER-1's AC is a 10,000-node hierarchy; a sidebar that fetched a
 * subtree would pull all of it and then probe every node in it, and the
 * probe is a PDP fan-out. What the reader opens is what gets asked about.
 *
 * # Nothing here is a permission
 *
 * The capability panel is a forecast (ADR-0058 decision 2). This bundle
 * never uses it to decide whether an act is allowed — only whether to offer
 * it — and the gateway decides again at the act's own seam.
 */

import { useCallback, useEffect, useState } from "react";

import {
  children,
  hierarchyRoot,
  nodeCapabilities,
  nodePolicy,
  nodeRoles,
  standingLapses,
  type Outcome,
} from "./api.mjs";
import {
  deniedCount,
  describeEnd,
  describeOrigin,
  isInherited,
  lapsesTouching,
  mayBind,
  mayDo,
  mayRead,
  type Capabilities,
  type EffectiveBindings,
  type EffectivePack,
  type Lapse,
  type LapseListing,
  type Node,
} from "./explorer.mjs";

export function Explorer() {
  const [root, setRoot] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [selected, setSelected] = useState<Node | null>(null);
  const [lapses, setLapses] = useState<Lapse[]>([]);

  const load = useCallback(async () => {
    const outcome = await hierarchyRoot();
    setRoot(outcome);
    if (outcome.kind === "ok") {
      setSelected(outcome.body as Node);
    }
    // Standing grants are read once for the whole screen rather than per
    // node: the scope-free listing is already the set this reader may see
    // anywhere, so asking again per selection would be the same answer
    // filtered twice.
    const grants = await standingLapses();
    if (grants.kind === "ok") {
      setLapses((grants.body as LapseListing).lapses ?? []);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (root.kind === "loading") {
    return <p className="muted">Reading the hierarchy…</p>;
  }
  if (root.kind !== "ok") {
    return <Failure state={root} onRetry={() => void load()} />;
  }

  return (
    <section className="explorer">
      <h2>Hierarchy &amp; policy</h2>
      <div className="split">
        <div className="tree" role="tree" aria-label="Scopes">
          <Branch node={root.body as Node} selected={selected} onSelect={setSelected} />
        </div>
        {selected ? (
          <NodeDetail key={selected.id} node={selected} lapses={lapses} />
        ) : (
          <p className="muted">Choose a scope.</p>
        )}
      </div>
    </section>
  );
}

/**
 * One node and, once opened, its children.
 *
 * Expansion state lives per branch rather than in a set held above, because
 * the only thing the parent needs to know is which node is selected — and a
 * tree that reported every open/closed toggle upward re-renders the whole
 * sidebar to draw one triangle.
 */
function Branch({
  node,
  selected,
  onSelect,
}: {
  node: Node;
  selected: Node | null;
  onSelect: (node: Node) => void;
}) {
  const [open, setOpen] = useState(false);
  const [kids, setKids] = useState<Outcome | { kind: "loading" } | null>(null);

  const toggle = useCallback(async () => {
    const next = !open;
    setOpen(next);
    // Fetched once and kept. A tree that re-read on every collapse would
    // make the cheapest interaction on the screen the most expensive.
    if (next && kids === null) {
      setKids({ kind: "loading" });
      setKids(await children(node.id));
    }
  }, [open, kids, node.id]);

  const isSelected = selected?.id === node.id;
  return (
    <div className="branch" role="treeitem" aria-expanded={open} aria-selected={isSelected}>
      <div className="branch-row">
        <button
          type="button"
          className="twisty"
          onClick={() => void toggle()}
          aria-label={open ? `Collapse ${node.slug}` : `Expand ${node.slug}`}
        >
          {open ? "▾" : "▸"}
        </button>
        <button
          type="button"
          className={isSelected ? "node selected" : "node"}
          onClick={() => onSelect(node)}
        >
          <span className="node-slug">{node.slug}</span>
          <span className="muted"> {node.kind}</span>
        </button>
      </div>
      {open ? (
        <div className="branch-children">
          {kids === null || kids.kind === "loading" ? (
            <p className="muted">…</p>
          ) : kids.kind === "ok" ? (
            ((kids.body as Node[]) ?? []).length === 0 ? (
              <p className="muted">no scopes under this one</p>
            ) : (
              ((kids.body as Node[]) ?? []).map((child) => (
                <Branch key={child.id} node={child} selected={selected} onSelect={onSelect} />
              ))
            )
          ) : (
            <p className="muted">{kids.kind === "forbidden" ? "not yours to read" : "unreadable"}</p>
          )}
        </div>
      ) : null}
    </div>
  );
}

/** The four panels, for the selected scope. */
function NodeDetail({ node, lapses }: { node: Node; lapses: Lapse[] }) {
  const [pack, setPack] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [roles, setRoles] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [caps, setCaps] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });

  useEffect(() => {
    void (async () => {
      setPack(await nodePolicy(node.id));
      setRoles(await nodeRoles(node.id, true));
      setCaps(await nodeCapabilities(node.id));
    })();
  }, [node.id]);

  const touching = lapsesTouching(lapses, node.id);
  return (
    <article className="node-detail">
      <h3>{node.path}</h3>
      <p className="muted">
        {node.name} · {node.kind}
      </p>

      <PackPanel state={pack} node={node} />
      <RolesPanel state={roles} node={node} />
      <LapsePanel lapses={touching} node={node} />
      <CapabilityPanel state={caps} node={node} />
    </article>
  );
}

function PackPanel({ state, node }: { state: Outcome | { kind: "loading" }; node: Node }) {
  return (
    <section>
      <h4>policy pack</h4>
      {state.kind === "loading" ? (
        <p className="muted">…</p>
      ) : state.kind !== "ok" ? (
        <PanelFailure state={state} />
      ) : (
        (() => {
          const pack = state.body as EffectivePack;
          return (
            <p>
              <strong>
                {pack.name}@{pack.version}
              </strong>{" "}
              <span className={isInherited(pack.origin, node.id) ? "tag inherited" : "tag"}>
                {describeOrigin(pack.origin, node.id)}
              </span>
            </p>
          );
        })()
      )}
    </section>
  );
}

function RolesPanel({ state, node }: { state: Outcome | { kind: "loading" }; node: Node }) {
  return (
    <section>
      <h4>roles in force</h4>
      {state.kind === "loading" ? (
        <p className="muted">…</p>
      ) : state.kind !== "ok" ? (
        <PanelFailure state={state} />
      ) : (
        (() => {
          const view = state.body as EffectiveBindings;
          if (view.bindings.length === 0) {
            return <p className="muted">nobody holds a role over this scope</p>;
          }
          return (
            <>
              <table>
                <thead>
                  <tr>
                    <th>role</th>
                    <th>subject</th>
                    <th>from</th>
                  </tr>
                </thead>
                <tbody>
                  {view.bindings.map((binding) => (
                    <tr key={`${binding.subject}:${binding.role}:${binding.scope_id ?? "tenant"}`}>
                      <td>{binding.role}</td>
                      <td className="subject">{binding.subject}</td>
                      <td>
                        <span
                          className={
                            isInherited(binding.origin, node.id) ? "tag inherited" : "tag"
                          }
                        >
                          {describeOrigin(binding.origin, node.id)}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {/* The chain is why the answer has the rows it has. A reader
                  who cannot see it is being asked to take the inheritance
                  on trust. */}
              <p className="muted">in force over {view.chain.length} scope(s) of chain</p>
            </>
          );
        })()
      )}
    </section>
  );
}

function LapsePanel({ lapses, node }: { lapses: Lapse[]; node: Node }) {
  return (
    <section>
      <h4>standing grants</h4>
      {lapses.length === 0 ? (
        <p className="muted">nothing is relaxed here</p>
      ) : (
        <ul className="lapses">
          {lapses.map((lapse) => {
            const receiving = lapse.grantee_scope_id === node.id;
            return (
              <li key={lapse.id}>
                <span className={`tag ${lapse.outcome}`}>{lapse.outcome}</span>{" "}
                {/* Which side of the grant this scope is on is the first
                    thing a steward needs: receiving access and disclosing
                    material are different facts about their team. */}
                <strong>{receiving ? "receives" : "discloses"}</strong> {lapse.action}{" "}
                {receiving
                  ? `from ${describeEnd(lapse.target_scope_path, lapse.target_scope_id)}`
                  : `to ${describeEnd(lapse.grantee_scope_path, lapse.grantee_scope_id)}`}
                <div className="muted">
                  until {new Date(lapse.expires_at).toISOString().slice(0, 16).replace("T", " ")} UTC
                  — {lapse.reason}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/**
 * What this reader may do here.
 *
 * The footer is not decoration. "may: channel.publish" reads exactly like a
 * permission, and it is not one — so the panel says which pack decided and
 * that every act decides again (ADR-0058 decision 2).
 */
function CapabilityPanel({ state, node }: { state: Outcome | { kind: "loading" }; node: Node }) {
  return (
    <section className="capabilities">
      <h4>what you may do here</h4>
      {state.kind === "loading" ? (
        <p className="muted">…</p>
      ) : state.kind !== "ok" ? (
        <PanelFailure state={state} />
      ) : (
        (() => {
          const caps = state.body as Capabilities;
          const allowed = mayDo(caps);
          const readable = mayRead(caps);
          const bindable = mayBind(caps);
          return (
            <>
              <p className="muted">
                your roles here: {caps.roles.length > 0 ? caps.roles.join(", ") : "none"}
              </p>
              {allowed.length === 0 ? (
                <p className="muted">nothing at this scope</p>
              ) : (
                <ul className="actions-list">
                  {allowed.map((action) => (
                    <li key={action}>{action}</li>
                  ))}
                </ul>
              )}
              {readable.length > 0 ? (
                <dl className="tiers">
                  {readable.map(([action, tiers]) => (
                    <div key={action}>
                      <dt>{action}</dt>
                      <dd>{tiers.join(", ")}</dd>
                    </div>
                  ))}
                </dl>
              ) : null}
              {bindable.length > 0 ? (
                <p className="muted">may bind: {bindable.join(", ")}</p>
              ) : null}
              <p className="muted forecast">
                {deniedCount(caps)} action(s) denied. Decided under {caps.pack.name}@
                {caps.pack.version} — a forecast, not a grant: every act decides again at its own
                seam.
              </p>
            </>
          );
        })()
      )}
    </section>
  );
}

function PanelFailure({ state }: { state: Outcome }) {
  if (state.kind === "forbidden") {
    // The distinction the whole screen turns on: told no, versus not there.
    return <p className="muted">your roles do not allow this: {state.message}</p>;
  }
  return <p className="muted">{messageOf(state)}</p>;
}

function Failure({ state, onRetry }: { state: Outcome; onRetry: () => void }) {
  if (state.kind === "unauthenticated") {
    return (
      <div className="banner error" role="alert">
        Your session has expired. Reload to sign in again.
      </div>
    );
  }
  return (
    <div className="banner error" role="alert">
      {messageOf(state)}
      <p>
        <button type="button" onClick={onRetry}>
          Try again
        </button>
      </p>
    </div>
  );
}

/** The message an outcome carries, for the kinds that carry one. */
function messageOf(state: Outcome): string {
  return state.kind === "ok" || state.kind === "unauthenticated" ? "" : state.message;
}
