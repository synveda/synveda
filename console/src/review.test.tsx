/** Acceptance for the shared VedaFlow review surface after the Skill cutover. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { Review } from "./Review.js";
import type { ProposalDetail } from "./review.mjs";
import { toText } from "./text.mjs";

function proposal(overrides: Partial<ProposalDetail> = {}): ProposalDetail {
  return {
    id: "proposal-1",
    title: "Publish the corrected request convention",
    state: "open",
    asset: "knowledge",
    effect: "apply",
    sensitivity: "internal",
    commit: "c".repeat(64),
    proposer_id: "0199bb11-1111-7111-8111-111111111110",
    proposer_subject: "alice@example.test",
    created_at: "2026-08-24T10:00:00Z",
    updated_at: "2026-08-24T10:10:00Z",
    target_scope_id: "scope-project",
    target_scope_path: "acme/pulseboard",
    source_scope_id: "scope-personal",
    source_scope_path: "acme/alice",
    required: {
      roles: [
        { role: "reviewer", count: 1 },
        { role: "curator", count: 1 },
      ],
      distinct_approvers: 2,
      forbid_author_approval: true,
      separate_effect_actor: false,
      origins: ["human"],
    },
    artifact_references: [
      {
        family: "knowledge",
        artifact_id: "knowledge-1",
        operation: "edit",
        version: "revision-2",
        expected_revision: "revision-1",
      },
    ],
    outstanding: "one curator approval",
    approvals: [
      {
        approver_id: "0199bb11-1111-7111-8111-111111111111",
        approver_subject: "robin@example.test",
        verdict: "approve",
        roles: ["reviewer"],
        counts: true,
        comment: "Source evidence checked.",
        created_at: "2026-08-24T10:10:00Z",
        commit: "c".repeat(64),
      },
      {
        approver_id: "0199bb11-1111-7111-8111-111111111112",
        approver_subject: "old-reviewer@example.test",
        verdict: "approve",
        roles: ["curator"],
        counts: false,
        created_at: "2026-08-24T09:55:00Z",
        commit: "b".repeat(64),
      },
    ],
    timeline: [
      {
        kind: "opened",
        actor_id: "0199bb11-1111-7111-8111-111111111110",
        actor_subject: "alice@example.test",
        at: "2026-08-24T10:00:00Z",
        commit: "c".repeat(64),
      },
      {
        kind: "approved",
        actor_id: "0199bb11-1111-7111-8111-111111111111",
        actor_subject: "robin@example.test",
        at: "2026-08-24T10:10:00Z",
        commit: "c".repeat(64),
        reason: "Source evidence checked.",
      },
    ],
    members: [
      {
        member: "knowledge-command",
        asset: "knowledge",
        object_hash: "a".repeat(64),
        unchanged: true,
        sensitivity: "internal",
        effect: "update",
        proposed: "Public requests use traceparent.",
        content: "Public requests use X-Request-Id.",
        baseline: { object_hash: "b".repeat(64), text: "Public requests use X-Request-Id." },
      },
      {
        member: "source-evidence",
        asset: "knowledge-source",
        object_hash: "d".repeat(64),
        unchanged: true,
        sensitivity: "internal",
        effect: "none",
        proposed: "event:session-2/5",
        content: "event:session-2/5",
      },
    ],
    ...overrides,
  };
}

test("the common review names requirement, approvals, immutable effect and source climb", () => {
  const rendered = toText(renderToStaticMarkup(<Review detail={proposal()} />));
  for (const fact of [
    "Publish the corrected request convention",
    "open",
    "acme/pulseboard",
    "knowledge/apply",
    "acme/alice",
    "a climb",
    "alice@example.test",
    "1 × reviewer",
    "1 × curator",
    "2 distinct approvers",
    "reviewer distinct from author",
    "one curator approval",
    "robin@example.test",
    "Source evidence checked.",
    "governed artifacts",
    "knowledge · edit · knowledge-1",
    "revision-2 from revision-1",
    "timeline",
    "old-reviewer@example.test",
    "does not count",
    "update knowledge-command",
    "Public requests use X-Request-Id.",
    "Public requests use traceparent.",
    "same source-evidence",
  ]) {
    assert.ok(rendered.includes(fact), `${JSON.stringify(fact)} is absent:\n\n${rendered}`);
  }
  assert.ok(!rendered.includes("security scan"), rendered);
  assert.ok(!rendered.includes("checklist"), rendered);
  assert.ok(!rendered.includes("quality override"), rendered);
});

test("member drift remains explicit and shows the bytes as they stand now", () => {
  const detail = proposal({
    members: [
      {
        ...proposal().members[0]!,
        unchanged: false,
        content: "Public requests use baggage.",
      },
    ],
  });
  const rendered = toText(renderToStaticMarkup(<Review detail={detail} />));
  assert.match(rendered, /changed since it was proposed; publishing will refuse/);
  assert.match(rendered, /Public requests use baggage\./);
});

test("a read-only review offers no verdict and an actionable rejection starts disabled", () => {
  const detail = proposal();
  const readOnly = toText(renderToStaticMarkup(<Review detail={detail} />));
  assert.doesNotMatch(readOnly, /Approve/);

  const actionable = renderToStaticMarkup(<Review detail={detail} onVerdict={() => {}} />);
  assert.match(actionable, />Approve<\/button>/);
  assert.match(actionable, />Reject<\/button>/);
  const rejection = actionable.slice(actionable.indexOf("<button", actionable.indexOf("Approve")));
  assert.match(rejection, /disabled/);
});

test("a reader without review authority gets a reason and no failing controls", () => {
  const rendered = toText(
    renderToStaticMarkup(
      <Review
        detail={proposal()}
        cannotReview="You hold viewer at acme/pulseboard, which does not include casting a verdict here."
      />,
    ),
  );
  assert.match(rendered, /your verdict/);
  assert.match(rendered, /viewer/);
  assert.match(rendered, /acme\/pulseboard/);
  assert.doesNotMatch(rendered, /Approve/);
});

test("an unreadable capability forecast fails closed and a gateway error stays visible", () => {
  const unknown = toText(
    renderToStaticMarkup(
      <Review
        detail={proposal()}
        cannotReview="Your capabilities here could not be read, so no verdict is offered."
        error="The proposal moved while you were reading it."
      />,
    ),
  );
  assert.match(unknown, /could not be read/);
  assert.match(unknown, /proposal moved/);
  assert.doesNotMatch(unknown, /Approve/);
});

test("the common surface exposes cancellation and the correct approved effect", () => {
  const apply = renderToStaticMarkup(
    <Review
      detail={proposal({ state: "approved", effect: "apply" })}
      onCancel={() => {}}
      onExecute={() => {}}
    />,
  );
  assert.match(apply, />Apply approved change<\/button>/);
  assert.match(apply, />Cancel proposal<\/button>/);

  const publish = renderToStaticMarkup(
    <Review
      detail={proposal({ state: "approved", effect: "published" })}
      onExecute={() => {}}
    />,
  );
  assert.match(publish, />Publish approved change<\/button>/);
  assert.doesNotMatch(publish, /Cancel proposal/);
});
