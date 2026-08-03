//! The skills registry API (SKIL-1, ADR-0051): `/v1/skills` behind tenant
//! resolution, uniform-404 ownership, and the PDP (`SkillWrite` to author a
//! draft, `SkillRead` to be served one).
//!
//! Three surfaces, shaped like the prompt registry's and not like the pack
//! registry's, because a skill **is** fetched by name:
//!
//! - **author** (`POST /v1/skills`) — validates the bundle against the open
//!   spec, runs MEM-2's scanner and then SKIL-2's security scanner over every
//!   file, writes the objects and the draft rows, and prunes the files the
//!   request dropped. It moves nothing a consumer installs.
//! - **resolve** (`GET /v1/skills/{name}`) — the consumer's call, and the
//!   one the CLI's `install` is built on. It returns the **whole bundle**
//!   from one commit, which is the difference from a prompt resolve: a
//!   client loads a skill whole, so serving files from two commits would be
//!   serving a version nobody reviewed.
//! - **list** (`GET /v1/skills?scope_id=…`) — the registry view at one
//!   scope.
//!
//! # What this module does not do
//!
//! It never writes a file. Seed §2.6 — the harness is a guest — and
//! ADR-0051 decision 12: a gateway that owned an archive format and a
//! per-client directory layout would need a release when one of forty
//! clients moved a folder. The materialisation is `synveda skill install`,
//! and the receipt lives in the CLI's own state, outside any client's skills
//! root, because a receipt inside the bundle is the modification the
//! acceptance criterion forbids.
//!
//! It also composes nothing. ADR-0049 option 4's third reason for refusing
//! "prompts as memory records" — a prompt is fetched by name where a record
//! is ranked by relevance — was inverted by PRMT-2 for packs and is restored
//! here (decision 9): the client's own progressive disclosure is the loader.
//!
//! # The security gate
//!
//! SKIL-2 (ADR-0052) puts a second scanner at the same authoring seam, and
//! the reason it is *here* rather than only at publication is that a draft is
//! installable: `at_scope`'s draft branch decides `SkillRead` at the scope
//! and not authorship, so anyone the pack lets read skills there could
//! materialise an unreviewed bundle. A gate at the publish seam alone is one
//! a malicious author walks around by never opening a proposal. A refused
//! bundle is therefore never stored at all, which is what makes "cannot reach
//! published" structural rather than procedural.
//!
//! # The consumer's pin
//!
//! `?commit=` is PRMT-1's, inherited whole (ADR-0049 decisions 9–11): a
//! request parameter stored nowhere, checked against what the scope
//! **serves** rather than its head, and refused with a `Conflict` naming
//! both commits when a rewind takes it off the line. For skills it is what
//! makes an install receipt reproducible — and what keeps FLOW-7's sixty
//! seconds true of an asset that lives on laptops.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::{BundleScan, ScanOutcome};
use synveda_policy::{Action, Resource};
use synveda_store::{hierarchy, rls, skills};
use synveda_types::{
    Channel, Error, Frontmatter, HierarchyNode, IdentityId, RedactionMode, Result, ScanSeverity,
    ScopeId, Sensitivity, SkillBundle, SkillChannel, SkillFile, SkillFilePath, SkillName,
    SkillPath, SkillScanConfig,
};
use synveda_vedaflow::{self as vedaflow, ChannelRef, SkillAsset};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::SKILL_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the outcome taxonomy every
/// governed plane uses.
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(SKILL_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

// ── Author ─────────────────────────────────────────────────────────────

/// One file as an author supplies it.
#[derive(Deserialize)]
pub(crate) struct FileBody {
    /// Its path within the bundle. Validated against filesystems rather
    /// than taste (ADR-0051 decision 7) — these bytes become a real file.
    path: SkillFilePath,
    /// Its content, verbatim. What an install writes, byte for byte.
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct AuthorBody {
    /// Where the skill is authored — the scope that will stand behind it,
    /// and the scope whose published channel a proposal would move.
    scope_id: ScopeId,
    /// Its name: one lower-case hyphenated segment, the agentskills.io
    /// grammar, and the directory an install creates (decision 6).
    name: SkillName,
    /// Its classification. Absent means `internal`. Per *skill* rather than
    /// per file (decision 11) — a client loads a bundle whole.
    #[serde(default)]
    sensitivity: Option<Sensitivity>,
    /// The bundle. `SKILL.md` is required and its frontmatter `name` must
    /// equal `name` above, which is the open spec's own rule and this
    /// registry's key at once (decision 5).
    ///
    /// **The request is the bundle** (decision 17): a file the author
    /// dropped is dropped from the draft, unlike a context pack, because a
    /// client loads a skill whole and a leftover file would be published
    /// back onto a laptop by the next proposal.
    files: Vec<FileBody>,
}

/// One security-scan finding, rendered for an API response (SKIL-2,
/// ADR-0052 decision 7).
///
/// The `title` is here and not only the rule id because the reader is a
/// reviewer rather than a machine, and "downloads a remote script and
/// pipes it straight into an interpreter" is what they need to weigh.
/// What is deliberately absent is the matched text.
#[derive(Serialize)]
pub(crate) struct ScanFindingView {
    path: String,
    rule: &'static str,
    /// Typed rather than a string, because these are ordered and
    /// `"critical" < "high" < "notice"` being the right order
    /// alphabetically is a coincidence nothing should depend on.
    severity: ScanSeverity,
    title: &'static str,
    /// 1-based, so it matches what an editor shows.
    line: usize,
    count: usize,
}

/// A bundle's scan as a reviewer or an author reads it.
#[derive(Serialize)]
pub(crate) struct ScanReport {
    /// Which rule table produced this. It moves, and a report that did
    /// not say which one produced it could not be compared with one
    /// taken at review time (ADR-0052 force 4).
    ruleset_version: u32,
    /// The worst severity found, absent when clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    worst: Option<ScanSeverity>,
    /// The severity the pack in force refuses at.
    blocks_at: ScanSeverity,
    /// Whether the pack in force would refuse this bundle. Always
    /// `false` in an author response — a blocked bundle is a refusal,
    /// not a view — and the field that matters in a review, where an
    /// approved-but-blocking bundle is exactly what the publish gate
    /// will stop.
    blocked: bool,
    /// How many findings at each severity.
    counts: BTreeMap<ScanSeverity, usize>,
    /// Every finding, worst first.
    findings: Vec<ScanFindingView>,
}

impl ScanReport {
    pub(crate) fn new(scan: &BundleScan, config: &SkillScanConfig) -> Self {
        let mut findings: Vec<ScanFindingView> = scan
            .files
            .iter()
            .flat_map(|file| {
                file.findings.iter().map(move |finding| ScanFindingView {
                    path: file.path.clone(),
                    rule: finding.rule,
                    severity: finding.severity,
                    title: finding.title,
                    line: finding.line,
                    count: finding.count,
                })
            })
            .collect();
        // Worst first, then a total order so two renders of the same
        // bundle read identically.
        findings.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line.cmp(&b.line))
                .then_with(|| a.rule.cmp(b.rule))
        });
        ScanReport {
            ruleset_version: scan.ruleset_version,
            worst: scan.worst(),
            blocks_at: config.threshold(),
            blocked: scan.blocked_by(config),
            counts: scan.counts(),
            findings,
        }
    }
}

