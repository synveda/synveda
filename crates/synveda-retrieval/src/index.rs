//! The per-tenant Tantivy BM25 sidecar (CTX-1, ADR-0024 decision 3).
//!
//! One index directory per tenant under a configurable root: BM25
//! corpus statistics stay tenant-local, tenant disposal is a directory
//! delete (TEN-5), and the tenant filter is structural — a search opens
//! exactly one tenant's index. Postgres remains the system of record:
//! the index stores the record id, the pushdown terms (scope,
//! sensitivity), and the tokenised content, never stored content — hits
//! are hydrated (and re-verified) from the database (ADR-0024
//! decision 6).
//!
//! Beside each tenant's index lives a state file carrying the index
//! schema version and the indexer's watermark. The two heal each other:
//! a missing state file wipes the index (unknown coverage), a missing
//! index ignores the state file (fresh build from epoch), and a schema
//! version bump rebuilds from scratch. Deleting a tenant's index
//! directory *is* the operator's rebuild procedure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use synveda_types::{Error, RecordId, Result, ScopeId, Sensitivity, TenantId};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermSetQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, Term, TextFieldIndexing, TextOptions,
    Value as _,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, StopWordFilter, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, doc};
use uuid::Uuid;

/// The sidecar's schema version. Bumping it (changed fields, changed
/// tokenisation) makes every existing tenant index rebuild from epoch
/// on its next sweep.
pub const SEARCH_SCHEMA_VERSION: u32 = 1;

/// One sparse-leg hit: a candidate id and its BM25 score, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseHit {
    /// The candidate record.
    pub record_id: RecordId,
    /// The BM25 score (higher is better; comparable only within one
    /// query's result list).
    pub score: f32,
}

/// The state file beside each tenant index: what schema the index was
/// built with, and how far the indexer's change scan has covered.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct IndexState {
    schema_version: u32,
    watermark: DateTime<Utc>,
}

/// The schema's fields, resolved once per opened index.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fields {
    pub(crate) record_id: Field,
    pub(crate) scope_id: Field,
    pub(crate) sensitivity: Field,
    pub(crate) content: Field,
}

/// One tenant's opened index: the handle, a manually reloaded reader
/// (the indexer reloads after each commit, so visibility is
/// deterministic), and the resolved fields.
pub(crate) struct TenantSlot {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) fields: Fields,
}

/// The process-wide sidecar manager: opens, caches, and heals tenant
/// indexes under one root directory. The gateway holds one in an `Arc`
/// shared by the search path and the indexer task.
pub struct SearchIndex {
    root: PathBuf,
    slots: RwLock<HashMap<TenantId, Arc<TenantSlot>>>,
}

