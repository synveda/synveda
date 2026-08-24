select current.id as "item_id!",
       current.updated_at as "updated_at!",
       (1.0 - (embedding.embedding::vector(16) <=> $3::real[]::vector(16)))::float8
           as "score!"
from knowledge_revision_embeddings embedding
join knowledge_current current
  on current.tenant_id = embedding.tenant_id
 and current.current_revision_id = embedding.knowledge_revision_id
where embedding.tenant_id = $1
  and embedding.model = $2
  and embedding.dim = 16
  and ($4::uuid is null or exists (
      select 1
      from workspaces workspace
      join scope_closure closure
        on closure.tenant_id = workspace.tenant_id
       and closure.ancestor_id = workspace.scope_id
       and closure.descendant_id = current.scope_id
      where workspace.tenant_id = current.tenant_id
        and workspace.id = $4
  ))
  and ($5::uuid is null or current.project_id = $5)
  and ($6::uuid is null or exists (
      select 1 from scope_closure closure
      where closure.tenant_id = current.tenant_id
        and closure.ancestor_id = $6
        and closure.descendant_id = current.scope_id
  ))
  and ($7::text is null or current.owner_principal_id = $7)
  and ($8::text is null or current.knowledge_type = $8)
  and ($9::text is null or current.origin = $9)
  and (($10::text is null and current.lifecycle_state = 'active')
       or current.lifecycle_state = $10)
  and ($11::text is null or $11 = any(current.tags))
  and ($12::text is null or exists (
      select 1
      from knowledge_revision_sources link
      join knowledge_sources source
        on source.tenant_id = link.tenant_id
       and source.id = link.knowledge_source_id
      where link.tenant_id = current.tenant_id
        and link.knowledge_revision_id = current.current_revision_id
        and source.source_type = $12
  ))
  and ($13::timestamptz is null or current.updated_at >= $13)
  and ($14::timestamptz is null or current.updated_at < $14)
  and ($15::bool is null or $15 = (
      current.lifecycle_state = 'stale'
      or (current.stale_after is not null and current.stale_after <= $16)
  ))
  and current.valid_from <= $16
  and (current.valid_to is null or $16 < current.valid_to)
  and (cardinality($18::uuid[]) = 0 or current.scope_id = any($18))
order by embedding.embedding::vector(16) <=> $3::real[]::vector(16),
         current.updated_at desc,
         current.id desc
limit $17
