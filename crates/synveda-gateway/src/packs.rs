//! The context-pack registry API (PRMT-2, ADR-0050): `/v1/context-packs`
//! behind tenant resolution, uniform-404 ownership, and the PDP
//! (`ContextPackWrite` to author, `ContextPackRead` to see a shelf).
//!
//! Authoring stores scanned immutable `context_pack_chunks`; VedaFlow
//! publication selects which version is current. A ContextRun resolves those
//! published chunks as separately authorised authored input, outside the
//! Knowledge semantic index.
//!
//! - **author** (`POST /v1/context-packs`) — it scans and chunks a bundle in
//!   one request. It moves nothing a session reads, which is the whole of "a
//!   pack reaches a session only through review": the published channel is
//!   somewhere else, and only the approval matrix moves it.
//! - **list** (`GET /v1/context-packs?scope_id=…`) — the registry view at
//!   one scope: what is drafted, what is published, and whether they are
//!   the same bytes.
//!
//! # Re-authoring an unchanged document costs nothing
//!
//! The chunker is deterministic and the document address covers exactly
//! what a reviewer consents to, so identical bytes produce the same address
//! and the same chunks. This route looks the address up first and skips the
//! scan and chunking for every document that has not moved.

use std::collections::HashMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::ScanOutcome;
use synveda_policy::{Action, Resource};
use synveda_store::{packs, rls, scopes};
use synveda_types::scope::Scope;
use synveda_types::{
    Channel, ContextPackChunkId, ContextPackName, DocumentChunk, DocumentName, DocumentPath, Error,
    IdentityId, MAX_PACK_DESCRIPTION_CHARS, MAX_PACK_DOCUMENTS, PackDocument, RedactionMode,
    Result, ScopeId, Sensitivity,
};
use synveda_vedaflow::{self as vedaflow, ContextPackAsset};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::CONTEXT_PACK_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the outcome taxonomy
/// every governed plane uses.
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(CONTEXT_PACK_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    crate::response::finish(state, op, result).await
}

// ── Author ─────────────────────────────────────────────────────────────

/// One document as an author supplies it.
#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = ContextPackDocumentBody)]
pub(crate) struct DocumentBody {
    /// Its name within the pack: path-shaped, so a bundle can carry
    /// `runbooks/payments.md` rather than flattening a directory.
    #[schema(value_type = String)]
    name: DocumentName,
    /// One line, read in a listing, at review, and in the index tier
    /// (ADR-0050 decision 10).
    #[serde(default)]
    title: String,
    /// The text.
    content: String,
    /// Its classification. Absent means `internal`. Per document rather
    /// than per pack (decision 12) — a glossary of public terms and an
    /// internal runbook are plausibly the same bundle.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    sensitivity: Option<Sensitivity>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = ContextPackAuthorBody)]
pub(crate) struct AuthorBody {
    /// Where the pack is authored — the scope that will stand behind it,
    /// and the scope whose published channel a proposal would move.
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// Its name: one segment, lower-case, and the identifier a scope's
    /// override is expressed in (ADR-0050 decision 1).
    #[schema(value_type = String)]
    name: ContextPackName,
    /// One line, read in a listing and at review.
    #[serde(default)]
    description: String,
    /// The documents. A request naming none writes the bundle row alone,
    /// which is how an empty pack gets created before anything is put in
    /// it.
    #[serde(default)]
    documents: Vec<DocumentBody>,
}

/// What a scope's published channel holds for one document right now.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ContextPackPublishedView)]
pub(crate) struct PublishedView {
    /// The commit the channel serves.
    commit: String,
    /// The address it names for this document.
    object_hash: String,
    /// Whether that is the draft's own address. `false` after an edit: the
    /// draft has moved and the reviewed version has not, which is what
    /// "behind review" looks like from the writing side — and, for a pack,
    /// is also exactly when the old version's chunks keep composing
    /// (decision 3).
    current: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ContextPackDocumentView)]
