//! The context-pack registry API (PRMT-2, ADR-0050): `/v1/context-packs`
//! behind tenant resolution, uniform-404 ownership, and the PDP
//! (`ContextPackWrite` to author, `ContextPackRead` to see a shelf).
//!
//! Two surfaces here, and **neither is the one that matters most**. A
//! prompt is fetched by name through its own route; a pack's content
//! arrives through `inject`, as pinned records the composition engine
//! ranks (ADR-0050 decision 2). So this module authors and lists, and the
//! read path lives in `synveda_retrieval::compose`.
//!
//! - **author** (`POST /v1/context-packs`) — the slowest write in the
//!   product, and deliberately: it chunks, scans and embeds a bundle in one
//!   request. It moves nothing a session reads, which is the whole of "a
//!   pack reaches a session only through review": the published channel is
//!   somewhere else, and only the approval matrix moves it.
//! - **list** (`GET /v1/context-packs?scope_id=…`) — the registry view at
//!   one scope: what is drafted, what is published, and whether they are
//!   the same bytes.
//!
//! # Why the expensive half happens here
//!
//! The AC says "chunked+embedded **on publish**". This is a departure with
//! a reason (ADR-0050 decision 4): embedding is a call to TEI, no proposal
//! approval has ever made a network call, and the literal reading makes a
//! curator's approval fail when a model server is down. Doing it at
//! authoring also makes the atomicity clause cheap — chunk rows land with
//! their embeddings or not at all, a publication is a ref move, and a
//! rewind restores a version with no re-embedding at all.
//!
//! The order inside it is ADR-0023 decision 5's, applied to authored bulk:
//! **chunk, scan, embed, commit**. The scanner runs before the embedder, so
//! a secret that never reaches `content` never reaches vector space.
//!
//! # Re-authoring an unchanged document costs nothing
//!
//! The chunker is deterministic and the document address covers exactly
//! what a reviewer consents to, so identical bytes produce the same address
//! and the same chunks. This route looks the address up first and skips the
//! scan, the chunking and the embedding for every document that has not
//! moved — which is what keeps "save the bundle again" from being a
//! thousand vectors of work.

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
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::{Action, Resource};
use synveda_store::{hierarchy, packs, records, rls};
use synveda_types::{
    Channel, ContextPackName, DocumentChunk, DocumentName, DocumentPath, Error, HierarchyNode,
    IdentityId, MAX_PACK_DESCRIPTION_CHARS, MAX_PACK_DOCUMENTS, PackDocument, RecordClass,
    RecordId, RecordKind, RedactionMode, Result, ScopeId, Sensitivity,
};
use synveda_vedaflow::{self as vedaflow, ContextPackAsset};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::hierarchy::{body, commit, found, tenant_id};
use crate::telemetry::CONTEXT_PACK_OPERATIONS_TOTAL;

