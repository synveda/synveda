//! Current Knowledge candidate generation and revision-vector maintenance
//! (CPR-17, ADR-0082).
//!
//! This module deliberately stops below authorisation. Every query is tenant
//! filtered and forced RLS is the backstop, but the rows are only candidates:
//! the gateway must decide each exact item and sensitivity through the PDP
//! before returning it. Keeping that distinction in the type names prevents a
//! future caller from mistaking a database filter for permission.
//!
//! Lexical rank comes from the immutable revision's stored `tsvector`.
//! Semantic candidates come from `knowledge_revision_embeddings`; both legs
//! address the same current Knowledge revision.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use std::collections::BTreeSet;
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::{
    Error, KnowledgeItemId, KnowledgeRevisionId, ProjectId, Result, ScopeId, TenantId, WorkspaceId,
};
use uuid::Uuid;

/// Reciprocal-rank candidate data before the gateway fuses the two legs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Stable aggregate id to hydrate from current database truth.
    pub item_id: KnowledgeItemId,
    /// Stable tie-breaker after relevance.
    pub updated_at: DateTime<Utc>,
    /// Leg-local relevance. The fusion currently uses rank, but retaining the
    /// native score makes diagnostics and tests able to prove the leg ran.
    pub score: f64,
}

/// Keyset for the non-query listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    /// Last candidate's update instant.
    pub updated_at: DateTime<Utc>,
    /// Last candidate's stable id.
    pub item_id: KnowledgeItemId,
}

/// Filters shared by plain listing and both query legs.
#[derive(Debug, Clone)]
pub struct Filters {
    /// Exact governed scopes admissible to a session planner. Empty means no
    /// additional exact-scope restriction; the ordinary public listing uses
    /// the independently shaped workspace/project/subtree filters below.
    pub scope_ids: Vec<ScopeId>,
    /// Workspace whose governed subtree contains the item.
    pub workspace_id: Option<WorkspaceId>,
    /// Exact project association.
    pub project_id: Option<ProjectId>,
    /// Governed scope subtree.
    pub scope_id: Option<ScopeId>,
    /// Exact owning principal label.
    pub owner_principal_id: Option<String>,
    /// Knowledge vocabulary filter.
    pub knowledge_type: Option<KnowledgeType>,
    /// Creation-origin filter.
    pub origin: Option<KnowledgeOrigin>,
    /// Explicit lifecycle filter. `None` means active only.
    pub lifecycle: Option<KnowledgeLifecycleState>,
    /// One canonical tag.
    pub tag: Option<String>,
    /// Provenance source family.
    pub source_type: Option<KnowledgeSourceType>,
    /// Updated on or after this instant.
    pub updated_from: Option<DateTime<Utc>>,
    /// Updated strictly before this instant.
    pub updated_before: Option<DateTime<Utc>>,
    /// Whether the current revision is due for verification.
    pub stale: Option<bool>,
    /// Valid-time instant at which aggregate state is evaluated.
    pub at: DateTime<Utc>,
    /// Transaction-time instant whose aggregate head is authoritative.
    pub as_known_at: DateTime<Utc>,
    /// Admit historical lifecycle heads (stale, superseded and archived).
    pub include_history: bool,
    /// Admit unresolved/future transitional heads.
    pub include_transitional: bool,
}

impl Filters {
    fn workspace_uuid(&self) -> Option<Uuid> {
        self.workspace_id.map(|id| id.as_uuid())
    }

    fn project_uuid(&self) -> Option<Uuid> {
        self.project_id.map(|id| id.as_uuid())
    }

    fn scope_uuid(&self) -> Option<Uuid> {
        self.scope_id.map(|id| id.as_uuid())
    }

    fn scope_uuids(&self) -> Vec<Uuid> {
        self.scope_ids.iter().map(ScopeId::as_uuid).collect()
    }

    fn knowledge_type_name(&self) -> Option<&str> {
        self.knowledge_type.map(KnowledgeType::as_str)
    }

    fn origin_name(&self) -> Option<&str> {
        self.origin.map(KnowledgeOrigin::as_str)
    }