pub(crate) struct DocumentView {
    name: String,
    title: String,
    #[schema(value_type = String)]
    sensitivity: Sensitivity,
    /// The draft's content address — what a proposal would bind.
    object_hash: String,
    /// How many chunks it cut into.
    chunks: u32,
    /// How many immutable chunk rows this request wrote. Zero means the
    /// exact document version already existed.
    written_chunks: u32,
    updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    updated_by: IdentityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<PublishedView>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ContextPackView)]
pub(crate) struct PackView {
    name: String,
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    scope_path: String,
    description: String,
    documents: Vec<DocumentView>,
    created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    created_by: IdentityId,
    updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    updated_by: IdentityId,
}

/// `POST /v1/context-packs` — author a pack: create it, or replace the
/// documents named in the request.
///
/// An overwrite is the authoring act rather than a conflict; what cannot
/// change is the pack's identity, which migration 0030's triggers enforce
/// below this handler. Documents *not* named in the request are left
/// alone — a bundle is edited a file at a time, and a request that dropped
/// the rest would make every save a full re-upload.
#[utoipa::path(
    post,
    path = "/v1/context-packs",
    operation_id = "author_context_pack",
    tag = "context-packs",
    request_body = AuthorBody,
    responses(
        (status = 200, description = "The authored context-pack draft", body = PackView),
        (status = 400, description = "The pack or document content is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Context-pack authoring is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The governing scope is absent", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "context_packs.author", skip_all)]
