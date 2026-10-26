//! Operator tenant suspension and resumption (ADR-0157).
//!
//! A transition runs inside one migrator tenant transaction that holds the
//! tenant's exclusive admission lock, so every ordinary transaction admitted
//! earlier has finished and every later one re-reads the new status.

use sqlx::PgConnection;
use synveda_types::{Error, Result, TenantId, TenantStatus};

/// The stable refusal while admitted work holds the tenant's shared lock.
pub const TENANT_BUSY: &str = "tenant_busy";

/// The stable refusal when another transition committed first.
pub const TENANT_STATE_CHANGED: &str = "tenant_state_changed";

/// The stable refusal for a tenant whose erasure has started; only erasure
/// may continue it.
pub const TENANT_ERASING: &str = "tenant_erasing";

/// The stable refusal for an erased tenant's tombstone.
pub const TENANT_ERASED: &str = "tenant_erased";

/// A requested lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// `active` to `suspended`.
    Suspend,
    /// `suspended` to `active`.
    Resume,
}

impl Transition {
    const fn from(self) -> TenantStatus {
        match self {
            Self::Suspend => TenantStatus::Active,
            Self::Resume => TenantStatus::Suspended,
        }
    }

    /// The status the transition produces.
    #[must_use]
    pub const fn to(self) -> TenantStatus {
        match self {
            Self::Suspend => TenantStatus::Suspended,
            Self::Resume => TenantStatus::Active,
        }
    }
}

/// What a transition should do given the status read under the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Move from the required prior status.
    Apply,
    /// The tenant is already in the requested status; change nothing.
    AlreadyThere,
    /// Erasure is one-way; the stable reason names why.
    Refused(&'static str),
}

/// Decides a transition from the locked status. Pure.
#[must_use]
pub fn plan(current: TenantStatus, transition: Transition) -> Plan {
    match current {
        TenantStatus::Erasing => Plan::Refused(TENANT_ERASING),
        TenantStatus::Erased => Plan::Refused(TENANT_ERASED),
        _ if current == transition.to() => Plan::AlreadyThere,
        _ => {
            debug_assert_eq!(current, transition.from());
            Plan::Apply
        }
    }
}

/// Acquires the tenant's exclusive admission lock. The migrator authority
/// snapshot's statement bound ends the wait before the lock bound; either
/// expiry is [`TENANT_BUSY`], never a storage fault, and leaves the
/// transaction unusable.
pub async fn lock_exclusive(connection: &mut PgConnection, tenant: TenantId) -> Result<()> {
    sqlx::query!(
        r#"select set_config('synveda.tenant_id', $1, true) as "tenant!", set_config('lock_timeout', '5s', true) as "lock_timeout!""#,
        tenant.as_uuid().to_string(),
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(storage_error)?;
    sqlx::query!(
        "select pg_advisory_xact_lock(hashtextextended('synveda.admission:' || $1::text, 0))",
        tenant.as_uuid().to_string(),
    )
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| refusal(error, &["55P03", "57014"], TENANT_BUSY))
}

/// Moves the tenant from the transition's required prior status. Returns
/// `false` when the row was not in that status, which a caller holding the
/// exclusive lock treats as a refusal. The migrator transaction reads one
/// snapshot, so a transition another operator committed meanwhile is
/// [`TENANT_STATE_CHANGED`].
pub async fn apply(
    connection: &mut PgConnection,
    tenant: TenantId,
    transition: Transition,
) -> Result<bool> {
    let updated = sqlx::query!(
        "update tenants set status = $3 where id = $1 and status = $2",
        tenant.as_uuid(),
        transition.from().as_str(),
        transition.to().as_str(),
    )
    .execute(&mut *connection)
    .await
    .map_err(|error| refusal(error, &["40001"], TENANT_STATE_CHANGED))?;
    Ok(updated.rows_affected() == 1)
}

fn refusal(error: sqlx::Error, codes: &[&str], reason: &str) -> Error {
    match &error {
        sqlx::Error::Database(db)
            if db.code().is_some_and(|code| codes.contains(&code.as_ref())) =>
        {
            Error::Conflict {
                message: reason.to_owned(),
            }
        }
        _ => storage_error(error),
    }
}

fn storage_error(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("tenant lifecycle transition: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeating_a_completed_transition_changes_nothing() {
        assert_eq!(
            plan(TenantStatus::Suspended, Transition::Suspend),
            Plan::AlreadyThere
        );
        assert_eq!(
            plan(TenantStatus::Active, Transition::Resume),
            Plan::AlreadyThere
        );
    }

    #[test]
    fn each_transition_names_its_prior_and_next_status() {
        assert_eq!(plan(TenantStatus::Active, Transition::Suspend), Plan::Apply);
        assert_eq!(
            plan(TenantStatus::Suspended, Transition::Resume),
            Plan::Apply
        );
        assert_eq!(Transition::Suspend.to(), TenantStatus::Suspended);
        assert_eq!(Transition::Resume.to(), TenantStatus::Active);
    }

    #[test]
    fn erasure_is_one_way() {
        for transition in [Transition::Suspend, Transition::Resume] {
            assert_eq!(
                plan(TenantStatus::Erasing, transition),
                Plan::Refused(TENANT_ERASING)
            );
            assert_eq!(
                plan(TenantStatus::Erased, transition),
                Plan::Refused(TENANT_ERASED)
            );
        }
    }
}