/// What a scope's published channel holds for one file right now.
#[derive(Serialize)]
struct PublishedFile {
    /// The address it names.
    object_hash: String,
    /// Whether that is the draft's own address. `false` after an edit: the
    /// draft has moved and the reviewed version has not, which is what
    /// "behind review" looks like from the writing side.
    current: bool,
}

#[derive(Serialize)]
struct FileView {
    path: String,
    /// The draft's content address — what a proposal would bind.
    object_hash: String,
    /// How many characters it carries. Never the content: the response is
    /// a registry view, and the bytes come back through `resolve`.
    chars: usize,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<PublishedFile>,
}

#[derive(Serialize)]
struct SkillView {
    name: String,
    scope_id: ScopeId,
    scope_path: String,
    description: String,
    sensitivity: Sensitivity,
    /// The frontmatter as the strict subset read it — the whole of it,
    /// including the client keys the product does not interpret, so a
    /// reviewer sees what a client will act on (decision 4).
    frontmatter: Frontmatter,
    files: Vec<FileView>,
    /// How many draft files this request removed, because the bundle it
    /// named did not include them (decision 17).
    removed: u64,
    /// What SKIL-2's security scanner found and admitted (ADR-0052).
    ///
    /// Present on every author, empty findings included, because "the
    /// scan ran and found nothing" and "no scan is reported here" must
    /// not look the same to an author. A blocking finding never reaches
    /// this struct — it is a refusal.
    scan: ScanReport,
    /// The commit the scope's skill channel serves, if any. Authoring never
    /// moves it, which is the whole of "reaches a client only through
    /// review".
    #[serde(skip_serializing_if = "Option::is_none")]
    published_commit: Option<String>,
    created_at: DateTime<Utc>,
    created_by: IdentityId,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
}