    fn lifecycle_name(&self) -> Option<&str> {
        self.lifecycle.map(KnowledgeLifecycleState::as_str)
    }

    fn source_name(&self) -> Option<&str> {
        self.source_type.map(KnowledgeSourceType::as_str)
    }
}

/// Immutable revision text awaiting one configured model's vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexableRevision {
    /// Exact immutable revision.
    pub revision_id: KnowledgeRevisionId,
    /// Bounded title/summary/body projection sent to the embedder.
    pub text: String,
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23505" | "40001") => {
                return Error::Conflict {
                    message: db.to_string(),
                };
            }
            Some("23503" | "23514" | "22001") => {
                return Error::Invalid {
                    message: db.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(db),
            _ => {}
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Scans current candidates newest-first with a true `(updated_at, id)`
/// keyset. `limit` is a candidate bound, not a served-row bound: the gateway
/// advances its response cursor over denied candidates too.
#[tracing::instrument(
    name = "store.knowledge_search.list_candidates",
    skip_all,
    fields(tenant.id = %tenant_id, limit),
    err(Display)
)]
pub async fn list_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    filters: &Filters,
    cursor: Option<ListCursor>,
    limit: i64,
) -> Result<Vec<Candidate>> {
    let cursor_at = cursor.map(|value| value.updated_at);
    let cursor_id = cursor.map(|value| value.item_id.as_uuid());
    let rows = sqlx::query!(
        r#"
        select current.id as "item_id!",
               current.updated_at as "updated_at!",
               0::float8 as "score!"
        from knowledge_item_versions current
        join knowledge_revisions revision
          on revision.tenant_id = current.tenant_id
         and revision.knowledge_item_id = current.id
         and revision.id = current.current_revision_id
        where current.tenant_id = $1
          and current.tx_from <= $19
          and (current.tx_to is null or $19 < current.tx_to)
          and ($2::uuid is null or exists (
              select 1
              from workspaces workspace
              join scope_closure closure
                on closure.tenant_id = workspace.tenant_id
               and closure.ancestor_id = workspace.scope_id
               and closure.descendant_id = current.scope_id
              where workspace.tenant_id = current.tenant_id
                and workspace.id = $2
          ))
          and ($3::uuid is null or current.project_id = $3)
          and ($4::uuid is null or exists (
              select 1 from scope_closure closure
              where closure.tenant_id = current.tenant_id
                and closure.ancestor_id = $4
                and closure.descendant_id = current.scope_id
          ))
          and ($5::text is null or current.owner_principal_id = $5)
          and ($6::text is null or current.knowledge_type = $6)
          and ($7::text is null or current.origin = $7)
          and (($8::text is not null and current.lifecycle_state = $8)
               or ($8::text is null and (
                   current.lifecycle_state = 'active'
                   or ($13 = true and current.lifecycle_state = 'stale')
                   or ($20 and current.lifecycle_state in ('stale', 'superseded', 'archived'))
                   or ($21 and current.lifecycle_state = 'transitional')
               )))
          and ($9::text is null or $9 = any(revision.tags))
          and ($10::text is null or exists (
              select 1
              from knowledge_revision_sources link
              join knowledge_sources source
                on source.tenant_id = link.tenant_id
               and source.id = link.knowledge_source_id
              where link.tenant_id = current.tenant_id
                and link.knowledge_revision_id = current.current_revision_id
                and source.source_type = $10
          ))
          and ($11::timestamptz is null or current.updated_at >= $11)
          and ($12::timestamptz is null or current.updated_at < $12)
          -- Non-date freshness signals are checked only after exact PDP
          -- authorisation during hydration; retain the typed filter argument
          -- here without pre-filtering candidate evidence.
          and ($13::bool is null or current.id is not null)
          and ((revision.valid_from <= $14
                and (revision.valid_to is null or $14 < revision.valid_to))
               or ($21 and current.lifecycle_state = 'transitional'))
          and ($20 or not exists (
              select 1
                from knowledge_relations transition
                join knowledge_item_versions successor
                  on successor.tenant_id = transition.tenant_id
                 and successor.id = transition.source_item_id
                 and successor.tx_from <= $19
                 and (successor.tx_to is null or $19 < successor.tx_to)
                join knowledge_revisions successor_revision
                  on successor_revision.tenant_id = successor.tenant_id
                 and successor_revision.knowledge_item_id = successor.id
                 and successor_revision.id = successor.current_revision_id
               where transition.tenant_id = current.tenant_id
                 and transition.target_item_id = current.id
                 and transition.relation_type = 'transitions_to'
                 and transition.created_at <= $19
                 and successor.lifecycle_state = 'active'
                 and successor_revision.valid_from <= $14
                 and (successor_revision.valid_to is null
                      or $14 < successor_revision.valid_to)
          ))
          and (cardinality($18::uuid[]) = 0 or current.scope_id = any($18))
          and ($15::timestamptz is null
               or current.updated_at < $15
               or (current.updated_at = $15 and current.id < $16))
        order by current.updated_at desc, current.id desc
        limit $17
        "#,
        tenant_id.as_uuid(),
        filters.workspace_uuid(),
        filters.project_uuid(),
        filters.scope_uuid(),
        filters.owner_principal_id.as_deref() as Option<&str>,
        filters.knowledge_type_name(),
        filters.origin_name(),
        filters.lifecycle_name(),
        filters.tag.as_deref() as Option<&str>,
        filters.source_name(),
        filters.updated_from,
        filters.updated_before,
        filters.stale,
        filters.at,
        cursor_at,
        cursor_id,
        limit.max(1),
        &filters.scope_uuids(),
        filters.as_known_at,
        filters.include_history,
        filters.include_transitional,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| Candidate {
            item_id: KnowledgeItemId::from_uuid(row.item_id),
            updated_at: row.updated_at,
            score: row.score,
        })
        .collect())
}

