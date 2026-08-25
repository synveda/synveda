/**
 * Advanced ▸ Scopes (CNSL-2; the governed-scope re-cut CPR-7; re-homed by
 * CPR-8) — scopes, effective Configuration and standing lapses on one page.
 *
 * CPR-7 converted this screen off the deleted hierarchy and onto the
 * governed scope plane (`/v1/admin/scopes`), which is where its calls
 * already point. CPR-8 moved it out of the application\'s entry point and
 * onto its own route behind `scope.read`, and renamed it after the thing it
 * is actually about.
 *
 * The screen exists because those three facts are one question. "How is
 * this scope governed" is answered by a pack, the grants standing across
 * it, and where each of those came from — and before this feature
 * answering it took calls that did not exist.
 *
 * # The tree is lazy, and that is a correctness property
 *
 * Children on expand, never `descendants` from the root (ADR-0058 decision
 * 5). A sidebar that fetched a subtree would pull all of it and then probe
 * every scope in it, and the probe is a PDP fan-out. What the reader opens
 * is what gets asked about.
 *
 * # Nothing here is a permission
 *
 * The capability panel is a forecast (ADR-0058 decision 2). This bundle
 * never uses it to decide whether an act is allowed — only whether to offer
 * it — and the gateway decides again at the act's own seam.
 */

import { useCallback, useEffect, useState } from "react";

import { PageHeading } from "./Shell.js";
import type { Outcome } from "./api.mjs";
import { request } from "./client.mjs";
import {
  deniedCount,
  describeEnd,
  lapsesTouching,
  mayDo,
  mayRead,
  type Capabilities,
  type CapabilityBatch,
  type Lapse,
  type LapseListing,
  type Node,
  type ScopeLevel,
} from "./explorer.mjs";
import type { EffectiveConfigurationView } from "./generated/api.js";

export function Scopes() {
  const [root, setRoot] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [selected, setSelected] = useState<Node | null>(null);
  const [lapses, setLapses] = useState<Lapse[]>([]);

  const load = useCallback(async () => {
    const outcome = await request("list_scopes", { query: {} });
    setRoot(outcome);
    if (outcome.kind === "ok") {
      const level = outcome.body as ScopeLevel;
      setSelected(level.parent ?? level.scopes[0] ?? null);
    }
    // Standing grants are read once for the whole screen rather than per
    // node: the scope-free listing is already the set this reader may see
    // anywhere, so asking again per selection would be the same answer
    // filtered twice.
    const grants = await request("list_lapses", { query: {} });
    if (grants.kind === "ok") {
      setLapses((grants.body as LapseListing).lapses ?? []);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (root.kind === "loading") {
    return (
      <>
        <PageHeading route="scopes" />
        <p className="muted">Reading the scope tree…</p>
      </>
    );
  }
  if (root.kind !== "ok") {
    return (
      <>
        <PageHeading route="scopes" />
        <Failure state={root} onRetry={() => void load()} />
      </>
    );
  }

  return (
    <section className="explorer">
      <PageHeading route="scopes" />
      <div className="split">
        <div className="tree" role="tree" aria-label="Scopes">
          <Branch node={(root.body as ScopeLevel).parent as Node} selected={selected} onSelect={setSelected} />
          {((root.body as ScopeLevel).scopes ?? []).map((child) => (
            <Branch key={child.id} node={child} selected={selected} onSelect={setSelected} />
          ))}
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
      const level = await request("list_scopes", { query: { parent_id: node.id } });
      // The expansion call answers a level; the tree renders its children.
      setKids(
        level.kind === "ok"
          ? { kind: "ok", body: (level.body as ScopeLevel).scopes ?? [] }
          : level,
      );
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

/** The three panels, for the selected scope. */
function NodeDetail({ node, lapses }: { node: Node; lapses: Lapse[] }) {
  const [configuration, setConfiguration] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [caps, setCaps] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });

  useEffect(() => {
    void (async () => {
      setConfiguration(
        await request("get_effective_configuration", { query: { scope_id: node.id } }),
      );
      const batch = await request("get_capabilities", { query: { scopes: node.id } });
      // The batch answers a list; this detail asked about one.
      setCaps(
        batch.kind === "ok"
          ? { kind: "ok", body: (batch.body as CapabilityBatch).capabilities[0] }
          : batch,
      );
    })();
  }, [node.id]);

  const touching = lapsesTouching(lapses, node.id);
  return (
    <article className="node-detail">
      <h3>{node.slug}</h3>
      <p className="muted">
        {node.display_name} · {node.kind}
      </p>

      <ConfigurationPanel state={configuration} node={node} />
      <LapsePanel lapses={touching} node={node} />
      <CapabilityPanel state={caps} node={node} />
    </article>
  );
}

function ConfigurationPanel({ state, node }: { state: Outcome | { kind: "loading" }; node: Node }) {
  return (
    <section>
      <h4>runtime Configuration</h4>
      {state.kind === "loading" ? (
        <p className="muted">…</p>
      ) : state.kind !== "ok" ? (
        <PanelFailure state={state} />
      ) : (
        (() => {
          const configuration = state.body as EffectiveConfigurationView;
          const inherited =
            configuration.binding_scope_id !== null &&
            configuration.binding_scope_id !== undefined &&
            configuration.binding_scope_id !== node.id;
          return (
            <>
              <p>
                <strong>{configuration.document.policy_pack}</strong>{" "}
                <span className={inherited ? "tag inherited" : "tag"}>
                  {configuration.fail_safe
                    ? "enterprise fail-safe"
                    : inherited
                      ? "inherited"
                      : "bound here"}
                </span>
              </p>
              <p className="muted">
                {configuration.version_id
                  ? `version ${configuration.version_id} · ${configuration.content_hash}`
                  : `built-in immutable document · ${configuration.content_hash}`}
              </p>
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
                    thing an administrator needs: receiving access and disclosing
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
              <p className="muted forecast">
                {deniedCount(caps)} action(s) denied. Decided under{" "}
                {caps.pack ? `${caps.pack.name}@${caps.pack.version}` : "the pack in force"} — a
                forecast, not a grant: every act decides again at its own seam.
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
