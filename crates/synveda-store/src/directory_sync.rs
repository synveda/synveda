//! The pull sync's own state (AUTH-5, ADR-0060): what a pass believes, and
//! what it is entitled to conclude from it.
//!
//! This module holds no lifecycle. Joiner, mover and leaver are
//! `scim::reconcile`'s, unchanged and not reimplemented here (ADR-0059
//! decision 3) — what lives here is the layer underneath that: the
//! completeness proof a pass earns, the absence it accumulates, and the
//! authorisation that releases a circuit-breaker trip.
//!
//! ## The one idea
//!
//! **Absence is a hypothesis.** On the push plane a leaver is an act; here it
//! is a person who was on page 3 last hour and is on no page now, which is
//! also what a throttled response, a truncated page and a narrowed
//! assignment filter look like. So nothing here seals. [`mark_absent`]
//! *counts*, [`absent_at_least`] *offers*, and the decision to act belongs to
//! the caller, which is the only place that knows whether the pass completed.
//!
//! ## The mirror's clock is not ours
//!
//! [`mark_absent`] and [`mark_present`] write `missing_passes` and
//! `missing_since` and deliberately **never** `updated_at` or `version`.
//! Those two are the directory resource's: `updated_at` is served as
//! `meta.lastModified` and `version` is the ETag a provisioning agent uses to
//! decide whether to re-send. Bumping either because *we* failed to see
//! somebody would tell a SCIM client the resource changed when the directory
//! never touched it — a poll storm at best, and at worst a client that
//! re-sends the world every time our connector has a bad afternoon.
//!
//! Everything here is tenant-scoped (forced RLS, ADR-0009): reach it inside
//! [`crate::rls::begin_tenant_tx`].

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{DirectoryUser, DirectoryUserId, Error, Result, TenantId};
use uuid::Uuid;

use crate::directory::UserRow;

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23514 check_violation: migration 0037's invariants — a half-reset
        // absence, an incomplete pass claiming the completeness proof, or a
        // partial seal authorisation. Each is a caller bug rather than a
        // storage failure, so it surfaces as `Invalid` and not `Storage`.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
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

/// One tenant's pull-sync state, as of the last pass that touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorySyncState {
    /// The tenant this state belongs to; one row each, at most.
    pub tenant_id: TenantId,
    /// Which connector wrote this. A change invalidates every absence count
    /// below it — the new connector has never seen anybody, so nobody it
    /// does not list is missing *yet* (see [`reset_absences`]).
    pub connector: String,
    /// Passes that **completed**. The completeness proof (ADR-0060 decision
    /// 3.1) is this number moving; an incomplete pass leaves it alone.
    pub passes_completed: i64,
    /// Every attempt, including the ones that failed. A gap between this and
    /// `last_complete_pass_at` is a connector that runs and never finishes —
    /// the state in which nobody is sealed and nothing looks wrong.
    pub last_pass_at: Option<DateTime<Utc>>,
    /// When a pass last got all the way through. `None` until one does.
    pub last_complete_pass_at: Option<DateTime<Utc>>,
    /// Set iff the **most recent** complete pass tripped the breaker.
    /// [`complete_pass`] clears it otherwise, so this is current state and
    /// not a log; the log is the chain's.
    pub breaker_tripped_at: Option<DateTime<Utc>>,
    /// How many that pass declined to seal — the size of the refusal, which
    /// is the number an operator needs to judge it.
    pub breaker_would_have_sealed: Option<i32>,
    /// The in-force release, if one has been granted and not yet spent.
    pub authorisation: Option<SealAuthorisation>,
    /// When this tenant was first synced.
    pub created_at: DateTime<Utc>,
    /// When this row last moved, for any reason.
    pub updated_at: DateTime<Utc>,
}

/// A human's authorisation to seal past the breaker (ADR-0060 decision 10).
///
/// Reasoned, time-boxed, signed, and bounded by a ceiling — the fourth being
/// what stops "authorise 300,
/// the directory degrades further, seal 5,000".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealAuthorisation {
    /// When it was signed, by the database's clock.
    pub granted_at: DateTime<Utc>,
    /// When it stops covering anything, whether or not it was used.
    pub expires_at: DateTime<Utc>,
    /// The most this authorisation permits. A pass proposing more trips
    /// again rather than proceeding.
    pub ceiling: i32,
    /// The principal who signed it — a name for the chain to carry, so
    /// "who authorised 300 seals" is answerable.
    pub granted_by: String,
    /// Why, in their words. Bounded at 512 characters by the schema.
    pub reason: String,
}

