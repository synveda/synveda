//! Membership and access assignment (CPR-5, ADR-0072): groups, scope grants
//! and invitations, and the resolution that turns them into "who may act
//! here".
//!
//! ## What this module decides, and what it does not
//!
//! It decides **who holds which role key where**. It decides nothing about
//! what a role key permits — that is the policy pack's, and there is
//! deliberately no permission table in this schema for it to disagree with
//! (ADR-0072 decision 2). Like [`crate::scopes`] and [`crate::workspaces`],
//! nothing here consults the PDP and nothing here chains an audit event: the
//! decision goes in front of the call and the event goes in the same
//! transaction, both at the gateway, because seed §2.2 puts exactly one
//! decision point on the request path.
//!
//! ## Inheritance is the scope tree
//!
//! [`members_of`] walks `scope_closure` upward, so a grant at a workspace's
//! scope is held at every project inside it — one row, resolved at read time.
//! Nothing materialises a per-project copy: a derived set is a set that can be
//! stale, and the copy would have to be repaired every time a scope moved.
//!
//! The one place the walk stops is a `principal`-shaped scope, which is
//! somebody's own. That rule is [`synveda_types::access::inherits_into`] and
//! it is applied here, in SQL, so a caller cannot forget it.
//!
//! ## A group is resolved, never expanded
//!
//! A grant whose subject is a group produces one entry per member at read
//! time. Adding somebody to a group therefore gives them everything the group
//! holds, everywhere, with no fan-out to keep consistent. An **archived** group
//! resolves to nobody — retiring a group is how a deployment withdraws what it
//! confers without hunting down every grant that names it.
//!
//! ## Transactions
//!
//! Reads take any executor. The mutations that run more than one statement —
//! [`update_group`], [`remove_member`], [`accept_invite`] — take a connection
//! and MUST be wrapped in a transaction; on the data path that means
//! [`crate::rls::begin_tenant_tx`].
//!
//! ## Tenancy
//!
//! Every query filters on `tenant_id` in SQL as well as relying on the
//! forced-RLS backstop, for [`crate::scopes`]'s reason: these functions also
//! run on owner connections — migrations, break-glass, the test harness —
//! where RLS does not bite. Another tenant's group, grant or invitation reads
//! as absent rather than forbidden (ADR-0008).

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::access::{
    GrantSource, GrantSubject, Group, GroupMember, GroupSource, InviteStatus, PendingInvite,
    RoleKey, ScopeGrant, SubjectKind, validate_directory_ref, validate_email,
    validate_principal_id,
};
use synveda_types::scope::{validate_display_name, validate_slug};
use synveda_types::workspace::{LifecycleStatus, validate_description};
use synveda_types::{Error, GrantId, GroupId, InviteId, Result, ScopeId, TenantId};
use uuid::Uuid;

/// Counter: access-plane mutations, labelled `object` = `group` | `membership`
/// | `grant` | `invite` and `operation` = `create` | `update` | `revoke` |
/// `accept`. Emitted here, described by the gateway where the recorder lives
/// (ADR-0007).
pub const ACCESS_MUTATIONS_TOTAL: &str = "synveda_access_mutations_total";

