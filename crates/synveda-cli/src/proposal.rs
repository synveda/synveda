//! `synveda proposal` — the review flow (FLOW-6, ADR-0035).
//!
//! Every verb here is an HTTP call to `/v1/proposals` under the reviewer's
//! own bearer, and that is the feature. Approving is a governed act: the
//! PDP decides who may cast a verdict (`ProposalReview`) and who may run
//! the effect (`ChannelPublish` + `MemoryRead`), the approval matrix
//! decides how many verdicts are needed, and the gateway chains the event.
//! A CLI that wrote the approval row itself would be the counting rule
//! acting as authority, from a laptop, with no decision anywhere in the
//! trail — so this module opens no database connection at all.
//!
//! What it renders is the proposal's **effect on the target's published
//! channel**: per record, whether publication would add it, replace an
//! older version, or change nothing, with a diff of the two sides for the
//! records it would replace. That is what a reviewer is actually voting
//! on, and it is the half of a review a console would otherwise be needed
//! for.

use std::io::{BufRead, IsTerminal, Write};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use synveda_types::{ProposalId, ProposalState, ProposalView, ScopeId, Sensitivity};

use crate::api::{Api, Origin};
use crate::diff::{self, Mark};

// ── The wire shapes (`crates/synveda-gateway/src/proposals.rs`) ─────────

#[derive(Deserialize)]
struct ListResponse {
    proposals: Vec<Summary>,
}

#[derive(Deserialize)]
struct Summary {
    id: ProposalId,
    target_scope_id: ScopeId,
    source_scope_id: ScopeId,
    #[serde(default)]
    target_scope_path: Option<String>,
    #[serde(default)]
    source_scope_path: Option<String>,
    asset: String,
    /// What running it would do: `published`, or `lapse` since AUTHZ-4.
    effect: String,
    state: ProposalView,
    sensitivity: Sensitivity,
    title: String,
    commit: String,
    proposer_subject: String,
    created_at: DateTime<Utc>,
    #[serde(default)]
    close_reason: Option<String>,
    required: Requirement,
    outstanding: String,
    /// Present when a FLOW-4 rule opened this rather than a person.
    #[serde(default)]
    promotion: Option<synveda_types::PromotionEvidence>,
}

#[derive(Deserialize)]
struct Requirement {
    roles: Vec<RequiredRole>,
    distinct_approvers: u8,
    #[serde(default)]
    subjects: Vec<String>,
    origins: Vec<String>,
}

#[derive(Deserialize)]
struct RequiredRole {
    role: String,
    count: u8,
}

#[derive(Deserialize)]
struct Detail {
    #[serde(flatten)]
    summary: Summary,
    members: Vec<Member>,
    approvals: Vec<Approval>,
}

#[derive(Deserialize)]
struct Member {
    /// The tree entry name: a record id for a memory, a path for a prompt
    /// (PRMT-1, ADR-0049 decision 3). The one field both asset kinds
    /// carry, and the one this surface displays.
    member: String,
    /// What kind of asset this proposal carries.
    asset: String,
    object_hash: String,
    unchanged: bool,
    /// A memory's class. Absent for an authored asset, which has none —
    /// `asset` is what the line says instead.
    #[serde(default)]
    class: Option<String>,
    sensitivity: Sensitivity,
    effect: Effect,
    proposed: String,
    #[serde(default)]
    baseline: Option<Baseline>,
}

impl Member {
    /// How the member is named on its line: a uuid is abbreviated the way
    /// `channel history` abbreviates a commit, and a path is not — a name
    /// a person typed is the whole point of the name.
    fn label(&self) -> String {
        let uuid_shaped = self.member.len() == 36
            && self
                .member
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-');
        if uuid_shaped {
            short(&self.member)
        } else {
            self.member.clone()
        }
    }

