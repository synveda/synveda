/**
 * One proposal, in full (CNSL-1, ADR-0056).
 *
 * The order of the blocks is the order a reviewer decides in — is it safe,
 * is it good, what changed — which is the CLI's order too, and for the same
 * reason rather than for consistency's sake: the diff says what the change
 * *is*, and the scan says what it can *do*.
 *
 * Every judgement on this screen arrives from the gateway. `blocking` is
 * rendered, not computed; a shortfall's sentence is displayed, not composed
 * (ADR-0056 decisions 5 and 6). What this file decides is where things sit
 * and what they look like, which is the half the ADR left to each client.
 */

import { useState } from "react";

import { diffLines } from "./diff.mjs";
import {
  describeRequirement,
  effectLabel,
  instant,
  label,
  readable,
  showsDiff,
  type Approval,
  type Finding,
  type Member,
  type ProposalDetail,
  type QualityReport,
  type ScanReport,
} from "./review.mjs";

export interface ReviewProps {
  detail: ProposalDetail;
  /** Cast a verdict. Absent when the screen is read-only. */
  onVerdict?: (verdict: "approve" | "reject", reason: string) => void;
  /** What the last attempt said, if it failed. */
  error?: string | null;
  busy?: boolean;
}

export function Review({ detail, onVerdict, error, busy }: ReviewProps) {
  return (
    <article className="review">
      <Heading detail={detail} />
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}
      <Reviews approvals={detail.approvals} />
      {detail.scan ? <Scan scan={detail.scan} /> : null}
      {detail.quality ? <Quality quality={detail.quality} id={detail.id} /> : null}
      <Effect detail={detail} />
      {onVerdict ? <Verdict onVerdict={onVerdict} busy={busy ?? false} /> : null}
    </article>
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
        {detail.promotion ? (
          <>
            <dt>opened by</dt>
            <dd>
              rule <code>{detail.promotion.rule}</code>
              <span className="muted">
                {" "}
                — checkable against audit seq {detail.promotion.from_seq}..=
                {detail.promotion.to_seq}
              </span>
            </dd>
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

function Scan({ scan }: { scan: ScanReport }) {
  return (
    <section>
      <h3>
        security scan{" "}
        <span className="muted">
          (ruleset v{scan.ruleset_version}, this pack refuses at {scan.blocks_at})
        </span>
      </h3>
      {scan.findings.length === 0 ? <p className="muted">nothing found</p> : null}
      <ul className="findings">
        {scan.findings.map((finding, index) => (
          <FindingRow key={index} finding={finding} />
        ))}
      </ul>
      {scan.blocked ? (
        <p className="refusal">
          this bundle will be REFUSED at publication (
          {scan.findings.filter((finding) => finding.blocking).length} findings at{" "}
          {scan.blocks_at} or above); approving it cannot make it publishable
        </p>
      ) : scan.worst ? (
        <p className="muted">
          worst is {scan.worst}; the pack in force reports it rather than refusing it, so this is
          yours to weigh
        </p>
      ) : null}
    </section>
  );
}

function FindingRow({ finding }: { finding: Finding }) {
  return (
    <li className={finding.blocking ? "finding blocking" : "finding"}>
      <span className={`severity ${finding.severity}`}>{finding.severity}</span>{" "}
      <code>
        {finding.path}:{finding.line}
      </code>{" "}
      <code>{finding.rule}</code>
      {finding.count > 1 ? <span className="muted"> ×{finding.count}</span> : null}
      {/* The verdict in the text and not only in the colour. A chip a
          screen reader announces and a copy-paste carries, because this is
          the one fact on the row that decides whether approving the
          proposal can achieve anything — and it matters most where the
          reader cannot reason around it, which is a severity served by a
          gateway newer than this bundle. */}
      {finding.blocking ? <span className="chip">blocks</span> : null}
      <div className="muted">{finding.title}</div>
    </li>
  );
}

function Quality({ quality, id }: { quality: QualityReport; id: string }) {
  const failed = quality.checks.filter((check) => !check.passed);
  return (
    <section>
      {/* Two numbers, never one (ADR-0053 decision 1). A reviewer shown an
          average cannot tell a well-formatted bundle nobody worked through
          from one somebody did. */}
      <h3>
        quality {quality.score}/100{" "}
        <span className="muted">
          (rubric v{quality.rubric_version},{" "}
          {quality.min_score === 0
            ? "this pack sets no bar"
            : `this pack asks for ${quality.min_score}`}
          )
        </span>
      </h3>
      {failed.length === 0 ? <p className="muted">every check passed</p> : null}
      <ul className="plain">
        {failed.map((check) => (
          <li key={check.check}>
            <span className="weight">-{check.weight}</span> <code>{check.check}</code>{" "}
            {check.title}
            {check.detail ? <div className="muted">{check.detail}</div> : null}
          </li>
        ))}
      </ul>

      <h4>
        checklist{" "}
        {quality.checklist ? (
          <span>
            {quality.checklist.complete ? "complete" : "PARTIAL"}{" "}
            <span className="muted">{instant(quality.checklist.reviewed_at)}</span>
          </span>
        ) : quality.requires_checklist ? (
          <span className="refusal">NONE recorded for these bytes — this pack requires one</span>
        ) : (
          <span className="muted">none recorded; this pack does not require one</span>
        )}
      </h4>
      {quality.checklist ? (
        <>
          <ul className="plain answers">
            {Object.entries(quality.checklist.answers).map(([item, verdict]) => (
              <li key={item} className={verdict === "no" ? "concern" : undefined}>
                <span className="verdict">{verdict}</span> {item}
              </li>
            ))}
          </ul>
          {quality.checklist.note ? (
            <p className="comment">“{quality.checklist.note}”</p>
          ) : null}
          {quality.checklist.concerns.length > 0 ? (
            <p className="refusal">
              a reviewer objected to {quality.checklist.concerns.join(", ")}; publishing over that
              needs an override under every pack
            </p>
          ) : null}
        </>
      ) : quality.requires_checklist ? (
        <p className="muted">
          record it with: <code>synveda proposal checklist {id}</code>
        </p>
      ) : null}

      {quality.needs_override ? (
        <p className="refusal">
          {/* The sentences are the gateway's, verbatim. A second author of
              one line is two lines, and nothing would ever fail when they
              diverged (ADR-0056 decision 6). */}
          publishing this needs a quality override (
          {quality.shortfalls.map((shortfall) => shortfall.detail).join("; ")}); approving it does
          not clear the bar
        </p>
      ) : null}
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
    ? diffLines(member.baseline ? readable(member.baseline.text) : null, readable(member.proposed))
    : [];
  return (
    <div className="member">
      <div className={`member-head ${member.effect}`}>
        <span className="effect">{effectLabel(member.effect)}</span> <code>{label(member.member)}</code>{" "}
        <span className="muted">
          {member.class ?? member.asset} · {member.sensitivity}
        </span>
      </div>
      {member.unchanged ? null : (
        <div className="drift">
          <p className="refusal">this has changed since it was proposed; publishing will refuse</p>
          {/* What it says *now*, which is a third thing beside the baseline
              and the proposal and belongs to nobody's decision yet
              (ADR-0035 decision 5). Telling a reviewer the bytes moved
              without telling them where to is telling them to go and look. */}
          <pre className="now">{readable(member.content)}</pre>
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
