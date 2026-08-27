/**
 * One proposal, in full (CNSL-1, ADR-0056).
 *
 * This is the common VedaFlow review: requirement, approvals and the exact
 * immutable members a verdict binds. CPR-24 removed its old Skill-specific
 * scan/checklist/quality branch; that evidence now belongs to the versioned
 * Skills Library, while this surface remains shared by every artifact family.
 */

import { useState } from "react";

import { diffLines } from "./diff.mjs";
import {
  describeRequirement,
  effectLabel,
  instant,
  label,
  showsDiff,
  type Approval,
  type Member,
  type ProposalDetail,
} from "./review.mjs";

export interface ReviewProps {
  detail: ProposalDetail;
  /** Cast a verdict. Absent when the screen is read-only. */
  onVerdict?: (verdict: "approve" | "reject", reason: string) => void;
  /** Cancel this open change as its author. */
  onCancel?: () => void;
  /** Execute an approved apply or publication effect. */
  onExecute?: () => void;
  /**
   * Why no verdict is offered, when none is (CNSL-2, ADR-0058).
   *
   * A sentence rather than a boolean, because "you cannot do this" without
   * a reason is the thing a reviewer takes to an administrator and cannot
   * describe. Present only when `onVerdict` is absent.
   */
  cannotReview?: string | null;
  /** What the last attempt said, if it failed. */
  error?: string | null;
  busy?: boolean;
}

export function Review({
  detail,
  onVerdict,
  onCancel,
  onExecute,
  cannotReview,
  error,
  busy,
}: ReviewProps) {
  return (
    <article className="review">
      <Heading detail={detail} />
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}
      <Reviews approvals={detail.approvals} />
      <Artifacts detail={detail} />
      <Timeline detail={detail} />
      <Effect detail={detail} />
      {onVerdict ? (
        <Verdict onVerdict={onVerdict} busy={busy ?? false} />
      ) : cannotReview ? (
        <section className="verdict-section">
          <h3>your verdict</h3>
          <p className="muted">{cannotReview}</p>
        </section>
      ) : null}
      {onCancel || onExecute ? (
        <section className="verdict-section">
          <h3>change lifecycle</h3>
          <div className="actions">
            {onExecute ? (
              <button type="button" disabled={busy} onClick={onExecute}>
                {detail.effect === "apply" ? "Apply approved change" : "Publish approved change"}
              </button>
            ) : null}
            {onCancel ? (
              <button type="button" disabled={busy} onClick={onCancel}>
                Cancel proposal
              </button>
            ) : null}
          </div>
        </section>
      ) : null}
    </article>
  );
}