    /// What it is, in one word: a record's class, or the asset kind.
    fn kind(&self) -> &str {
        self.class.as_deref().unwrap_or(&self.asset)
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Effect {
    Add,
    Update,
    None,
}

#[derive(Deserialize)]
struct Baseline {
    object_hash: String,
    text: String,
}

#[derive(Deserialize)]
struct Approval {
    approver_subject: String,
    verdict: String,
    roles: Vec<String>,
    counts: bool,
    #[serde(default)]
    comment: Option<String>,
    created_at: DateTime<Utc>,
}

// ── Verbs ──────────────────────────────────────────────────────────────

/// `synveda proposal list`.
pub async fn list(
    profile: &str,
    scope: Option<ScopeId>,
    state: Option<ProposalState>,
    limit: Option<i64>,
    as_json: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    let path = list_path(scope, state, limit);
    let body = api.get(&path).await.map_err(|error| widen(&error, scope))?;
    if as_json {
        println!("{body}");
        return Ok(());
    }
    let listing: ListResponse = serde_json::from_value(body)
        .map_err(|err| format!("the gateway's listing is not the shape expected: {err}"))?;
    announce(&api, &origin);
    if listing.proposals.is_empty() {
        eprintln!("synveda: nothing open here");
        return Ok(());
    }
    for summary in &listing.proposals {
        println!("{}", row(summary));
    }
    Ok(())
}

/// `synveda proposal show <id>`.
pub async fn show(profile: &str, id: ProposalId, as_json: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    let body = api.get(&format!("/v1/proposals/{id}")).await?;
    if as_json {
        println!("{body}");
        return Ok(());
    }
    let detail: Detail = serde_json::from_value(body)
        .map_err(|err| format!("the gateway's proposal is not the shape expected: {err}"))?;
    announce(&api, &origin);
    print!("{}", render_detail(&detail, colour()));
    Ok(())
}

/// `synveda proposal approve <id>`.
pub async fn approve(profile: &str, id: ProposalId, comment: Option<String>) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    cast_approval(&api, id, comment.as_deref()).await
}

/// `synveda proposal reject <id> --reason`.
pub async fn reject(profile: &str, id: ProposalId, reason: String) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    cast_rejection(&api, id, &reason).await
}

/// `synveda proposal withdraw <id>` — the proposer closing their own.
pub async fn withdraw(profile: &str, id: ProposalId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let summary: Summary = api
        .post_as(&format!("/v1/proposals/{id}/withdraw"), None)
        .await?;
    eprintln!("synveda: proposal {id} withdrawn");
    println!("{}", row(&summary));
    Ok(())
}

/// `synveda proposal classify <id>` — run an approved classification
/// proposal's effect (AUTHZ-5, ADR-0038 decision 9).
///
/// A sibling verb rather than a mode of `publish`, for the reason the two
/// routes are separate: they install different things, and a reviewer who
/// approved a tier change did not approve a channel move.
pub async fn classify(profile: &str, id: ProposalId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let classified = api
        .post(&format!("/v1/proposals/{id}/classify"), None)
        .await?;
    let field = |name: &str| {
        classified
            .get(name)
            .and_then(|value| value.as_str())
            .unwrap_or("?")
            .to_owned()
    };
    let records = classified
        .get("records")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or(0);
    eprintln!(
        "synveda: reclassified — {records} record(s) at scope {} now carry {}",
        field("scope_id"),
        field("sensitivity"),
    );
    Ok(())
}

/// `synveda proposal publish <id>` — run an approved proposal's effect.
///
/// A separate act by design (ADR-0032 decision 9): the deciding approval
/// does not publish, because auto-publishing would run under system
/// authority exactly when a `compliance` reviewer casts the deciding vote,
/// and `compliance` holds no publish grant in any pack. Reachable from
/// here because a review flow that cannot conclude is not a review flow
/// (ADR-0035 decision 3).
pub async fn publish(profile: &str, id: ProposalId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let published = api
        .post(&format!("/v1/proposals/{id}/publish"), None)
        .await?;
    let field = |name: &str| {
        published
            .get(name)
            .map(|value| match value.as_str() {
                Some(text) => text.to_owned(),
                None => value.to_string(),
            })
            .unwrap_or_default()
    };
    eprintln!(
        "synveda: published — {} at scope {} is now commit {} ({} members, {} added)",
        field("channel"),
        field("scope_id"),
        short(&field("commit")),
        field("members"),
        field("added"),
    );
    Ok(())
}