/// Produces the lexical leg, best matching immutable current revisions first.
#[tracing::instrument(
    name = "store.knowledge_search.lexical_candidates",
    skip_all,
    fields(tenant.id = %tenant_id, limit),
    err(Display)
)]
pub async fn lexical_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    filters: &Filters,
    query: &str,
    limit: i64,
) -> Result<Vec<Candidate>> {
    let query = lexical_websearch_query(query);
    let rows = sqlx::query!(
        r#"
        select current.id as "item_id!",
               current.updated_at as "updated_at!",
               ts_rank_cd(
                   revision.search_document,
                   websearch_to_tsquery('simple'::regconfig, $2)
               )::float8 as "score!"
        from knowledge_item_versions current
        join knowledge_revisions revision
          on revision.tenant_id = current.tenant_id
         and revision.knowledge_item_id = current.id
         and revision.id = current.current_revision_id
        where current.tenant_id = $1
          and current.tx_from <= $18
          and (current.tx_to is null or $18 < current.tx_to)
          and revision.search_document @@ websearch_to_tsquery('simple'::regconfig, $2)
          and ($3::uuid is null or exists (
              select 1
              from workspaces workspace
              join scope_closure closure
                on closure.tenant_id = workspace.tenant_id
               and closure.ancestor_id = workspace.scope_id
               and closure.descendant_id = current.scope_id
              where workspace.tenant_id = current.tenant_id
                and workspace.id = $3
          ))
          and ($4::uuid is null or current.project_id = $4)
          and ($5::uuid is null or exists (
              select 1 from scope_closure closure
              where closure.tenant_id = current.tenant_id
                and closure.ancestor_id = $5
                and closure.descendant_id = current.scope_id
          ))
          and ($6::text is null or current.owner_principal_id = $6)
          and ($7::text is null or current.knowledge_type = $7)
          and ($8::text is null or current.origin = $8)
          and (($9::text is not null and current.lifecycle_state = $9)
               or ($9::text is null and (
                   current.lifecycle_state = 'active'
                   or ($14 = true and current.lifecycle_state = 'stale')
                   or ($19 and current.lifecycle_state in ('stale', 'superseded', 'archived'))
                   or ($20 and current.lifecycle_state = 'transitional')
               )))
          and ($10::text is null or $10 = any(revision.tags))
          and ($11::text is null or exists (
              select 1
              from knowledge_revision_sources link
              join knowledge_sources source
                on source.tenant_id = link.tenant_id
               and source.id = link.knowledge_source_id
              where link.tenant_id = current.tenant_id
                and link.knowledge_revision_id = current.current_revision_id
                and source.source_type = $11
          ))
          and ($12::timestamptz is null or current.updated_at >= $12)
          and ($13::timestamptz is null or current.updated_at < $13)
          and ($14::bool is null or current.id is not null)
          and ((revision.valid_from <= $15
                and (revision.valid_to is null or $15 < revision.valid_to))
               or ($20 and current.lifecycle_state = 'transitional'))
          and ($19 or not exists (
              select 1
                from knowledge_relations transition
                join knowledge_item_versions successor
                  on successor.tenant_id = transition.tenant_id
                 and successor.id = transition.source_item_id
                 and successor.tx_from <= $18
                 and (successor.tx_to is null or $18 < successor.tx_to)
                join knowledge_revisions successor_revision
                  on successor_revision.tenant_id = successor.tenant_id
                 and successor_revision.knowledge_item_id = successor.id
                 and successor_revision.id = successor.current_revision_id
               where transition.tenant_id = current.tenant_id
                 and transition.target_item_id = current.id
                 and transition.relation_type = 'transitions_to'
                 and transition.created_at <= $18
                 and successor.lifecycle_state = 'active'
                 and successor_revision.valid_from <= $15
                 and (successor_revision.valid_to is null
                      or $15 < successor_revision.valid_to)
          ))
          and (cardinality($17::uuid[]) = 0 or current.scope_id = any($17))
        order by 3 desc, current.updated_at desc, current.id desc
        limit $16
        "#,
        tenant_id.as_uuid(),
        query,
        filters.workspace_uuid(),
        filters.project_uuid(),
        filters.scope_uuid(),
        filters.owner_principal_id.as_deref() as Option<&str>,
        filters.knowledge_type_name(),
        filters.origin_name(),
        filters.lifecycle_name(),
        filters.tag.as_deref() as Option<&str>,
        filters.source_name(),
        filters.updated_from,
        filters.updated_before,
        filters.stale,
        filters.at,
        limit.max(1),
        &filters.scope_uuids(),
        filters.as_known_at,
        filters.include_history,
        filters.include_transitional,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| Candidate {
            item_id: KnowledgeItemId::from_uuid(row.item_id),
            updated_at: row.updated_at,
            score: row.score,
        })
        .collect())
}