impl SearchIndex {
    /// Opens the manager over `root`, creating the directory if needed.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|err| Error::Internal {
            message: format!("search index root {}: {err}", root.display()),
        })?;
        Ok(Self {
            root,
            slots: RwLock::new(HashMap::new()),
        })
    }

    /// The BM25 leg (ADR-0024 decision 6): top `limit` current-index
    /// hits for `query` within the allowed scopes and sensitivities. A
    /// tenant with no (valid) index yet returns no hits — the engine
    /// degrades to dense-only while the indexer catches up; it never
    /// errors on cold state.
    #[tracing::instrument(
        name = "retrieval.sparse",
        skip_all,
        fields(tenant.id = %tenant_id, scopes.count = scopes.len(), limit, hits = tracing::field::Empty),
        err(Display)
    )]
    pub fn search_sparse(
        &self,
        tenant_id: TenantId,
        query: &str,
        scopes: &[ScopeId],
        sensitivities: &[Sensitivity],
        limit: usize,
    ) -> Result<Vec<SparseHit>> {
        let Some(slot) = self.reader_slot(tenant_id)? else {
            tracing::Span::current().record("hits", 0);
            return Ok(vec![]);
        };
        let fields = slot.fields;
        let parser = QueryParser::for_index(&slot.index, vec![fields.content]);
        // Lenient: a user query is never a syntax error, at worst a
        // weaker match (unparseable fragments drop out).
        let (content_query, _errors) = parser.parse_query_lenient(query);
        let scope_terms = TermSetQuery::new(
            scopes
                .iter()
                .map(|scope| Term::from_field_text(fields.scope_id, &scope.to_string())),
        );
        let sensitivity_terms = TermSetQuery::new(
            sensitivities
                .iter()
                .map(|level| Term::from_field_text(fields.sensitivity, level.as_str())),
        );
        let filtered = BooleanQuery::new(vec![
            (Occur::Must, content_query),
            (Occur::Must, Box::new(scope_terms) as Box<dyn Query>),
            (Occur::Must, Box::new(sensitivity_terms) as Box<dyn Query>),
        ]);
        let searcher = slot.reader.searcher();
        let top = searcher
            .search(
                &filtered,
                &TopDocs::with_limit(limit.max(1)).order_by_score(),
            )
            .map_err(tantivy_error)?;
        let mut hits = Vec::with_capacity(top.len());
        for (score, address) in top {
            let document: TantivyDocument = searcher.doc(address).map_err(tantivy_error)?;
            let id = document
                .get_first(fields.record_id)
                .and_then(|value| value.as_str())
                .and_then(|text| Uuid::parse_str(text).ok())
                .ok_or_else(|| Error::Internal {
                    message: "search index document without a record id".to_owned(),
                })?;
            hits.push(SparseHit {
                record_id: RecordId::from_uuid(id),
                score,
            });
        }
        tracing::Span::current().record("hits", hits.len());
        Ok(hits)
    }

    /// The indexer's entry: the tenant's slot (created or healed as
    /// needed) plus the persisted watermark to scan from.
    pub(crate) fn open_for_write(
        &self,
        tenant_id: TenantId,
    ) -> Result<(Arc<TenantSlot>, DateTime<Utc>)> {
        // A cached slot was opened against a validated state file; the
        // watermark is re-read each sweep (cheap, and it keeps the file
        // authoritative).
        if let Some(slot) = self.cached(tenant_id)
            && let Some(state) = self.load_state(tenant_id)
            && state.schema_version == SEARCH_SCHEMA_VERSION
        {
            return Ok((slot, state.watermark));
        }
        let dir = self.tenant_dir(tenant_id);
        let state = self.load_state(tenant_id);
        let index_present = dir.join("meta.json").exists();
        let valid = index_present
            && state.is_some_and(|state| state.schema_version == SEARCH_SCHEMA_VERSION);
        let (index, watermark) = if valid {
            let index = Index::open_in_dir(&dir).map_err(tantivy_error)?;
            (index, state.map(|state| state.watermark).unwrap_or(epoch()))
        } else {
            // Unknown coverage (no state), foreign schema, or no index:
            // rebuild from scratch (module docs).
            self.wipe_tenant(tenant_id)?;
            std::fs::create_dir_all(&dir).map_err(|err| Error::Internal {
                message: format!("search index dir {}: {err}", dir.display()),
            })?;
            let index = Index::create_in_dir(&dir, schema()).map_err(tantivy_error)?;
            self.write_state(
                tenant_id,
                IndexState {
                    schema_version: SEARCH_SCHEMA_VERSION,
                    watermark: epoch(),
                },
            )?;
            (index, epoch())
        };
        let slot = Arc::new(open_slot(index)?);
        write(&self.slots).insert(tenant_id, Arc::clone(&slot));
        Ok((slot, watermark))
    }

    /// Persists the watermark after a committed sweep (temp file +
    /// rename: a torn write parses as no state and rebuilds — safe,
    /// never wrong).
    pub(crate) fn store_watermark(
        &self,
        tenant_id: TenantId,
        watermark: DateTime<Utc>,
    ) -> Result<()> {
        self.write_state(
            tenant_id,
            IndexState {
                schema_version: SEARCH_SCHEMA_VERSION,
                watermark,
            },
        )
    }

    /// The search path's slot lookup: cached, or opened read-only when
    /// a valid index exists. `None` means "no index yet" — cold, absent,
    /// or built by a foreign schema version (the indexer will rebuild).
    fn reader_slot(&self, tenant_id: TenantId) -> Result<Option<Arc<TenantSlot>>> {
        if let Some(slot) = self.cached(tenant_id) {
            return Ok(Some(slot));
        }
        let valid_state = self
            .load_state(tenant_id)
            .is_some_and(|state| state.schema_version == SEARCH_SCHEMA_VERSION);
        let dir = self.tenant_dir(tenant_id);
        if !valid_state || !dir.join("meta.json").exists() {
            return Ok(None);
        }
        let index = Index::open_in_dir(&dir).map_err(tantivy_error)?;
        let slot = Arc::new(open_slot(index)?);
        write(&self.slots).insert(tenant_id, Arc::clone(&slot));
        Ok(Some(slot))
    }

    fn cached(&self, tenant_id: TenantId) -> Option<Arc<TenantSlot>> {
        read(&self.slots).get(&tenant_id).cloned()
    }

    fn tenant_dir(&self, tenant_id: TenantId) -> PathBuf {
        self.root.join(tenant_id.to_string())
    }

    fn state_path(&self, tenant_id: TenantId) -> PathBuf {
        self.root.join(format!("{tenant_id}.state.json"))
    }

    fn load_state(&self, tenant_id: TenantId) -> Option<IndexState> {
        let bytes = std::fs::read(self.state_path(tenant_id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn write_state(&self, tenant_id: TenantId, state: IndexState) -> Result<()> {
        let path = self.state_path(tenant_id);
        let temp = self.root.join(format!("{tenant_id}.state.json.tmp"));
        let io = |err: std::io::Error| Error::Internal {
            message: format!("search index state {}: {err}", path.display()),
        };
        let bytes = serde_json::to_vec(&state).map_err(|err| Error::Internal {
            message: format!("search index state serialisation: {err}"),
        })?;
        std::fs::write(&temp, bytes).map_err(io)?;
        // Windows refuses to rename over an existing file.
        if path.exists() {
            std::fs::remove_file(&path).map_err(io)?;
        }
        std::fs::rename(&temp, &path).map_err(io)?;
        Ok(())
    }

    /// Drops the tenant's cached slot and deletes its index directory
    /// and state file.
    fn wipe_tenant(&self, tenant_id: TenantId) -> Result<()> {
        write(&self.slots).remove(&tenant_id);
        let dir = self.tenant_dir(tenant_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|err| Error::Internal {
                message: format!("search index wipe {}: {err}", dir.display()),
            })?;
        }
        let state = self.state_path(tenant_id);
        if state.exists() {
            std::fs::remove_file(&state).map_err(|err| Error::Internal {
                message: format!("search index wipe {}: {err}", state.display()),
            })?;
        }
        Ok(())
    }
}

/// The content tokenizer's name; registered on every opened index (the
/// registry is per-instance, never persisted).
const CONTENT_TOKENIZER: &str = "en_stop";

/// The classic Lucene English stopword list. RRF is rank-based, so the
/// sparse leg must not hand out ranks for "and"-grade matches — a
/// stopword hit at sparse rank 5 would outweigh a genuine dense hit at
/// rank 8 (ADR-0024 decision 6).
const STOPWORDS: [&str; 33] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is", "it",
    "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there", "these",
    "they", "this", "to", "was", "will", "with",
];