/// `synveda proposal review` — the interactive queue (ADR-0035 decision
/// 4).
///
/// Oldest first: a review queue that starves its oldest entry is how a
/// proposal quietly never gets read. EOF on stdin ends the walk having
/// cast nothing, so an unattended invocation is a no-op rather than a
/// blind approval — the fail-safe direction for a surface whose verdicts
/// move content across a trust boundary.
pub async fn review(
    profile: &str,
    id: Option<ProposalId>,
    scope: Option<ScopeId>,
    limit: Option<i64>,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);

    let queue: Vec<ProposalId> = match id {
        Some(id) => vec![id],
        None => {
            let path = list_path(scope, Some(ProposalState::Open), limit);
            let listing: ListResponse = api
                .get_as(&path)
                .await
                .map_err(|error| widen(&error, scope))?;
            // The listing is newest-first; a queue is drained the other way.
            listing
                .proposals
                .iter()
                .rev()
                .map(|summary| summary.id)
                .collect()
        }
    };
    if queue.is_empty() {
        eprintln!("synveda: nothing open to review here");
        return Ok(());
    }
    eprintln!(
        "synveda: {} proposal(s) to review, oldest first",
        queue.len()
    );

    let colour = colour();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    for (position, id) in queue.iter().enumerate() {
        let detail: Detail = match api.get_as(&format!("/v1/proposals/{id}")).await {
            Ok(detail) => detail,
            // Closed or made invisible since the listing: report and move
            // on. One unreadable proposal must not end a review session.
            Err(error) => {
                eprintln!("synveda: skipping {id}: {error}");
                continue;
            }
        };
        println!();
        println!("── {} of {} ──", position + 1, queue.len());
        print!("{}", render_detail(&detail, colour));

        match prompt(&mut input)? {
            Verdict::Approve(comment) => cast_approval(&api, *id, comment.as_deref()).await?,
            Verdict::Reject(reason) => cast_rejection(&api, *id, &reason).await?,
            Verdict::Skip => eprintln!("synveda: skipped {id}"),
            Verdict::Quit => {
                eprintln!("synveda: stopped; nothing further was cast");
                return Ok(());
            }
        }
    }
    Ok(())
}

// ── The two acts, shared by `review` and the direct verbs ───────────────