/// Turns ordinary task prose into a recall-oriented `websearch_to_tsquery`
/// expression. PostgreSQL's `simple` dictionary intentionally preserves
/// product terms, identifiers and commands, but it also preserves question
/// furniture (`what`, `did`, `about`). Requiring every such token made a
/// natural question match only prose that repeated the question verbatim.
///
/// Explicit web-search syntax is left intact while it is within the same
/// complexity bound. Plain prose keeps its meaningful terms and joins them
/// with `OR`; ranking still favours revisions matching several terms, while
/// one changed tense cannot erase an otherwise exact lexical hit. Overlong
/// explicit input degrades to those bounded plain terms rather than handing an
/// attacker—or a large captured transcript—an unbounded PostgreSQL expression.
/// The resulting string remains a bound SQL parameter.
fn lexical_websearch_query(query: &str) -> String {
    let explicit = query.contains('"')
        || query.split_whitespace().any(|part| part == "OR")
        || query.split_whitespace().any(|part| part.starts_with('-'));
    if explicit
        && query.chars().any(char::is_alphanumeric)
        && query.chars().count() <= MAX_EXPLICIT_LEXICAL_CHARS
        && query.split_whitespace().count() <= MAX_LEXICAL_TERMS
    {
        return query.to_owned();
    }

    let mut seen = BTreeSet::new();
    let mut terms = Vec::new();
    for raw in query.split(|character: char| !character.is_alphanumeric() && character != '_') {
        let term = raw
            .to_lowercase()
            .chars()
            .take(MAX_LEXICAL_TERM_CHARS)
            .collect::<String>();
        if term.len() <= 1
            || LEXICAL_STOP_WORDS.contains(&term.as_str())
            || !seen.insert(term.clone())
        {
            continue;
        }
        terms.push(term);
        if terms.len() == MAX_LEXICAL_TERMS {
            break;
        }
    }
    if terms.is_empty() {
        EMPTY_LEXICAL_QUERY.to_owned()
    } else {
        terms.join(" OR ")
    }
}