pub(crate) async fn author(
    State(state): State<AppState>,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Response {
    let result = author_inner(&state, payload).await;
    respond(&state, "author", result).await
}

/// One document's work, as the authoring pipeline computes it before
/// anything is written.
struct Prepared {
    asset: ContextPackAsset,
    /// The chunks to write, empty when the document has not moved.
    chunks: Vec<DocumentChunk>,
    /// How many chunks this document cuts into in total, moved or not.
    total: u32,
}

async fn author_inner(
    state: &AppState,
    payload: std::result::Result<Json<AuthorBody>, JsonRejection>,
) -> Result<Json<PackView>> {
    let body = body(payload)?;
    if body.description.chars().count() > MAX_PACK_DESCRIPTION_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a context pack description must be at most {MAX_PACK_DESCRIPTION_CHARS} characters"
            ),
        });
    }
    if body.documents.len() > MAX_PACK_DOCUMENTS {
        return Err(Error::Invalid {
            message: format!(
                "a context pack may hold at most {MAX_PACK_DOCUMENTS} documents, and this \
                 request names {}; split it into two packs",
                body.documents.len()
            ),
        });
    }
    // The assets, validated and addressed, before a transaction is opened:
    // all of it is pure, and the address is what the next step looks up.
    let mut assets: Vec<ContextPackAsset> = Vec::with_capacity(body.documents.len());
    for document in &body.documents {
        let sensitivity = document.sensitivity.unwrap_or(Sensitivity::WORKING);
        if sensitivity == Sensitivity::Restricted {
            return Err(Error::Invalid {
                message: format!(
                    "document {} cannot be `restricted`: the only path to that tier is a \
                     classification proposal over records, priced at compliance plus two \
                     distinct approvers (ADR-0038 decision 8), and no such path exists for \
                     an authored asset — so nothing could read the document back \
                     (ADR-0050 decision 12)",
                    document.name
                ),
            });
        }
        let content = PackDocument {
            name: document.name.clone(),
            title: document.title.clone(),
            content: document.content.clone(),
        };
        content.validate()?;
        assets.push(ContextPackAsset {
            scope_id: body.scope_id,
            pack: body.name.clone(),
            sensitivity,
            document: content,
        });
    }
    if let Some(duplicate) = first_duplicate(&assets) {
        return Err(Error::Invalid {
            message: format!(
                "document {duplicate} is named twice in one request; the second would \
                 silently win"
            ),
        });
    }

    let tenant_id = tenant_id()?;

    // ── Decide, and find out what has actually moved ───────────────────
    //
    // A read-only transaction: it writes nothing, so dropping it costs
    // nothing, and holding one open across the embedder's network call
    // would pin a connection for the length of a model server's worst day.
    let (node, author, authorized, redaction, unmoved) = {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            scopes::get(&mut *tx, tenant_id, body.scope_id).await?,
            tenant_id,
            body.scope_id,
        )?;
        let input = authz::gather(
            state,
            &mut tx,
            Some(&node),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            state,
            &input,
            Action::ContextPackWrite,
            Resource::Scope(body.scope_id),
        )?;
        let author = identity_of(&input)?;
        // The authoring scope's effective redaction config — the same
        // resolution `observe` takes, at the scope the content is landing
        // at (ADR-0050 decision 11).
        let redaction = state
            .pdp
            .effective(tenant_id, Resource::Scope(body.scope_id), &input.context())
            .redaction;
        // Which documents already have their chunks. The address is the
        // question: identical bytes, identical address, identical chunks.
        let mut unmoved: Vec<bool> = Vec::with_capacity(assets.len());
        for asset in &assets {
            let existing = packs::chunks_of(
                &mut *tx,
                tenant_id,
                body.scope_id,
                *asset.address().as_bytes(),
            )
            .await?;
            unmoved.push(!existing.is_empty());
        }
        (node, author, authorized, redaction, unmoved)
    };

    // ── Scan and chunk — outside any transaction ───────────────────────
    let mut prepared: Vec<Prepared> = Vec::with_capacity(assets.len());
    let mut scanned = 0_usize;
    let mut redacted = 0_usize;
    for (asset, unmoved) in assets.into_iter().zip(unmoved) {
        let total =
            u32::try_from(synveda_types::chunk(&asset.document.content).len()).unwrap_or(u32::MAX);
        if unmoved {
            // Nothing to scan and nothing to cut. The bytes
            // were admitted once and are addressed by exactly what was
            // admitted.
            prepared.push(Prepared {
                asset,
                chunks: Vec::new(),
                total,
            });
            continue;
        }
        scanned += 1;
        // The scanner first, always: a secret that does not survive this
        // seam never reaches stored authored context.
        //
        // The ladder is MEM-2's own (ADR-0021 decision 4, ADR-0050
        // decision 11), which means `redact` **scrubs and continues** here
        // exactly as it does on the observe path — the finding is gone from
        // the text before anything is chunked or addressed, so
        // what a reviewer reads and what a session composes are the
        // scrubbed bytes. Only `quarantine` and `deny` stop the write.
        let mut asset = asset;
        let scan = scan_document(&asset.document).await?;
        match scan.disposition(&redaction) {
            None => {}
            Some(RedactionMode::Redact) => {
                asset.document.content = scrubbed(&scan, &asset.document.content);
                redacted += 1;
            }
            Some(mode) => {
                return refuse_scanned(state, tenant_id, &body.name, &asset, &scan, mode).await;
            }
        }
        let chunks = synveda_types::chunk(&asset.document.content);
        prepared.push(Prepared {
            asset,
            chunks,
            total,
        });
    }

    // ── Write, in one transaction ──────────────────────────────────────
    //
    // Every scanned chunk row lands with its object address or none do. A
    // publication cannot point at a partially admitted document version.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let stored_pack = packs::upsert_pack(
        &mut *tx,
        tenant_id,
        &packs::NewPack {
            scope_id: body.scope_id,
            name: &body.name,
            description: &body.description,
            author,
        },
    )
    .await?;

    let mut written_total = 0_u32;
    let mut written: Vec<(packs::StoredDocument, u32, String)> = Vec::with_capacity(prepared.len());
    for item in &prepared {
        let object = vedaflow::put_context_pack(&mut tx, tenant_id, &item.asset).await?;
        let address = *object.hash.as_bytes();
        let stored = packs::upsert_document(
            &mut *tx,
            tenant_id,
            &packs::NewDocument {
                scope_id: body.scope_id,
                pack_name: &body.name,
                document_name: &item.asset.document.name,
                title: &item.asset.document.title,
                sensitivity: item.asset.sensitivity,
                object_hash: address,
                chunks: item.total,
                author,
            },
        )
        .await?;
        let mut written_chunks = 0_u32;
        for chunk in &item.chunks {
            packs::record_chunk(
                &mut *tx,
                tenant_id,
                &packs::NewChunk {
                    id: ContextPackChunkId::new(),
                    scope_id: body.scope_id,
                    pack_name: &body.name,
                    document_name: &item.asset.document.name,
                    title: &item.asset.document.title,
                    sensitivity: item.asset.sensitivity,
                    document_hash: address,
                    ordinal: chunk.ordinal,
                    heading: chunk.heading.as_deref(),
                    content: &chunk.content,
                    content_hash: *blake3::hash(chunk.content.as_bytes()).as_bytes(),
                },
            )
            .await?;
            written_chunks += 1;
        }
        written_total += written_chunks;
        written.push((stored, written_chunks, object.hash.to_hex()));
    }

    let published = published_at(&mut tx, tenant_id, body.scope_id).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::ContextPackAuthored,
        Resource::Scope(body.scope_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::ContextPackWrite, &authorized),
            "asset": synveda_types::AssetKind::ContextPack.as_str(),
            "pack": body.name.as_str(),
            // The addresses, the counts and the tiers. Never document text,
            // and never chunk text — the discipline every plane has followed
            // since AUD-1.
            "documents": written
                .iter()
                .map(|(stored, written_chunks, hash)| json!({
                    "document": stored.document_name.as_str(),
                    "sensitivity": stored.sensitivity.as_str(),
                    "object_hash": hash,
                    "chunks": stored.chunks,
                    "written_chunks": written_chunks,
                }))
                .collect::<Vec<_>>(),
            "scanned": scanned,
            "redacted": redacted,
            "written_chunks": written_total,
            // What a session is being served *now*, which is the point of
            // the whole surface: authoring moved nothing.
            "published_commit": published.as_ref().map(|(commit, _)| commit.to_hex()),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(view(&node, stored_pack, written, published)))
}