async fn cast_approval(api: &Api, id: ProposalId, comment: Option<&str>) -> Result<(), String> {
    let body = comment.map(|comment| json!({ "comment": comment }));
    let response = api
        .post(&format!("/v1/proposals/{id}/approve"), body)
        .await?;
    let summary: Summary = serde_json::from_value(response.clone())
        .map_err(|err| format!("the gateway's answer is not the shape expected: {err}"))?;
    let counted = response
        .get("counted_roles")
        .and_then(|value| value.as_array())
        .map(|roles| {
            roles
                .iter()
                .filter_map(|role| role.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    eprintln!("synveda: approved {id} as {counted}");
    eprintln!("         state: {}", summary.state.as_str());
    if summary.state == ProposalView::Approved {
        eprintln!("         run its effect with `synveda proposal publish {id}`");
    } else {
        eprintln!("         still outstanding: {}", summary.outstanding);
    }
    Ok(())
}

async fn cast_rejection(api: &Api, id: ProposalId, reason: &str) -> Result<(), String> {
    if reason.trim().is_empty() {
        return Err("a rejection must say why".to_owned());
    }
    let _: Summary = api
        .post_as(
            &format!("/v1/proposals/{id}/reject"),
            Some(json!({ "reason": reason })),
        )
        .await?;
    eprintln!("synveda: rejected {id} — {reason}");
    eprintln!("         terminal: a revision is a new proposal");
    Ok(())
}

// ── Rendering ──────────────────────────────────────────────────────────

/// One listing row: enough to decide whether to open it.
fn row(summary: &Summary) -> String {
    let scope = summary
        .target_scope_path
        .clone()
        .unwrap_or_else(|| summary.target_scope_id.to_string());
    let climb = if summary.source_scope_id == summary.target_scope_id {
        String::new()
    } else {
        let from = summary
            .source_scope_path
            .clone()
            .unwrap_or_else(|| summary.source_scope_id.to_string());
        format!("  ({from} ↑)")
    };
    format!(
        "{}  {:<9}  {:<10}  {}{}\n    {} — {} · by {} · {}",
        short(&summary.id.to_string()),
        summary.state.as_str(),
        summary.sensitivity.as_str(),
        scope,
        climb,
        summary.title,
        summary.outstanding,
        summary.proposer_subject,
        summary.created_at.format("%Y-%m-%d %H:%M"),
    )
}

/// The full review view: what it proposes, what it needs, who has acted,
/// and what it would do to the channel.
fn render_detail(detail: &Detail, colour: bool) -> String {
    let summary = &detail.summary;
    let mut out = String::new();
    let paint = |mark: Mark, text: &str| paint_line(mark, text, colour);

    out.push_str(&format!("proposal {}\n", summary.id));
    out.push_str(&format!("  title        {}\n", summary.title));
    out.push_str(&format!("  state        {}\n", summary.state.as_str()));
    out.push_str(&format!(
        "  target       {} · {}/{}\n",
        summary
            .target_scope_path
            .clone()
            .unwrap_or_else(|| summary.target_scope_id.to_string()),
        summary.asset,
        summary.effect,
    ));
    if summary.source_scope_id != summary.target_scope_id {
        out.push_str(&format!(
            "  source       {}  (a climb: this scope holds the material)\n",
            summary
                .source_scope_path
                .clone()
                .unwrap_or_else(|| summary.source_scope_id.to_string()),
        ));
    }
    out.push_str(&format!(
        "  sensitivity  {}\n",
        summary.sensitivity.as_str()
    ));
    out.push_str(&format!(
        "  proposed by  {} at {}\n",
        summary.proposer_subject,
        summary.created_at.format("%Y-%m-%d %H:%M:%S")
    ));
    out.push_str(&format!("  commit       {}\n", summary.commit));
    out.push_str(&format!("  requires     {}\n", describe(&summary.required)));
    out.push_str(&format!("  outstanding  {}\n", summary.outstanding));
    if let Some(reason) = &summary.close_reason {
        out.push_str(&format!("  closed       {reason}\n"));
    }
    if let Some(evidence) = &summary.promotion {
        out.push_str(&format!(
            "  opened by    rule `{}` — {}\n",
            evidence.rule,
            evidence.summary()
        ));
        out.push_str(&format!(
            "               checkable against audit seq {}..={}\n",
            evidence.from_seq, evidence.to_seq
        ));
    }

    out.push_str("\n  reviews\n");
    if detail.approvals.is_empty() {
        out.push_str("    (none yet)\n");
    }
    for approval in &detail.approvals {
        let stale = if approval.counts {
            ""
        } else {
            "  [of an earlier commit — does not count]"
        };
        out.push_str(&format!(
            "    {} {} as {} at {}{}\n",
            approval.verdict,
            approval.approver_subject,
            approval.roles.join(", "),
            approval.created_at.format("%Y-%m-%d %H:%M"),
            stale,
        ));
        if let Some(comment) = &approval.comment {
            out.push_str(&format!("      \"{comment}\"\n"));
        }
    }

    out.push_str(&format!(
        "\n  effect on {} {}/{}\n",
        summary
            .target_scope_path
            .clone()
            .unwrap_or_else(|| summary.target_scope_id.to_string()),
        summary.asset,
        summary.effect,
    ));
    for member in &detail.members {
        let (mark, label) = match member.effect {
            Effect::Add => (Mark::Added, "add   "),
            Effect::Update => (Mark::Meta, "update"),
            Effect::None => (Mark::Plain, "same  "),
        };
        out.push_str(&paint(
            mark,
            &format!(
                "    {label}  {}  {} · {}",
                member.label(),
                member.kind(),
                member.sensitivity.as_str()
            ),
        ));
        out.push('\n');
        if !member.unchanged {
            out.push_str(&paint(
                Mark::Removed,
                "              this has changed since it was proposed; \
                 publishing will refuse",
            ));
            out.push('\n');
        }
        if member.effect == Effect::None {
            continue;
        }
        let before = member
            .baseline
            .as_ref()
            .map(|baseline| baseline.text.as_str());
        for line in diff::render(before, &member.proposed) {
            out.push_str(&paint(line.mark, &format!("        {}", line.text)));
            out.push('\n');
        }
        if let Some(baseline) = &member.baseline {
            out.push_str(&format!(
                "        (replacing object {})\n",
                short(&baseline.object_hash)
            ));
        }
        out.push_str(&format!(
            "        (proposed object {})\n",
            short(&member.object_hash)
        ));
    }
    out
}

/// The requirement in one line: what the matrix asks for, and where each
/// part of it came from.
fn describe(requirement: &Requirement) -> String {
    let mut parts: Vec<String> = requirement
        .roles
        .iter()
        .map(|required| format!("{} × {}", required.count, required.role))
        .collect();
    if requirement.distinct_approvers > 1 {
        parts.push(format!(
            "{} distinct approvers",
            requirement.distinct_approvers
        ));
    }
    for subject in &requirement.subjects {
        parts.push(format!("@{subject}"));
    }
    if parts.is_empty() {
        parts.push("nothing".to_owned());
    }
    format!(
        "{}  (from: {})",
        parts.join(" + "),
        requirement.origins.join(", ")
    )
}

/// A 12-character prefix: enough to name a proposal, a record, or an
/// object at a glance, and short enough to read in a list.
fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

fn list_path(scope: Option<ScopeId>, state: Option<ProposalState>, limit: Option<i64>) -> String {
    let mut query: Vec<String> = Vec::new();
    if let Some(scope) = scope {
        query.push(format!("scope_id={scope}"));
    }
    if let Some(state) = state {
        query.push(format!("state={}", state.as_str()));
    }
    if let Some(limit) = limit {
        query.push(format!("limit={limit}"));
    }
    if query.is_empty() {
        "/v1/proposals".to_owned()
    } else {
        format!("/v1/proposals?{}", query.join("&"))
    }
}

/// A tenant-wide listing is a *tenant*-resource decision, which the packs
/// grant to tenant-wide review and admin roles only — being a curator at
/// one team is deliberately not a reason to see every proposal in the
/// tenant. A curator who hits that denial needs `--scope`, so the refusal
/// says so rather than leaving them to read a Cedar file.
fn widen(error: &str, scope: Option<ScopeId>) -> String {
    if scope.is_none() && error.contains("policy denied") {
        return format!(
            "{error}\n         a tenant-wide listing needs a tenant-wide review role; \
             pass --scope <id> to list the proposals at one scope"
        );
    }
    error.to_owned()
}

/// Says which identity is about to act, once, on stderr. "Which identity
/// am I approving as" is the first thing a reviewer needs and the last
/// thing they should have to guess.
fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(name) => eprintln!(
            "synveda: {} at {} (profile `{name}`)",
            api.subject,
            api.gateway()
        ),
        Origin::Environment => eprintln!("synveda: acting with SYNVEDA_TOKEN at {}", api.gateway()),
    }
}

// ── The interactive prompt ─────────────────────────────────────────────

enum Verdict {
    Approve(Option<String>),
    Reject(String),
    Skip,
    Quit,
}

/// Asks for one verdict. EOF is [`Verdict::Quit`], which is what makes a
/// non-interactive `synveda proposal review` cast nothing.
fn prompt(input: &mut impl BufRead) -> Result<Verdict, String> {
    loop {
        eprint!("  [a]pprove  [r]eject  [s]kip  [q]uit > ");
        let _ = std::io::stderr().flush();
        let Some(answer) = read_line(input)? else {
            eprintln!();
            return Ok(Verdict::Quit);
        };
        match answer.trim() {
            "a" | "approve" => {
                eprint!("  comment (optional) > ");
                let _ = std::io::stderr().flush();
                let comment = read_line(input)?
                    .map(|line| line.trim().to_owned())
                    .filter(|line| !line.is_empty());
                return Ok(Verdict::Approve(comment));
            }
            "r" | "reject" => {
                // Mandatory, and re-asked rather than defaulted: a
                // rejection an auditor cannot read the reason for is not a
                // review (ADR-0032 decision 12).
                loop {
                    eprint!("  reason (required) > ");
                    let _ = std::io::stderr().flush();
                    let Some(reason) = read_line(input)? else {
                        eprintln!();
                        return Ok(Verdict::Quit);
                    };
                    let reason = reason.trim().to_owned();
                    if !reason.is_empty() {
                        return Ok(Verdict::Reject(reason));
                    }
                    eprintln!("  a rejection must say why");
                }
            }
            "s" | "skip" | "" => return Ok(Verdict::Skip),
            "q" | "quit" => return Ok(Verdict::Quit),
            other => eprintln!("  `{other}`? answer a, r, s, or q"),
        }
    }
}

/// One line, or `None` at end of input.
fn read_line(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(line)),
        Err(err) => Err(format!("read from stdin: {err}")),
    }
}