/// `POST /v1/skills` — author a skill: create it, or replace it.
#[tracing::instrument(name = "skills.author", skip_all)]
pub(crate) async fn author(
    State(state): State<AppState>,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Response {
    let result = author_inner(&state, payload).await;
    respond(&state, "author", result).await
}

async fn author_inner(
    state: &AppState,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Result<Json<SkillView>> {
    let body = body(payload)?;
    let sensitivity = body.sensitivity.unwrap_or(Sensitivity::WORKING);
    if sensitivity == Sensitivity::Restricted {
        return Err(Error::Invalid {
            message: format!(
                "skill {} cannot be `restricted`: the only path to that tier is a \
                 classification proposal over records, priced at compliance plus two \
                 distinct approvers (ADR-0038 decision 8), and no such path exists for an \
                 authored asset — so nothing could read the bundle back (ADR-0051 \
                 decision 11)",
                body.name
            ),
        });
    }

    // The bundle, validated against the open spec before a transaction is
    // opened: all of it is pure, and a bundle no client would load must not
    // reach a reviewer (decision 5).
    let bundle = SkillBundle {
        name: body.name.clone(),
        files: body
            .files
            .iter()
            .map(|file| SkillFile {
                path: file.path.clone(),
                content: file.content.clone(),
            })
            .collect(),
    };
    let frontmatter = bundle.validate()?;

    let tenant_id = tenant_id()?;

    // ── Decide, and read the effective redaction config ────────────────
    //
    // A read-only transaction: it writes nothing, so dropping it costs
    // nothing, and the scanner below is CPU that should not hold a
    // connection.
    let (node, author, authorized, redaction, scan_config, pack) = {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            hierarchy::node(&mut *tx, body.scope_id).await?,
            tenant_id,
            body.scope_id,
        )?;
        let input = authz::gather(state, &mut tx, Some(&node)).await?;
        let authorized = authz::decide(
            state,
            &input,
            Action::SkillWrite,
            Resource::Scope(body.scope_id),
            None,
        )?;
        let author = identity_of(&input)?;
        let effective =
            state
                .pdp
                .effective(tenant_id, Resource::Scope(body.scope_id), &input.context());
        let pack = format!("{}@{}", effective.name, effective.version);
        (
            node,
            author,
            authorized,
            effective.redaction,
            effective.scan,
            pack,
        )
    };

    // ── Scan, outside any transaction ──────────────────────────────────
    //
    // ADR-0051 decision 14, on ADR-0050 decision 11's ladder: `redact`
    // scrubs and continues, `quarantine` and `deny` refuse to the author.
    // The guarantee is stronger here than it is for a pack only because the
    // destination is: a pack's secret would have reached vector space, and a
    // skill's reaches a laptop.
    let mut assets: Vec<SkillAsset> = Vec::with_capacity(bundle.files.len());
    let mut redacted = 0_usize;
    for file in bundle.files {
        let mut file = file;
        let scan = scan_file(&file).await?;
        match scan.disposition(&redaction) {
            None => {}
            Some(RedactionMode::Redact) => {
                file.content = scrubbed(&scan, &file.content);
                redacted += 1;
            }
            Some(mode) => {
                return refuse_scanned(
                    state,
                    tenant_id,
                    body.scope_id,
                    &body.name,
                    &file,
                    &scan,
                    mode,
                )
                .await;
            }
        }
        assets.push(SkillAsset {
            scope_id: body.scope_id,
            skill: body.name.clone(),
            sensitivity,
            file,
        });
    }
    // A scrub can change `SKILL.md`, so the spec check runs again over what
    // will actually be stored. A redaction that broke the frontmatter must
    // be a refusal here rather than a bundle no client can load.
    let scrubbed_bundle = SkillBundle {
        name: body.name.clone(),
        files: assets.iter().map(|asset| asset.file.clone()).collect(),
    };
    let frontmatter = if redacted > 0 {
        scrubbed_bundle.validate()?
    } else {
        frontmatter
    };

    // ── The security gate, over exactly what would be stored ───────────
    //
    // SKIL-2, ADR-0052 decisions 4 and 5. It runs *after* the redaction
    // pass rather than beside it, on the same discipline that made the
    // spec check run twice: the bundle that matters is the one that will
    // be written, and a scrub can change it. It also runs after MEM-2's
    // ladder, so a bundle carrying both a live credential and a
    // fetch-and-execute is refused for the credential — which is the
    // right order, because the credential is live now and the code is
    // not yet.
    let security = scan_security(&scrubbed_bundle.files).await?;
    if security.blocked_by(&scan_config) {
        return refuse_scan(
            state,
            tenant_id,
            body.scope_id,
            body.name.as_str(),
            &security,
            &scan_config,
            &pack,
            "authoring",
        )
        .await;
    }

    // ── Write, in one transaction ──────────────────────────────────────
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let stored_skill = skills::upsert_skill(
        &mut *tx,
        tenant_id,
        &skills::NewSkill {
            scope_id: body.scope_id,
            name: &body.name,
            description: &frontmatter.description,
            sensitivity,
            author,
        },
    )
    .await?;

    let mut written: Vec<(skills::StoredFile, usize)> = Vec::with_capacity(assets.len());
    for asset in &assets {
        let object = vedaflow::put_skill(&mut tx, tenant_id, asset).await?;
        let stored = skills::upsert_file(
            &mut *tx,
            tenant_id,
            &skills::NewFile {
                scope_id: body.scope_id,
                skill_name: &body.name,
                path: &asset.file.path,
                object_hash: *object.hash.as_bytes(),
                author,
            },
        )
        .await?;
        written.push((stored, asset.file.content.chars().count()));
    }
    // Decision 17: the request is the bundle.
    let keep: Vec<SkillFilePath> = assets.iter().map(|asset| asset.file.path.clone()).collect();
    let removed =
        skills::prune_files(&mut *tx, tenant_id, body.scope_id, &body.name, &keep).await?;

    let published = published_at(&mut tx, tenant_id, body.scope_id).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SkillAuthored,
        Resource::Scope(body.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::SkillWrite, &authorized),
            "asset": synveda_types::AssetKind::Skill.as_str(),
            "skill": body.name.as_str(),
            "sensitivity": sensitivity.as_str(),
            // The addresses and the counts. Never SKILL.md text and never
            // file content — the discipline every plane has followed since
            // AUD-1.
            "files": written
                .iter()
                .map(|(stored, chars)| json!({
                    "path": stored.path.as_str(),
                    "object_hash": vedaflow::hash::ObjectHash::from_bytes(stored.object_hash)
                        .to_hex(),
                    "chars": chars,
                }))
                .collect::<Vec<_>>(),
            "removed": removed,
            "redacted": redacted,
            // What the security scan found and let through (SKIL-2).
            // A clean scan gets no event of its own — ADR-0052
            // decision 8 — but a bundle that was *reported on* and
            // admitted anyway is exactly what an auditor asking "what
            // did we let past" needs, and it rides the event the
            // authoring already chains.
            "scan": scan_payload(&security, &scan_config),
            // What a client would be served *now*, which is the point of the
            // whole surface: authoring moved nothing.
            "published_commit": published.as_ref().map(|(commit, _)| commit.to_hex()),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(view(
        &node,
        stored_skill,
        frontmatter,
        written,
        removed,
        published.as_ref(),
        ScanReport::new(&security, &scan_config),
    )))
}

