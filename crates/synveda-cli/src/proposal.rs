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
use serde_json::{Value, json};
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
    /// The security scan of the bytes this proposal would publish
    /// (SKIL-2, ADR-0052). Present for `skill` proposals only, which is
    /// why it is defaulted rather than required: this struct is the same
    /// one four asset kinds come back in.
    #[serde(default)]
    scan: Option<ScanReport>,
    /// The quality of the bytes this proposal would publish (SKIL-3,
    /// ADR-0053). Present for `skill` proposals only.
    #[serde(default)]
    quality: Option<QualityReport>,
}

/// A bundle's security scan, as `synveda proposal review` renders it.
///
/// FLOW-6's claim is that a full review is possible without the console,
/// and for a skill that claim is now partly this block: a reviewer who
/// cannot see what the scanner found is a reviewer being asked to
/// approve executable code on the strength of a diff.
#[derive(Deserialize)]
struct ScanReport {
    ruleset_version: u32,
    #[serde(default)]
    worst: Option<String>,
    blocks_at: String,
    blocked: bool,
    findings: Vec<ScanFinding>,
}

#[derive(Deserialize)]
struct ScanFinding {
    path: String,
    rule: String,
    severity: String,
    title: String,
    line: usize,
    count: usize,
    /// The gateway's verdict on this finding (ADR-0056 decision 5).
    ///
    /// Absent only from a gateway older than this CLI, which is the one
    /// skew direction that survives — the console cannot be out of step
    /// with its gateway because the gateway ships it, and this binary
    /// can.
    #[serde(default)]
    blocking: Option<bool>,
}

impl ScanReport {
    /// Whether this finding is one the pack in force refuses.
    ///
    /// **The served verdict wins.** The gateway holds both the severity
    /// order and the pack, so it is the only participant that can answer
    /// this without guessing; ADR-0056 decision 5 moved the answer there
    /// so that two renderers could not disagree about it.
    ///
    /// The rank comparison below is what remains, and it is a fallback
    /// for an *older* gateway rather than a second implementation of the
    /// rule. It compares by rank rather than by string, so `critical`
    /// under a `high` threshold blocks where equality would have said it
    /// did not, and a severity this binary has never heard of ranks above
    /// everything and is treated as blocking rather than as decoration.
    fn blocks(&self, finding: &ScanFinding) -> bool {
        if let Some(blocking) = finding.blocking {
            return blocking;
        }
        fn rank(severity: &str) -> u8 {
            match severity {
                "notice" => 0,
                "high" => 1,
                "critical" => 2,
                _ => u8::MAX,
            }
        }
        rank(&finding.severity) >= rank(&self.blocks_at)
    }
}

/// A bundle's quality, as `synveda proposal review` renders it (SKIL-3,
/// ADR-0053 decision 11).
///
/// **Two numbers, never one.** The rubric measures the bundle; the
/// checklist is what a person checked. A reviewer who sees them averaged
/// cannot tell a well-formatted bundle nobody worked through from one
/// somebody did, which is the whole reason they are rendered apart.
#[derive(Deserialize)]
struct QualityReport {
    rubric_version: u32,
    score: u8,
    min_score: u8,
    requires_checklist: bool,
    checks: Vec<QualityCheck>,
    #[serde(default)]
    checklist: Option<ChecklistView>,
    #[serde(default)]
    shortfalls: Vec<Shortfall>,
    needs_override: bool,
}

#[derive(Deserialize)]
struct QualityCheck {
    check: String,
    passed: bool,
    weight: u8,
    title: String,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Deserialize)]
struct ChecklistView {
    answers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    note: Option<String>,
    complete: bool,
    concerns: Vec<String>,
    reviewed_at: DateTime<Utc>,
}