/// PostgreSQL can represent much larger queries, but its tsquery parser uses a
/// finite expression stack. Thirty-two unique terms leaves generous headroom
/// for the `OR` nodes and bounds matching cost independently of API text size.
const MAX_LEXICAL_TERMS: usize = 32;
const MAX_LEXICAL_TERM_CHARS: usize = 64;
const MAX_EXPLICIT_LEXICAL_CHARS: usize = 256;
const EMPTY_LEXICAL_QUERY: &str = "synveda_no_lexical_terms";

const LEXICAL_STOP_WORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "at", "be", "before", "did", "do", "does", "for", "from",
    "how", "i", "in", "is", "it", "my", "of", "on", "or", "our", "the", "to", "was", "we", "what",
    "when", "where", "which", "with",
];

/// Bounded trace-only candidates for a non-current lifecycle.
///
/// Supersession normally closes valid time, so the ordinary current query is
/// intentionally unable to return that row. The context planner still needs
/// to explain that a visible lexical match was rejected as stale or
/// superseded. This query bypasses only the valid-time projection; callers
/// must use it solely for non-selectable trace candidates and must still run
/// the exact Knowledge PDP decision before retaining an address.
#[tracing::instrument(
    name = "store.knowledge_search.lifecycle_trace_candidates",
    skip_all,
    fields(tenant.id = %tenant_id, lifecycle = %lifecycle, limit),
    err(Display)
)]
pub async fn lifecycle_trace_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_ids: &[ScopeId],
    lifecycle: KnowledgeLifecycleState,
    query: Option<&str>,
    limit: i64,
) -> Result<Vec<Candidate>> {
    let scope_ids: Vec<uuid::Uuid> = scope_ids.iter().map(|id| id.as_uuid()).collect();
    let query = query.map(lexical_websearch_query);
    let rows = sqlx::query!(
        r#"
        select current.id as "item_id!",
               current.updated_at as "updated_at!",
               case when $2::text is null then 0.0::float8
                    else ts_rank_cd(
                        revision.search_document,
                        websearch_to_tsquery('simple'::regconfig, $2)
                    )::float8
               end as "score!"
        from knowledge_current current
        join knowledge_revisions revision
          on revision.tenant_id = current.tenant_id
         and revision.id = current.current_revision_id
        where current.tenant_id = $1
          and current.lifecycle_state = $3
          and (cardinality($4::uuid[]) = 0 or current.scope_id = any($4))
          and ($2::text is null or revision.search_document @@
               websearch_to_tsquery('simple'::regconfig, $2))
        order by 3 desc, current.updated_at desc, current.id desc
        limit $5
        "#,
        tenant_id.as_uuid(),
        query.as_deref(),
        lifecycle.as_str(),
        &scope_ids,
        limit.max(1),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| Candidate {
            item_id: KnowledgeItemId::from_uuid(row.item_id),
            updated_at: row.updated_at,
            score: row.score,
        })
        .collect())
}

/// Produces the semantic leg for one actually configured model.
///
/// The fixed-dimension dispatch matches the reviewed HNSW expressions in
/// migration 0049. An untested dimension is refused rather than silently
/// falling back to an unindexed full scan.
#[tracing::instrument(
    name = "store.knowledge_search.semantic_candidates",
    skip_all,
    fields(tenant.id = %tenant_id, model, dim = query_vector.len(), limit),
    err(Display)
)]
pub async fn semantic_candidates(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    filters: &Filters,
    model: &str,
    query_vector: &[f32],
    limit: i64,
) -> Result<Vec<Candidate>> {
    match query_vector.len() {
        16 => semantic_candidates_16(conn, tenant_id, filters, model, query_vector, limit).await,
        1024 => {
            semantic_candidates_1024(conn, tenant_id, filters, model, query_vector, limit).await
        }
        dimension => Err(Error::Invalid {
            message: format!(
                "no Knowledge ANN index for {dimension}-dimension vectors; supported: [16, 1024]"
            ),
        }),
    }
}

