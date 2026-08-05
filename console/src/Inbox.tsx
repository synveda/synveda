/**
 * The proposals inbox (CNSL-1) — the queue, and the review beside it.
 *
 * The screen this feature exists for. FLOW-6 established that a full review
 * is possible without a console; what a console adds is that the queue and
 * the thing being reviewed are visible at once, which a terminal cannot do
 * and which is most of why a reviewer with forty open proposals prefers one.
 *
 * It calls no endpoint the CLI does not (ADR-0056 decision 9): `GET
 * /v1/proposals`, `GET /v1/proposals/{id}`, and the two verdict routes.
 */

import { useCallback, useEffect, useState } from "react";

import {
  approve,
  listProposals,
  nodeCapabilities,
  readProposal,
  reject,
  type Outcome,
} from "./api.mjs";
import { offers, type Capabilities } from "./explorer.mjs";
import { Review } from "./Review.js";
import type { Proposal, ProposalDetail } from "./review.mjs";

export function Inbox() {
  const [queue, setQueue] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [selected, setSelected] = useState<string | null>(null);

  const load = useCallback(async () => {
    setQueue(await listProposals());
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section className="inbox">
      <h2>Proposals</h2>
      <div className="split">
        <Queue state={queue} selected={selected} onSelect={setSelected} onRetry={() => void load()} />
        {selected ? (
          <Detail
            key={selected}
            id={selected}
            // A verdict changes the queue — the state moves, the
            // outstanding line shrinks — so the list is re-read rather
            // than patched. A surface that edited its own copy of a
            // requirement would be the second implementation of a
            // judgement this feature spent two decisions removing.
            onSettled={() => void load()}
          />
        ) : (
          <p className="muted">Choose a proposal to review it.</p>
        )}
      </div>
    </section>
  );
}

function Queue({
  state,
  selected,
  onSelect,
  onRetry,
}: {
  state: Outcome | { kind: "loading" };
  selected: string | null;
  onSelect: (id: string) => void;
  onRetry: () => void;
}) {
  if (state.kind === "loading") {
    return <p className="muted">Reading the queue…</p>;
  }
  if (state.kind !== "ok") {
    return <Failure state={state} onRetry={onRetry} />;
  }
  const proposals = (state.body as { proposals?: Proposal[] }).proposals ?? [];
  if (proposals.length === 0) {
    return <p className="muted">Nothing open here.</p>;
  }
  return (
    <ul className="queue">
      {proposals.map((proposal) => (
        <li key={proposal.id}>
          <button
            type="button"
            className={proposal.id === selected ? "row selected" : "row"}
            onClick={() => onSelect(proposal.id)}
          >
            <span className="row-title">{proposal.title}</span>
            <span className="muted">
              {proposal.state} · {proposal.target_scope_path ?? proposal.target_scope_id} ·{" "}
              {proposal.asset}
            </span>
            {/* What it still lacks, in the gateway's own words. A queue
                that showed only a state would make a reviewer open every
                row to find the one waiting on them. */}
            <span className="muted">{proposal.outstanding}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function Detail({ id, onSettled }: { id: string; onSettled: () => void }) {
  const [state, setState] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // CNSL-1's one deferral, closed where ADR-0056 sent it. That feature
  // offered approve and reject unconditionally, because which acts a
  // proposal admits is a function of its state, the pack in force and the
  // reader's own roles — and only the first was on the wire. CNSL-2's probe
  // puts the third there.
  //
  // It decides what to *render* and never what to allow (ADR-0058 decision
  // 2): a reader who gets past this because the forecast aged still meets
  // the gateway's refusal, which `onVerdict` already displays.
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);

  const load = useCallback(async () => {
    const outcome = await readProposal(id);
    setState(outcome);
    if (outcome.kind === "ok") {
      const scope = (outcome.body as ProposalDetail).target_scope_id;
      const probe = await nodeCapabilities(scope);
      // A probe that fails leaves the forecast `null`, which offers
      // nothing — fail closed, so an unreachable PDP shows a reviewer no
      // buttons rather than buttons that will not work.
      setCapabilities(probe.kind === "ok" ? (probe.body as Capabilities) : null);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const onVerdict = useCallback(
    async (verdict: "approve" | "reject", reason: string) => {
      setBusy(true);
      setError(null);
      const outcome = verdict === "approve" ? await approve(id, reason) : await reject(id, reason);
      setBusy(false);
      if (outcome.kind !== "ok") {
        // The gateway's own sentence. A refusal reworded here is a refusal
        // whose wording nothing keeps in step with the one the CLI shows.
        setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
        return;
      }
      // Re-read rather than assume: an approval that satisfies the matrix
      // moves the proposal to `approved`, and the only place that decision
      // is made is the gateway.
      await load();
      onSettled();
    },
    [id, load, onSettled],
  );

  if (state.kind === "loading") {
    return <p className="muted">Reading the proposal…</p>;
  }
  if (state.kind !== "ok") {
    return <Failure state={state} onRetry={() => void load()} />;
  }
  const mayReview = offers(capabilities, "proposal.review");
  return (
    <Review
      detail={state.body as ProposalDetail}
      // Absent rather than disabled: a disabled Approve button is a promise
      // that signing in harder would enable it, and it would not — the
      // answer is a role this reader does not hold at this scope.
      onVerdict={mayReview ? (verdict, reason) => void onVerdict(verdict, reason) : undefined}
      cannotReview={
        mayReview
          ? null
          : capabilities === null
            ? "Your capabilities here could not be read, so no verdict is offered."
            : `You hold ${
                capabilities.roles.length > 0 ? capabilities.roles.join(", ") : "no role"
              } at ${capabilities.scope_path ?? "this scope"}, which does not include casting a verdict here.`
      }
      error={error}
      busy={busy}
    />
  );
}

/** A refused or unreachable read, told apart the way `api.mts` tells them apart. */
function Failure({ state, onRetry }: { state: Outcome; onRetry: () => void }) {
  switch (state.kind) {
    case "ok":
      return null;
    case "unauthenticated":
      return (
        <div className="banner error" role="alert">
          Your session has expired. Reload to sign in again.
        </div>
      );
    case "forbidden":
      return (
        <div className="banner error" role="alert">
          Your roles do not allow this: {state.message}
          <p className="muted">Signing in again will not change the answer.</p>
        </div>
      );
    case "unavailable":
      return (
        <div className="banner error" role="alert">
          The gateway is not answering: {state.message}
          <p>
            <button type="button" onClick={onRetry}>
              Try again
            </button>
          </p>
        </div>
      );
    default:
      return (
        <div className="banner error" role="alert">
          {state.message}
        </div>
      );
  }
}