/// Runs MEM-2's scanner over one bundled file (ADR-0051 decision 14).
///
/// The file goes in as a JSON object because that is the shape
/// `synveda_ingest::scan` walks, and it is the *same* scanner the observe
/// and pack-authoring paths use rather than a third one with its own rule
/// list.
///
/// CPU work, O(file bytes), so it goes off the reactor exactly as the others
/// do.
async fn scan_file(file: &SkillFile) -> Result<ScanOutcome> {
    let payload = json!({
        "path": file.path.as_str(),
        "content": file.content,
    });
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::scan(payload)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("redaction scan task failed: {err}"),
    })
}

/// The file's text with every finding scrubbed, as MEM-2's scanner rewrote
/// it.
fn scrubbed(scan: &ScanOutcome, original: &str) -> String {
    scan.payload
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| original.to_owned())
}

/// The refusal a scanned file gets, and the event that chains it.
///
/// ADR-0050 decision 11's departure, inherited: a synchronous authoring
/// surface reviews by refusing to its author, because there is somebody on
/// the other end of the request who can fix the file. What is not a
/// departure is the guarantee — the bundle is not stored, so no secret
/// reaches a client's disk.
async fn refuse_scanned<T>(
    state: &AppState,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
    skill: &SkillName,
    file: &SkillFile,
    scan: &ScanOutcome,
    mode: RedactionMode,
) -> Result<T> {
    let rules: Vec<&str> = scan.findings.iter().map(|finding| finding.rule).collect();
    if mode == RedactionMode::Quarantine {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::SkillQuarantined,
            Resource::Scope(scope_id).to_string(),
            // The scan stopped the write, so the operation did not complete
            // — `failure` rather than `deny`, which is the PDP's word and no
            // PDP denied anything here.
            Outcome::Failure,
            json!({
                "asset": synveda_types::AssetKind::Skill.as_str(),
                "skill": skill.as_str(),
                "path": file.path.as_str(),
                // The rules that fired and how often — never the matched
                // text.
                "findings": scan
                    .findings
                    .iter()
                    .map(|finding| json!({
                        "rule": finding.rule,
                        "category": finding.category.as_str(),
                        "count": finding.count,
                    }))
                    .collect::<Vec<_>>(),
                "disposition": mode.as_str(),
            }),
        )
        .await?;
        commit(tx).await?;
    }
    Err(Error::Invalid {
        message: format!(
            "{} was stopped by the redaction scanner ({}): {}. The bundle was not stored — \
             a skill is bulk external text that becomes files on a fleet of laptops, so the \
             scan runs before anything is written and no secret reaches a client's disk \
             (ADR-0051 decision 14). Remove the finding and author again",
            file.path,
            mode.as_str(),
            rules.join(", "),
        ),
    })
}

/// Runs SKIL-2's security scanner over the bundle (ADR-0052 decision 2:
/// every file, `SKILL.md` included).
///
/// CPU work bounded by ADR-0051's own bundle limits, so it goes off the
/// reactor exactly as MEM-2's sibling does.
pub(crate) async fn scan_security(files: &[SkillFile]) -> Result<BundleScan> {
    let files = files.to_vec();
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        synveda_ingest::scan_bundle(&files)
    })
    .await
    .map_err(|err| Error::Internal {
        message: format!("skill security scan task failed: {err}"),
    })
}

/// A scan rendered for an audit payload or a response.
///
/// Rule ids, severities, counts and 1-based lines — **never file content
/// and never the matched span**, which for a credential rule is a path to
/// a credential (ADR-0052 decision 7).
fn scan_payload(scan: &BundleScan, config: &SkillScanConfig) -> serde_json::Value {
    json!({
        "ruleset_version": scan.ruleset_version,
        "worst": scan.worst().map(|worst| worst.as_str()),
        "blocks_at": config.threshold().as_str(),
        "findings": scan
            .files
            .iter()
            .flat_map(|file| file.findings.iter().map(move |finding| json!({
                "path": file.path,
                "rule": finding.rule,
                "severity": finding.severity.as_str(),
                "line": finding.line,
                "count": finding.count,
            })))
            .collect::<Vec<_>>(),
    })
}

