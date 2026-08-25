/**
 * Advanced ▸ Reviews (CNSL-1; re-homed by CPR-8) — the queue, and the
 * review beside it.
 *
 * The screen CNSL-1 exists for. FLOW-6 established that a full review is
 * possible without a console; what a console adds is that the queue and the
 * thing being reviewed are visible at once, which a terminal cannot do and
 * which is most of why a reviewer with forty open proposals prefers one.
 *
 * It calls no endpoint the CLI does not (ADR-0056 decision 9): `GET
 * /v1/proposals`, `GET /v1/proposals/{id}`, and the two verdict routes.
 *
 * **What CPR-8 changed is where it is, and nothing else.** Until then this
 * was mounted directly by `App.tsx`, so signing in put every reader in front
 * of a review queue — the right first screen for the person who reviews
 * other people's publications and the wrong one for the person who just
 * installed the product. It is now a route under Advanced, behind
 * `proposal.read`, where somebody goes to govern rather than lands to work.
 * Not one line of the review itself moved.
 */

import { useCallback, useEffect, useState } from "react";

import type { Outcome } from "./api.mjs";
import { request } from "./client.mjs";
import { offers, type Capabilities, type CapabilityBatch } from "./explorer.mjs";
import type { MeView } from "./generated/api.js";
import { Review } from "./Review.js";
import { PageHeading } from "./Shell.js";
import type { Proposal, ProposalDetail } from "./review.mjs";

export function Reviews() {
  const [queue, setQueue] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });
  const [selected, setSelected] = useState<string | null>(null);
  const [family, setFamily] = useState("");

  const load = useCallback(async () => {
    setQueue(
      await request("list_proposals", {
        query: { state: "open", artifact_family: family || undefined },
      }),
    );
  }, [family]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section className="inbox">
      <PageHeading route="reviews" />
      <label>
        Artifact family{" "}
        <select value={family} onChange={(event) => setFamily(event.target.value)}>
          <option value="">All governed artifacts</option>
          <option value="knowledge">Knowledge</option>
          <option value="skill">Skills</option>
          <option value="tool_server">Tool servers</option>
          <option value="tool_binding">Tool bindings</option>
          <option value="configuration">Configuration</option>
          <option value="policy_relaxation">Policy relaxations</option>
          <option value="okf_import">OKF imports</option>
          <option value="prompt">Prompts</option>
          <option value="context_pack">Context packs</option>
        </select>
      </label>
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
            <span className="muted">
              {Array.from(new Set(proposal.artifact_references.map((reference) => reference.family))).join(", ")}
            </span>
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
  const [principalIdentity, setPrincipalIdentity] = useState<string | null>(null);

  const load = useCallback(async () => {
    const outcome = await request("get_proposal", { path: { id } });
    setState(outcome);
    if (outcome.kind === "ok") {
      const scope = (outcome.body as ProposalDetail).target_scope_id;
      const [batch, me] = await Promise.all([
        request("get_capabilities", { query: { scopes: scope } }),
        request("get_me", {}),
      ]);
      const probe =
        batch.kind === "ok"
          ? { kind: "ok", body: (batch.body as CapabilityBatch).capabilities[0] }
          : batch;
      // A probe that fails leaves the forecast `null`, which offers
      // nothing — fail closed, so an unreachable PDP shows a reviewer no
      // buttons rather than buttons that will not work.
      setCapabilities(probe.kind === "ok" ? (probe.body as Capabilities) : null);
      setPrincipalIdentity(
        me.kind === "ok" ? ((me.body as MeView).principal.identity_id ?? null) : null,
      );
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const onVerdict = useCallback(
    async (verdict: "approve" | "reject", reason: string) => {
      setBusy(true);
      setError(null);
      const outcome =
        verdict === "approve"
          ? await request("approve_proposal", {
              path: { id },
              body: {
                expected_commit: (state as { kind: "ok"; body: ProposalDetail }).body.commit,
                ...(reason.trim().length > 0 ? { comment: reason.trim() } : {}),
              },
            })
          : await request("reject_proposal", {
              path: { id },
              body: {
                expected_commit: (state as { kind: "ok"; body: ProposalDetail }).body.commit,
                reason: reason.trim(),
              },
            });
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
    [id, load, onSettled, state],
  );

  const onTransition = useCallback(
    async (operation: "cancel" | "execute") => {
      setBusy(true);
      setError(null);
      const detail = (state as { kind: "ok"; body: ProposalDetail }).body;
      const outcome =
        operation === "cancel"
          ? await request("withdraw_proposal", { path: { id } })
          : detail.effect === "apply"
            ? await request("apply_proposal", { path: { id } })
            : await request("publish_proposal", { path: { id } });
      setBusy(false);
      if (outcome.kind !== "ok") {
        setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
        return;
      }
      await load();
      onSettled();
    },
    [id, load, onSettled, state],
  );

  if (state.kind === "loading") {
    return <p className="muted">Reading the proposal…</p>;
  }
  if (state.kind !== "ok") {
    return <Failure state={state} onRetry={() => void load()} />;
  }
  const detail = state.body as ProposalDetail;
  const hasReviewAuthority = offers(capabilities, "proposal.review");
  const separatedReviewer =
    !detail.required.forbid_author_approval || principalIdentity !== detail.proposer_id;
  const mayReview = hasReviewAuthority && separatedReviewer;
  const mayCancel =
    principalIdentity === detail.proposer_id &&
    (detail.state === "open" || detail.state === "approved") &&
    offers(capabilities, "proposal.open");
  const effectAction =
    detail.effect === "published"
      ? "channel.publish"
      : detail.asset === "knowledge"
        ? "knowledge.write"
        : detail.asset === "skill"
          ? "skill.write"
          : detail.asset === "tool"
            ? "tool.write"
            : detail.asset === "configuration"
              ? "configuration.write"
              : detail.asset === "policy"
                ? "relaxation.write"
                : "channel.publish";
  const separatedActor =
    !detail.required.separate_effect_actor ||
    (principalIdentity !== null &&
      principalIdentity !== detail.proposer_id &&
      detail.approvals.every((approval) => approval.approver_id !== principalIdentity));
  const mayExecute =
    detail.state === "approved" && separatedActor && offers(capabilities, effectAction);
  return (
    <Review
      detail={detail}
      // Absent rather than disabled: a disabled Approve button is a promise
      // that signing in harder would enable it, and it would not — the
      // answer is a role this reader does not hold at this scope.
      onVerdict={mayReview ? (verdict, reason) => void onVerdict(verdict, reason) : undefined}
      cannotReview={
        mayReview
          ? null
          : hasReviewAuthority && !separatedReviewer
            ? "This approval matrix requires a reviewer distinct from the proposal author. Cancel the proposal or ask another authorised reviewer."
          : capabilities === null
            ? "Your capabilities here could not be read, so no verdict is offered."
            : `You hold ${
                capabilities.roles.length > 0 ? capabilities.roles.join(", ") : "no role"
              } at ${capabilities.scope_path ?? "this scope"}, which does not include casting a verdict here.`
      }
      error={error}
      busy={busy}
      onCancel={mayCancel ? () => void onTransition("cancel") : undefined}
      onExecute={mayExecute ? () => void onTransition("execute") : undefined}
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
