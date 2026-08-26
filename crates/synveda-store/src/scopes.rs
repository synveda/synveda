//! The generic governed scope substrate (CPR-3, ADR-0068 decision 4,
//! ADR-0070): closure table over `scopes` / `scope_closure`, and the internal
//! services that are the only supported way to change either.
//!
//! A scope is a named node with a parent and a subtree. There is no rank: a
//! [`ScopeKind`] decides which kinds may be a scope's *parent* and nothing
//! else, `org_unit` nests inside itself to arbitrary depth, and a deployment
//! with one person has a tenant root and a principal rather than an
//! organisation containing a team. The epoch baseline enforces each
//! structural rule — most of them are database facts, and
//! this module is the layer that turns the two which need the parent row into
//! errors with a sentence in them.
//!
//! ## Services, and where governance attaches
//!
//! [`create`], [`rename`] and [`move_scope`] are the mutating services;
//! [`get`], [`children`], [`ancestors`], [`descendants`], [`tenant_root`],
//! [`path`] and [`resolve_path`] are the reads. They are **internal**: no
//! route, no CLI command and no adapter reaches them at this prompt, by
//! design — the governed entry points (a PDP decision before the call, an
//! audit event after it, VedaFlow where the change is a governed one) attach
//! at the API boundary the later prompts of this programme add, exactly as
//! they do today for the hierarchy this substrate replaces. Nothing here
//! decides authorisation, and nothing here should: a store function that
//! consulted the PDP would be a second decision point beside the one seed
//! §2.2 puts on the request path.
//!
//! ## Transactions
//!
//! Reads take any executor. [`create`] and [`move_scope`] run several
//! statements and take a connection: callers MUST wrap them in a transaction —
//! on the data path that means [`crate::rls::begin_tenant_tx`] — or a failure
//! between statements leaves the closure inconsistent with the adjacency.
//! Closure maintenance is deliberately explicit SQL here, not triggers
//! (ADR-0011 decision 2, kept).
//!
//! ## Tenancy
//!
//! Every query here filters on `tenant_id` in SQL as well as relying on the
//! forced-RLS backstop (ADR-0009). The two are not redundant: RLS binds the
//! application role on a transaction that set the GUC, and these functions are
//! also called on owner connections — migrations, break-glass, the test
//! harness — where it does not bite. A scope of another tenant is
//! indistinguishable from one that does not exist, which is the same
//! no-existence-oracle doctrine ADR-0008 set for tenants.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::access::MAX_PRINCIPAL_CHARS;
use synveda_types::scope::{
    Scope, ScopeKind, ScopeStatus, parse_path, validate_attributes, validate_display_name,
    validate_slug,
};
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId};
use uuid::Uuid;

/// Counter: scope mutations, labelled `operation` = `create` | `rename` |
/// `move`. Emitted here, described by the gateway where the recorder lives
/// (ADR-0007).
pub const SCOPE_MUTATIONS_TOTAL: &str = "synveda_scope_mutations_total";

/// What [`create`] needs to mint a scope.
///
/// The id is the caller's to choose (UUIDv7, mintable anywhere — ADR-0005),
/// because the aggregate id is stable for the scope's whole life and the
/// caller is usually about to reference it in the same transaction.
#[derive(Debug, Clone)]
pub struct NewScope {
    /// The scope's identity.
    pub id: ScopeId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// What shape of thing this is.
    pub kind: ScopeKind,
    /// Parent; `None` mints the tenant root, and only [`ScopeKind::Tenant`]
    /// may be created that way.
    pub parent_scope_id: Option<ScopeId>,
    /// Sibling-unique handle.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Open labelling bag; a JSON object.
    pub attributes: serde_json::Value,
    /// The token subject this scope belongs to. Required for
    /// [`ScopeKind::Principal`] and refused for every other kind — a CHECK
    /// says the same thing, and [`create`] says it with a sentence
    /// (CPR-6, ADR-0073 decision 2).
    pub principal_id: Option<String>,
    /// The identity creating the scope, when one is. `None` records that the
    /// deployment created it.
    pub created_by: Option<IdentityId>,
}