/// One bar the bundle misses.
///
/// The data fields ADR-0053 defined are still on the wire and are
/// deliberately not deserialised here: since ADR-0056 decision 6 the
/// gateway serves `detail` — [`QualityShortfall::describe`], the same
/// sentence its refusals and audit payloads carry — and this CLI's
/// reconstruction of that sentence is deleted rather than kept beside it.
/// Two authors of one sentence is two sentences, and a shortfall
/// explained one way at review and another at publication is the drift a
/// second renderer would have made permanent.
#[derive(Deserialize)]
struct Shortfall {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

impl Shortfall {
    /// One line a reviewer can act on, as the gateway wrote it.
    ///
    /// The fallback is for a gateway *older* than this CLI, which serves
    /// the slug and no sentence. Naming the slug is worse than a sentence
    /// and better than silence: an unexplained bar cannot be acted on at
    /// all.
    fn describe(&self) -> String {
        if let Some(detail) = &self.detail {
            return detail.clone();
        }
        format!(
            "{} (this gateway is older than the CLI and did not say why; run the publish to \
             see the refusal)",
            self.kind.as_deref().unwrap_or("an unnamed bar")
        )
    }
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
    /// The member's text **as it stands now**, which is neither the
    /// baseline nor the proposal when somebody has edited underneath an
    /// open review. Rendered only in that case: everywhere else it is a
    /// second copy of `proposed`, and a review surface that printed it
    /// twice would be inviting a reader to look for a difference that is
    /// not there.
    #[serde(default)]
    content: String,
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

/// `synveda proposal override-quality` — record a decision to publish a
/// skill the quality gate refuses (SKIL-3, ADR-0053 decision 8).
///
/// Its own verb rather than a flag on `publish`, because it is its own
/// authority: a steward grants this and cannot publish a skill (no content
/// read), a curator publishes and cannot grant this. Two acts, two people.
pub async fn override_quality(profile: &str, id: ProposalId, reason: &str) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let granted = api
        .post(
            &format!("/v1/proposals/{id}/quality-override"),
            Some(json!({"reason": reason})),
        )
        .await?;
    let digest = granted
        .get("bundle_digest")
        .and_then(Value::as_str)
        .unwrap_or_default();
    eprintln!(
        "synveda: override recorded for {} at {}/100 — bound to bundle {}",
        granted
            .get("skill")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        granted.get("score").and_then(Value::as_u64).unwrap_or(0),
        short(digest),
    );
    eprintln!(
        "synveda: it stands over exactly these bytes; an edit needs a new one. \
         Whoever ordinarily publishes can now run `synveda proposal publish {id}`"
    );
    Ok(())
}

/// `synveda proposal checklist` — record the reviewer's half of a skill's
/// quality score (SKIL-3, ADR-0053 decision 6).
///
/// The answers are bound to the bundle's **bytes**, which is why the
/// response echoes the digest: a reviewer who sees it change between two
/// runs is a reviewer whose author edited something underneath them, and
/// the previous answers no longer apply to anything.
pub async fn checklist(
    profile: &str,
    id: ProposalId,
    items: &[String],
    note: Option<String>,
) -> Result<(), String> {
    let mut answers = serde_json::Map::new();
    for item in items {
        let (name, verdict) = item.split_once('=').ok_or_else(|| {
            format!("--item wants ITEM=VERDICT, got {item:?} (e.g. --item tested=yes)")
        })?;
        let name = name.trim();
        let verdict = verdict.trim();
        // Spelling is checked at the gateway, where the vocabulary lives.
        // What is checked here is the *shape*, because `--item tested` with
        // no verdict is a typo a round trip should not be spent on.
        if name.is_empty() || verdict.is_empty() {
            return Err(format!(
                "--item wants ITEM=VERDICT with both halves, got {item:?}"
            ));
        }
        answers.insert(name.to_owned(), Value::String(verdict.to_owned()));
    }

    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let mut body = serde_json::Map::new();
    body.insert("answers".to_owned(), Value::Object(answers));
    if let Some(note) = note {
        body.insert("note".to_owned(), Value::String(note));
    }
    let recorded = api
        .post(
            &format!("/v1/proposals/{id}/checklist"),
            Some(Value::Object(body)),
        )
        .await?;

    let text = |name: &str| {
        recorded
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let complete = recorded
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let concerns: Vec<&str> = recorded
        .get("concerns")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    eprintln!(
        "synveda: checklist recorded for {} — {}, bound to bundle {}",
        text("skill"),
        if complete { "complete" } else { "PARTIAL" },
        short(&text("bundle_digest")),
    );
    if !complete {
        eprintln!(
            "synveda: a pack that requires a checklist will not accept a partial one; \
             answer the rest before publishing"
        );
    }
    if !concerns.is_empty() {
        eprintln!(
            "synveda: you answered `no` to {} — publishing over that needs an override \
             under every pack, which somebody holding SkillQualityOverride records with \
             `synveda proposal override-quality <id> --reason ...`",
            concerns.join(", "),
        );
    }
    if let Some(quality) = recorded.get("quality") {
        let score = quality.get("score").and_then(Value::as_u64).unwrap_or(0);
        let min = quality
            .get("min_score")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let needs = quality
            .get("needs_override")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        eprintln!(
            "synveda: the rubric scores it {score}/100 against this pack's {min}{}",
            if needs {
                " — publishing will need an override"
            } else {
                ""
            },
        );
    }
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

    // The scan goes *before* the diff, because it is what a reviewer has
    // to weigh first: the diff says what changed, and this says what the
    // change can do.
    if let Some(scan) = &detail.scan {
        out.push_str(&format!(
            "\n  security scan  (ruleset v{}, this pack refuses at {})\n",
            scan.ruleset_version, scan.blocks_at,
        ));
        if scan.findings.is_empty() {
            out.push_str(&paint(Mark::Plain, "    nothing found"));
            out.push('\n');
        }
        for finding in &scan.findings {
            // Only a blocking finding is painted as a removal: the rest
            // are things to weigh, and colouring them all red would make
            // the one that stops publication indistinguishable from a
            // `pip install`.
            let blocking = scan.blocks(finding);
            let mark = if blocking { Mark::Removed } else { Mark::Meta };
            let times = if finding.count > 1 {
                format!(" ×{}", finding.count)
            } else {
                String::new()
            };
            // The verdict in the text and not only in the colour. Colour is
            // the fast read for somebody at a terminal, and it is *nothing*
            // to a review piped to a file, read by a screen reader, or
            // pasted into a ticket — and this is the one fact on the line
            // that decides whether approving the proposal can achieve
            // anything. It matters most in the case the reader cannot
            // reason around: a severity from a gateway newer than this
            // binary, where the name itself tells them nothing.
            let blocks = if blocking { "  [blocks]" } else { "" };
            out.push_str(&paint(
                mark,
                &format!(
                    "    {:<8} {}:{}  {}{}{}",
                    finding.severity, finding.path, finding.line, finding.rule, times, blocks,
                ),
            ));
            out.push('\n');
            out.push_str(&format!("             {}\n", finding.title));
        }
        if scan.blocked {
            out.push_str(&paint(
                Mark::Removed,
                &format!(
                    "    this bundle will be REFUSED at publication ({} findings at {} \
                     or above); approving it cannot make it publishable",
                    scan.findings
                        .iter()
                        .filter(|finding| scan.blocks(finding))
                        .count(),
                    scan.blocks_at,
                ),
            ));
            out.push('\n');
        } else if let Some(worst) = &scan.worst {
            out.push_str(&format!(
                "    worst is {worst}; the pack in force reports it rather than refusing it, \
                 so this is yours to weigh\n"
            ));
        }
    }

    // Quality after the scan and before the diff. The order is the order a
    // reviewer decides in: is it safe, is it good, what changed.
    if let Some(quality) = &detail.quality {
        let bar = if quality.min_score == 0 {
            "this pack sets no bar".to_owned()
        } else {
            format!("this pack asks for {}", quality.min_score)
        };
        out.push_str(&format!(
            "\n  quality  {}/100  (rubric v{}, {bar})\n",
            quality.score, quality.rubric_version,
        ));
        // Only the failures are listed. A reviewer reading eight lines of
        // "passed" is a reviewer who stops reading this block, and the
        // score already says how much passed.
        for check in quality.checks.iter().filter(|check| !check.passed) {
            out.push_str(&paint(
                Mark::Meta,
                &format!(
                    "    -{:<3} {:<24} {}",
                    check.weight, check.check, check.title
                ),
            ));
            out.push('\n');
            if let Some(detail) = &check.detail {
                out.push_str(&format!("             {detail}\n"));
            }
        }
        if quality.checks.iter().all(|check| check.passed) {
            out.push_str(&paint(Mark::Plain, "    every check passed"));
            out.push('\n');
        }

        // The reviewer's half, rendered as its own thing rather than
        // folded into the number above it (ADR-0053 decision 1).
        match &quality.checklist {
            Some(checklist) => {
                out.push_str(&format!(
                    "\n    checklist  {} {}\n",
                    if checklist.complete {
                        "complete"
                    } else {
                        "PARTIAL"
                    },
                    checklist.reviewed_at.format("%Y-%m-%d %H:%M"),
                ));
                for (item, verdict) in &checklist.answers {
                    let mark = if verdict == "no" {
                        Mark::Removed
                    } else {
                        Mark::Meta
                    };
                    out.push_str(&paint(mark, &format!("      {verdict:<4} {item}")));
                    out.push('\n');
                }
                if let Some(note) = &checklist.note {
                    out.push_str(&format!("      \"{note}\"\n"));
                }
                if !checklist.concerns.is_empty() {
                    out.push_str(&paint(
                        Mark::Removed,
                        &format!(
                            "      a reviewer objected to {}; publishing over that needs an \
                             override under every pack",
                            checklist.concerns.join(", "),
                        ),
                    ));
                    out.push('\n');
                }
            }
            None if quality.requires_checklist => {
                out.push_str(&paint(
                    Mark::Removed,
                    "\n    checklist  NONE recorded for these bytes — this pack requires one",
                ));
                out.push('\n');
                out.push_str(&format!(
                    "      record it with:  synveda proposal checklist {}\n",
                    summary.id,
                ));
            }
            None => {
                out.push_str("\n    checklist  none recorded; this pack does not require one\n");
            }
        }

        if quality.needs_override {
            out.push_str(&paint(
                Mark::Removed,
                &format!(
                    "    publishing this needs a quality override ({}); approving it does \
                     not clear the bar",
                    quality
                        .shortfalls
                        .iter()
                        .map(Shortfall::describe)
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            ));
            out.push('\n');
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
            // And what it says *now*, which is a third thing beside the
            // baseline and the proposal and belongs to nobody's decision
            // yet (ADR-0035 decision 5). Telling a reviewer the bytes moved
            // without telling them where to is telling them to go and look
            // — and the diff below is deliberately not this text, because
            // what the approvals bind is what was proposed.
            for line in member.content.lines() {
                out.push_str(&paint(Mark::Meta, &format!("              now: {line}")));
                out.push('\n');
            }
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
            member: "0198f000-0000-7000-8000-000000000001".to_owned(),
            asset: "memory".to_owned(),
            object_hash: "b".repeat(64),
            unchanged: true,
            class: Some("procedure".to_owned()),
            sensitivity: Sensitivity::Internal,
            effect,
            proposed: after.to_owned(),
            // Undrifted, so the record still says what was proposed.
            content: after.to_owned(),
            baseline: before.map(|text| Baseline {
                object_hash: "c".repeat(64),
                text: text.to_owned(),
            }),
        }
    }

    /// A prompt member: named by path, with no class (PRMT-1, ADR-0049
    /// decision 3).
    fn prompt_member(effect: Effect, before: Option<&str>, after: &str) -> Member {
        Member {
            member: "support/triage-reply".to_owned(),
            asset: "prompt".to_owned(),
            class: None,
            ..member(effect, before, after)
        }
    }

    fn asset(content: &str) -> String {
        serde_json::json!({"class": "procedure", "content": content, "sensitivity": "internal"})
            .to_string()
    }

    /// The per-asset-kind renderer ADR-0035 predicted, as a rendering
    /// rather than a paragraph: a record id is abbreviated the way a
    /// commit is, a prompt's path is shown whole because it is a name a
    /// person typed, and a member with no class says what it is instead.
    #[test]
    fn a_prompt_member_is_named_by_path_and_labelled_by_its_asset_kind() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme/eng/platform", "acme/eng/platform"),
            members: vec![prompt_member(
                Effect::Update,
                Some("be brief"),
                "be brief, and link the runbook",
            )],
            approvals: Vec::new(),
            scan: None,
            quality: None,
        };
        let rendered = render_detail(&detail, false);
        assert!(
            rendered.contains("support/triage-reply  prompt · internal"),
            "a prompt member is named whole and labelled by its kind:\n{rendered}"
        );
        assert!(
            !rendered.contains("support/triag "),
            "and never abbreviated like an id:\n{rendered}"
        );
        // The review still shows both sides — the disclosure ADR-0035
        // decision 8 admits, unchanged by the asset kind.
        assert!(rendered.contains("- be brief"), "{rendered}");
        assert!(
            rendered.contains("+ be brief, and link the runbook"),
            "{rendered}"
        );
    }

    /// A memory member keeps the abbreviation, so the two kinds are
    /// distinguishable at a glance in one queue.
    #[test]
    fn a_record_id_is_still_abbreviated() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme", "acme"),
            members: vec![member(Effect::Add, None, &asset("brand new"))],
            approvals: Vec::new(),
            scan: None,
            quality: None,
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("0198f000-000"), "{rendered}");
        assert!(
            !rendered.contains("0198f000-0000-7000-8000-000000000001"),
            "a uuid-shaped member is shortened:\n{rendered}"
        );
        assert!(rendered.contains("procedure · internal"), "{rendered}");
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
            scan: None,
            quality: None,
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
            scan: None,
            quality: None,
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
            scan: None,
            quality: None,
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
            scan: None,
            quality: None,
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("matches the runbook"), "{rendered}");
        assert!(rendered.contains("does not count"), "{rendered}");
    }

    /// SKIL-2's half of FLOW-6's "a full review is possible without the
    /// console": a reviewer of executable code has to be able to see what
    /// the scanner found, and — when the pack in force will refuse it —
    /// that approving cannot make it publishable.
    #[test]
    fn a_blocking_scan_says_so_before_the_diff() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme/eng", "acme/eng"),
            members: Vec::new(),
            approvals: Vec::new(),
            scan: Some(ScanReport {
                ruleset_version: 1,
                worst: Some("critical".to_owned()),
                blocks_at: "critical".to_owned(),
                blocked: true,
                findings: vec![
                    ScanFinding {
                        path: "helper/scripts/setup.sh".to_owned(),
                        rule: "fetch-and-execute".to_owned(),
                        severity: "critical".to_owned(),
                        title: "downloads a remote script and pipes it straight into an \
                                interpreter"
                            .to_owned(),
                        line: 4,
                        count: 1,
                        blocking: Some(true),
                    },
                    ScanFinding {
                        path: "helper/SKILL.md".to_owned(),
                        rule: "package-install".to_owned(),
                        severity: "notice".to_owned(),
                        title: "installs packages at run time".to_owned(),
                        line: 9,
                        count: 2,
                        blocking: Some(false),
                    },
                ],
            }),
            quality: None,
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("security scan"), "{rendered}");
        assert!(rendered.contains("ruleset v1"), "{rendered}");
        assert!(
            rendered.contains("helper/scripts/setup.sh:4"),
            "the file and the line a reviewer opens: {rendered}"
        );
        assert!(rendered.contains("fetch-and-execute"), "{rendered}");
        assert!(rendered.contains("×2"), "counts render: {rendered}");
        assert!(
            rendered.contains("will be REFUSED at publication"),
            "{rendered}"
        );
        // The scan comes before the diff, because it says what the change
        // can do and the diff only says what it is.
        let scan_at = rendered.find("security scan").unwrap();
        let effect_at = rendered.find("effect on").unwrap();
        assert!(scan_at < effect_at, "{rendered}");
    }

    #[test]
    fn a_reported_scan_hands_the_judgement_to_the_reviewer() {
        let detail = Detail {
            summary: summary(ProposalView::Open, "acme/eng", "acme/eng"),
            members: Vec::new(),
            approvals: Vec::new(),
            scan: Some(ScanReport {
                ruleset_version: 1,
                worst: Some("high".to_owned()),
                blocks_at: "critical".to_owned(),
                blocked: false,
                findings: vec![ScanFinding {
                    path: "helper/scripts/build.sh".to_owned(),
                    rule: "privilege-change".to_owned(),
                    severity: "high".to_owned(),
                    title: "escalates privileges or makes a file executable".to_owned(),
                    line: 2,
                    count: 1,
                    blocking: Some(false),
                }],
            }),
            quality: None,
        };
        let rendered = render_detail(&detail, false);
        assert!(rendered.contains("yours to weigh"), "{rendered}");
        assert!(!rendered.contains("REFUSED"), "{rendered}");
    }

    fn finding(severity: &str, blocking: Option<bool>) -> ScanFinding {
        ScanFinding {
            path: "helper/scripts/setup.sh".to_owned(),
            rule: "fetch-and-execute".to_owned(),
            severity: severity.to_owned(),
            title: "does something worth weighing".to_owned(),
            line: 1,
            count: 1,
            blocking,
        }
    }

    /// ADR-0056 decision 5: the gateway holds the rule table and the pack,
    /// so its verdict is the answer and this CLI's comparison does not get
    /// a vote. The case that proves it is one where the two disagree —
    /// a `notice` the gateway calls blocking under a `critical`
    /// threshold, which the rank comparison would have painted as
    /// decoration.
    #[test]
    fn the_gateways_verdict_wins_over_the_local_comparison() {
        let report = ScanReport {
            ruleset_version: 1,
            worst: Some("notice".to_owned()),
            blocks_at: "critical".to_owned(),
            blocked: true,
            findings: Vec::new(),
        };
        assert!(report.blocks(&finding("notice", Some(true))));
        assert!(!report.blocks(&finding("critical", Some(false))));
    }

    /// The fallback, for a gateway older than this binary: the rank
    /// comparison, not the string one. `critical` under a `high`
    /// threshold blocks, and equality would have said it did not.
    #[test]
    fn without_a_served_verdict_a_severity_above_the_threshold_blocks() {
        let report = ScanReport {
            ruleset_version: 1,
            worst: Some("critical".to_owned()),
            blocks_at: "high".to_owned(),
            blocked: true,
            findings: Vec::new(),
        };
        assert!(report.blocks(&finding("critical", None)));
        assert!(report.blocks(&finding("high", None)));
        assert!(!report.blocks(&finding("notice", None)));
        // A severity a newer gateway grew is treated as blocking rather
        // than as decoration. Unreachable in practice now — a gateway new
        // enough to have grown a severity is new enough to serve
        // `blocking` — and kept because the fallback has to be safe on
        // its own terms.
        assert!(report.blocks(&finding("catastrophic", None)));
    }

    fn quality(score: u8, min: u8, checklist: Option<ChecklistView>) -> QualityReport {
        QualityReport {
            rubric_version: 1,
            score,
            min_score: min,
            requires_checklist: true,
            checks: vec![
                QualityCheck {
                    check: "description-states-when".to_owned(),
                    passed: true,
                    weight: 20,
                    title: "the description says when to use the skill".to_owned(),
                    detail: None,
                },
                QualityCheck {
                    check: "has-examples".to_owned(),
                    passed: false,
                    weight: 15,
                    title: "SKILL.md shows at least one concrete example".to_owned(),
                    detail: Some("no fenced code block in SKILL.md".to_owned()),
                },
            ],
            checklist,
            shortfalls: Vec::new(),
            needs_override: false,
        }
    }

    fn answered(pairs: &[(&str, &str)], concerns: &[&str]) -> ChecklistView {
        ChecklistView {
            answers: pairs
                .iter()
                .map(|(item, verdict)| ((*item).to_owned(), (*verdict).to_owned()))
                .collect(),
            note: None,
            complete: true,
            concerns: concerns.iter().map(|item| (*item).to_owned()).collect(),
            reviewed_at: Utc::now(),
        }
    }

    fn detail_with(quality: QualityReport) -> Detail {
        Detail {
            summary: summary(ProposalView::Open, "acme/eng", "acme/eng"),
            members: Vec::new(),
            approvals: Vec::new(),
            scan: None,
            quality: Some(quality),
        }
    }

    /// The block a reviewer reads: the score against the pack's bar, the
    /// checks that *failed* and not the ones that passed, and the
    /// checklist rendered as its own thing rather than folded into the
    /// number (ADR-0053 decision 1).
    #[test]
    fn the_quality_block_shows_the_score_the_failures_and_the_checklist_apart() {
        let rendered = render_detail(
            &detail_with(quality(
                85,
                70,
                Some(answered(
                    &[("instructions-correct", "yes"), ("tested", "yes")],
                    &[],
                )),
            )),
            false,
        );
        assert!(rendered.contains("quality  85/100"), "{rendered}");
        assert!(rendered.contains("this pack asks for 70"), "{rendered}");
        // Failures are listed with what they cost; passes are not, because
        // eight lines of "passed" is a block nobody finishes reading.
        assert!(rendered.contains("has-examples"), "{rendered}");
        assert!(rendered.contains("-15"), "{rendered}");
        assert!(
            !rendered.contains("description-states-when"),
            "a passing check must not be listed:\n{rendered}"
        );
        // The two halves are visibly two halves.
        assert!(rendered.contains("checklist  complete"), "{rendered}");
        assert!(rendered.contains("yes  instructions-correct"), "{rendered}");
        // And quality comes after the safety question and before the diff.
        let quality_at = rendered.find("quality  85").unwrap();
        let effect_at = rendered.find("effect on").unwrap();
        assert!(quality_at < effect_at, "{rendered}");
    }

    /// A pack that requires a checklist and has none says so where a
    /// reviewer will act on it, and names the command that fixes it.
    #[test]
    fn a_missing_checklist_names_the_command_that_records_one() {
        let rendered = render_detail(&detail_with(quality(85, 70, None)), false);
        assert!(rendered.contains("NONE recorded"), "{rendered}");
        assert!(
            rendered.contains("synveda proposal checklist"),
            "a refusal a reviewer cannot act on is half a refusal:\n{rendered}"
        );
    }

    /// A written-down `no` is painted as a removal and says what it costs,
    /// because that is the finding a reviewer must not skim past.
    #[test]
    fn a_concern_says_that_publishing_over_it_needs_an_override() {
        let mut report = quality(100, 0, Some(answered(&[("tested", "no")], &["tested"])));
        report.needs_override = true;
        report.shortfalls = vec![Shortfall {
            kind: Some("checklist-concerns".to_owned()),
            detail: Some("a reviewer answered `no` to tested".to_owned()),
        }];
        let rendered = render_detail(&detail_with(report), false);
        assert!(
            rendered.contains("a reviewer objected to tested"),
            "{rendered}"
        );
        assert!(rendered.contains("needs a quality override"), "{rendered}");
        assert!(
            rendered.contains("a reviewer answered `no` to tested"),
            "the shortfall is spelled out, not left as a slug:\n{rendered}"
        );
        // Even at a perfect score and a pack with no bar: a concern
        // refuses under every config (ADR-0053 decision 7).
        assert!(rendered.contains("quality  100/100"), "{rendered}");
        assert!(rendered.contains("this pack sets no bar"), "{rendered}");
    }

    /// ADR-0056 decision 6: the sentence is the gateway's, so a kind this
    /// binary has never heard of needs no local prose to be explained. A
    /// shortfall invented after this CLI was built renders as well as one
    /// invented before it.
    #[test]
    fn a_shortfall_kind_this_binary_never_heard_of_still_explains_itself() {
        let unknown = Shortfall {
            kind: Some("licence-missing".to_owned()),
            detail: Some("no licence file is present and this pack requires one".to_owned()),
        };
        assert_eq!(
            unknown.describe(),
            "no licence file is present and this pack requires one"
        );
    }

    /// The other direction, which is the one that now degrades: a gateway
    /// older than this CLI serves the slug and no sentence. Name it rather
    /// than drop it — an unexplained bar is worse than an unnamed one, and
    /// a refusal a reviewer cannot see is the failure mode worth avoiding.
    #[test]
    fn an_unexplained_shortfall_is_named_rather_than_swallowed() {
        let bare = Shortfall {
            kind: Some("checklist-missing".to_owned()),
            detail: None,
        };
        let described = bare.describe();
        assert!(described.contains("checklist-missing"), "{described}");
        assert!(
            described.contains("older than the CLI"),
            "the skew is named so the reader knows why the sentence is missing: {described}"
        );
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

    // ── The parity corpus (CNSL-1, ADR-0056 decision 7) ─────────────────

    /// Every case in `console/fixtures/`. Kept in step with the list in
    /// `synveda-gateway/tests/console_parity.rs`, which is what records
    /// them; a case added there and not here is a case only one surface
    /// answers.
    const CASES: &[&str] = &[
        "memory-update",
        "memory-drifted",
        "skill-clean",
        "skill-below-bar",
        "skill-checklist-stale",
        "skill-blocking-scan",
        "skill-unknown-severity",
    ];

    fn corpus_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../console/fixtures")
    }

    /// A case recorded over in the gateway and not listed here is a case
    /// this surface never answers, and the list above is a comment asking
    /// somebody to remember. This is the same request, addressed to a test.
    #[test]
    fn every_case_in_the_corpus_is_answered_here() {
        let mut found: Vec<String> = std::fs::read_dir(corpus_dir())
            .expect("read console/fixtures")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".facts.json").map(str::to_owned)
            })
            .collect();
        found.sort();
        let mut listed: Vec<String> = CASES.iter().map(|case| (*case).to_owned()).collect();
        listed.sort();
        assert_eq!(
            found, listed,
            "the corpus on disk and the cases this suite answers have diverged",
        );
    }

    fn corpus(case: &str, extension: &str) -> Value {
        let path = corpus_dir().join(format!("{case}.{extension}"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("{case}.{extension}: {err}"))
    }

    /// Asserts the rendering **names** a fact, with a message that says
    /// which case and which fact rather than dumping a transcript and
    /// leaving the reader to find the missing line.
    fn names(case: &str, rendered: &str, needle: &str, what: &str) {
        assert!(
            rendered.contains(needle),
            "{case}: the review does not name {what} ({needle:?})\n\n{rendered}",
        );
    }

    /// How a member is identifiable in a rendering.
    ///
    /// Derived from the *shape* of the name rather than by calling the
    /// renderer's own `label`, which would make the assertion agree with
    /// whatever the renderer did. An address a reader cannot type is
    /// abbreviated by both surfaces; a path somebody chose is not, because
    /// a name a person typed is the whole point of the name.
    fn identifier(name: &str) -> String {
        let uuid_shaped =
            name.len() == 36 && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        if uuid_shaped {
            short(name)
        } else {
            name.to_owned()
        }
    }

    /// The part of a rendering from one heading to the next.
    ///
    /// Facts are asserted where a reviewer would look for them, which is
    /// not pedantry: a scan finding names the file it was found in, and a
    /// member names the same file, so a search over the whole transcript
    /// would let the scan block satisfy an assertion about the diff and
    /// the review would pass with no members rendered at all.
    fn section<'a>(case: &str, rendered: &'a str, from: &str, to: Option<&str>) -> &'a str {
        let start = rendered
            .find(from)
            .unwrap_or_else(|| panic!("{case}: no {from:?} block\n\n{rendered}"));
        let rest = &rendered[start..];
        to.and_then(|to| rest.find(to))
            .map_or(rest, |end| &rest[..end])
    }

    /// The line of the rendering that mentions `needle`.
    fn line_with<'a>(case: &str, rendered: &'a str, needle: &str) -> &'a str {
        rendered
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{case}: no line mentions {needle:?}\n\n{rendered}"))
    }

    /// **The acceptance criterion, this half of it.**
    ///
    /// CNSL-1 asks for full review parity with the CLI, and the only
    /// version of that a test can fail is one where both surfaces answer
    /// the same corpus. `console/fixtures/<case>.facts.json` says what a
    /// review has to name; this asserts that `synveda proposal review`
    /// names all of it, and the console's suite asserts the same file
    /// against its own rendering.
    ///
    /// Every assertion here is about **naming a fact**, never about
    /// layout: that a blocking finding is distinguishable, not that it is
    /// red; that both quality numbers appear, not the shape of the line
    /// they appear on. ADR-0056 rejected serving a display model precisely
    /// so a terminal and a browser could differ where they should, and a
    /// parity suite that pinned wording would be that display model
    /// arriving through the back door.
    #[test]
    fn the_cli_names_every_fact_the_corpus_requires() {
        for case in CASES {
            let detail: Detail = serde_json::from_value(corpus(case, "json"))
                .unwrap_or_else(|err| panic!("{case} is not a proposal detail: {err}"));
            let facts = corpus(case, "facts.json");
            // No colour: a review piped to a file or read by a screen
            // reader has to carry every fact in its text, and the console
            // has no ANSI to lean on either.
            let rendered = render_detail(&detail, false);

            names(
                case,
                &rendered,
                facts["state"].as_str().expect("state"),
                "the proposal's state",
            );
            names(
                case,
                &rendered,
                facts["outstanding"].as_str().expect("outstanding"),
                "what the requirement still lacks",
            );

            let reviews = section(case, &rendered, "  reviews", Some("\n  effect on "));
            let members = section(case, &rendered, "  effect on ", None);

            for approval in facts["approvals"].as_array().expect("approvals") {
                let subject = approval["subject"].as_str().expect("subject");
                names(case, reviews, subject, "an approver");
                let line = line_with(case, reviews, subject);
                assert!(
                    line.contains(approval["verdict"].as_str().expect("verdict")),
                    "{case}: {subject}'s verdict is not on the line naming them: {line:?}",
                );
                if approval["counts"] == json!(false) {
                    assert!(
                        line.contains("does not count"),
                        "{case}: an approval of an earlier commit must be marked as \
                         not counting, or a reviewer reads a requirement as met that \
                         is not: {line:?}",
                    );
                }
            }

            for member in facts["members"].as_array().expect("members") {
                let name = member["name"].as_str().expect("name");
                // A uuid-shaped name may be abbreviated and a path may not
                // (both surfaces abbreviate an address the reader cannot
                // type); what parity asks is that the member is
                // identifiable, so the assertion is the twelve characters
                // that make it so.
                let identifier = identifier(name);
                names(case, members, &identifier, "a member");
                let line = line_with(case, members, &identifier);
                let label = match member["effect"].as_str().expect("effect") {
                    "add" => "add",
                    "update" => "update",
                    "none" => "same",
                    other => panic!("{case}: unknown effect {other}"),
                };
                assert!(
                    line.contains(label),
                    "{case}: what publishing would do to {name} is not on its line: \
                     {line:?}",
                );
                if member["drifted"] == json!(true) {
                    names(
                        case,
                        members,
                        "publishing will refuse",
                        "that the member drifted under the review",
                    );
                }
                for (field, what) in [
                    ("baseline", "the bytes a publication would overwrite"),
                    ("proposed", "the bytes under review"),
                    ("current", "the member as it stands now"),
                ] {
                    let Some(text) = member[field].as_str() else {
                        continue;
                    };
                    for line in text.lines().filter(|line| !line.trim().is_empty()) {
                        names(case, members, line, what);
                    }
                }
            }

            if let Some(scan) = facts.get("scan") {
                let scan_block =
                    section(case, &rendered, "  security scan", Some("\n  effect on "));
                for finding in scan["findings"].as_array().expect("findings") {
                    let rule = finding["rule"].as_str().expect("rule");
                    names(case, scan_block, rule, "a scan finding");
                    let line = line_with(case, scan_block, rule);
                    for part in [
                        finding["path"].as_str().expect("path").to_owned(),
                        finding["line"].to_string(),
                        finding["severity"].as_str().expect("severity").to_owned(),
                    ] {
                        assert!(
                            line.contains(&part),
                            "{case}: {rule} is missing {part:?} from its line: {line:?}",
                        );
                    }
                    // ADR-0056 decision 5: the gateway's verdict, and a
                    // reader who cannot see colour still has to be able to
                    // tell which findings stop the publication — including
                    // in the case where the severity means nothing to them.
                    assert_eq!(
                        line.contains("blocks"),
                        finding["blocking"] == json!(true),
                        "{case}: {rule} is served blocking={} and its line does not \
                         say so: {line:?}",
                        finding["blocking"],
                    );
                }
                if scan["blocked"] == json!(true) {
                    names(
                        case,
                        scan_block,
                        "REFUSED",
                        "that the pack in force will refuse this bundle",
                    );
                }
            }

            if let Some(quality) = facts.get("quality") {
                // Two numbers, never one (ADR-0053 decision 1).
                names(
                    case,
                    &rendered,
                    &format!("{}/100", quality["score"]),
                    "the rubric score",
                );
                names(
                    case,
                    &rendered,
                    &quality["min_score"].to_string(),
                    "the bar the pack asks for",
                );
                let checklist = match quality["checklist"].as_str().expect("checklist") {
                    "complete" => "complete",
                    "partial" => "PARTIAL",
                    _ if quality["checklist_required"] == json!(true) => "NONE recorded",
                    _ => "none recorded",
                };
                names(case, &rendered, checklist, "the state of the checklist");
                for shortfall in quality["shortfalls"].as_array().expect("shortfalls") {
                    // Verbatim: the sentence is the gateway's, and a surface
                    // that reworded it would be the second author decision 6
                    // exists to prevent.
                    names(
                        case,
                        &rendered,
                        shortfall.as_str().expect("a sentence"),
                        "a bar this bundle misses",
                    );
                }
                if quality["needs_override"] == json!(true) {
                    names(
                        case,
                        &rendered,
                        "quality override",
                        "that publishing needs an override",
                    );
                }
            }
        }
    }
}