/// Counts the operation and renders the result — the outcome taxonomy
/// every governed plane uses.
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
    metrics::counter!(CONTEXT_PACK_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

// ── Author ─────────────────────────────────────────────────────────────

/// One document as an author supplies it.
#[derive(Deserialize)]
pub(crate) struct DocumentBody {
    /// Its name within the pack: path-shaped, so a bundle can carry
    /// `runbooks/payments.md` rather than flattening a directory.
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
    sensitivity: Option<Sensitivity>,
}

#[derive(Deserialize)]
pub(crate) struct AuthorBody {
    /// Where the pack is authored — the scope that will stand behind it,
    /// and the scope whose published channel a proposal would move.
    scope_id: ScopeId,
    /// Its name: one segment, lower-case, and the identifier a scope's
    /// override is expressed in (ADR-0050 decision 1).
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
#[derive(Serialize)]
struct PublishedView {
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

#[derive(Serialize)]
struct DocumentView {
    name: String,
    title: String,
    sensitivity: Sensitivity,
    /// The draft's content address — what a proposal would bind.
    object_hash: String,
    /// How many chunks it cut into.
    chunks: u32,
    /// How many of those this request actually embedded. Zero for a
    /// document whose bytes did not move, which is the observable half of
    /// "re-authoring an unchanged document re-embeds nothing".
    embedded: u32,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    published: Option<PublishedView>,
}

#[derive(Serialize)]
struct PackView {
    name: String,
    scope_id: ScopeId,
    scope_path: String,
    description: String,
    documents: Vec<DocumentView>,
    created_at: DateTime<Utc>,
    created_by: IdentityId,
    updated_at: DateTime<Utc>,
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
    /// Their vectors, in the same order.
    vectors: Vec<Vec<f32>>,
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
            hierarchy::node(&mut *tx, body.scope_id).await?,
            tenant_id,
            body.scope_id,
        )?;
        let input = authz::gather(state, &mut tx, Some(&node)).await?;
        let authorized = authz::decide(
            state,
            &input,
            Action::ContextPackWrite,
            Resource::Scope(body.scope_id),
            None,
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
            let existing =
                packs::chunks_of(&mut *tx, tenant_id, *asset.address().as_bytes()).await?;
            unmoved.push(!existing.is_empty());
        }
        (node, author, authorized, redaction, unmoved)
    };

    // ── Scan, chunk, embed — outside any transaction ───────────────────
    let mut prepared: Vec<Prepared> = Vec::with_capacity(assets.len());
    let mut scanned = 0_usize;
    let mut redacted = 0_usize;
    for (asset, unmoved) in assets.into_iter().zip(unmoved) {
        let total =
            u32::try_from(synveda_types::chunk(&asset.document.content).len()).unwrap_or(u32::MAX);
        if unmoved {
            // Nothing to scan, nothing to cut, nothing to embed. The bytes
            // were admitted once and are addressed by exactly what was
            // admitted.
            prepared.push(Prepared {
                asset,
                chunks: Vec::new(),
                vectors: Vec::new(),
                total,
            });
            continue;
        }
        scanned += 1;
        // The scanner first, always (ADR-0023 decision 5's order applied to
        // authored bulk): a secret that never reaches `content` never
        // reaches vector space.
        //
        // The ladder is MEM-2's own (ADR-0021 decision 4, ADR-0050
        // decision 11), which means `redact` **scrubs and continues** here
        // exactly as it does on the observe path — the finding is gone from
        // the text before anything is chunked, addressed or embedded, so
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
        let inputs: Vec<String> = chunks.iter().map(|piece| piece.content.clone()).collect();
        let vectors = if inputs.is_empty() {
            Vec::new()
        } else {
            let vectors = state.embedder.embed(&inputs).await?;
            if vectors.len() != inputs.len() {
                return Err(Error::Dependency {
                    service: "embedder".to_owned(),
                    message: format!(
                        "the embedder returned {} vectors for {} chunks of {}; a document \
                         lands with all of its embeddings or none (ADR-0023 decision 2)",
                        vectors.len(),
                        inputs.len(),
                        asset.document.name
                    ),
                });
            }
            vectors
        };
        prepared.push(Prepared {
            asset,
            chunks,
            vectors,
            total,
        });
    }

    // ── Write, in one transaction ──────────────────────────────────────
    //
    // Every chunk row lands with its embedding or none do (ADR-0023
    // decision 2, and migration 0015's deferred constraint trigger behind
    // it). A publication cannot then move the ref to a commit whose chunks
    // are not all embedded, because the commit names addresses that only
    // exist once this transaction committed — which is the whole of "re-
    // embeds atomically" (ADR-0050 decision 5).
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

    let now = Utc::now();
    let model = state.embedder.model().to_owned();
    let mut embedded_total = 0_u32;
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
        let mut embedded = 0_u32;
        for (chunk, vector) in item.chunks.iter().zip(&item.vectors) {
            let record_id = RecordId::new();
            records::insert(
                &mut *tx,
                record_id,
                tenant_id,
                &records::RecordState {
                    scope_id: body.scope_id,
                    owner_id: author,
                    // A pack's published content composes as pinned records
                    // — the decision the rest of the feature hangs on
                    // (ADR-0050 decision 2). It is also what buys ADR-0040's
                    // exemption from expiry, destruction and staleness, and
                    // ADR-0039's derived-only supersession, without a second
                    // implementation of either.
                    kind: RecordKind::Pinned,
                    // The class vocabulary describes what a *memory*
                    // asserts. A pack states what is so at its scope, so
                    // `fact` is the honest neutral — and the description
                    // that actually matters for a chunk is its
                    // `pack/document § heading`, which is why ADR-0041
                    // decision 4 made the index slot a per-`AssetKind` seam.
                    class: RecordClass::Fact,
                    content: chunk.content.clone(),
                    // Every chunk inherits its document's tier
                    // (decision 12), which is what CTX-4 and ADR-0038's
                    // per-scope tier check then apply per entry.
                    sensitivity: item.asset.sensitivity,
                    provenance: json!({
                        "source": "context-pack",
                        "pack": body.name.as_str(),
                        "document": item.asset.document.name.as_str(),
                        "ordinal": chunk.ordinal,
                        // The address it was cut from — the same value the
                        // chunk row carries, so a record read alone still
                        // says which reviewed version it came from.
                        "document_hash": object.hash.to_hex(),
                        "embedding_model": model,
                    }),
                    valid_from: now,
                    valid_to: None,
                },
                &records::RecordEmbedding {
                    model: model.clone(),
                    vector: vector.clone(),
                },
            )
            .await?;
            packs::record_chunk(
                &mut *tx,
                tenant_id,
                &packs::NewChunk {
                    record_id,
                    scope_id: body.scope_id,
                    pack_name: &body.name,
                    document_name: &item.asset.document.name,
                    title: &item.asset.document.title,
                    document_hash: address,
                    ordinal: chunk.ordinal,
                    heading: chunk.heading.as_deref(),
                },
            )
            .await?;
            embedded += 1;
        }
        embedded_total += embedded;
        written.push((stored, embedded, object.hash.to_hex()));
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
                .map(|(stored, embedded, hash)| json!({
                    "document": stored.document_name.as_str(),
                    "sensitivity": stored.sensitivity.as_str(),
                    "object_hash": hash,
                    "chunks": stored.chunks,
                    "embedded": embedded,
                }))
                .collect::<Vec<_>>(),
            "scanned": scanned,
            "redacted": redacted,
            "embedded": embedded_total,
            "embedding_model": model,
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
/// surface can honestly do: the observe path stages a quarantined event
/// because it is asynchronous and there is nobody to tell, while here there
/// is an author on the other end of the request who can fix the document.
/// So the review this decision asks for is the author's own, and the
/// `context_pack.quarantined` event is what puts it on the chain — no
/// second review queue, and no row in `observe_quarantine`, whose contents
/// are observe events (a departure from decision 11's literal wording,
/// recorded in ADR-0050's consequences).
///
/// What is not a departure is the guarantee: the document is not stored,
/// not chunked and not embedded, so no secret reaches vector space.
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
             not chunked and not embedded — a pack is the first surface where bulk external \
             text enters the product, and the scan runs ahead of the embedder so no secret \
             reaches vector space (ADR-0050 decision 11). Remove the finding and author again",
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

#[derive(Serialize)]
struct ListEntry {
    name: String,
    description: String,
    documents: Vec<DocumentView>,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
}

#[derive(Serialize)]
struct ListResponse {
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
        hierarchy::node(&mut *tx, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(state, &mut tx, Some(&node)).await?;
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
        scope_path: node.path.clone(),
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
    embedded: u32,
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
        embedded,
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
    node: &HierarchyNode,
    pack: packs::StoredPack,
    written: Vec<(packs::StoredDocument, u32, String)>,
    published: Option<PublishedState>,
) -> PackView {
    let documents = written
        .iter()
        .filter_map(|(document, embedded, _)| {
            document_view(document, *embedded, published.as_ref()).ok()
        })
        .collect();
    PackView {
        name: pack.name.to_string(),
        scope_id: pack.scope_id,
        scope_path: node.path.clone(),
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