/// Registers the content tokenizer (simple word split → lowercase →
/// stopword removal) on an index instance.
fn register_tokenizers(index: &Index) {
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(StopWordFilter::remove(
            STOPWORDS.iter().map(|word| (*word).to_owned()),
        ))
        .build();
    index.tokenizers().register(CONTENT_TOKENIZER, analyzer);
}

/// The sidecar schema (version [`SEARCH_SCHEMA_VERSION`]): raw-term
/// fields for the document key and pushdown filters, stopword-filtered
/// tokenised content.
fn schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("record_id", STRING | STORED);
    builder.add_text_field("scope_id", STRING);
    builder.add_text_field("sensitivity", STRING);
    builder.add_text_field(
        "content",
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(CONTENT_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        ),
    );
    builder.build()
}

fn open_slot(index: Index) -> Result<TenantSlot> {
    register_tokenizers(&index);
    let schema = index.schema();
    let field = |name: &str| {
        schema.get_field(name).map_err(|_| Error::Internal {
            message: format!("search index schema missing field {name}"),
        })
    };
    let fields = Fields {
        record_id: field("record_id")?,
        scope_id: field("scope_id")?,
        sensitivity: field("sensitivity")?,
        content: field("content")?,
    };
    // Manual reload: the indexer reloads after each commit, so tests
    // and callers see deterministic visibility instead of a background
    // thread's timing.
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(tantivy_error)?;
    Ok(TenantSlot {
        index,
        reader,
        fields,
    })
}

/// Builds the indexable document for one record projection.
pub(crate) fn record_document(
    fields: Fields,
    record: &synveda_store::search::IndexableRecord,
) -> TantivyDocument {
    doc!(
        fields.record_id => record.id.to_string(),
        fields.scope_id => record.scope_id.to_string(),
        fields.sensitivity => record.sensitivity.as_str(),
        fields.content => record.content.as_str(),
    )
}

/// The document-key term for `record_id` — shared by upsert's
/// delete-then-add and by deletes.
pub(crate) fn record_term(fields: Fields, record_id: RecordId) -> Term {
    Term::from_field_text(fields.record_id, &record_id.to_string())
}

/// A transient writer for one sweep's batch of operations. Opened per
/// sweep and dropped after commit — holding writers open per tenant
/// would pin their memory budgets for idle tenants.
pub(crate) fn sweep_writer(index: &Index) -> Result<IndexWriter<TantivyDocument>> {
    index
        .writer_with_num_threads(2, 64_000_000)
        .map_err(tantivy_error)
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// Sidecar failures are ours to fix (an ops/corruption condition, never
/// the caller's input): map into the taxonomy as internal.
fn tantivy_error(err: impl std::fmt::Display) -> Error {
    Error::Internal {
        message: format!("search index: {err}"),
    }
}

/// Lock helpers in the scope-chain cache's pattern (ADR-0016): a
/// poisoned lock means a panic mid-`HashMap` operation; the map is
/// still structurally sound, so recover the guard.
fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}