// ── Errors ───────────────────────────────────────────────────────────────────

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
///
/// The classification [`crate::scopes`] and [`crate::workspaces`] use: a unique
/// or foreign-key violation is a conflict with concurrent state, a check
/// violation is a caller who sent something invalid, and a trigger firing is an
/// application defect rather than the caller's fault.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        if matches!(
            db.code().as_deref(),
            Some("23505") | Some("23503") | Some("40P01")
        ) {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("P0001") {
            return Error::Internal {
                message: db.message().to_owned(),
            };
        }
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn group_not_found(id: GroupId) -> Error {
    Error::NotFound {
        entity: format!("group {id}"),
    }
}

fn grant_not_found(id: GrantId) -> Error {
    Error::NotFound {
        entity: format!("grant {id}"),
    }
}

fn invite_not_found(id: InviteId) -> Error {
    Error::NotFound {
        entity: format!("invitation {id}"),
    }
}

/// The refusal a directory-managed row gets.
///
/// It names the directory rather than saying "forbidden", because the caller
/// is not being denied an authority — they are being told the change would be
/// undone. A person who deleted a directory grant and watched it return on the
/// next sync learns that revocation in this product is unreliable; a person who
/// reads this learns where to make the change.
fn directory_managed(what: &str) -> Error {
    Error::Conflict {
        message: format!(
            "{what} is managed by a directory: change it there. Editing it here \
             would be reverted by the next sync, which is worse than refusing it"
        ),
    }
}

// ── Rows ─────────────────────────────────────────────────────────────────────

struct GroupRow {
    id: Uuid,
    tenant_id: Uuid,
    slug: String,
    display_name: String,
    description: Option<String>,
    source: String,
    directory_ref: Option<String>,
    status: String,
    revision: i64,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// A stored value outside its vocabulary means the schema and the code have
/// drifted — a bug here, never a caller's fault.
fn decoded<T: std::str::FromStr<Err = Error>>(value: &str) -> Result<T> {
    value.parse().map_err(|err| Error::Internal {
        message: format!("stored value outside vocabulary: {err}"),
    })
}

impl TryFrom<GroupRow> for Group {
    type Error = Error;

    fn try_from(row: GroupRow) -> Result<Self> {
        Ok(Group {
            id: GroupId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            slug: row.slug,
            display_name: row.display_name,
            description: row.description,
            source: decoded::<GroupSource>(&row.source)?,
            directory_ref: row.directory_ref,
            status: decoded::<LifecycleStatus>(&row.status)?,
            revision: row.revision,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

struct GrantRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    subject_kind: String,
    principal_id: Option<String>,
    group_id: Option<Uuid>,
    role_key: String,
    source: String,
    invite_id: Option<Uuid>,
    granted_by: Option<String>,
    created_at: DateTime<Utc>,
}

impl TryFrom<GrantRow> for ScopeGrant {
    type Error = Error;

    fn try_from(row: GrantRow) -> Result<Self> {
        Ok(ScopeGrant {
            id: GrantId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            subject_kind: decoded::<SubjectKind>(&row.subject_kind)?,
            principal_id: row.principal_id,
            group_id: row.group_id.map(GroupId::from_uuid),
            role_key: decoded::<RoleKey>(&row.role_key)?,
            source: decoded::<GrantSource>(&row.source)?,
            invite_id: row.invite_id.map(InviteId::from_uuid),
            granted_by: row.granted_by,
            created_at: row.created_at,
        })
    }
}

struct InviteRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    role_key: String,
    email: Option<String>,
    status: String,
    expires_at: DateTime<Utc>,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
    accepted_by: Option<String>,
    accepted_at: Option<DateTime<Utc>>,
    revoked_by: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TryFrom<InviteRow> for PendingInvite {
    type Error = Error;

    fn try_from(row: InviteRow) -> Result<Self> {
        Ok(PendingInvite {
            id: InviteId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            role_key: decoded::<RoleKey>(&row.role_key)?,
            email: row.email,
            status: decoded::<InviteStatus>(&row.status)?,
            expires_at: row.expires_at,
            created_by: row.created_by,
            created_at: row.created_at,
            accepted_by: row.accepted_by,
            accepted_at: row.accepted_at,
            revoked_by: row.revoked_by,
            revoked_at: row.revoked_at,
        })
    }
}

// ── Groups ───────────────────────────────────────────────────────────────────

/// What [`create_group`] needs.
#[derive(Debug, Clone)]
pub struct NewGroup {
    /// The group's identity (UUIDv7, mintable anywhere — ADR-0005).
    pub id: GroupId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Tenant-unique handle.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Optional prose.
    pub description: Option<String>,
    /// Whose group it is. A directory group carries its external reference.
    pub source: GroupSource,
    /// The external id, required for a directory group and refused otherwise.
    pub directory_ref: Option<String>,
    /// The subject creating it, when a caller is.
    pub created_by: Option<String>,
}

/// A partial update, applied under a revision precondition.
///
/// `description` is a double option, [`crate::workspaces::WorkspaceUpdate`]'s
/// shape and for its reason: absent leaves it, `Some(None)` clears it,
/// `Some(Some(text))` replaces it.
///
/// `members` is a **full replacement** rather than a delta, and that is the
/// decision worth reading (ADR-0072 decision 6). A membership list has no
/// natural precondition of its own, so an add/remove pair would race: two
/// callers each removing one person can both succeed and leave a list neither
/// intended. A replacement under `expected_revision` cannot — the second one
/// is refused, and the caller re-reads. It is also the shape a directory sync
/// sends, so the enterprise path is the same code.
#[derive(Debug, Clone, Default)]
pub struct GroupUpdate {
    /// New display name, when renaming.
    pub display_name: Option<String>,
    /// New description; see the struct docs for the three cases.
    pub description: Option<Option<String>>,
    /// New lifecycle status. An archived group resolves to nobody.
    pub status: Option<LifecycleStatus>,
    /// The complete membership after this update, when replacing it.
    pub members: Option<Vec<String>>,
}

impl GroupUpdate {
    /// Whether this update would change anything. An empty PATCH is a client
    /// bug, and answering it with a bumped revision hides one.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.description.is_none()
            && self.status.is_none()
            && self.members.is_none()
    }
}

/// Creates a group.
///
/// Fails with [`Error::Invalid`] for a malformed slug, name, description or
/// directory reference, and [`Error::Conflict`] when the slug is taken.
#[tracing::instrument(
    name = "store.access.create_group",
    skip_all,
    fields(tenant.id = %new.tenant_id, group.id = %new.id, group.source = %new.source),
    err(Display)
)]
pub async fn create_group(executor: impl PgExecutor<'_>, new: &NewGroup) -> Result<Group> {
    validate_slug(&new.slug)?;
    validate_display_name(&new.display_name)?;
    validate_description(new.description.as_deref())?;
    match (new.source, new.directory_ref.as_deref()) {
        (GroupSource::Directory, Some(reference)) => validate_directory_ref(reference)?,
        (GroupSource::Directory, None) => {
            return Err(Error::Invalid {
                message: "a directory group carries the reference its directory knows it by"
                    .to_owned(),
            });
        }
        (GroupSource::Direct, Some(_)) => {
            return Err(Error::Invalid {
                message: "only a directory group carries a directory reference".to_owned(),
            });
        }
        (GroupSource::Direct, None) => {}
    }
    if let Some(created_by) = &new.created_by {
        validate_principal_id(created_by)?;
    }

    let row = sqlx::query_as!(
        GroupRow,
        r#"
        insert into groups
            (id, tenant_id, slug, display_name, description, source, directory_ref,
             status, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        returning id, tenant_id, slug, display_name, description, source,
                  directory_ref, status, revision, created_by, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.slug,
        new.display_name,
        new.description.as_deref() as Option<&str>,
        new.source.as_str(),
        new.directory_ref.as_deref() as Option<&str>,
        LifecycleStatus::Active.as_str(),
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "group",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

/// Fetches one group.
#[tracing::instrument(
    name = "store.access.get_group",
    skip_all,
    fields(tenant.id = %tenant_id, group.id = %id),
    err(Display)
)]
pub async fn get_group(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GroupId,
) -> Result<Option<Group>> {
    let row = sqlx::query_as!(
        GroupRow,
        r#"
        select id, tenant_id, slug, display_name, description, source,
               directory_ref, status, revision, created_by, created_at, updated_at
        from groups
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists a tenant's groups, by slug. Archived ones included: a listing that
/// omitted them would make an archived group indistinguishable from one that
/// never existed.
#[tracing::instrument(
    name = "store.access.list_groups",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn list_groups(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<Group>> {
    let rows = sqlx::query_as!(
        GroupRow,
        r#"
        select id, tenant_id, slug, display_name, description, source,
               directory_ref, status, revision, created_by, created_at, updated_at
        from groups
        where tenant_id = $1
        order by slug
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The members of one group, by principal id.
#[tracing::instrument(
    name = "store.access.group_members",
    skip_all,
    fields(tenant.id = %tenant_id, group.id = %group_id),
    err(Display)
)]
pub async fn group_members(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    group_id: GroupId,
) -> Result<Vec<GroupMember>> {
    let rows = sqlx::query!(
        r#"
        select tenant_id, group_id, principal_id, source, added_by, created_at
        from group_members
        where tenant_id = $1 and group_id = $2
        order by principal_id
        "#,
        tenant_id.as_uuid(),
        group_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(GroupMember {
                tenant_id: TenantId::from_uuid(row.tenant_id),
                group_id: GroupId::from_uuid(row.group_id),
                principal_id: row.principal_id,
                source: decoded::<GrantSource>(&row.source)?,
                added_by: row.added_by,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// Every group membership in the tenant, so a listing renders without one
/// query per group.
///
/// An admin screen showing forty groups must not be forty round trips, and the
/// alternative — a `member_count` column on `groups` — would be a denormalised
/// number to keep true.
#[tracing::instrument(
    name = "store.access.all_group_members",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn all_group_members(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Vec<GroupMember>> {
    let rows = sqlx::query!(
        r#"
        select tenant_id, group_id, principal_id, source, added_by, created_at
        from group_members
        where tenant_id = $1
        order by group_id, principal_id
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(GroupMember {
                tenant_id: TenantId::from_uuid(row.tenant_id),
                group_id: GroupId::from_uuid(row.group_id),
                principal_id: row.principal_id,
                source: decoded::<GrantSource>(&row.source)?,
                added_by: row.added_by,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// Sets a group's membership at creation, returning the principals stored.
///
/// The same replacement [`update_group`] performs, exposed for the one caller
/// that has no revision to precondition on because it just minted the row.
/// Must run inside the creating transaction.
#[tracing::instrument(
    name = "store.access.set_group_members",
    skip_all,
    fields(tenant.id = %tenant_id, group.id = %group_id, members = members.len()),
    err(Display)
)]
pub async fn set_group_members(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    group_id: GroupId,
    members: &[String],
    actor: Option<&str>,
) -> Result<Vec<String>> {
    for member in members {
        validate_principal_id(member)?;
    }
    replace_members(
        &mut *conn,
        tenant_id,
        group_id,
        members,
        GrantSource::Direct,
        actor,
    )
    .await?;
    Ok(group_members(&mut *conn, tenant_id, group_id)
        .await?
        .into_iter()
        .map(|member| member.principal_id)
        .collect())
}

/// Applies an update under a revision precondition, replacing the membership
/// when one is given.
///
/// `expected_revision` is the revision the caller last saw; a mismatch is
/// [`Error::Conflict`] and nothing is written. A directory-managed group is
/// refused outright — see [`directory_managed`].
///
/// Must run inside a transaction: the membership replacement is a second and
/// third statement.
#[tracing::instrument(
    name = "store.access.update_group",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        group.id = %id,
        group.expected_revision = expected_revision,
    ),
    err(Display)
)]
pub async fn update_group(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: GroupId,
    expected_revision: i64,
    update: &GroupUpdate,
    actor: Option<&str>,
) -> Result<Group> {
    if update.is_empty() {
        return Err(Error::Invalid {
            message: "nothing to update: provide display_name, description, status or members"
                .to_owned(),
        });
    }
    if let Some(display_name) = &update.display_name {
        validate_display_name(display_name)?;
    }
    if let Some(description) = &update.description {
        validate_description(description.as_deref())?;
    }
    if let Some(members) = &update.members {
        for member in members {
            validate_principal_id(member)?;
        }
    }

    // Ownership and provenance before the write, so a directory group is
    // refused for what it is rather than for a revision it happens to be at.
    let current = get_group(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| group_not_found(id))?;
    if current.source.is_directory_managed() {
        return Err(directory_managed(&format!("group {}", current.slug)));
    }

    let row = sqlx::query_as!(
        GroupRow,
        r#"
        update groups
           set display_name = coalesce($4, display_name),
               description  = case when $5 then $6 else description end,
               status       = coalesce($7, status),
               revision     = revision + 1,
               updated_at   = now()
         where tenant_id = $1 and id = $2 and revision = $3
        returning id, tenant_id, slug, display_name, description, source,
                  directory_ref, status, revision, created_by, created_at, updated_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        expected_revision,
        update.display_name.as_deref() as Option<&str>,
        update.description.is_some(),
        update.description.as_ref().and_then(Option::as_deref) as Option<&str>,
        update.status.map(|status| status.as_str()) as Option<&str>,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        return Err(Error::Conflict {
            message: format!(
                "group {id} is at revision {}, not {expected_revision}",
                current.revision
            ),
        });
    };
    let group: Group = row.try_into()?;

    if let Some(members) = &update.members {
        replace_members(
            &mut *conn,
            tenant_id,
            id,
            members,
            GrantSource::Direct,
            actor,
        )
        .await?;
    }

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "group",
        "operation" => "update",
    )
    .increment(1);
    Ok(group)
}

/// Replaces a group's membership wholesale.
///
/// Delete-then-insert rather than a diff: the result is the same set, the code
/// is one statement shorter than the three a diff needs, and the caller has
/// already been serialised by the revision precondition above it — so the
/// window a diff would optimise does not exist.
async fn replace_members(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    group_id: GroupId,
    members: &[String],
    source: GrantSource,
    actor: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        "delete from group_members where tenant_id = $1 and group_id = $2",
        tenant_id.as_uuid(),
        group_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    if members.is_empty() {
        return Ok(());
    }
    // One statement for the whole set: `unnest` over the principal array,
    // `on conflict do nothing` so a caller that listed somebody twice gets one
    // membership rather than an error about their own duplicate.
    sqlx::query!(
        r#"
        insert into group_members (tenant_id, group_id, principal_id, source, added_by)
        select $1, $2, principal, $4, $5
        from unnest($3::text[]) as principal
        on conflict do nothing
        "#,
        tenant_id.as_uuid(),
        group_id.as_uuid(),
        members,
        source.as_str(),
        actor as Option<&str>,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "membership",
        "operation" => "update",
    )
    .increment(1);
    Ok(())
}

// ── Grants ───────────────────────────────────────────────────────────────────

/// What [`create_grant`] needs.
#[derive(Debug, Clone)]
pub struct NewGrant {
    /// The grant's identity.
    pub id: GrantId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The scope it is at; the subtree inherits it.
    pub scope_id: ScopeId,
    /// Who holds it.
    pub subject: GrantSubject,
    /// What they hold.
    pub role_key: RoleKey,
    /// Where it came from.
    pub source: GrantSource,
    /// The invitation that produced it — required for
    /// [`GrantSource::Invite`] and refused otherwise.
    pub invite_id: Option<InviteId>,
    /// The subject granting it, when a caller is.
    pub granted_by: Option<String>,
}

/// Creates a grant.
///
/// Fails with [`Error::Invalid`] for a malformed principal or a source that
/// disagrees with `invite_id`, and [`Error::Conflict`] when the subject already
/// holds that role at that scope, when the scope or group does not exist, or
/// when either belongs to another tenant.
#[tracing::instrument(
    name = "store.access.create_grant",
    skip_all,
    fields(
        tenant.id = %new.tenant_id,
        grant.id = %new.id,
        scope.id = %new.scope_id,
        grant.role = %new.role_key,
        grant.source = %new.source,
    ),
    err(Display)
)]
pub async fn create_grant(executor: impl PgExecutor<'_>, new: &NewGrant) -> Result<ScopeGrant> {
    validate_new_grant(new)?;
    let row = sqlx::query_as!(
        GrantRow,
        r#"
        insert into scope_grants
            (id, tenant_id, scope_id, subject_kind, principal_id, group_id,
             role_key, source, invite_id, granted_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        returning id, tenant_id, scope_id, subject_kind, principal_id, group_id,
                  role_key, source, invite_id, granted_by, created_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.subject.kind().as_str(),
        new.subject.principal_id() as Option<&str>,
        new.subject.group_id().map(|id| id.as_uuid()) as Option<Uuid>,
        new.role_key.as_str(),
        new.source.as_str(),
        new.invite_id.map(|id| id.as_uuid()) as Option<Uuid>,
        new.granted_by.as_deref() as Option<&str>,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "grant",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

fn validate_new_grant(new: &NewGrant) -> Result<()> {
    if let Some(principal_id) = new.subject.principal_id() {
        validate_principal_id(principal_id)?;
    }
    if let Some(granted_by) = &new.granted_by {
        validate_principal_id(granted_by)?;
    }
    if (new.source == GrantSource::Invite) != new.invite_id.is_some() {
        return Err(Error::Invalid {
            message: "an invite-sourced grant names its invitation, and no other source may"
                .to_owned(),
        });
    }
    Ok(())
}

/// Fetches one grant.
#[tracing::instrument(
    name = "store.access.get_grant",
    skip_all,
    fields(tenant.id = %tenant_id, grant.id = %id),
    err(Display)
)]
pub async fn get_grant(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: GrantId,
) -> Result<Option<ScopeGrant>> {
    let row = sqlx::query_as!(
        GrantRow,
        r#"
        select id, tenant_id, scope_id, subject_kind, principal_id, group_id,
               role_key, source, invite_id, granted_by, created_at
        from scope_grants
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Which grants a listing wants. Every field is optional and they intersect;
/// all-`None` lists the tenant's grants.
#[derive(Debug, Clone, Default)]
pub struct GrantFilter {
    /// Only grants written **at** this scope. Deliberately not "grants in
    /// force here": that is [`members_of`], and conflating the stored row with
    /// the resolved authority is exactly the confusion this plane exists to
    /// remove.
    pub scope_id: Option<ScopeId>,
    /// Only grants naming this principal directly. Group-derived authority is
    /// absent for the same reason.
    pub principal_id: Option<String>,
}

/// Lists a tenant's grants, oldest first.
#[tracing::instrument(
    name = "store.access.list_grants",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn list_grants(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    filter: &GrantFilter,
) -> Result<Vec<ScopeGrant>> {
    let rows = sqlx::query_as!(
        GrantRow,
        r#"
        select id, tenant_id, scope_id, subject_kind, principal_id, group_id,
               role_key, source, invite_id, granted_by, created_at
        from scope_grants
        where tenant_id = $1
          and ($2::uuid is null or scope_id = $2)
          and ($3::text is null or principal_id = $3)
        order by created_at, id
        "#,
        tenant_id.as_uuid(),
        filter.scope_id.map(|id| id.as_uuid()) as Option<Uuid>,
        filter.principal_id.as_deref() as Option<&str>,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Revokes one grant by id, returning what was revoked.
///
/// Fails with [`Error::NotFound`] for a grant that is not this tenant's and
/// [`Error::Conflict`] for one a directory manages.
#[tracing::instrument(
    name = "store.access.revoke_grant",
    skip_all,
    fields(tenant.id = %tenant_id, grant.id = %id),
    err(Display)
)]
pub async fn revoke_grant(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: GrantId,
) -> Result<ScopeGrant> {
    let grant = get_grant(&mut *conn, tenant_id, id)
        .await?
        .ok_or_else(|| grant_not_found(id))?;
    if grant.source.is_directory_managed() {
        return Err(directory_managed(&format!("grant {id}")));
    }
    let deleted = sqlx::query!(
        "delete from scope_grants where tenant_id = $1 and id = $2",
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?
    .rows_affected();
    if deleted == 0 {
        // Another transaction revoked it between the read and the delete. The
        // caller wanted it gone and it is gone, but reporting success would
        // claim this call did it, and the audit event that follows would say
        // so.
        return Err(grant_not_found(id));
    }

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "grant",
        "operation" => "revoke",
    )
    .increment(1);
    Ok(grant)
}

/// Removes a principal from a scope: revokes every grant written **at that
/// scope** naming them directly.
///
/// It deliberately does not touch inherited or group-derived authority, and
/// says so rather than silently doing less than the caller asked: removing
/// somebody from a project when their access comes from the workspace is a
/// change to the workspace, and a route that quietly succeeded would leave
/// them with the access they had.
///
/// Fails with [`Error::NotFound`] when they hold nothing here at all,
/// [`Error::Conflict`] naming where the authority actually lives when it is
/// inherited or group-derived, and [`Error::Conflict`] when a directory
/// manages one of the rows.
///
/// Must run inside a transaction.
#[tracing::instrument(
    name = "store.access.remove_member",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn remove_member(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
    principal_id: &str,
) -> Result<Vec<ScopeGrant>> {
    validate_principal_id(principal_id)?;
    let direct = list_grants(
        &mut *conn,
        tenant_id,
        &GrantFilter {
            scope_id: Some(scope_id),
            principal_id: Some(principal_id.to_owned()),
        },
    )
    .await?;

    if direct.is_empty() {
        let effective = members_of(&mut *conn, tenant_id, scope_id).await?;
        let elsewhere: Vec<&AccessEntry> = effective
            .iter()
            .filter(|entry| entry.principal_id == principal_id)
            .collect();
        return Err(match elsewhere.first() {
            None => Error::NotFound {
                entity: format!("member {principal_id} of scope {scope_id}"),
            },
            Some(entry) if entry.via_group.is_some() => Error::Conflict {
                message: format!(
                    "{principal_id} holds access here through group {}: remove them from \
                     the group, or revoke the group's grant",
                    entry
                        .via_group
                        .as_ref()
                        .map(|group| group.slug.as_str())
                        .unwrap_or_default()
                ),
            },
            Some(entry) => Error::Conflict {
                message: format!(
                    "{principal_id} holds access here from scope {}, not from this one: \
                     revoke it there, or this removal would leave the access in place",
                    entry.scope_id
                ),
            },
        });
    }
    if let Some(managed) = direct
        .iter()
        .find(|grant| grant.source.is_directory_managed())
    {
        return Err(directory_managed(&format!("grant {}", managed.id)));
    }

    sqlx::query!(
        "delete from scope_grants where tenant_id = $1 and scope_id = $2 and principal_id = $3",
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        principal_id,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "grant",
        "operation" => "revoke",
    )
    .increment(1);
    Ok(direct)
}

// ── Effective membership ─────────────────────────────────────────────────────

/// The group a grant reached a principal through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViaGroup {
    /// The group's id.
    pub id: GroupId,
    /// Its handle, so a listing reads without a second query.
    pub slug: String,
}

/// One principal's one role at one scope, with everything a reader needs to
/// answer "why".
///
/// This is the "access-source visibility" the feature exists for: the source,
/// the scope the grant is actually written at, whether it was inherited, and
/// the group it came through if it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessEntry {
    /// The grant this came from — what a revocation names.
    pub grant_id: GrantId,
    /// The scope the grant is written at, which is **not** necessarily the
    /// scope that was asked about.
    pub scope_id: ScopeId,
    /// Whether it was inherited from an ancestor rather than written here.
    pub inherited: bool,
    /// The principal.
    pub principal_id: String,
    /// The group it reached them through, when it did.
    pub via_group: Option<ViaGroup>,
    /// What they hold.
    pub role_key: RoleKey,
    /// Where the grant came from.
    pub source: GrantSource,
    /// Whether a directory manages it, and it therefore cannot be edited here.
    pub directory_managed: bool,
    /// When the grant was made.
    pub granted_at: DateTime<Utc>,
}

/// Everybody who holds a role at `scope_id`, direct and inherited, principal
/// and group-derived.
///
/// The three rules, all in the one query so no caller can apply two of them:
///
/// 1. **Inheritance** — grants on the scope's ancestry are in force here.
/// 2. **Principal privacy** — a `principal`-shaped scope inherits nothing;
///    only grants written at it apply
///    ([`synveda_types::access::inherits_into`]).
/// 3. **Groups resolve, archived groups resolve to nobody.**
///
/// Ordered nearest-scope first, then principal, then role — so a reader sees
/// the most specific authority at the top.
#[tracing::instrument(
    name = "store.access.members_of",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id, members = tracing::field::Empty),
    err(Display)
)]
pub async fn members_of(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<AccessEntry>> {
    let rows = sqlx::query!(
        r#"
        with target as (
            select kind from scopes where tenant_id = $1 and id = $2
        ),
        chain as (
            select c.ancestor_id, c.distance
            from scope_closure c
            where c.tenant_id = $1
              and c.descendant_id = $2
              -- Principal-private scope isolation: a `principal` scope is
              -- somebody's own, and nothing above it reaches in. Expressed
              -- here rather than in Rust so that every caller gets it.
              and (c.distance = 0
                   or (select kind from target) <> 'principal')
        )
        select g.id                as "grant_id!",
               g.scope_id          as "scope_id!",
               ch.distance         as "distance!",
               g.role_key          as "role_key!",
               g.source            as "source!",
               g.created_at        as "granted_at!",
               coalesce(g.principal_id, gm.principal_id) as "principal_id!",
               grp.id              as "group_id?",
               grp.slug            as "group_slug?"
        from chain ch
        join scope_grants g
          on g.tenant_id = $1 and g.scope_id = ch.ancestor_id
        left join groups grp
          on grp.tenant_id = $1 and grp.id = g.group_id and grp.status = 'active'
        left join group_members gm
          on gm.tenant_id = $1 and gm.group_id = grp.id
        where g.subject_kind = 'principal' or gm.principal_id is not null
        order by ch.distance, coalesce(g.principal_id, gm.principal_id), g.role_key, g.id
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;

    let entries = rows
        .into_iter()
        .map(|row| {
            let source = decoded::<GrantSource>(&row.source)?;
            Ok(AccessEntry {
                grant_id: GrantId::from_uuid(row.grant_id),
                scope_id: ScopeId::from_uuid(row.scope_id),
                inherited: row.distance > 0,
                principal_id: row.principal_id,
                via_group: match (row.group_id, row.group_slug) {
                    (Some(id), Some(slug)) => Some(ViaGroup {
                        id: GroupId::from_uuid(id),
                        slug,
                    }),
                    _ => None,
                },
                role_key: decoded::<RoleKey>(&row.role_key)?,
                source,
                directory_managed: source.is_directory_managed(),
                granted_at: row.granted_at,
            })
        })
        .collect::<Result<Vec<AccessEntry>>>()?;
    tracing::Span::current().record("members", entries.len());
    Ok(entries)
}

// ── Invitations ──────────────────────────────────────────────────────────────

/// What [`create_invite`] needs.
#[derive(Debug, Clone)]
pub struct NewInvite {
    /// The invitation's identity.
    pub id: InviteId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The scope it grants at.
    pub scope_id: ScopeId,
    /// What it grants.
    pub role_key: RoleKey,
    /// Who it is meant for, when the inviter said.
    pub email: Option<String>,
    /// SHA-256 of the whole token. The token itself is never given to this
    /// crate: [`synveda_identity::invite`] mints it, the gateway shows it once,
    /// and only the hash crosses the storage boundary.
    pub token_hash: [u8; 32],
    /// When it stops being redeemable.
    pub expires_at: DateTime<Utc>,
    /// The subject issuing it.
    pub created_by: Option<String>,
}

/// Creates an invitation.
///
/// Fails with [`Error::Invalid`] for a malformed email and [`Error::Conflict`]
/// when the scope does not exist in this tenant or — vanishingly — when the
/// token hash collides.
#[tracing::instrument(
    name = "store.access.create_invite",
    skip_all,
    fields(
        tenant.id = %new.tenant_id,
        invite.id = %new.id,
        scope.id = %new.scope_id,
        invite.role = %new.role_key,
    ),
    err(Display)
)]
pub async fn create_invite(
    executor: impl PgExecutor<'_>,
    new: &NewInvite,
) -> Result<PendingInvite> {
    if let Some(email) = &new.email {
        validate_email(email)?;
    }
    if let Some(created_by) = &new.created_by {
        validate_principal_id(created_by)?;
    }
    let row = sqlx::query_as!(
        InviteRow,
        r#"
        insert into pending_invites
            (id, tenant_id, scope_id, role_key, email, token_hash, status,
             expires_at, created_by)
        values ($1, $2, $3, $4, $5, $6, 'pending', $7, $8)
        returning id, tenant_id, scope_id, role_key, email, status, expires_at,
                  created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.role_key.as_str(),
        new.email.as_deref() as Option<&str>,
        &new.token_hash[..],
        new.expires_at,
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "invite",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

/// Fetches one invitation by id.
#[tracing::instrument(
    name = "store.access.get_invite",
    skip_all,
    fields(tenant.id = %tenant_id, invite.id = %id),
    err(Display)
)]
pub async fn get_invite(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: InviteId,
) -> Result<Option<PendingInvite>> {
    let row = sqlx::query_as!(
        InviteRow,
        r#"
        select id, tenant_id, scope_id, role_key, email, status, expires_at,
               created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        from pending_invites
        where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Every invitation at a scope, newest first.
///
/// Redeemed and withdrawn ones included: "who was invited here and what
/// happened" is the question the listing answers, and one that showed only
/// outstanding invitations would answer a different one.
#[tracing::instrument(
    name = "store.access.list_invites",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn list_invites(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<PendingInvite>> {
    let rows = sqlx::query_as!(
        InviteRow,
        r#"
        select id, tenant_id, scope_id, role_key, email, status, expires_at,
               created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        from pending_invites
        where tenant_id = $1 and scope_id = $2
        order by created_at desc, id
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Withdraws an outstanding invitation.
///
/// Fails with [`Error::NotFound`] for an invitation that is not this tenant's
/// and [`Error::Conflict`] for one that is already terminal — an invitation
/// somebody redeemed cannot be un-redeemed, and saying so is more use than a
/// no-op that reads as success.
#[tracing::instrument(
    name = "store.access.revoke_invite",
    skip_all,
    fields(tenant.id = %tenant_id, invite.id = %id),
    err(Display)
)]
pub async fn revoke_invite(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: InviteId,
    actor: Option<&str>,
) -> Result<PendingInvite> {
    if let Some(actor) = actor {
        validate_principal_id(actor)?;
    }
    let row = sqlx::query_as!(
        InviteRow,
        r#"
        update pending_invites
           set status = 'revoked', revoked_at = now(), revoked_by = $3
         where tenant_id = $1 and id = $2 and status = 'pending'
        returning id, tenant_id, scope_id, role_key, email, status, expires_at,
                  created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        actor as Option<&str>,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        return Err(match get_invite(&mut *conn, tenant_id, id).await? {
            Some(current) => Error::Conflict {
                message: format!(
                    "invitation {id} is already {}; an invitation is one-time",
                    current.status
                ),
            },
            None => invite_not_found(id),
        });
    };

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "invite",
        "operation" => "revoke",
    )
    .increment(1);
    row.try_into()
}

/// What redeeming an invitation produced.
#[derive(Debug, Clone)]
pub struct Accepted {
    /// The invitation, now terminal.
    pub invite: PendingInvite,
    /// The grant it produced — or the one the acceptor already held.
    pub grant: ScopeGrant,
    /// Whether this call is a replay of an earlier acceptance by the same
    /// principal, rather than the acceptance itself. The route answers 200
    /// rather than 201 for a replay.
    pub replayed: bool,
}

/// Redeems an invitation for `principal_id`, minting the grant it carries.
///
/// **One-time, and the window ends here.** The row is locked, its status is
/// checked, and its expiry is compared against `now` — expiry is a property of
/// this decision rather than of a sweep (ADR-0037 decision 4), so an
/// invitation stops working at the instant it says it will, whether or not any
/// job has run.
///
/// A second call by the **same** principal replays: the invitation is already
/// theirs, the grant already exists, and answering a retrying client with a
/// conflict would punish the network rather than the caller. A call by a
/// different principal is refused — that is what one-time means.
///
/// Fails with [`Error::NotFound`] when no invitation in this tenant hashes to
/// this token, and [`Error::Conflict`] when it is expired, withdrawn, or
/// already somebody else's.
///
/// Must run inside a transaction.
#[tracing::instrument(
    name = "store.access.accept_invite",
    skip_all,
    fields(tenant.id = %tenant_id, invite.id = tracing::field::Empty),
    err(Display)
)]
pub async fn accept_invite(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    token_hash: &[u8; 32],
    principal_id: &str,
    now: DateTime<Utc>,
) -> Result<Accepted> {
    validate_principal_id(principal_id)?;

    // Locked, so two redemptions of one token serialise rather than both
    // finding it pending.
    let row = sqlx::query_as!(
        InviteRow,
        r#"
        select id, tenant_id, scope_id, role_key, email, status, expires_at,
               created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        from pending_invites
        where tenant_id = $1 and token_hash = $2
        for update
        "#,
        tenant_id.as_uuid(),
        &token_hash[..],
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    // A token that hashes to nothing is not found. The message deliberately
    // does not say whether it never existed, belonged to another tenant, or
    // was deleted: an invitation token is a secret, and a distinguishable
    // refusal is an oracle for guessing them.
    let Some(row) = row else {
        return Err(Error::NotFound {
            entity: "invitation".to_owned(),
        });
    };
    let invite: PendingInvite = row.try_into()?;
    tracing::Span::current().record("invite.id", tracing::field::display(invite.id));

    match invite.status {
        InviteStatus::Accepted if invite.accepted_by.as_deref() == Some(principal_id) => {
            let grant = grant_of_invite(&mut *conn, tenant_id, invite.id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!(
                        "the grant invitation {} produced (it was revoked since)",
                        invite.id
                    ),
                })?;
            return Ok(Accepted {
                invite,
                grant,
                replayed: true,
            });
        }
        InviteStatus::Accepted => {
            return Err(Error::Conflict {
                message: "this invitation has already been accepted by somebody else".to_owned(),
            });
        }
        InviteStatus::Revoked => {
            return Err(Error::Conflict {
                message: "this invitation was withdrawn".to_owned(),
            });
        }
        InviteStatus::Expired => {
            // Unreachable: `expired` is never stored (ADR-0072 decision 4).
            return Err(Error::Internal {
                message: format!("invitation {} is stored as expired", invite.id),
            });
        }
        InviteStatus::Pending => {}
    }
    if !invite.is_redeemable(now) {
        return Err(Error::Conflict {
            message: format!(
                "this invitation expired at {}; ask for another",
                invite.expires_at.to_rfc3339()
            ),
        });
    }

    let accepted = sqlx::query_as!(
        InviteRow,
        r#"
        update pending_invites
           set status = 'accepted', accepted_at = now(), accepted_by = $3
         where tenant_id = $1 and id = $2 and status = 'pending'
        returning id, tenant_id, scope_id, role_key, email, status, expires_at,
                  created_by, created_at, accepted_by, accepted_at, revoked_by, revoked_at
        "#,
        tenant_id.as_uuid(),
        invite.id.as_uuid(),
        principal_id,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let accepted: PendingInvite = accepted.try_into()?;

    // Redeeming an invitation for access somebody already holds consumes the
    // invitation and hands back the grant they have. The alternative is a
    // conflict for a person who did nothing wrong — they were invited to a
    // role they had, and the product's answer would be an error.
    let existing = sqlx::query_as!(
        GrantRow,
        r#"
        insert into scope_grants
            (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source, invite_id)
        values ($1, $2, $3, 'principal', $4, $5, 'invite', $6)
        on conflict do nothing
        returning id, tenant_id, scope_id, subject_kind, principal_id, group_id,
                  role_key, source, invite_id, granted_by, created_at
        "#,
        GrantId::new().as_uuid(),
        tenant_id.as_uuid(),
        accepted.scope_id.as_uuid(),
        principal_id,
        accepted.role_key.as_str(),
        accepted.id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    let grant = match existing {
        Some(row) => row.try_into()?,
        None => held_grant(
            &mut *conn,
            tenant_id,
            accepted.scope_id,
            principal_id,
            accepted.role_key,
        )
        .await?
        .ok_or_else(|| Error::Internal {
            message: "the grant this invitation collided with vanished".to_owned(),
        })?,
    };

    metrics::counter!(
        ACCESS_MUTATIONS_TOTAL,
        "object" => "invite",
        "operation" => "accept",
    )
    .increment(1);
    Ok(Accepted {
        invite: accepted,
        grant,
        replayed: false,
    })
}

/// The grant one invitation produced, if it still stands.
async fn grant_of_invite(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    invite_id: InviteId,
) -> Result<Option<ScopeGrant>> {
    let row = sqlx::query_as!(
        GrantRow,
        r#"
        select id, tenant_id, scope_id, subject_kind, principal_id, group_id,
               role_key, source, invite_id, granted_by, created_at
        from scope_grants
        where tenant_id = $1 and invite_id = $2
        "#,
        tenant_id.as_uuid(),
        invite_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// The grant a principal already holds at a scope for a role, if any.
async fn held_grant(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
    principal_id: &str,
    role_key: RoleKey,
) -> Result<Option<ScopeGrant>> {
    let row = sqlx::query_as!(
        GrantRow,
        r#"
        select id, tenant_id, scope_id, subject_kind, principal_id, group_id,
               role_key, source, invite_id, granted_by, created_at
        from scope_grants
        where tenant_id = $1 and scope_id = $2 and principal_id = $3 and role_key = $4
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        principal_id,
        role_key.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_group_update_changes_nothing_and_says_so() {
        assert!(GroupUpdate::default().is_empty());
        assert!(
            !GroupUpdate {
                members: Some(Vec::new()),
                ..Default::default()
            }
            .is_empty(),
            "emptying a group is a change"
        );
        assert!(
            !GroupUpdate {
                description: Some(None),
                ..Default::default()
            }
            .is_empty(),
            "clearing a description is a change"
        );
    }

    /// The source and the invitation column are two halves of one fact, and a
    /// row that carried one without the other would be provenance nobody can
    /// check. The database says the same thing; this is the half that produces
    /// a sentence.
    #[test]
    fn a_grants_source_and_its_invitation_agree_or_it_is_refused() {
        let base = NewGrant {
            id: GrantId::new(),
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            subject: GrantSubject::Principal {
                principal_id: "sam".to_owned(),
            },
            role_key: RoleKey::Member,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: None,
        };
        validate_new_grant(&base).expect("a direct grant names no invitation");

        let claimed = NewGrant {
            source: GrantSource::Invite,
            invite_id: None,
            ..base.clone()
        };
        assert!(validate_new_grant(&claimed).is_err());

        let unclaimed = NewGrant {
            source: GrantSource::Direct,
            invite_id: Some(InviteId::new()),
            ..base.clone()
        };
        assert!(validate_new_grant(&unclaimed).is_err());

        let honest = NewGrant {
            source: GrantSource::Invite,
            invite_id: Some(InviteId::new()),
            ..base.clone()
        };
        validate_new_grant(&honest).expect("an invite grant names its invitation");

        let blank = NewGrant {
            subject: GrantSubject::Principal {
                principal_id: "  ".to_owned(),
            },
            ..base
        };
        assert!(validate_new_grant(&blank).is_err());
    }

    /// The refusal has to send somebody to the directory rather than sound
    /// like a permission problem, or the next thing they do is ask for more
    /// permission.
    #[test]
    fn the_directory_refusal_says_where_to_make_the_change() {
        let error = directory_managed("group engineering");
        let message = error.to_string();
        assert!(message.contains("directory"), "{message}");
        assert!(message.contains("engineering"), "{message}");
        assert!(
            message.contains("next sync"),
            "the refusal explains why, not only that: {message}"
        );
    }
}