/// Runs MEM-2's scanner over one document (ADR-0050 decision 11).
///
/// The document goes in as a JSON object because that is the shape
/// `synveda_ingest::scan` walks, and it is the *same* scanner the observe
/// path uses rather than a second one with its own rule list — which is the
/// point: this is the surface that would otherwise have been the easy way
/// around MEM-2's guarantee.
///
/// CPU work, O(document bytes), so it goes off the reactor exactly as the
/// observe path's does.
async fn scan_document(document: &PackDocument) -> Result<ScanOutcome> {
    let payload = json!({
        "title": document.title,
        "content": document.content,
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

/// The document's text with every finding scrubbed, as MEM-2's scanner
/// rewrote it.
///
/// `synveda_ingest::scan` redacts in place and hands back the whole
/// payload, so this is a field read rather than a second redaction — one
/// implementation of what a secret looks like, which is the whole point of
/// running the observe path's scanner here rather than a pack-shaped copy
/// of it.
fn scrubbed(scan: &ScanOutcome, original: &str) -> String {
    scan.payload
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| original.to_owned())
}

/// The refusal a scanned document gets, and the event that chains it.
///
/// Both dispositions refuse, and that is what a **synchronous** authoring
/// surface can honestly do: there is an author on the other end of the
/// request who can fix the document. The `context_pack.quarantined` event
/// puts that refusal on the chain without creating a second review queue
/// (ADR-0050).
///
/// What is not a departure is the guarantee: the document is not stored,
/// not chunked, so no secret reaches served context.
async fn refuse_scanned<T>(
    state: &AppState,
    tenant_id: synveda_types::TenantId,
    pack: &ContextPackName,
    asset: &ContextPackAsset,
    scan: &ScanOutcome,
    mode: RedactionMode,
) -> Result<T> {
    let rules: Vec<&str> = scan.findings.iter().map(|finding| finding.rule).collect();
    if mode == RedactionMode::Quarantine {
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::ContextPackQuarantined,
            Resource::Scope(asset.scope_id).to_string(),
            // The scan stopped the write, so the operation did not
            // complete — `failure` rather than `deny`, which is the PDP's
            // word and no PDP denied anything here.
            Outcome::Failure,
            json!({
                "asset": synveda_types::AssetKind::ContextPack.as_str(),
                "pack": pack.as_str(),
                "document": asset.document.name.as_str(),
                // The rules that fired and how often — never the matched
                // text, which is the thing the scanner exists to keep out of
                // places it should not be.
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
            "document {} was stopped by the redaction scanner ({}): {}. It was not stored, \
             not chunked — a pack is a surface where bulk external text enters the product, \
             and the scan runs before persistence (ADR-0050 decision 11). Remove the finding \
             and author again",
            asset.document.name,
            mode.as_str(),
            rules.join(", "),
        ),
    })
}

/// The first document name that appears twice, if any.
fn first_duplicate(assets: &[ContextPackAsset]) -> Option<&DocumentName> {
    let mut seen: Vec<&DocumentName> = Vec::with_capacity(assets.len());
    for asset in assets {
        if seen.contains(&&asset.document.name) {
            return Some(&asset.document.name);
        }
        seen.push(&asset.document.name);
    }
    None
}

// ── List ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ListParams {
    /// The scope whose registry to list. Required: a listing is a scope's
    /// own shelf, and a tenant-wide one would be a different question with
    /// a different resource.
    scope_id: ScopeId,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ContextPackListEntry)]
