/**
 * Governed scopes, effective Configuration and policy relaxations.
 *
 * Children load only when expanded, which bounds both listing and PDP work.
 * Capabilities control presentation only; every action is decided again at
 * its own gateway seam (ADR-0058).
 */

import { useCallback, useEffect, useState } from "react";

import { PageHeading } from "./Shell.js";
import type { Outcome } from "./api.mjs";
import { request } from "./client.mjs";
import {
  deniedCount,
  mayDo,
  mayRead,
  relaxationsAt,
  type Capabilities,
  type CapabilityBatch,
  type Node,
  type RelaxationListing,
  type ScopeLevel,
} from "./explorer.mjs";
import type { EffectiveConfigurationView } from "./generated/api.js";

type PanelState = Outcome | { kind: "loading" };

export function Scopes() {
  const [root, setRoot] = useState<PanelState>({ kind: "loading" });
  const [selected, setSelected] = useState<Node | null>(null);
  const [relaxations, setRelaxations] = useState<PanelState>({ kind: "loading" });

  const load = useCallback(async () => {
    setRelaxations({ kind: "loading" });
    const outcome = await request("list_scopes", { query: {} });
    setRoot(outcome);
    if (outcome.kind === "ok") {
      const level = outcome.body as ScopeLevel;
      setSelected(level.parent ?? level.scopes[0] ?? null);
    }
    // The scope-free listing is already a per-row PDP-filtered set, so read
    // it once and select the exact governed target locally.
    const governed = await request("list_relaxations", { query: { limit: "200" } });
    setRelaxations(governed);
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
          <NodeDetail key={selected.id} node={selected} relaxationState={relaxations} />
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
function NodeDetail({ node, relaxationState }: { node: Node; relaxationState: PanelState }) {
  const [configuration, setConfiguration] = useState<PanelState>({ kind: "loading" });
  const [caps, setCaps] = useState<PanelState>({ kind: "loading" });

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

  return (
    <article className="node-detail">
      <h3>{node.slug}</h3>
      <p className="muted">
        {node.display_name} · {node.kind}
      </p>

      <ConfigurationPanel state={configuration} node={node} />
      <RelaxationPanel state={relaxationState} scopeId={node.id} />
      <CapabilityPanel state={caps} node={node} />
    </article>
  );
}

function ConfigurationPanel({ state, node }: { state: PanelState; node: Node }) {
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

export function RelaxationPanel({ state, scopeId }: { state: PanelState; scopeId: string }) {
  if (state.kind === "loading") {
    return (
      <section>
        <h4>governed relaxations</h4>
        <p className="muted">…</p>
      </section>
    );
  }
  if (state.kind !== "ok") {
    return (
      <section>
        <h4>governed relaxations</h4>
        <PanelFailure state={state} />
      </section>
    );
  }
  const listing = state.body as RelaxationListing;
  const relaxations = relaxationsAt(listing.relaxations ?? [], scopeId);
  const hasMore = listing.next_cursor !== null && listing.next_cursor !== undefined;
  return (
    <section>
      <h4>governed relaxations</h4>
      {relaxations.length === 0 ? (
        <p className="muted">
          {hasMore
            ? "none for this scope in the first visible page; more results are available"
            : "nothing is relaxed here"}
        </p>
      ) : (
        <>
          <ul className="relaxations">
            {relaxations.map((relaxation) => (
              <li key={relaxation.id}>
                <span className={`tag ${relaxation.status}`}>{relaxation.status}</span>{" "}
                <strong>{relaxation.current.action}</strong> for {relaxation.current.subject}
                <div className="muted">
                  through {relaxation.current.max_sensitivity} until{" "}
                  {new Date(relaxation.current.hard_expires_at).toISOString().slice(0, 16).replace("T", " ")} UTC
                  {" — "}{relaxation.current.reason}
                </div>
                <div className="muted">
                  version {relaxation.current.ordinal} ·{" "}
                  {relaxation.current.auto_applied
                    ? "auto-applied through VedaFlow"
                    : `${relaxation.current.approver_ids.length} recorded approver(s)`}
                </div>
              </li>
            ))}
          </ul>
          {hasMore ? <p className="muted">more visible results are available</p> : null}
        </>
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
function CapabilityPanel({ state, node }: { state: PanelState; node: Node }) {
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
