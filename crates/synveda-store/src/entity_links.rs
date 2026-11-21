//! Business links between Knowledge items and their ends (ADR-0164). Both
//! tables are append-only: ending a link adds an end row, so a read at any
//! transaction time sees exactly the rows recorded by then.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::knowledge::{EntityLink, EntityPredicate, validate_knowledge_principal};
use synveda_types::{EntityLinkId, Error, KnowledgeItemId, KnowledgeRevisionId, Result, TenantId};
use uuid::Uuid;

use crate::knowledge::{lock_items, storage_error, stored};

/// Most links an item can have in its lifetime, ended ones included. Rows
/// are append-only and any instant can be read, so the bound is on what was
/// ever recorded; an item's links read then never has to refuse a reader
/// over links they cannot see.
pub const MAX_ITEM_LINKS: i64 = 1_000;

/// A link to record.
pub struct NewEntityLink<'a> {
    /// Stable link id.
    pub id: EntityLinkId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Item asserting the link.
    pub source_item_id: KnowledgeItemId,
    /// Item the link points to.
    pub target_item_id: KnowledgeItemId,
    /// Source revision current when the link is asserted.
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Governed predicate.
    pub predicate: EntityPredicate,
    /// Start of the valid-time window.
    pub valid_from: DateTime<Utc>,
    /// Actor asserting the link.
    pub created_by: &'a str,
}

struct LinkRow {
    id: Uuid,
    tenant_id: Uuid,
    source_item_id: Uuid,
    target_item_id: Uuid,
    asserting_revision_id: Uuid,
    predicate: String,
    valid_from: DateTime<Utc>,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
}

impl TryFrom<LinkRow> for EntityLink {
    type Error = synveda_types::Error;

    fn try_from(row: LinkRow) -> Result<Self> {
        Ok(Self {
            id: EntityLinkId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            source_item_id: KnowledgeItemId::from_uuid(row.source_item_id),
            target_item_id: KnowledgeItemId::from_uuid(row.target_item_id),
            asserting_revision_id: KnowledgeRevisionId::from_uuid(row.asserting_revision_id),
            predicate: stored(&row.predicate)?,
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            created_by: row.created_by,
            created_at: row.created_at,
            ended_at: row.ended_at,
        })
    }
}

/// Records a link unless either item already has [`MAX_ITEM_LINKS`]. Both
/// items are locked first, so their counts hold until the commit. The
/// database refuses a self-link, an unknown predicate and an asserting
/// revision that is not the source's.
pub async fn insert(conn: &mut PgConnection, new: &NewEntityLink<'_>) -> Result<()> {
    validate_knowledge_principal(Some(new.created_by), "link creator")?;
    let ends = [new.source_item_id, new.target_item_id];
    lock_items(conn, new.tenant_id, &ends).await?;
    if let Some(full) = full_item(conn, new.tenant_id, &ends).await? {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge item {full} already has {MAX_ITEM_LINKS} business links, the most an \
                 item can have"
            ),
        });
    }
    sqlx::query!(
        r#"
        insert into entity_links
            (id, tenant_id, source_item_id, target_item_id, asserting_revision_id,
             predicate, valid_from, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.source_item_id.as_uuid(),
        new.target_item_id.as_uuid(),
        new.asserting_revision_id.as_uuid(),
        new.predicate.as_str(),
        new.valid_from,
        new.created_by,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// The first of the items that already has [`MAX_ITEM_LINKS`] links. Each
/// branch counts on its own index, and a link never joins an item to itself,
/// so no link is counted twice.
async fn full_item(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_ids: &[KnowledgeItemId],
) -> Result<Option<KnowledgeItemId>> {
    let ids: Vec<Uuid> = item_ids.iter().map(|id| id.as_uuid()).collect();
    let full = sqlx::query_scalar!(
        r#"
        select item.id as "id!"
          from unnest($2::uuid[]) as item(id)
         where (select count(*)
                  from (select 1
                          from entity_links link
                         where link.tenant_id = $1 and link.source_item_id = item.id
                        union all
                        select 1
                          from entity_links link
                         where link.tenant_id = $1 and link.target_item_id = item.id
                         limit $3) touching) >= $3
         order by item.id
         limit 1
        "#,
        tenant_id.as_uuid(),
        &ids,
        MAX_ITEM_LINKS,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(full.map(KnowledgeItemId::from_uuid))
}

/// One link with its end, if one is recorded.
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    link_id: EntityLinkId,
) -> Result<Option<EntityLink>> {
    let row = sqlx::query_as!(
        LinkRow,
        r#"
        select link.id, link.tenant_id, link.source_item_id, link.target_item_id,
               link.asserting_revision_id, link.predicate, link.valid_from,
               link.created_by, link.created_at,
               link_end.valid_to as "valid_to?", link_end.created_at as "ended_at?"
          from entity_links link
          left join entity_link_ends link_end
            on link_end.tenant_id = link.tenant_id and link_end.link_id = link.id
         where link.tenant_id = $1 and link.id = $2
        "#,
        tenant_id.as_uuid(),
        link_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Records the end of a link. `false` means nothing was written: the link
/// is absent, already ended, or `valid_to` would not fall after its start.
pub async fn end(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    link_id: EntityLinkId,
    valid_to: DateTime<Utc>,
    ended_by: &str,
) -> Result<bool> {
    validate_knowledge_principal(Some(ended_by), "link ender")?;
    let ended = sqlx::query!(
        r#"
        insert into entity_link_ends (tenant_id, link_id, valid_to, ended_by)
        select link.tenant_id, link.id, $3, $4
          from entity_links link
         where link.tenant_id = $1 and link.id = $2 and link.valid_from < $3
        on conflict do nothing
        "#,
        tenant_id.as_uuid(),
        link_id.as_uuid(),
        valid_to,
        ended_by,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(ended.rows_affected() == 1)
}

/// At most `limit` links into or out of an item that were valid at `as_of`,
/// as recorded by `as_known_at`, or as recorded now when it is absent. An
/// end recorded later is not yet known, so its link reads as open.
pub async fn for_item(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    as_of: DateTime<Utc>,
    as_known_at: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<EntityLink>> {
    let rows = sqlx::query_as!(
        LinkRow,
        r#"
        select link.id, link.tenant_id, link.source_item_id, link.target_item_id,
               link.asserting_revision_id, link.predicate, link.valid_from,
               link.created_by, link.created_at,
               link_end.valid_to as "valid_to?", link_end.created_at as "ended_at?"
          from entity_links link
          left join entity_link_ends link_end
            on link_end.tenant_id = link.tenant_id and link_end.link_id = link.id
           and ($4::timestamptz is null or link_end.created_at <= $4)
         where link.tenant_id = $1
           and (link.source_item_id = $2 or link.target_item_id = $2)
           and ($4::timestamptz is null or link.created_at <= $4)
           and link.valid_from <= $3
           and (link_end.valid_to is null or link_end.valid_to > $3)
         order by link.created_at, link.id
         limit $5
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        as_of,
        as_known_at,
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}