/// The refusal a scanned bundle gets, and the event that chains it.
///
/// One helper for both seams (ADR-0052 decisions 4 and 5) because the
/// refusal is the same act at either: `stage` is the only thing that
/// differs, and it is on the event so an auditor can tell an author who
/// was stopped from a proposal that was.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refuse_scan<T>(
    state: &AppState,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
    skill: &str,
    scan: &BundleScan,
    config: &SkillScanConfig,
    pack: &str,
    stage: &'static str,
) -> Result<T> {
    let blocking = scan.blocking(config);
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SkillScanRejected,
        Resource::Scope(scope_id).to_string(),
        // The scan stopped the write; no PDP denied anything, so
        // `failure` rather than `deny` — MEM-2's distinction at the
        // sibling seam, unchanged.
        Outcome::Failure,
        json!({
            "asset": synveda_types::AssetKind::Skill.as_str(),
            "skill": skill,
            "stage": stage,
            "policy_pack": pack,
            "scan": scan_payload(scan, config),
        }),
    )
    .await?;
    commit(tx).await?;

    let named = blocking
        .iter()
        .map(|(path, finding)| {
            format!(
                "{path}:{} {} ({})",
                finding.line, finding.rule, finding.severity
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let worst = blocking
        .first()
        .map_or("critical".to_owned(), |(_, finding)| {
            finding.severity.to_string()
        });
    Err(Error::Invalid {
        message: format!(
            "skill {skill} was refused by the security scanning gate at {worst}: {named}. \
             {}. The bundle was not stored — a skill becomes files a client executes, so a \
             finding this severe is refused before anything is written rather than left for \
             a reviewer to catch (SKIL-2, ADR-0052). Remove the finding and author again",
            blocking
                .first()
                .map_or_else(String::new, |(_, finding)| finding.title.to_owned()),
        ),
    })
}

// ── Resolve ────────────────────────────────────────────────────────────

/// Where a served bundle came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Origin {
    /// A scope's draft row: unreviewed, and only ever served to a caller who
    /// named that scope and that channel (ADR-0049 decision 15).
    Draft,
    /// The channel's head.
    Head,
    /// A standing FLOW-7 pin at the scope — its readers are held at an
    /// earlier state, and that is the ceiling a consumer pin may reach.
    ChannelPin,
    /// The commit the caller asked for.
    PinnedCommit,
}

#[derive(Deserialize)]
pub(crate) struct ResolveParams {
    /// Which scope's copy. Absent walks the caller's own placement chain.
    #[serde(default)]
    scope_id: Option<ScopeId>,
    /// `published` (default) or `draft`.
    #[serde(default)]
    channel: Option<SkillChannel>,
    /// The commit a consumer was built against (ADR-0049 decision 9).
    #[serde(default)]
    commit: Option<String>,
}

#[derive(Serialize)]
struct ResolvedFile {
    path: String,
    /// The content address — what an install re-hashes what it wrote
    /// against, which is what makes "installs unmodified" a measurement.
    object_hash: String,
    /// The bytes, verbatim. Nothing added, nothing wrapped: this is the
    /// whole of ADR-0051 force 1.
    content: String,
}

#[derive(Serialize)]
struct ResolveResponse {
    name: String,
    scope_id: ScopeId,
    scope_path: String,
    channel: SkillChannel,
    origin: Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    sensitivity: Sensitivity,
    description: String,
    frontmatter: Frontmatter,
    /// Every file of the bundle, in path order — the order an install writes
    /// them in, and all of them from **one** commit.
    files: Vec<ResolvedFile>,
}

/// `GET /v1/skills/{name}` — resolve a bundle.
#[tracing::instrument(name = "skills.resolve", skip_all)]
pub(crate) async fn resolve(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<ResolveParams>,
) -> Response {
    let result = resolve_inner(&state, &name, &params).await;
    respond(&state, "resolve", result).await
}

async fn resolve_inner(
    state: &AppState,
    raw_name: &str,
    params: &ResolveParams,
) -> Result<Json<ResolveResponse>> {
    let name: SkillName = raw_name.parse()?;
    let channel = params.channel.unwrap_or(SkillChannel::Published);
    let pinned = params
        .commit
        .as_deref()
        .map(str::parse::<vedaflow::CommitHash>)
        .transpose()?;
    if channel == SkillChannel::Draft && pinned.is_some() {
        return Err(Error::Invalid {
            message: "a draft is on no channel, so there is no commit to pin it to".to_owned(),
        });
    }
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;

    let resolved = match (params.scope_id, channel) {
        (None, SkillChannel::Draft) => {
            return Err(Error::Invalid {
                message: "reading a draft names its scope: unreviewed content reaches a \
                          caller who asked for that scope's unreviewed content, and nobody \
                          else (ADR-0049 decision 15)"
                    .to_owned(),
            });
        }
        (None, SkillChannel::Published) => {
            if pinned.is_some() {
                return Err(Error::Invalid {
                    message: "pinning a commit names its scope too — a commit belongs to \
                              one scope's channel, and the resolve response carries the \
                              pair to pin with"
                        .to_owned(),
                });
            }
            walk_chain(state, &mut tx, tenant_id, &name).await?
        }
        (Some(scope_id), channel) => {
            at_scope(state, &mut tx, tenant_id, scope_id, &name, channel, pinned).await?
        }
    };

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SkillResolved,
        Resource::Scope(resolved.node.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::SkillRead, &resolved.authorized),
            "asset": synveda_types::AssetKind::Skill.as_str(),
            "skill": name.as_str(),
            "channel": channel.as_str(),
            "origin": resolved.origin,
            "sensitivity": resolved.sensitivity.as_str(),
            // The addresses and the commit — the citation an install
            // records and an auditor recomputes. This event *is* the
            // provenance of what is about to become files on a machine,
            // because nothing inside the installed directory can carry a
            // watermark (ADR-0051 force 2).
            "commit": resolved.commit.map(|commit| commit.to_hex()),
            "files": resolved
                .files
                .iter()
                .map(|(path, address, _)| json!({
                    "path": path.as_str(),
                    "object_hash": address.to_hex(),
                }))
                .collect::<Vec<_>>(),
            "chain_position": resolved.position,
        }),
    )
    .await?;
    commit(tx).await?;

    let frontmatter = resolved.frontmatter()?;
    Ok(Json(ResolveResponse {
        name: name.to_string(),
        scope_id: resolved.node.id,
        scope_path: resolved.node.path.clone(),
        channel,
        origin: resolved.origin,
        commit: resolved.commit.map(|commit| commit.to_hex()),
        sensitivity: resolved.sensitivity,
        description: frontmatter.description.clone(),
        frontmatter,
        files: resolved
            .files
            .iter()
            .map(|(path, address, content)| ResolvedFile {
                path: path.to_string(),
                object_hash: address.to_hex(),
                content: content.clone(),
            })
            .collect(),
    }))
}