function Artifacts({ detail }: { detail: ProposalDetail }) {
  return (
    <section>
      <h3>governed artifacts</h3>
      <ul className="plain">
        {detail.artifact_references.map((reference) => (
          <li key={`${reference.family}:${reference.artifact_id}:${reference.operation}`}>
            <strong>{reference.family}</strong> · {reference.operation} · <code>{reference.artifact_id}</code>
            <div className="muted">
              proposed <code>{reference.version}</code>
              {reference.expected_revision ? (
                <> from <code>{reference.expected_revision}</code></>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Timeline({ detail }: { detail: ProposalDetail }) {
  return (
    <section>
      <h3>timeline</h3>
      <ol className="plain">
        {detail.timeline.map((event, index) => (
          <li key={`${event.kind}:${event.at}:${index}`}>
            {event.kind} {event.actor_subject ? `by ${event.actor_subject}` : ""} at {instant(event.at)}
            {event.reason ? <div className="comment">“{event.reason}”</div> : null}
          </li>
        ))}
      </ol>
    </section>
  );
}

function Heading({ detail }: { detail: ProposalDetail }) {
  const target = detail.target_scope_path ?? detail.target_scope_id;
  return (
    <section className="heading">
      <h2>{detail.title}</h2>
      <dl>
        <dt>state</dt>
        <dd>{detail.state}</dd>
        <dt>target</dt>
        <dd>
          {target} · {detail.asset}/{detail.effect}
        </dd>
        {detail.source_scope_id !== detail.target_scope_id ? (
          <>
            <dt>source</dt>
            <dd>
              {detail.source_scope_path ?? detail.source_scope_id}{" "}
              <span className="muted">(a climb: this scope holds the material)</span>
            </dd>
          </>
        ) : null}
        <dt>sensitivity</dt>
        <dd>{detail.sensitivity}</dd>
        <dt>proposed by</dt>
        <dd>
          {detail.proposer_subject} at {instant(detail.created_at)}
        </dd>
        <dt>commit</dt>
        <dd>
          <code>{detail.commit}</code>
        </dd>
        <dt>requires</dt>
        <dd>{describeRequirement(detail.required)}</dd>
        <dt>outstanding</dt>
        <dd>{detail.outstanding}</dd>
        {detail.close_reason ? (
          <>
            <dt>closed</dt>
            <dd>{detail.close_reason}</dd>
          </>
        ) : null}
      </dl>
    </section>
  );
}

function Reviews({ approvals }: { approvals: Approval[] }) {
  return (
    <section>
      <h3>reviews</h3>
      {approvals.length === 0 ? <p className="muted">(none yet)</p> : null}
      <ul className="plain">
        {approvals.map((approval, index) => (
          <li key={index} className={approval.counts ? undefined : "stale"}>
            {approval.verdict} {approval.approver_subject} as {approval.roles.join(", ")} at{" "}
            {instant(approval.created_at)}
            {/* An act cast against an earlier commit is evidence about other
                content. Saying so is not decoration: a reviewer who reads it
                as live reads a requirement as met that is not. */}
            {approval.counts ? null : " [of an earlier commit — does not count]"}
            {approval.comment ? <div className="comment">“{approval.comment}”</div> : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function Effect({ detail }: { detail: ProposalDetail }) {
  const target = detail.target_scope_path ?? detail.target_scope_id;
  return (
    <section>
      <h3>
        effect on {target} {detail.asset}/{detail.effect}
      </h3>
      {detail.members.map((member) => (
        <MemberRow key={member.member} member={member} />
      ))}
    </section>
  );
}

function MemberRow({ member }: { member: Member }) {
  const rows = showsDiff(member)
    ? diffLines(member.baseline?.text ?? null, member.proposed)
    : [];
  return (
    <div className="member">
      <div className={`member-head ${member.effect}`}>
        <span className="effect">{effectLabel(member.effect)}</span> <code>{label(member.member)}</code>{" "}
        <span className="muted">
          {member.asset} · {member.sensitivity}
        </span>
      </div>
      {member.unchanged ? null : (
        <div className="drift">
          <p className="refusal">this has changed since it was proposed; publishing will refuse</p>
          {/* What it says *now*, which is a third thing beside the baseline
              and the proposal and belongs to nobody's decision yet
              (ADR-0035 decision 5). Telling a reviewer the bytes moved
              without telling them where to is telling them to go and look. */}
          <pre className="now">{member.content}</pre>
        </div>
      )}
      {rows.length > 0 ? (
        <pre className="diff">
          {rows.map((row, index) => (
            <div key={index} className={row.mark}>
              {row.mark === "added" ? "+ " : row.mark === "removed" ? "- " : "  "}
              {row.text}
            </div>
          ))}
        </pre>
      ) : null}
      <p className="muted addresses">
        {member.baseline ? <>replacing object <code>{member.baseline.object_hash.slice(0, 12)}</code>; </> : null}
        proposed object <code>{member.object_hash.slice(0, 12)}</code>
      </p>
    </div>
  );
}

/**
 * Approve or reject.
 *
 * A rejection must say why, and the field is required rather than merely
 * asked for — the CLI re-asks until it gets one, for the reason FLOW-5
 * inherits: a rejection an auditor cannot read the reason for is not a
 * review. An approval's comment is optional, because a bare "I approve" is
 * the common and honest case.
 *
 * Both chain under the reviewer's own identity and are indistinguishable
 * from the CLI's in the audit trail (ADR-0056 decision 9). That is the
 * point: the trail answers *who approved this*, and the answer is a person
 * rather than a surface.
 */
function Verdict({
  onVerdict,
  busy,
}: {
  onVerdict: (verdict: "approve" | "reject", reason: string) => void;
  busy: boolean;
}) {
  const [reason, setReason] = useState("");
  const canReject = reason.trim().length > 0;
  return (
    <section className="verdict-section">
      <h3>your verdict</h3>
      <label htmlFor="reason">
        why <span className="muted">(required to reject, optional on an approval)</span>
      </label>
      <textarea
        id="reason"
        rows={3}
        value={reason}
        onChange={(event) => setReason(event.target.value)}
      />
      <div className="actions">
        <button type="button" disabled={busy} onClick={() => onVerdict("approve", reason)}>
          Approve
        </button>
        <button
          type="button"
          disabled={busy || !canReject}
          onClick={() => onVerdict("reject", reason)}
        >
          Reject
        </button>
        {canReject ? null : (
          <span className="muted">a rejection has to say why before it can be cast</span>
        )}
      </div>
    </section>
  );
}