macro_rules! semantic_query {
    ($name:ident, $file:literal) => {
        async fn $name(
            conn: &mut PgConnection,
            tenant_id: TenantId,
            filters: &Filters,
            model: &str,
            query_vector: &[f32],
            limit: i64,
        ) -> Result<Vec<Candidate>> {
            let rows = sqlx::query_file!(
                $file,
                tenant_id.as_uuid(),
                model,
                query_vector,
                filters.workspace_uuid(),
                filters.project_uuid(),
                filters.scope_uuid(),
                filters.owner_principal_id.as_deref() as Option<&str>,
                filters.knowledge_type_name(),
                filters.origin_name(),
                filters.lifecycle_name(),
                filters.tag.as_deref() as Option<&str>,
                filters.source_name(),
                filters.updated_from,
                filters.updated_before,
                filters.stale,
                filters.at,
                limit.max(1),
                &filters.scope_uuids(),
                filters.as_known_at,
                filters.include_history,
                filters.include_transitional,
            )
            .fetch_all(&mut *conn)
            .await
            .map_err(storage_error)?;
            Ok(rows
                .into_iter()
                .map(|row| Candidate {
                    item_id: KnowledgeItemId::from_uuid(row.item_id),
                    updated_at: row.updated_at,
                    score: row.score,
                })
                .collect())
        }
    };
}

semantic_query!(semantic_candidates_16, "queries/knowledge_semantic_16.sql");
semantic_query!(
    semantic_candidates_1024,
    "queries/knowledge_semantic_1024.sql"
);

/// Finds immutable revisions this model has not embedded yet.
#[tracing::instrument(
    name = "store.knowledge_search.unembedded_revisions",
    skip_all,
    fields(tenant.id = %tenant_id, model, limit),
    err(Display)
)]
pub async fn unembedded_revisions(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    model: &str,
    limit: i64,
) -> Result<Vec<IndexableRevision>> {
    let rows = sqlx::query!(
        r#"
        select revision.id as "revision_id!",
               concat_ws(E'\n\n', revision.title, revision.summary, revision.body_markdown)
                   as "text!"
        from knowledge_revisions revision
        left join knowledge_revision_embeddings embedding
          on embedding.tenant_id = revision.tenant_id
         and embedding.knowledge_revision_id = revision.id
         and embedding.model = $2
        where revision.tenant_id = $1
          and embedding.knowledge_revision_id is null
        order by revision.transaction_time, revision.id
        limit $3
        "#,
        tenant_id.as_uuid(),
        model,
        limit.max(1),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| IndexableRevision {
            revision_id: KnowledgeRevisionId::from_uuid(row.revision_id),
            text: row.text,
        })
        .collect())
}

/// Idempotently records one immutable revision vector.
#[tracing::instrument(
    name = "store.knowledge_search.insert_embedding",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.revision.id = %revision_id, model, dim = vector.len()),
    err(Display)
)]
pub async fn insert_embedding(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    revision_id: KnowledgeRevisionId,
    model: &str,
    vector: &[f32],
) -> Result<bool> {
    if model.trim().is_empty() || model.chars().count() > 512 {
        return Err(Error::Invalid {
            message: "embedding model must be non-blank and at most 512 characters".to_owned(),
        });
    }
    if vector.is_empty() {
        return Err(Error::Invalid {
            message: "Knowledge embedding must not be empty".to_owned(),
        });
    }
    let result = sqlx::query!(
        r#"
        insert into knowledge_revision_embeddings
            (tenant_id, knowledge_revision_id, model, dim, embedding)
        values ($1, $2, $3, $4, $5::real[]::vector)
        on conflict (tenant_id, knowledge_revision_id, model) do nothing
        "#,
        tenant_id.as_uuid(),
        revision_id.as_uuid(),
        model,
        i32::try_from(vector.len()).map_err(|_| Error::Invalid {
            message: "Knowledge embedding dimension exceeds integer range".to_owned(),
        })?,
        vector,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() == 1)
}