/// What a resolution found, before it is rendered or audited.
struct Resolved {
    node: HierarchyNode,
    sensitivity: Sensitivity,
    /// Every file of the bundle in path order, with its address and bytes.
    files: Vec<(SkillFilePath, vedaflow::hash::ObjectHash, String)>,
    commit: Option<vedaflow::CommitHash>,
    origin: Origin,
    /// Distance up the caller's chain — 0 at home.
    position: usize,
    authorized: crate::authz::Authorized,
}

impl Resolved {
    /// The bundle's frontmatter, parsed from the `SKILL.md` that was served.
    ///
    /// A resolution that got this far has a `SKILL.md` — nothing without one
    /// can be authored — so its absence is drift rather than a caller error.
    fn frontmatter(&self) -> Result<Frontmatter> {
        let manifest = self
            .files
            .iter()
            .find(|(path, _, _)| path.is_manifest())
            .ok_or_else(|| Error::Internal {
                message: format!(
                    "the bundle served from {} has no {}, which authoring cannot produce",
                    self.node.path,
                    synveda_types::SKILL_MANIFEST
                ),
            })?;
        Frontmatter::parse(&manifest.2)
    }
}

/// The gradient walk (ADR-0049 decision 8): the caller's own placement
/// chain, nearest first, serving the first scope that publishes the name
/// **and** permits the read.
///
/// For skills the gradient has a physical form: the name is the installed
/// directory name and a client's skills root is flat, so a team's
/// `code-review` and the org's cannot both exist on disk. This walk is what
/// decides which one does.
async fn walk_chain(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    name: &SkillName,
) -> Result<Resolved> {
    let input = authz::gather_at_home(state, tx).await?;
    let chain: Vec<HierarchyNode> = input.chain.to_vec();
    let scope_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let published =
        vedaflow::read_skill_members(tx, tenant_id, &scope_ids, Channel::Published).await?;

    for (position, node) in chain.iter().enumerate() {
        let Some(state_at) = published.iter().find(|state| state.scope_id == node.id) else {
            continue;
        };
        let members = state_at.bundle(name);
        if members.is_empty() {
            continue;
        }
        let Some((sensitivity, files, authorized)) =
            admit(state, tx, tenant_id, &input, position, node, &members).await?
        else {
            continue;
        };
        return Ok(Resolved {
            node: node.clone(),
            sensitivity,
            files,
            commit: Some(state_at.commit),
            origin: if state_at.pinned {
                Origin::ChannelPin
            } else {
                Origin::Head
            },
            position,
            authorized,
        });
    }
    Err(not_found(name))
}