/// Raw row; converted with `TryFrom` so `kind` and `status` decode through the
/// `synveda-types` enums (the pattern [`crate::tenants`] set).
struct ScopeRow {
    id: Uuid,
    tenant_id: Uuid,
    kind: String,
    parent_scope_id: Option<Uuid>,
    slug: String,
    display_name: String,
    status: String,
    attributes: serde_json::Value,
    principal_id: Option<String>,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ScopeRow> for Scope {
    type Error = Error;

    fn try_from(row: ScopeRow) -> Result<Self> {
        let vocabulary = |err: Error| Error::Internal {
            message: format!("stored value outside vocabulary: {err}"),
        };
        Ok(Scope {
            id: ScopeId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            // The CHECK constraints keep these inside their vocabularies; a
            // parse failure means schema and code have drifted — a bug.
            kind: row.kind.parse().map_err(vocabulary)?,
            parent_scope_id: row.parent_scope_id.map(ScopeId::from_uuid),
            slug: row.slug,
            display_name: row.display_name,
            status: row.status.parse().map_err(vocabulary)?,
            principal_id: row.principal_id,
            attributes: row.attributes,
            created_by: row.created_by.map(IdentityId::from_uuid),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation (sibling slug, a second tenant root),
        // 23503 foreign_key_violation (the parent vanished under a concurrent
        // write), 40P01 deadlock_detected (two moves locking in opposite
        // order): all conflicts with concurrent state, retryable by the
        // caller.
        if matches!(
            db.code().as_deref(),
            Some("23505") | Some("23503") | Some("40P01")
        ) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 23514 check_violation: a value outside a vocabulary or a placement
        // the tree does not admit — the caller sent something invalid.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // P0001 raise_exception: the immutability trigger. No service here
        // writes any of the columns it guards, so this firing means the code
        // changed and the guard caught it — an application defect, classified
        // like the RLS backstop below rather than blamed on the caller. The
        // trigger's own message is the useful half and is kept verbatim.
        if db.code().as_deref() == Some("P0001") {
            return Error::Internal {
                message: db.message().to_owned(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (TEN-2, ADR-0009)
        // rejected a write for a tenant other than the transaction's GUC.
        // An application defect, never the caller's fault.
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn not_found(id: ScopeId) -> Error {
    Error::NotFound {
        entity: format!("scope {id}"),
    }
}

/// Fetches a scope with a row lock, serialising concurrent structural edits
/// against it (create-under, move).
async fn lock_scope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Option<Scope>> {
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        select id, tenant_id, kind, parent_scope_id, slug, display_name,
               status, attributes, principal_id, created_by, created_at, updated_at
        from scopes
        where id = $1 and tenant_id = $2
        for update
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Locks one known scope for the caller's transaction and returns it.
///
/// Structural writers normally call the private [`lock_scope`] inside a
/// larger operation. Cross-subsystem tenant-wide invariants use this narrow
/// primitive to serialise on the tenant root without duplicating its checked
/// query or introducing an advisory-lock namespace.
#[tracing::instrument(
    name = "store.scopes.lock_for_update",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn lock_for_update(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Scope> {
    lock_scope(conn, tenant_id, id)
        .await?
        .ok_or_else(|| not_found(id))
}

/// Locks a scope and every scope beneath it, in a deterministic order.
///
/// [`move_scope`] takes this rather than the one row lock its subject needs,
/// because the row it is about to rewrite the ancestry of is not the only row
/// that ancestry belongs to. A [`create`] under a *descendant* of the moving
/// subtree derives its closure rows from that descendant's ancestry, and a
/// create that lands between this move's unlink and its relink derives them
/// from an ancestry the move has already deleted. `create` locks its parent;
/// this is what makes the two locks meet, so the rule is *a move owns its
/// subtree for the duration* — one sentence a reviewer can check.
///
/// Being exact about what this adds, because a test cannot show it: without
/// this lock that race does not corrupt the closure either, and the reason is
/// incidental rather than designed — the relink inserts a closure row per
/// subtree member, each of which takes a foreign-key share lock on that
/// member's `scopes` row, which is enough to conflict with `create`'s `for
/// update` on its parent. The window where it is not enough (a create that
/// commits between the unlink and the relink) ends in the *move* failing on
/// the closure's primary key, so the outcome there is a spurious conflict for
/// the writer that was doing nothing wrong. This lock replaces both of those
/// accidents with an ordering.
///
/// The order (distance, then id) is fixed so that two moves inside one
/// subtree queue rather than deadlock.
async fn lock_subtree(conn: &mut PgConnection, tenant_id: TenantId, id: ScopeId) -> Result<()> {
    sqlx::query_scalar!(
        r#"
        select s.id
        from scope_closure c
        join scopes s on s.id = c.descendant_id and s.tenant_id = c.tenant_id
        where c.ancestor_id = $1 and c.tenant_id = $2
        order by c.distance, s.id
        for update of s
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Creates a scope and its closure rows.
///
/// The root (parent `None`) must be a [`ScopeKind::Tenant`] and is unique per
/// tenant; any other scope must sit under a kind its own kind permits
/// ([`ScopeKind::permits_parent`]). Fails with [`Error::Invalid`] for a
/// malformed slug, name or attribute bag and for a placement the tree does not
/// admit, [`Error::NotFound`] when the parent does not exist *in this tenant*,
/// and [`Error::Conflict`] on a sibling-slug or second-root collision.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.scopes.create",
    skip_all,
    fields(tenant.id = %new.tenant_id, scope.id = %new.id, scope.kind = %new.kind),
    err(Display)
)]
pub async fn create(conn: &mut PgConnection, new: &NewScope) -> Result<Scope> {
    validate_slug(&new.slug)?;
    validate_display_name(&new.display_name)?;
    validate_attributes(&new.attributes)?;
    validate_principal_id(new.kind, new.principal_id.as_deref())?;

    let parent_kind = match new.parent_scope_id {
        None => {
            if !new.kind.is_tenant_root() {
                return Err(Error::Invalid {
                    message: format!(
                        "only a {} scope has no parent; a {} needs one",
                        ScopeKind::Tenant,
                        new.kind
                    ),
                });
            }
            None
        }
        Some(parent_id) => {
            if new.kind.is_tenant_root() {
                return Err(Error::Invalid {
                    message: format!(
                        "the {} scope is the root and has no parent",
                        ScopeKind::Tenant
                    ),
                });
            }
            // Another tenant's scope is indistinguishable from a missing one:
            // no existence oracle across tenants (ADR-0008).
            let parent = lock_scope(&mut *conn, new.tenant_id, parent_id)
                .await?
                .ok_or_else(|| not_found(parent_id))?;
            if !new.kind.permits_parent(parent.kind) {
                return Err(Error::Invalid {
                    message: format!(
                        "a {} cannot sit under a {}; permitted parents: {}",
                        new.kind,
                        parent.kind,
                        describe_kinds(new.kind.permitted_parents()),
                    ),
                });
            }
            Some(parent.kind)
        }
    };

    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        insert into scopes
            (id, tenant_id, kind, parent_scope_id, parent_kind, slug, display_name,
             status, attributes, principal_id, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        returning id, tenant_id, kind, parent_scope_id, slug, display_name,
                  status, attributes, principal_id, created_by, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.kind.as_str(),
        new.parent_scope_id.map(|parent| parent.as_uuid()) as Option<Uuid>,
        parent_kind.map(|kind| kind.as_str()) as Option<&str>,
        new.slug,
        new.display_name,
        ScopeStatus::Active.as_str(),
        new.attributes,
        new.principal_id.as_deref() as Option<&str>,
        new.created_by.map(|by| by.as_uuid()) as Option<Uuid>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Self-row plus one row per ancestor, derived from the parent's own
    // ancestry (empty when $2 is null — the root case).
    sqlx::query!(
        r#"
        insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
        select c.tenant_id, c.ancestor_id, $1::uuid, c.distance + 1
          from scope_closure c
         where c.descendant_id = $2 and c.tenant_id = $3
        union all
        select $3::uuid, $1::uuid, $1::uuid, 0
        "#,
        new.id.as_uuid(),
        new.parent_scope_id.map(|parent| parent.as_uuid()) as Option<Uuid>,
        new.tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(SCOPE_MUTATIONS_TOTAL, "operation" => "create").increment(1);
    row.try_into()
}

/// The `principal_id` rule, in the store as well as in the CHECK: present
/// exactly on a `principal`-shaped scope, and a non-blank subject when it is.
///
/// Said twice deliberately. The CHECK is what holds when something reaches the
/// table another way; this is what turns "constraint violated" into a sentence
/// naming which of the two directions was wrong.
fn validate_principal_id(kind: ScopeKind, principal_id: Option<&str>) -> Result<()> {
    match (kind, principal_id) {
        (ScopeKind::Principal, None) => Err(Error::Invalid {
            message: "a principal scope must name the subject it belongs to".to_owned(),
        }),
        (kind, Some(_)) if kind != ScopeKind::Principal => Err(Error::Invalid {
            message: format!(
                "a {kind} scope belongs to nobody in particular; principal_id is only for a {} scope",
                ScopeKind::Principal
            ),
        }),
        (ScopeKind::Principal, Some(subject)) => {
            if subject.trim().is_empty() {
                return Err(Error::Invalid {
                    message: "principal id must not be blank".to_owned(),
                });
            }
            if subject.chars().count() > MAX_PRINCIPAL_CHARS {
                return Err(Error::Invalid {
                    message: format!(
                        "principal id must be at most {MAX_PRINCIPAL_CHARS} characters"
                    ),
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The slug a principal scope gets.
///
/// A token subject is not a slug — it can be an email, an `auth0|…` string or
/// a UUID — and it must not become one anyway: a slug is half of a path
/// somebody may write down, and putting somebody's login in a shared path is a
/// disclosure nobody asked for. So the slug is a digest, and the subject lives
/// in its own column where the unique index can hold it.
#[must_use]
pub fn principal_slug(principal_id: &str) -> String {
    let digest = blake3::hash(principal_id.as_bytes());
    format!("p-{}", &digest.to_hex().as_str()[..16])
}

/// Fetches the scope belonging to `principal_id`, if one has been minted.
#[tracing::instrument(
    name = "store.scopes.principal_scope",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn principal_scope(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    principal_id: &str,
) -> Result<Option<Scope>> {
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        select id, tenant_id, kind, parent_scope_id, slug, display_name,
               status, attributes, principal_id, created_by, created_at, updated_at
        from scopes
        where tenant_id = $1 and principal_id = $2
        "#,
        tenant_id.as_uuid(),
        principal_id,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Returns the scope belonging to `principal_id`, minting it — and the tenant
/// root it hangs off — if it is not there yet.
///
/// [`ensure_tenant_root`]'s shape, one level down, and for the same reason: a
/// person's own scope is the one nobody thinks to create, and it is what makes
/// "my own notes" expressible before they have joined anything. A principal
/// scope hangs directly off the tenant root
/// ([`ScopeKind::permits_parent`]), so this is a root plus one row.
///
/// **Nothing above it reaches in** — that is
/// [`synveda_types::access::inherits_into`], applied by the anchor resolver
/// and restated by the PDP's base layer. Minting one therefore confers
/// nothing on anybody else, which is why it is safe to do on demand.
///
/// Concurrency: two callers racing both try the insert and the
/// `scopes_one_per_principal` unique index admits one. The loser gets
/// [`Error::Conflict`] and retries, exactly as [`ensure_tenant_root`] does —
/// a transaction that has already written cannot swallow a conflict.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.scopes.ensure_principal_scope",
    skip_all,
    fields(tenant.id = %tenant_id, scope.created = tracing::field::Empty),
    err(Display)
)]
pub async fn ensure_principal_scope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    principal_id: &str,
    display_name: &str,
) -> Result<Scope> {
    if let Some(scope) = principal_scope(&mut *conn, tenant_id, principal_id).await? {
        tracing::Span::current().record("scope.created", false);
        return Ok(scope);
    }
    let root = ensure_tenant_root(&mut *conn, tenant_id).await?;
    let display_name = if display_name.trim().is_empty() {
        principal_id
    } else {
        display_name
    };
    // A subject can be longer than a display name may be, and a display name
    // is not an identifier — truncating one is a cosmetic loss, refusing to
    // mint somebody's own scope over it would not be.
    let display_name: String = display_name.trim().chars().take(200).collect();
    let new = NewScope {
        id: ScopeId::new(),
        tenant_id,
        kind: ScopeKind::Principal,
        parent_scope_id: Some(root.id),
        slug: principal_slug(principal_id),
        display_name,
        attributes: serde_json::json!({}),
        principal_id: Some(principal_id.to_owned()),
        // No author: nobody creates somebody else's own scope.
        created_by: None,
    };
    match create(&mut *conn, &new).await {
        Ok(scope) => {
            // **Your own scope is yours** (CPR-7, ADR-0074 decision 8), and
            // it is a *grant* that says so rather than a clause in every
            // permit. A principal scope inherits nothing (ADR-0072), so
            // without this row the person whose memory it is holds no role
            // key there and cannot publish, propose about or govern their
            // own material — while the privacy floor happily lets them
            // read it. That gap is not a policy anybody wrote; it is the
            // absence of the one grant the model already mints for every
            // other thing somebody owns (CPR-5: creating a workspace or a
            // project mints an `owner` grant for its creator). The scope
            // and the grant land in one transaction, like every other
            // subtype's.
            crate::access::create_grant(
                &mut *conn,
                &crate::access::NewGrant {
                    id: synveda_types::GrantId::new(),
                    tenant_id,
                    scope_id: scope.id,
                    subject: synveda_types::access::GrantSubject::Principal {
                        principal_id: principal_id.to_owned(),
                    },
                    role_key: synveda_types::access::RoleKey::Owner,
                    source: synveda_types::access::GrantSource::Owner,
                    invite_id: None,
                    granted_by: None,
                },
            )
            .await?;
            tracing::Span::current().record("scope.created", true);
            Ok(scope)
        }
        Err(Error::Conflict { .. }) => Err(Error::Conflict {
            message: format!("the principal scope in tenant {tenant_id} was created concurrently"),
        }),
        Err(other) => Err(other),
    }
}

/// Renders a kind list for a refusal. A refusal that says only "no" makes the
/// caller read the schema; this says what would have been legal.
fn describe_kinds(kinds: &[ScopeKind]) -> String {
    if kinds.is_empty() {
        return "nothing".to_owned();
    }
    kinds
        .iter()
        .map(ScopeKind::as_str)
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Fetches one scope.
#[tracing::instrument(
    name = "store.scopes.get",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Option<Scope>> {
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        select id, tenant_id, kind, parent_scope_id, slug, display_name,
               status, attributes, principal_id, created_by, created_at, updated_at
        from scopes
        where id = $1 and tenant_id = $2
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Returns the tenant's root scope, creating it from the tenant's own slug
/// and name if it is not there yet.
///
/// The root is the one scope nobody asks for: a person creating their first
/// workspace has no reason to create a tenant root first. The first operation
/// that needs a parent therefore mints it from the tenant isolation boundary
/// (ADR-0068 decision 3; ADR-0070).
///
/// Concurrency: two callers racing both try the insert and the
/// one-root-per-tenant unique index admits one. The loser sees
/// [`Error::Conflict`] and re-reads the winner's row, which is why this takes
/// a connection and reports the outcome rather than simply returning the
/// scope — a caller inside a transaction that has already written cannot
/// swallow a conflict, and this one does not hide that it happened.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.scopes.ensure_tenant_root",
    skip_all,
    fields(tenant.id = %tenant_id, scope.created = tracing::field::Empty),
    err(Display)
)]
pub async fn ensure_tenant_root(conn: &mut PgConnection, tenant_id: TenantId) -> Result<Scope> {
    if let Some(root) = tenant_root(&mut *conn, tenant_id).await? {
        tracing::Span::current().record("scope.created", false);
        return Ok(root);
    }
    let tenant = crate::tenants::by_id(&mut *conn, tenant_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("tenant {tenant_id}"),
        })?;
    let new = NewScope {
        id: ScopeId::new(),
        tenant_id,
        kind: ScopeKind::Tenant,
        parent_scope_id: None,
        slug: tenant.slug.clone(),
        display_name: tenant.name.clone(),
        attributes: serde_json::json!({}),
        principal_id: None,
        // No author: the deployment created it, and inventing a synthetic one
        // would lose that distinction (0040's header).
        created_by: None,
    };
    match create(&mut *conn, &new).await {
        Ok(root) => {
            tracing::Span::current().record("scope.created", true);
            Ok(root)
        }
        Err(Error::Conflict { .. }) => {
            // The unique index admitted somebody else. Their row is the root;
            // this transaction is now poisoned, so the caller retries.
            Err(Error::Conflict {
                message: format!("tenant {tenant_id} root scope was created concurrently"),
            })
        }
        Err(other) => Err(other),
    }
}
/// Replaces the open labelling bag. Validated by the same rules as
/// creation; never an authorisation input (ADR-0070).
#[tracing::instrument(
    name = "store.scopes.set_attributes",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn set_attributes(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: ScopeId,
    attributes: &serde_json::Value,
) -> Result<Scope> {
    validate_attributes(attributes)?;
    let scope = sqlx::query_as!(
        ScopeRow,
        r#"
        update scopes
           set attributes = $3, updated_at = now()
         where tenant_id = $1 and id = $2
        returning id, tenant_id, kind, parent_scope_id, slug, display_name,
                  status, attributes, principal_id, created_by, created_at, updated_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        attributes,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::NotFound {
        entity: format!("scope {id}"),
    })?;
    TryInto::try_into(scope)
}

/// Sets a scope's status. The one transition in the substrate, and it exists
/// because a subtype's lifecycle and its scope's must not disagree: an
/// archived workspace whose scope still reads `active` would compose, resolve
/// and accept writes exactly as before.
///
/// Returns [`Error::NotFound`] for a scope that is not this tenant's.
#[tracing::instrument(
    name = "store.scopes.set_status",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id, scope.status = %status),
    err(Display)
)]
pub async fn set_status(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
    status: ScopeStatus,
) -> Result<Scope> {
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        update scopes
           set status = $3, updated_at = now()
         where id = $1 and tenant_id = $2
        returning id, tenant_id, kind, parent_scope_id, slug, display_name,
                  status, attributes, principal_id, created_by, created_at, updated_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        status.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    let scope: Scope = row.ok_or_else(|| not_found(id))?.try_into()?;
    metrics::counter!(SCOPE_MUTATIONS_TOTAL, "operation" => "status").increment(1);
    Ok(scope)
}

/// Fetches a tenant's root scope, if one has been created.
#[tracing::instrument(
    name = "store.scopes.tenant_root",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn tenant_root(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<Scope>> {
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        select id, tenant_id, kind, parent_scope_id, slug, display_name,
               status, attributes, principal_id, created_by, created_at, updated_at
        from scopes
        where tenant_id = $1 and parent_scope_id is null
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists a scope's direct children, ordered by slug.
#[tracing::instrument(
    name = "store.scopes.children",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn children(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Vec<Scope>> {
    let rows = sqlx::query_as!(
        ScopeRow,
        r#"
        select id, tenant_id, kind, parent_scope_id, slug, display_name,
               status, attributes, principal_id, created_by, created_at, updated_at
        from scopes
        where parent_scope_id = $1 and tenant_id = $2
        order by slug
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists a scope's ancestors, nearest first (parent, …, tenant root),
/// excluding the scope itself. One closure index scan.
#[tracing::instrument(
    name = "store.scopes.ancestors",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn ancestors(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Vec<Scope>> {
    let rows = sqlx::query_as!(
        ScopeRow,
        r#"
        select s.id, s.tenant_id, s.kind, s.parent_scope_id, s.slug, s.display_name,
               s.status, s.attributes, s.principal_id, s.created_by, s.created_at, s.updated_at
        from scope_closure c
        join scopes s on s.id = c.ancestor_id and s.tenant_id = c.tenant_id
        where c.descendant_id = $1 and c.tenant_id = $2 and c.distance > 0
        order by c.distance
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Lists a scope's whole subtree, excluding the scope itself, in a stable
/// order (nearest first, then by slug). One closure index scan.
#[tracing::instrument(
    name = "store.scopes.descendants",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn descendants(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Vec<Scope>> {
    let rows = sqlx::query_as!(
        ScopeRow,
        r#"
        select s.id, s.tenant_id, s.kind, s.parent_scope_id, s.slug, s.display_name,
               s.status, s.attributes, s.principal_id, s.created_by, s.created_at, s.updated_at
        from scope_closure c
        join scopes s on s.id = c.descendant_id and s.tenant_id = c.tenant_id
        where c.ancestor_id = $1 and c.tenant_id = $2 and c.distance > 0
        order by c.distance, s.slug
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The scope's path: the slug chain from the tenant root, root first
/// (`acme/platform/payments`). `None` for a scope that does not exist in this
/// tenant.
///
/// Derived from the closure on every call rather than materialised on the row.
/// The old hierarchy stored a `path` column and rewrote every descendant's
/// copy on a move; a derived path cannot be stale, and this is one index scan
/// of the same rows an ancestor query already walks.
#[tracing::instrument(
    name = "store.scopes.path",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn path(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
) -> Result<Option<String>> {
    let slugs: Vec<String> = sqlx::query_scalar!(
        r#"
        select s.slug as "slug!"
        from scope_closure c
        join scopes s on s.id = c.ancestor_id and s.tenant_id = c.tenant_id
        where c.descendant_id = $1 and c.tenant_id = $2
        order by c.distance desc
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    if slugs.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        slugs.join(&synveda_types::scope::PATH_SEPARATOR.to_string()),
    ))
}

/// Resolves a path (`acme/platform/payments`) to the scope it names, walking
/// from the tenant root. `None` when no scope in this tenant sits at that
/// path.
///
/// The first segment is the tenant root's own slug, so a path is a complete
/// address rather than one relative to a root the caller has to know. One
/// statement: a recursive walk down the adjacency, matching a slug per level,
/// so nothing can change under the walk between segments.
///
/// # Errors
///
/// [`Error::Invalid`] when the path is empty or holds a segment that is not a
/// slug — a malformed address is a mistake, not a miss.
#[tracing::instrument(
    name = "store.scopes.resolve_path",
    skip_all,
    fields(tenant.id = %tenant_id, scope.path = path),
    err(Display)
)]
pub async fn resolve_path(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    path: &str,
) -> Result<Option<Scope>> {
    let segments: Vec<String> = parse_path(path)?
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();

    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        with recursive walk as (
            select s.id, 1 as depth
              from scopes s
             where s.tenant_id = $1
               and s.parent_scope_id is null
               and s.slug = ($2::text[])[1]
            union all
            select s.id, w.depth + 1
              from walk w
              join scopes s
                on s.parent_scope_id = w.id
               and s.tenant_id = $1
               and s.slug = ($2::text[])[w.depth + 1]
        )
        select s.id, s.tenant_id, s.kind, s.parent_scope_id, s.slug, s.display_name,
               s.status, s.attributes, s.principal_id, s.created_by, s.created_at, s.updated_at
        from walk w
        join scopes s on s.id = w.id and s.tenant_id = $1
        where w.depth = cardinality($2::text[])
        "#,
        tenant_id.as_uuid(),
        &segments,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Renames a scope: `display_name` and nothing else. Slugs are immutable, in
/// the database as well as here — a path somebody wrote down is half slugs.
/// Returns [`Error::NotFound`] for a scope that is not this tenant's.
#[tracing::instrument(
    name = "store.scopes.rename",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id),
    err(Display)
)]
pub async fn rename(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ScopeId,
    display_name: &str,
) -> Result<Scope> {
    validate_display_name(display_name)?;
    let row = sqlx::query_as!(
        ScopeRow,
        r#"
        update scopes
           set display_name = $3, updated_at = now()
         where id = $1 and tenant_id = $2
        returning id, tenant_id, kind, parent_scope_id, slug, display_name,
                  status, attributes, principal_id, created_by, created_at, updated_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        display_name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    let scope: Scope = row.ok_or_else(|| not_found(id))?.try_into()?;
    metrics::counter!(SCOPE_MUTATIONS_TOTAL, "operation" => "rename").increment(1);
    Ok(scope)
}

/// Moves a scope, and its whole subtree, under a new parent: closure surgery
/// inside the caller's transaction.
///
/// Eligibility, in the order it is checked: the scope exists in this tenant;
/// it is not the tenant root (which has no parent and therefore no move); the
/// destination exists in this tenant; the destination's kind is one this
/// scope's kind permits; and the destination is not the scope itself or
/// anything beneath it. The last of those is what makes a cycle an error with
/// a sentence in it — the closure's own CHECK makes it impossible either way
/// (the `scopes` cycle constraint in the epoch baseline).
///
/// A [`ScopeKind::Principal`] passes all five only when the destination is the
/// tenant root, which is where it already is: its move is a no-op by
/// construction rather than by a special case.
///
/// Must run inside a transaction (see module docs).
#[tracing::instrument(
    name = "store.scopes.move",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %id, scope.new_parent = %new_parent_id),
    err(Display)
)]
pub async fn move_scope(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: ScopeId,
    new_parent_id: ScopeId,
) -> Result<Scope> {
    // The subject's own row first — it is the existence check and it carries
    // the kind the eligibility rules are about — then everything beneath it,
    // before any of those rules run: a refusal that released the subtree it had
    // half-locked would be a slower way of doing nothing.
    let scope = lock_scope(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    lock_subtree(&mut *conn, tenant_id, id).await?;

    if scope.kind.is_tenant_root() {
        return Err(Error::Invalid {
            message: format!("the {} root scope cannot move", ScopeKind::Tenant),
        });
    }
    if new_parent_id == id {
        return Err(Error::Invalid {
            message: "a scope cannot move under itself".to_owned(),
        });
    }
    let parent = lock_scope(&mut *conn, tenant_id, new_parent_id)
        .await?
        .ok_or_else(|| not_found(new_parent_id))?;
    if !scope.kind.permits_parent(parent.kind) {
        return Err(Error::Invalid {
            message: format!(
                "a {} cannot sit under a {}; permitted parents: {}",
                scope.kind,
                parent.kind,
                describe_kinds(scope.kind.permitted_parents()),
            ),
        });
    }
    let descends = sqlx::query_scalar!(
        r#"
        select exists(
            select 1 from scope_closure
            where ancestor_id = $1 and descendant_id = $2 and tenant_id = $3
        ) as "descends!"
        "#,
        id.as_uuid(),
        new_parent_id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    if descends {
        return Err(Error::Invalid {
            message: "a scope cannot move under its own descendant".to_owned(),
        });
    }

    // Adjacency first: a sibling-slug collision at the destination fails here,
    // before any closure surgery. `parent_kind` moves with the pointer — the
    // composite foreign key would refuse the row otherwise, which is the point
    // of carrying it.
    sqlx::query!(
        r#"
        update scopes
           set parent_scope_id = $3, parent_kind = $4, updated_at = now()
         where id = $1 and tenant_id = $2
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        new_parent_id.as_uuid(),
        parent.kind.as_str(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Unlink: drop every closure row that ties an outside ancestor to the
    // subtree. Rows internal to the subtree (ancestor inside it) survive.
    sqlx::query!(
        r#"
        delete from scope_closure
        where tenant_id = $2
          and descendant_id in
                (select descendant_id from scope_closure
                  where ancestor_id = $1 and tenant_id = $2)
          and ancestor_id not in
                (select descendant_id from scope_closure
                  where ancestor_id = $1 and tenant_id = $2)
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    // Relink: cross-join the new parent's ancestry (self-row included) with
    // the subtree (self-row included).
    sqlx::query!(
        r#"
        insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
        select super.tenant_id, super.ancestor_id, sub.descendant_id,
               super.distance + sub.distance + 1
        from scope_closure super
        cross join scope_closure sub
        where super.descendant_id = $2
          and sub.ancestor_id = $1
          and super.tenant_id = $3
          and sub.tenant_id = $3
        "#,
        id.as_uuid(),
        new_parent_id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(SCOPE_MUTATIONS_TOTAL, "operation" => "move").increment(1);
    get(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("scope {id} vanished mid-move"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_names_the_placements_that_would_have_been_legal() {
        assert_eq!(
            describe_kinds(ScopeKind::Project.permitted_parents()),
            "workspace"
        );
        assert_eq!(
            describe_kinds(ScopeKind::OrgUnit.permitted_parents()),
            "tenant or org_unit"
        );
        assert_eq!(
            describe_kinds(ScopeKind::Tenant.permitted_parents()),
            "nothing"
        );
    }
}