struct StateRow {
    tenant_id: Uuid,
    connector: String,
    passes_completed: i64,
    last_pass_at: Option<DateTime<Utc>>,
    last_complete_pass_at: Option<DateTime<Utc>>,
    breaker_tripped_at: Option<DateTime<Utc>>,
    breaker_would_have_sealed: Option<i32>,
    seal_authorised_at: Option<DateTime<Utc>>,
    seal_authorised_until: Option<DateTime<Utc>>,
    seal_authorised_ceiling: Option<i32>,
    seal_authorised_by: Option<String>,
    seal_authorised_reason: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<StateRow> for DirectorySyncState {
    fn from(row: StateRow) -> Self {
        // All five or none — `directory_sync_state_authorisation_pair_check`
        // makes a partial row unrepresentable. This reads all five anyway
        // rather than testing one and unwrapping the rest, because code that
        // would panic if a constraint were ever relaxed is code that has
        // moved the invariant out of the schema and into a habit.
        let authorisation = match (
            row.seal_authorised_at,
            row.seal_authorised_until,
            row.seal_authorised_ceiling,
            row.seal_authorised_by,
            row.seal_authorised_reason,
        ) {
            (Some(granted_at), Some(expires_at), Some(ceiling), Some(granted_by), Some(reason)) => {
                Some(SealAuthorisation {
                    granted_at,
                    expires_at,
                    ceiling,
                    granted_by,
                    reason,
                })
            }
            _ => None,
        };
        DirectorySyncState {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            connector: row.connector,
            passes_completed: row.passes_completed,
            last_pass_at: row.last_pass_at,
            last_complete_pass_at: row.last_complete_pass_at,
            breaker_tripped_at: row.breaker_tripped_at,
            breaker_would_have_sealed: row.breaker_would_have_sealed,
            authorisation,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

// ── The pass ────────────────────────────────────────────────────────────

/// This tenant's sync state, or `None` if it has never been synced.
#[tracing::instrument(
    name = "store.directory_sync.state",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn state(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<DirectorySyncState>> {
    let row = sqlx::query_as!(
        StateRow,
        r#"
        select tenant_id, connector, passes_completed, last_pass_at,
               last_complete_pass_at, breaker_tripped_at, breaker_would_have_sealed,
               seal_authorised_at, seal_authorised_until, seal_authorised_ceiling,
               seal_authorised_by, seal_authorised_reason, created_at, updated_at
        from directory_sync_state
        where tenant_id = $1
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Stamps the start of an attempt, creating the state row on first sync.
///
/// Deliberately does **not** compare the stored connector with the one
/// starting the pass. That comparison is a decision — it invalidates every
/// absence count for the tenant — and decisions belong to the caller, which
/// reads [`state`] first and calls [`reset_absences`] if the answer changed.
/// Returning the row this stamped would make the caller's comparison
/// impossible: by then the old value is gone.
#[tracing::instrument(
    name = "store.directory_sync.begin_pass",
    skip_all,
    fields(tenant.id = %tenant_id, sync.connector = connector),
    err(Display)
)]
pub async fn begin_pass(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    connector: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into directory_sync_state (tenant_id, connector, last_pass_at)
        values ($1, $2, now())
        on conflict (tenant_id) do update
           set connector = excluded.connector,
               last_pass_at = now(),
               updated_at = now()
        "#,
        tenant_id.as_uuid(),
        connector,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Records that a pass **completed**, and what the breaker made of it.
///
/// One statement for both because the ordering hazard is real: a separate
/// "record the trip" call could be overwritten by a later "the pass finished"
/// call, and the row would then say a pass completed cleanly when it had
/// refused to seal 300 people. `would_have_sealed` is `Some` iff the breaker
/// tripped, and passing `None` clears any earlier trip — so the column means
/// "the most recent complete pass tripped", which is a fact about now rather
/// than a log entry. The log is the chain's.
///
/// Advancing `passes_completed` is what makes the tenant's absence counts
/// mean anything: an incomplete pass never calls this, so it cannot
/// contribute to a conclusion about who is gone (ADR-0060 decision 3.1).
#[tracing::instrument(
    name = "store.directory_sync.complete_pass",
    skip_all,
    fields(tenant.id = %tenant_id, sync.breaker = would_have_sealed),
    err(Display)
)]
pub async fn complete_pass(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    would_have_sealed: Option<i32>,
) -> Result<bool> {
    let done = sqlx::query!(
        r#"
        update directory_sync_state
           set passes_completed = passes_completed + 1,
               last_complete_pass_at = now(),
               breaker_tripped_at = case when $2::int is null then null else now() end,
               breaker_would_have_sealed = $2,
               updated_at = now()
         where tenant_id = $1
        "#,
        tenant_id.as_uuid(),
        would_have_sealed,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(done.rows_affected() == 1)
}

// ── Absence ─────────────────────────────────────────────────────────────

/// Advances the absence count for every live mirror row the pass did **not**
/// see, and stamps `missing_since` on the ones newly missing.
///
/// Call this only after a pass has completed. Nothing in the schema can tell
/// a complete pass from a truncated one, so this function trusts its caller
/// about the one thing that matters — which is why [`complete_pass`] exists
/// as a separate act and why the loop calls them together or not at all.
///
/// **An empty `seen` marks everybody**, and that is correct rather than a
/// special case to guard: a directory that completed a pass and listed
/// nobody has either lost every user or lost its mind, and both are the
/// breaker's business, not this statement's. It is stated here because an
/// empty array is the input most likely to be reached for in a test and
/// least likely to be intended in production.
///
/// Returns how many rows moved. Never touches `updated_at` or `version`;
/// see the module docs.
#[tracing::instrument(
    name = "store.directory_sync.mark_absent",
    skip_all,
    fields(tenant.id = %tenant_id, sync.seen = seen.len(), sync.absent = tracing::field::Empty),
    err(Display)
)]
pub async fn mark_absent(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    seen: &[DirectoryUserId],
) -> Result<u64> {
    let ids: Vec<Uuid> = seen.iter().map(DirectoryUserId::as_uuid).collect();
    let moved = sqlx::query!(
        r#"
        update scim_users
           set missing_passes = missing_passes + 1,
               missing_since = coalesce(missing_since, now())
         where tenant_id = $1
           and directory_source = $2
           and active
           and not (id = any($3))
        "#,
        tenant_id.as_uuid(),
        directory_source,
        &ids,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    let moved = moved.rows_affected();
    tracing::Span::current().record("sync.absent", moved);
    Ok(moved)
}

/// Clears the absence hypothesis for everybody the pass **did** see.
///
/// Both columns together, because a half-reset is refused by
/// `scim_users_missing_pair_check` — the counter and the timestamp say one
/// thing or the schema does not accept them.
///
/// Returns how many had been missing and are not any more, which is the
/// number worth logging: rows that were already present move nothing.
#[tracing::instrument(
    name = "store.directory_sync.mark_present",
    skip_all,
    fields(tenant.id = %tenant_id, sync.seen = seen.len(), sync.returned = tracing::field::Empty),
    err(Display)
)]
pub async fn mark_present(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    seen: &[DirectoryUserId],
) -> Result<u64> {
    let ids: Vec<Uuid> = seen.iter().map(DirectoryUserId::as_uuid).collect();
    let returned = sqlx::query!(
        r#"
        update scim_users
           set missing_passes = 0, missing_since = null
         where tenant_id = $1
           and directory_source = $2
           and id = any($3)
           and missing_passes > 0
        "#,
        tenant_id.as_uuid(),
        directory_source,
        &ids,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    let returned = returned.rows_affected();
    tracing::Span::current().record("sync.returned", returned);
    Ok(returned)
}

/// Forgets every absence hypothesis in the tenant.
///
/// For a connector change: a directory we have never enumerated has not
/// failed to list anybody, so carrying counts across would let one
/// connector's blind spot seal people under another's name.
#[tracing::instrument(
    name = "store.directory_sync.reset_absences",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn reset_absences(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
) -> Result<u64> {
    let cleared = sqlx::query!(
        r#"
        update scim_users
           set missing_passes = 0, missing_since = null
         where tenant_id = $1 and directory_source = $2 and missing_passes > 0
        "#,
        tenant_id.as_uuid(),
        directory_source,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(cleared.rows_affected())
}

/// The live mirror rows absent for at least `passes` consecutive complete
/// passes — the set a leaver signal would be built from.
///
/// This **offers**; it does not seal. What the caller does with the set is
/// ADR-0060 decision 3.3's question, and the answer depends on how big it is.
/// Ordered by how long they have been missing so a truncated read is the
/// oldest rather than an arbitrary slice.
#[tracing::instrument(
    name = "store.directory_sync.absent_at_least",
    skip_all,
    fields(tenant.id = %tenant_id, sync.threshold = passes, sync.found = tracing::field::Empty),
    err(Display)
)]
pub async fn absent_at_least(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    directory_source: &str,
    passes: i32,
) -> Result<Vec<DirectoryUser>> {
    let rows = sqlx::query_as!(
        UserRow,
        r#"
        select id, tenant_id, directory_source, external_id, user_name, active, display_name,
               given_name, family_name, work_email, identity_id, version,
               created_at, updated_at
        from scim_users
        where tenant_id = $1
          and directory_source = $2
          and active
          and missing_passes >= $3
        order by missing_since, id
        "#,
        tenant_id.as_uuid(),
        directory_source,
        passes,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("sync.found", rows.len());
    Ok(rows.into_iter().map(Into::into).collect())
}

// ── The breaker's release ───────────────────────────────────────────────

/// Grants a seal authorisation (ADR-0060 decision 10).
///
/// The window starts **now** and both ends come from the database's clock,
/// so an authorisation can never be born already expired by a skewed one.
/// Any authorisation already in
/// force is replaced rather than added to: at most one stands at a time, so
/// there is never a question of which ceiling applies.
///
/// Returns `false` when the tenant has no sync state, which is the honest
/// answer to "authorise seals for a directory we have never enumerated" —
/// there is no trip to release and no connector on record.
#[tracing::instrument(
    name = "store.directory_sync.authorise_seals",
    skip_all,
    fields(tenant.id = %tenant_id, sync.ceiling = ceiling, sync.granted_by = granted_by),
    err(Display)
)]
pub async fn authorise_seals(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    ceiling: i32,
    duration_secs: f64,
    granted_by: &str,
    reason: &str,
) -> Result<bool> {
    let granted = sqlx::query!(
        r#"
        update directory_sync_state
           set seal_authorised_at = now(),
               seal_authorised_until = now() + make_interval(secs => $2::double precision),
               seal_authorised_ceiling = $3,
               seal_authorised_by = $4,
               seal_authorised_reason = $5,
               updated_at = now()
         where tenant_id = $1
        "#,
        tenant_id.as_uuid(),
        duration_secs,
        ceiling,
        granted_by,
        reason,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(granted.rows_affected() == 1)
}

/// Spends the in-force authorisation if it covers `proposed` seals.
///
/// One conditional statement, and the condition is the whole rule: in force
/// at the database's own `now()`, and a ceiling at least as large as what
/// the pass actually proposes. A pass that finds 305 where the operator
/// sized 300 spends nothing and trips again — the ceiling is a bound and not
/// a hint, which is what refuses "authorise 300, the directory degrades
/// further, seal 5,000".
///
/// **One-shot**: the row is cleared by the same statement that reads it, so
/// two passes cannot both spend one authorisation and a granted window
/// cannot outlive the incident it was granted for. The caller keeps the
/// [`SealAuthorisation`] it read from [`state`] for the chain event; nothing
/// is returned here but whether it fired, because a second read after the
/// clear would find nothing by design.
///
/// The clock is the database's `now()`, which is `transaction_timestamp()`
/// and therefore **frozen for the life of the calling transaction**. So the
/// window is judged as at the moment the transaction opened, not the moment
/// this statement runs: one pass judges one clock throughout, matching the
/// policy-relaxation read predicate. It also
/// means an authorisation cannot expire out from under a pass mid-way, and
/// that a pass held open for longer than the window could still spend one —
/// immaterial while windows are operator-sized in hours and this runs in a
/// short transaction, and worth knowing before somebody wraps a whole
/// enumeration in a single transaction and finds the semantics have moved.
#[tracing::instrument(
    name = "store.directory_sync.spend_seal_authorisation",
    skip_all,
    fields(tenant.id = %tenant_id, sync.proposed = proposed, sync.spent = tracing::field::Empty),
    err(Display)
)]
pub async fn spend_seal_authorisation(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    proposed: i32,
) -> Result<bool> {
    let spent = sqlx::query!(
        r#"
        update directory_sync_state
           set seal_authorised_at = null,
               seal_authorised_until = null,
               seal_authorised_ceiling = null,
               seal_authorised_by = null,
               seal_authorised_reason = null,
               updated_at = now()
         where tenant_id = $1
           and seal_authorised_until > now()
           and seal_authorised_ceiling >= $2
        "#,
        tenant_id.as_uuid(),
        proposed,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    let spent = spent.rows_affected() == 1;
    tracing::Span::current().record("sync.spent", spent);
    Ok(spent)
}