/// Counts one model's revision vectors. Used by health evidence and focused
/// tests without exposing vector bytes.
pub async fn embedding_count(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    model: &str,
) -> Result<i64> {
    sqlx::query_scalar!(
        r#"
        select count(*) as "count!"
        from knowledge_revision_embeddings
        where tenant_id = $1 and model = $2
        "#,
        tenant_id.as_uuid(),
        model,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)
}

/// Distinct governed source scopes attached to one revision. This is an
/// internal planning read: the gateway decides each scope before it passes the
/// resulting allowed set to `knowledge::visible_sources`.
pub async fn source_scopes(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    revision_id: KnowledgeRevisionId,
) -> Result<Vec<ScopeId>> {
    let rows = sqlx::query_scalar!(
        r#"
        select distinct source.scope_id as "scope_id!"
        from knowledge_revision_sources link
        join knowledge_sources source
          on source.tenant_id = link.tenant_id
         and source.id = link.knowledge_source_id
        where link.tenant_id = $1 and link.knowledge_revision_id = $2
        order by source.scope_id
        "#,
        tenant_id.as_uuid(),
        revision_id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(ScopeId::from_uuid).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_names_are_the_closed_domain_vocabulary() {
        let filters = Filters {
            scope_ids: Vec::new(),
            workspace_id: None,
            project_id: None,
            scope_id: None,
            owner_principal_id: None,
            knowledge_type: Some(KnowledgeType::Convention),
            origin: Some(KnowledgeOrigin::Authored),
            lifecycle: Some(KnowledgeLifecycleState::Archived),
            tag: None,
            source_type: Some(KnowledgeSourceType::Repository),
            updated_from: None,
            updated_before: None,
            stale: None,
            at: Utc::now(),
            as_known_at: Utc::now(),
            include_history: false,
            include_transitional: false,
        };
        assert_eq!(filters.knowledge_type_name(), Some("convention"));
        assert_eq!(filters.origin_name(), Some("authored"));
        assert_eq!(filters.lifecycle_name(), Some("archived"));
        assert_eq!(filters.source_name(), Some("repository"));
    }

    #[test]
    fn ordinary_task_prose_is_recall_oriented_without_question_furniture() {
        assert_eq!(
            lexical_websearch_query("what did we decide about payment retries"),
            "decide OR payment OR retries"
        );
        assert_eq!(
            lexical_websearch_query("which command do I run before pushing a branch"),
            "command OR run OR pushing OR branch"
        );
    }

    #[test]
    fn explicit_websearch_syntax_is_preserved() {
        assert_eq!(
            lexical_websearch_query("\"payment retries\" OR jitter"),
            "\"payment retries\" OR jitter"
        );
        assert_eq!(
            lexical_websearch_query("payment -vacuum"),
            "payment -vacuum"
        );
    }

    #[test]
    fn lexical_queries_are_bounded_and_deduplicated_before_postgres_parses_them() {
        let oversized = (0..200)
            .map(|index| format!("term{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let bounded = lexical_websearch_query(&oversized);
        assert_eq!(bounded.split(" OR ").count(), MAX_LEXICAL_TERMS);
        assert!(bounded.starts_with("term0 OR term1 OR term2"));
        assert!(bounded.ends_with("term31"));

        assert_eq!(
            lexical_websearch_query("payment payment payment retries retries"),
            "payment OR retries"
        );
        assert_eq!(
            lexical_websearch_query(&format!("{} useful", "x".repeat(200))),
            format!("{} OR useful", "x".repeat(MAX_LEXICAL_TERM_CHARS))
        );
    }

    #[test]
    fn oversized_explicit_or_empty_queries_degrade_to_safe_terms() {
        let oversized = (0..80)
            .map(|index| format!("\"term{index}\""))
            .collect::<Vec<_>>()
            .join(" OR ");
        let bounded = lexical_websearch_query(&oversized);
        assert_eq!(bounded.split(" OR ").count(), MAX_LEXICAL_TERMS);
        assert!(!bounded.contains('"'));

        assert_eq!(
            lexical_websearch_query("what did we do and where"),
            EMPTY_LEXICAL_QUERY
        );
        assert_eq!(lexical_websearch_query("---"), EMPTY_LEXICAL_QUERY);
    }
}