pub(crate) struct ListEntry {
    name: String,
    description: String,
    documents: Vec<DocumentView>,
    updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    updated_by: IdentityId,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = ContextPackListResponse)]
pub(crate) struct ListResponse {
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    scope_path: String,
    packs: Vec<ListEntry>,
}

/// `GET /v1/context-packs?scope_id=…` — the registry at one scope: every
/// pack, its documents, and what the published channel holds for each.
///
/// Documents the caller may not read at their tier are omitted rather than
/// refused, for the reason composition skips them: a listing that refused
/// wholesale would make one `confidential` runbook hide the rest.
#[utoipa::path(
    get,
    path = "/v1/context-packs",
    operation_id = "list_context_packs",
    tag = "context-packs",
    params(("scope_id" = String, Query, format = "uuid")),
    responses(
        (status = 200, description = "Visible context-pack drafts at the scope", body = ListResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Context-pack read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The governing scope is absent", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "context_packs.list", skip_all)]
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
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    // The gate first, at the working tier — the question a listing asks.
    let authorized = authz::decide_context_pack_read(
        state,
        &input,
        Resource::Scope(scope_id),
        Sensitivity::WORKING,
    )?;
    let bundles = packs::list_packs(&mut *tx, tenant_id, scope_id).await?;
    let documents = packs::list_all_documents(&mut *tx, tenant_id, scope_id).await?;
    // Then one decision per tier the shelf actually carries — at most
    // three, and usually one. The `retrieval::plan` shape (ADR-0038
    // decision 3): ask per tier, keep the answers as a set.
    let mut permitted: HashMap<Sensitivity, bool> = HashMap::new();
    for document in &documents {
        if let std::collections::hash_map::Entry::Vacant(slot) =
            permitted.entry(document.sensitivity)
        {
            slot.insert(permit(state, &input, scope_id, document.sensitivity)?.is_some());
        }
    }
    let published = published_at(&mut tx, tenant_id, scope_id).await?;

    let mut entries = Vec::with_capacity(bundles.len());
    for bundle in bundles {
        let views: Vec<DocumentView> = documents
            .iter()
            .filter(|document| document.pack_name == bundle.name)
            .filter(|document| {
                permitted
                    .get(&document.sensitivity)
                    .copied()
                    .unwrap_or(false)
            })
            .map(|document| document_view(document, 0, published.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        entries.push(ListEntry {
            name: bundle.name.to_string(),
            description: bundle.description,
            documents: views,
            updated_at: bundle.updated_at,
            updated_by: bundle.updated_by,
        });
    }

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Scope(scope_id).to_string(),
        Outcome::Allow,
        json!({
            "authz": audit::decision_context(Action::ContextPackRead, &authorized),
            "op": "context_packs.list",
            "packs": entries.len(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(ListResponse {
        scope_id,
        scope_path: node.slug.clone(),
        packs: entries,
    }))
}

// ── Shared ─────────────────────────────────────────────────────────────

/// What `scope`'s published pack channel holds: the commit it serves and
/// the address it names for every document path.
async fn published_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    scope_id: ScopeId,
) -> Result<
    Option<(
        vedaflow::CommitHash,
        HashMap<DocumentPath, vedaflow::hash::ObjectHash>,
    )>,
> {
    Ok(
        vedaflow::read_context_pack_members(tx, tenant_id, &[scope_id], Channel::Published)
            .await?
            .into_iter()
            .next()
            .map(|state| (state.commit, state.members)),
    )
}

type PublishedState = (
    vedaflow::CommitHash,
    HashMap<DocumentPath, vedaflow::hash::ObjectHash>,
);

fn document_view(
    document: &packs::StoredDocument,
    written_chunks: u32,
    published: Option<&PublishedState>,
) -> Result<DocumentView> {
    let address = vedaflow::hash::ObjectHash::from_bytes(document.object_hash);
    let path = DocumentPath::new(document.pack_name.clone(), document.document_name.clone());
    Ok(DocumentView {
        name: document.document_name.to_string(),
        title: document.title.clone(),
        sensitivity: document.sensitivity,
        object_hash: address.to_hex(),
        chunks: document.chunks,
        written_chunks,
        updated_at: document.updated_at,
        updated_by: document.updated_by,
        published: published.and_then(|(commit, members)| {
            members.get(&path).map(|hash| PublishedView {
                commit: commit.to_hex(),
                object_hash: hash.to_hex(),
                current: *hash == address,
            })
        }),
    })
}

fn view(
    node: &Scope,
    pack: packs::StoredPack,
    written: Vec<(packs::StoredDocument, u32, String)>,
    published: Option<PublishedState>,
) -> PackView {
    let documents = written
        .iter()
        .filter_map(|(document, written_chunks, _)| {
            document_view(document, *written_chunks, published.as_ref()).ok()
        })
        .collect();
    PackView {
        name: pack.name.to_string(),
        scope_id: pack.scope_id,
        scope_path: node.slug.clone(),
        description: pack.description,
        documents,
        created_at: pack.created_at,
        created_by: pack.created_by,
        updated_at: pack.updated_at,
        updated_by: pack.updated_by,
    }
}

/// One `ContextPackRead` decision at `scope_id`, as an option rather than
/// an error: the listing surface omits what it may not show.
fn permit(
    state: &AppState,
    input: &DecisionInput,
    scope_id: ScopeId,
    sensitivity: Sensitivity,
) -> Result<Option<crate::authz::Authorized>> {
    match authz::decide_context_pack_read(state, input, Resource::Scope(scope_id), sensitivity) {
        Ok(authorized) => Ok(Some(authorized)),
        Err(Error::PolicyDenied { .. }) => Ok(None),
        Err(other) => Err(other),
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
            message: "authoring a context pack requires a provisioned identity".to_owned(),
        })
}