// ── Colour ─────────────────────────────────────────────────────────────

/// Colour when stdout is a terminal and `NO_COLOR` is unset. Piped output
/// stays plain, which is what makes the demo's assertions readable.
fn colour() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn paint_line(mark: Mark, text: &str, colour: bool) -> String {
    if !colour {
        return text.to_owned();
    }
    let code = match mark {
        Mark::Added => "32",
        Mark::Removed => "31",
        Mark::Meta => "36",
        Mark::Plain => return text.to_owned(),
    };
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(state: ProposalView, target: &str, source: &str) -> Summary {
        Summary {
            id: ProposalId::new(),
            target_scope_id: ScopeId::new(),
            source_scope_id: ScopeId::new(),
            target_scope_path: Some(target.to_owned()),
            source_scope_path: Some(source.to_owned()),
            asset: "memory".to_owned(),
            effect: "published".to_owned(),
            state,
            sensitivity: Sensitivity::Internal,
            title: "promote the key-rotation runbook".to_owned(),
            commit: "a".repeat(64),
            proposer_subject: "dana".to_owned(),
            created_at: Utc::now(),
            close_reason: None,
            required: Requirement {
                roles: vec![RequiredRole {
                    role: "curator".to_owned(),
                    count: 1,
                }],
                distinct_approvers: 1,
                subjects: Vec::new(),
                origins: vec!["pack regulated-strict".to_owned()],
            },
            outstanding: "1 × curator".to_owned(),
            promotion: None,
        }
    }

    fn member(effect: Effect, before: Option<&str>, after: &str) -> Member {
        Member {
            record_id: "0198f000-0000-7000-8000-000000000001".to_owned(),
            object_hash: "b".repeat(64),
            unchanged: true,
            class: "procedure".to_owned(),
            sensitivity: Sensitivity::Internal,
            effect,
            proposed: after.to_owned(),
            baseline: before.map(|text| Baseline {
                object_hash: "c".repeat(64),
                text: text.to_owned(),
            }),
        }
    }

    fn asset(content: &str) -> String {
        serde_json::json!({"class": "procedure", "content": content, "sensitivity": "internal"})
            .to_string()
    }

    #[test]
    fn a_climb_names_both_scopes_and_a_same_scope_proposal_names_one() {
        let mut climbing = summary(ProposalView::Open, "acme/eng", "acme/eng/platform");
        let rendered = row(&climbing);
        assert!(rendered.contains("acme/eng"), "{rendered}");
        assert!(
            rendered.contains("acme/eng/platform ↑"),
            "a climb must show where it came from:\n{rendered}"
        );

        climbing.source_scope_id = climbing.target_scope_id;
        let same = row(&climbing);
        assert!(
            !same.contains('↑'),
            "a same-scope proposal is not a climb:\n{same}"
        );
    }

    #[test]
    fn the_detail_view_renders_the_effect_on_the_channel() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme/eng/platform", "acme/eng/platform"),
            members: vec![
                member(Effect::Add, None, &asset("brand new")),
                member(
                    Effect::Update,
                    Some(&asset("rotate every 180 days")),
                    &asset("rotate every 90 days"),
                ),
            ],
            approvals: Vec::new(),
        };

        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("1 × curator"), "{rendered}");
        assert!(rendered.contains("(none yet)"), "{rendered}");
        assert!(rendered.contains("add   "), "{rendered}");
        assert!(rendered.contains("update"), "{rendered}");
        // The replacement's two sides.
        assert!(rendered.contains("- rotate every 180 days"), "{rendered}");
        assert!(rendered.contains("+ rotate every 90 days"), "{rendered}");
        assert!(rendered.contains("replacing object"), "{rendered}");
    }

    #[test]
    fn a_no_op_member_renders_no_diff_at_all() {
        let detail = Detail {
            summary: summary(ProposalView::Approved, "acme", "acme"),
            members: vec![member(Effect::None, None, &asset("already published"))],
            approvals: Vec::new(),
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("same  "), "{rendered}");
        assert!(
            !rendered.contains("already published"),
            "a member the channel already holds has nothing to diff:\n{rendered}"
        );
    }

    #[test]
    fn drift_is_called_out_before_anyone_votes() {
        let mut detail = Detail {
            summary: summary(ProposalView::Open, "acme", "acme"),
            members: vec![member(Effect::Add, None, &asset("as proposed"))],
            approvals: Vec::new(),
        };
        detail.members[0].unchanged = false;
        let rendered = render_detail(&detail, false);
        assert!(
            rendered.contains("publishing will refuse"),
            "an edited record must be visible as such:\n{rendered}"
        );
    }

    #[test]
    fn an_approval_of_an_earlier_commit_is_marked_as_not_counting() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme", "acme"),
            members: Vec::new(),
            approvals: vec![
                Approval {
                    approver_subject: "cora".to_owned(),
                    verdict: "approve".to_owned(),
                    roles: vec!["curator".to_owned()],
                    counts: true,
                    comment: Some("matches the runbook".to_owned()),
                    created_at: Utc::now(),
                },
                Approval {
                    approver_subject: "sam".to_owned(),
                    verdict: "approve".to_owned(),
                    roles: vec!["steward".to_owned()],
                    counts: false,
                    comment: None,
                    created_at: Utc::now(),
                },
            ],
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("matches the runbook"), "{rendered}");
        assert!(rendered.contains("does not count"), "{rendered}");
    }

    #[test]
    fn the_requirement_names_its_parts_and_where_they_came_from() {
        let requirement = Requirement {
            roles: vec![
                RequiredRole {
                    role: "curator".to_owned(),
                    count: 1,
                },
                RequiredRole {
                    role: "compliance".to_owned(),
                    count: 1,
                },
            ],
            distinct_approvers: 2,
            subjects: vec!["sam".to_owned()],
            origins: vec!["floor".to_owned(), "pack regulated-strict".to_owned()],
        };
        let described = describe(&requirement);
        assert!(described.contains("1 × curator"), "{described}");
        assert!(described.contains("1 × compliance"), "{described}");
        assert!(described.contains("2 distinct approvers"), "{described}");
        assert!(described.contains("@sam"), "{described}");
        assert!(described.contains("floor"), "{described}");

        let nothing = Requirement {
            roles: Vec::new(),
            distinct_approvers: 0,
            subjects: Vec::new(),
            origins: vec!["pack open-collaboration".to_owned()],
        };
        assert!(
            describe(&nothing).starts_with("nothing"),
            "a pack may ask for none"
        );
    }

    #[test]
    fn the_query_string_carries_only_what_was_asked_for() {
        assert_eq!(list_path(None, None, None), "/v1/proposals");
        let scoped = ScopeId::new();
        assert_eq!(
            list_path(Some(scoped), Some(ProposalState::Open), Some(5)),
            format!("/v1/proposals?scope_id={scoped}&state=open&limit=5")
        );
    }

    #[test]
    fn a_tenant_wide_denial_names_the_flag_that_fixes_it() {
        let denied = "policy denied ProposalRead on tenant 0198: no policy permits it";
        let widened = widen(denied, None);
        assert!(widened.contains("--scope"), "{widened}");
        // With a scope already given, the denial is about that scope and
        // there is no flag to suggest.
        assert_eq!(widen(denied, Some(ScopeId::new())), denied);
        // And an unrelated failure is never dressed up as a policy hint.
        assert_eq!(widen("HTTP 502 Bad Gateway", None), "HTTP 502 Bad Gateway");
    }

    #[test]
    fn end_of_input_casts_nothing() {
        // `synveda proposal review < /dev/null` must be a no-op, not an
        // approval (ADR-0035 decision 4).
        let mut empty = std::io::Cursor::new(Vec::new());
        assert!(matches!(prompt(&mut empty).expect("prompt"), Verdict::Quit));

        // And an approval interrupted before its comment quits rather than
        // casting a bare one.
        let mut interrupted = std::io::Cursor::new(b"a\n".to_vec());
        match prompt(&mut interrupted).expect("prompt") {
            Verdict::Approve(comment) => assert!(comment.is_none()),
            _ => panic!("`a` is an approval"),
        }
    }

    #[test]
    fn a_rejection_is_re_asked_until_it_says_why() {
        let mut input = std::io::Cursor::new(b"r\n\n   \nthe runbook is wrong\n".to_vec());
        match prompt(&mut input).expect("prompt") {
            Verdict::Reject(reason) => assert_eq!(reason, "the runbook is wrong"),
            _ => panic!("`r` is a rejection"),
        }
    }

    #[test]
    fn an_unrecognised_answer_asks_again_and_a_bare_newline_skips() {
        let mut input = std::io::Cursor::new(b"x\ns\n".to_vec());
        assert!(matches!(prompt(&mut input).expect("prompt"), Verdict::Skip));

        let mut blank = std::io::Cursor::new(b"\n".to_vec());
        assert!(matches!(prompt(&mut blank).expect("prompt"), Verdict::Skip));

        let mut quit = std::io::Cursor::new(b"q\n".to_vec());
        assert!(matches!(prompt(&mut quit).expect("prompt"), Verdict::Quit));
    }

    #[test]
    fn colour_is_only_ever_applied_when_asked_for() {
        assert_eq!(paint_line(Mark::Added, "+ x", false), "+ x");
        let painted = paint_line(Mark::Added, "+ x", true);
        assert!(painted.starts_with("\u{1b}[32m") && painted.ends_with("\u{1b}[0m"));
        // Plain lines stay plain either way, so a piped diff is byte-clean.
        assert_eq!(paint_line(Mark::Plain, "  x", true), "  x");
    }
}