/// A resolve that named its scope: the draft rows, the channel head, or the
/// commit the caller pinned.
async fn at_scope(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
    name: &SkillName,
    channel: SkillChannel,
    pinned: Option<vedaflow::CommitHash>,
) -> Result<Resolved> {
    let node = found(
        hierarchy::node(&mut *tx, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(state, tx, Some(&node)).await?;

    if channel == SkillChannel::Draft {
        let Some(draft) = skills::skill(&mut *tx, tenant_id, scope_id, name).await? else {
            return Err(not_found(name));
        };
        let Some(authorized) = permit(state, &input, scope_id, draft.sensitivity)? else {
            return Err(not_found(name));
        };
        let members: HashMap<SkillFilePath, vedaflow::hash::ObjectHash> =
            skills::files_of(&mut *tx, tenant_id, scope_id, name)
                .await?
                .into_iter()
                .map(|file| {
                    (
                        file.path,
                        vedaflow::hash::ObjectHash::from_bytes(file.object_hash),
                    )
                })
                .collect();
        if members.is_empty() {
            return Err(not_found(name));
        }
        // The draft row's tier is the authoritative one here — the objects
        // carry it too, and they agree by construction.
        let (_, files) = read_bundle(tx, tenant_id, &node, &members).await?;
        return Ok(Resolved {
            node,
            sensitivity: draft.sensitivity,
            files,
            commit: None,
            origin: Origin::Draft,
            position: 0,
            authorized,
        });
    }

    // Published, at a named scope. The pin's refusal is a `Conflict` and
    // every other outcome is the uniform `NotFound`, so this working-tier
    // decision comes first: it tells nothing about the channel to a caller
    // who could not read skills here anyway.
    if permit(state, &input, scope_id, Sensitivity::WORKING)?.is_none() {
        return Err(not_found(name));
    }
    let channel_ref = ChannelRef::skill(Channel::Published);
    let head = vedaflow::read_ref(&mut *tx, tenant_id, scope_id, &channel_ref.name())
        .await?
        .ok_or_else(|| not_found(name))?;
    // What this scope actually serves: its head, unless a standing FLOW-7
    // pin is holding its readers at an earlier state (ADR-0036 decision 6).
    let standing = vedaflow::read_pin(&mut *tx, tenant_id, scope_id, channel_ref).await?;
    let served = standing.as_ref().map_or(head.commit_hash, |pin| pin.commit);

    let (commit_hash, origin) = match pinned {
        None => (
            served,
            standing
                .as_ref()
                .map_or(Origin::Head, |_| Origin::ChannelPin),
        ),
        Some(wanted) => {
            // ADR-0049 decision 10, inherited whole — measured against what
            // the scope **serves**, so a scope's own hold is the ceiling a
            // consumer pin may reach at or below and never over.
            if !vedaflow::is_first_parent_ancestor(&mut *tx, tenant_id, wanted, served).await? {
                return Err(Error::Conflict {
                    message: format!(
                        "{} is not a state {} at this scope has held; it now serves {}. \
                         A rewind withdrew that version, and serving it anyway would make \
                         a rollback partial (FLOW-7); re-resolve to take the current one \
                         deliberately",
                        wanted.to_hex(),
                        channel_ref,
                        served.to_hex(),
                    ),
                });
            }
            (wanted, Origin::PinnedCommit)
        }
    };

    let members = members_at(&mut *tx, tenant_id, commit_hash, name).await?;
    if members.is_empty() {
        return Err(not_found(name));
    }
    let Some((sensitivity, files, authorized)) =
        admit(state, tx, tenant_id, &input, 0, &node, &members).await?
    else {
        return Err(not_found(name));
    };
    Ok(Resolved {
        node,
        sensitivity,
        files,
        commit: Some(commit_hash),
        origin,
        position: 0,
        authorized,
    })
}

/// Reads a bundle's objects and decides `SkillRead` at the tier it carries.
///
/// `None` means the decision denied — the caller skips or answers
/// `NotFound`, never a policy error, so the two are indistinguishable.
///
/// The tier comes from the bundle rather than from a file, and the check
/// that they agree is not decoration: a bundle whose files disagreed would
/// be one a client loads whole under two different classifications, which
/// ADR-0051 decision 11 exists to prevent.
async fn admit(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    input: &DecisionInput,
    position: usize,
    node: &HierarchyNode,
    members: &HashMap<SkillFilePath, vedaflow::hash::ObjectHash>,
) -> Result<
    Option<(
        Sensitivity,
        Vec<(SkillFilePath, vedaflow::hash::ObjectHash, String)>,
        crate::authz::Authorized,
    )>,
> {
    let (sensitivity, files) = read_bundle(tx, tenant_id, node, members).await?;
    let authorized = authz::decide_skill_read_from(
        state,
        input,
        position,
        Resource::Scope(node.id),
        sensitivity,
    );
    match authorized {
        Ok(authorized) => Ok(Some((sensitivity, files, authorized))),
        Err(Error::PolicyDenied { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// A bundle's tier and its files in path order, with their addresses and
/// bytes — one object read each, because a resolve is what `install` calls
/// and the envelope carries the tier beside the content.
///
/// The tier is the *bundle's* (ADR-0051 decision 11), and taking the maximum
/// is a clamp rather than a refusal: authoring writes one tier onto every
/// file, so a commit whose files disagree was not built by this product, and
/// the safe reading of two tiers is the higher one.
async fn read_bundle(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    node: &HierarchyNode,
    members: &HashMap<SkillFilePath, vedaflow::hash::ObjectHash>,
) -> Result<(
    Sensitivity,
    Vec<(SkillFilePath, vedaflow::hash::ObjectHash, String)>,
)> {
    // Sorted, because an install writes in this order and a byte-identity
    // check compares two listings.
    let ordered: BTreeMap<&SkillFilePath, &vedaflow::hash::ObjectHash> = members.iter().collect();
    let mut files = Vec::with_capacity(ordered.len());
    let mut sensitivity: Option<Sensitivity> = None;
    for (path, address) in ordered {
        let object = vedaflow::read_object(&mut *tx, tenant_id, *address)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!(
                    "{} names object {} which the append-only store does not hold",
                    node.path,
                    address.to_hex()
                ),
            })?;
        let asset = SkillAsset::from_bytes(&object.content)?;
        sensitivity = Some(match sensitivity {
            Some(held) => held.max(asset.sensitivity),
            None => asset.sensitivity,
        });
        files.push((path.clone(), *address, asset.file.content));
    }
    let sensitivity = sensitivity.ok_or_else(|| Error::Internal {
        message: format!("{} names a skill with no files", node.path),
    })?;
    Ok((sensitivity, files))
}

/// One `SkillRead` decision at `scope_id`, as an option rather than an
/// error: the resolve surface turns every denial into the uniform
/// `NotFound`.
fn permit(
    state: &AppState,
    input: &DecisionInput,
    scope_id: ScopeId,
    sensitivity: Sensitivity,
) -> Result<Option<crate::authz::Authorized>> {
    match authz::decide_skill_read(state, input, Resource::Scope(scope_id), sensitivity) {
        Ok(authorized) => Ok(Some(authorized)),
        Err(Error::PolicyDenied { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// The one answer a resolve gives for absent, unpublished, and denied alike
/// (ADR-0041's rule for handles, applied to names).
fn not_found(name: &SkillName) -> Error {
    Error::NotFound {
        entity: format!("skill {name}"),
    }
}

/// The addresses a commit's tree names for one skill.
async fn members_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    commit_hash: vedaflow::CommitHash,
    name: &SkillName,
) -> Result<HashMap<SkillFilePath, vedaflow::hash::ObjectHash>> {
    let Some(stored) = vedaflow::read_commit(&mut *tx, tenant_id, commit_hash).await? else {
        return Ok(HashMap::new());
    };
    // The store is append-only, so a commit's tree always resolves; a miss
    // would be corruption rather than an absent member.
    let tree = vedaflow::read_tree(&mut *tx, tenant_id, stored.tree)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!(
                "commit {} names tree {} which the append-only store does not hold",
                commit_hash.to_hex(),
                stored.tree.to_hex()
            ),
        })?;
    let mut members = HashMap::new();
    for entry in tree {
        let Ok(path) = entry.name.parse::<SkillPath>() else {
            continue;
        };
        if let (true, vedaflow::TreeTarget::Object(hash)) = (&path.skill == name, entry.target) {
            members.insert(path.file, hash);
        }
    }
    Ok(members)
}

// ── List ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// The scope whose registry to list. Required: a listing is a scope's
    /// own shelf.
    scope_id: ScopeId,
}

#[derive(Serialize)]
struct ListEntry {
    name: String,
    description: String,
    sensitivity: Sensitivity,
    files: Vec<FileView>,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
}

#[derive(Serialize)]
struct ListResponse {
    scope_id: ScopeId,
    scope_path: String,
    skills: Vec<ListEntry>,
}

/// `GET /v1/skills?scope_id=…` — the registry at one scope.
///
/// Skills the caller may not read at their tier are omitted rather than
/// refused, for the reason a pack listing omits documents: a listing that
/// refused wholesale would make one `confidential` bundle hide the rest.
#[tracing::instrument(name = "skills.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = list_inner(&state, params.scope_id).await;
    respond(&state, "list", result).await
}

async fn list_inner(state: &AppState, scope_id: ScopeId) -> Result<Json<ListResponse>> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        hierarchy::node(&mut *tx, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
    // The gate first, at the working tier — the question a listing asks.
    let authorized = authz::decide_skill_read(
        state,
        &input,
        Resource::Scope(scope_id),
        Sensitivity::WORKING,
    )?;
    let bundles = skills::list_skills(&mut *tx, tenant_id, scope_id).await?;
    let files = skills::list_all_files(&mut *tx, tenant_id, scope_id).await?;
    // Then one decision per tier the shelf actually carries — at most three,
    // and usually one (the `retrieval::plan` shape, ADR-0038 decision 3).
    let mut permitted: HashMap<Sensitivity, bool> = HashMap::new();
    for bundle in &bundles {
        if let std::collections::hash_map::Entry::Vacant(slot) = permitted.entry(bundle.sensitivity)
        {
            slot.insert(permit(state, &input, scope_id, bundle.sensitivity)?.is_some());
        }
    }
    let published = published_at(&mut tx, tenant_id, scope_id).await?;

    let entries: Vec<ListEntry> = bundles
        .into_iter()
        .filter(|bundle| permitted.get(&bundle.sensitivity).copied().unwrap_or(false))
        .map(|bundle| ListEntry {
            files: files
                .iter()
                .filter(|file| file.skill_name == bundle.name)
                .map(|file| file_view(file, 0, published.as_ref()))
                .collect(),
            name: bundle.name.to_string(),
            description: bundle.description,
            sensitivity: bundle.sensitivity,
            updated_at: bundle.updated_at,
            updated_by: bundle.updated_by,
        })
        .collect();

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Scope(scope_id).to_string(),
        Outcome::Allow,
        json!({
            "authz": audit::decision_context(Action::SkillRead, &authorized),
            "op": "skills.list",
            "skills": entries.len(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ListResponse {
        scope_id,
        scope_path: node.path.clone(),
        skills: entries,
    }))
}

// ── Shared ─────────────────────────────────────────────────────────────

type PublishedState = (
    vedaflow::CommitHash,
    HashMap<SkillPath, vedaflow::hash::ObjectHash>,
);

/// What `scope`'s published skill channel holds: the commit it serves and
/// the address it names for every skill path.
async fn published_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
) -> Result<Option<PublishedState>> {
    Ok(
        vedaflow::read_skill_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|state| (state.commit, state.members)),
    )
}

fn file_view(
    file: &skills::StoredFile,
    chars: usize,
    published: Option<&PublishedState>,
) -> FileView {
    let address = vedaflow::hash::ObjectHash::from_bytes(file.object_hash);
    let path = SkillPath::new(file.skill_name.clone(), file.path.clone());
    FileView {
        path: file.path.to_string(),
        object_hash: address.to_hex(),
        chars,
        updated_at: file.updated_at,
        updated_by: file.updated_by,
        published: published.and_then(|(commit_hash, members)| {
            let _ = commit_hash;
            members.get(&path).map(|hash| PublishedFile {
                object_hash: hash.to_hex(),
                current: *hash == address,
            })
        }),
    }
}

fn view(
    node: &HierarchyNode,
    skill: skills::StoredSkill,
    frontmatter: Frontmatter,
    written: Vec<(skills::StoredFile, usize)>,
    removed: u64,
    published: Option<&PublishedState>,
    scan: ScanReport,
) -> SkillView {
    SkillView {
        name: skill.name.to_string(),
        scope_id: skill.scope_id,
        scope_path: node.path.clone(),
        description: skill.description,
        sensitivity: skill.sensitivity,
        frontmatter,
        files: written
            .iter()
            .map(|(file, chars)| file_view(file, *chars, published))
            .collect(),
        removed,
        scan,
        published_commit: published.map(|(commit_hash, _)| commit_hash.to_hex()),
        created_at: skill.created_at,
        created_by: skill.created_by,
        updated_at: skill.updated_at,
        updated_by: skill.updated_by,
    }
}

/// The authoring identity. A verified subject with no identity row cannot
/// reach here — every pack requires either a binding or placement — but the
/// check is explicit rather than an unwrap.
fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "authoring a skill requires a provisioned identity".to_owned(),
        })
}
