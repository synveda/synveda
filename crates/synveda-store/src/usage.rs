//! Durable per-tenant daily usage and expiring budget overrides.
//!
//! The accounting day is the database's UTC date, never a process clock. A
//! charge is one statement inside the transaction that accepts the measured
//! work, so a rollback, a duplicate or an idempotent replay charges nothing and
//! concurrent charges serialise on the day's row. The effective daily limit is
//! the deployment ceiling, narrowed by an unexpired override; an override can
//! never raise it. Runtime grants allow inserting and adding to a day's units
//! but not deleting them.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, Result, TenantId};

/// Most days the usage view reads back, including today.
pub const MAX_USAGE_DAYS: i32 = 31;
/// Longest lifetime of one budget override.
pub const MAX_OVERRIDE_SECS: i64 = 90 * 86_400;
/// Longest override reason, in characters.
pub const MAX_REASON_CHARS: usize = 500;

/// A durable usage meter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Meter {
    /// Compact JSON bytes of accepted, stored Session event payloads.
    SessionEventBytes,
    /// Created ContextRuns.
    ContextRuns,
}

impl Meter {
    /// Every meter, in storage order.
    pub const ALL: [Self; 2] = [Self::ContextRuns, Self::SessionEventBytes];

    /// Stable storage and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionEventBytes => "session_event_bytes",
            Self::ContextRuns => "context_runs",
        }
    }
}

impl fmt::Display for Meter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Meter {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|meter| meter.as_str() == value)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown usage meter {value:?}"),
            })
    }
}

/// The compact JSON encoding length, the same measure that bounds one stored
/// event payload, counted without allocating the encoding.
#[must_use]
pub fn encoded_bytes(value: &serde_json::Value) -> i64 {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len() as u64);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    // Writing a `Value` to an infallible sink cannot fail.
    let _ = serde_json::to_writer(&mut counter, value);
    i64::try_from(counter.0).unwrap_or(i64::MAX)
}

/// A landed charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charged {
    /// The day's units after this charge.
    pub units: i64,
    /// Whether the day now exceeds its effective limit, which only an
    /// unenforced charge can do.
    pub over_budget: bool,
}

/// Adds `units` to today's meter.
///
/// With `enforce`, a charge that would take the day past its effective limit
/// writes nothing and returns `None`; the caller refuses the work and rolls
/// back. Without it the charge always lands and reports the overrun.
///
/// # Errors
///
/// [`Error::Invalid`] for a non-positive charge or ceiling;
/// [`Error::Storage`] when the database fails.
pub async fn charge(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    meter: Meter,
    units: i64,
    ceiling: i64,
    enforce: bool,
) -> Result<Option<Charged>> {
    if units <= 0 || ceiling <= 0 {
        return Err(Error::Invalid {
            message: "a usage charge and its ceiling are positive".to_owned(),
        });
    }
    let row = sqlx::query!(
        r#"
        with budget as (
            select coalesce(
                (select least(override.daily_limit, $4)
                   from tenant_usage_budgets as override
                  where override.tenant_id = $1 and override.meter = $2
                    and override.expires_at > statement_timestamp()),
                $4) as daily_limit
        )
        insert into tenant_usage as usage (tenant_id, meter, usage_day, units)
        select $1, $2, (statement_timestamp() at time zone 'UTC')::date, $3
          from budget
         where not $5 or $3 <= budget.daily_limit
        on conflict (tenant_id, meter, usage_day) do update
           set units = usage.units + excluded.units
         where not $5 or usage.units + excluded.units <= (select daily_limit from budget)
        returning usage.units as "units!",
                  usage.units > (select daily_limit from budget) as "over_budget!"
        "#,
        tenant_id.as_uuid(),
        meter.as_str(),
        units,
        ceiling,
        enforce,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(row.map(|row| Charged {
        units: row.units,
        over_budget: row.over_budget,
    }))
}

/// Today's units and effective limit for one meter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Standing {
    /// Units charged today.
    pub used: i64,
    /// The effective daily limit.
    pub daily_limit: i64,
}

/// Reads today's standing without charging, so expensive work can be refused
/// before it starts. The charge that accepts the work still decides.
///
/// # Errors
///
/// [`Error::Storage`] when the database fails.
pub async fn standing(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    meter: Meter,
    ceiling: i64,
) -> Result<Standing> {
    let row = sqlx::query!(
        r#"
        select coalesce(
                   (select units from tenant_usage
                     where tenant_id = $1 and meter = $2
                       and usage_day = (statement_timestamp() at time zone 'UTC')::date),
                   0) as "used!",
               coalesce(
                   (select least(daily_limit, $3) from tenant_usage_budgets
                     where tenant_id = $1 and meter = $2
                       and expires_at > statement_timestamp()),
                   $3) as "daily_limit!"
        "#,
        tenant_id.as_uuid(),
        meter.as_str(),
        ceiling,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(Standing {
        used: row.used,
        daily_limit: row.daily_limit,
    })
}

/// One meter's units on one UTC day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageDay {
    /// The meter.
    pub meter: Meter,
    /// The UTC day.
    pub day: NaiveDate,
    /// Units charged that day.
    pub units: i64,
}

/// The database's UTC day and the tenant's usage for the last `days` days,
/// newest first.
///
/// # Errors
///
/// [`Error::Invalid`] for a window outside `1..=MAX_USAGE_DAYS`;
/// [`Error::Storage`] when the database fails.
pub async fn recent(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    days: i32,
) -> Result<(NaiveDate, Vec<UsageDay>)> {
    if !(1..=MAX_USAGE_DAYS).contains(&days) {
        return Err(Error::Invalid {
            message: format!("a usage window is 1 to {MAX_USAGE_DAYS} days"),
        });
    }
    let today = sqlx::query_scalar!(
        r#"select (statement_timestamp() at time zone 'UTC')::date as "today!""#
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let limit = i64::from(days) * Meter::ALL.len() as i64;
    let rows = sqlx::query!(
        r#"
        select meter, usage_day, units
          from tenant_usage
         where tenant_id = $1
           and meter in ('session_event_bytes', 'context_runs')
           and usage_day > $2::date - $3::int
         order by usage_day desc, meter
         limit $4
        "#,
        tenant_id.as_uuid(),
        today,
        days,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    let usage = rows
        .into_iter()
        .map(|row| {
            Ok(UsageDay {
                meter: row.meter.parse()?,
                day: row.usage_day,
                units: row.units,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((today, usage))
}

/// An unexpired narrowing of one meter's daily limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetOverride {
    /// The meter it narrows.
    pub meter: Meter,
    /// The daily limit, applied as at most the deployment ceiling.
    pub daily_limit: i64,
    /// Why it was set.
    pub reason: String,
    /// The verified subject that set it.
    pub set_by: String,
    /// When it was set, in database time.
    pub set_at: DateTime<Utc>,
    /// When it stops applying, in database time.
    pub expires_at: DateTime<Utc>,
}

struct OverrideRow {
    meter: String,
    daily_limit: i64,
    reason: String,
    set_by: String,
    set_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl TryFrom<OverrideRow> for BudgetOverride {
    type Error = Error;

    fn try_from(row: OverrideRow) -> Result<Self> {
        Ok(Self {
            meter: row.meter.parse()?,
            daily_limit: row.daily_limit,
            reason: row.reason,
            set_by: row.set_by,
            set_at: row.set_at,
            expires_at: row.expires_at,
        })
    }
}

/// The tenant's unexpired overrides.
///
/// # Errors
///
/// [`Error::Storage`] when the database fails.
pub async fn overrides(
    conn: &mut PgConnection,
    tenant_id: TenantId,
) -> Result<Vec<BudgetOverride>> {
    sqlx::query_as!(
        OverrideRow,
        r#"
        select meter, daily_limit, reason, set_by, set_at, expires_at
          from tenant_usage_budgets
         where tenant_id = $1 and expires_at > statement_timestamp()
         order by meter
         limit 2
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?
    .into_iter()
    .map(BudgetOverride::try_from)
    .collect()
}

/// A requested override.
#[derive(Clone, Debug)]
pub struct NewOverride<'a> {
    /// The meter to narrow.
    pub meter: Meter,
    /// The narrowed daily limit; zero stops the meter's work for the day.
    pub daily_limit: i64,
    /// Why, without customer content.
    pub reason: &'a str,
    /// The verified subject setting it.
    pub set_by: &'a str,
    /// Seconds until it expires.
    pub ttl_secs: i64,
}

/// Replaces the meter's override and returns the one it replaced, if that
/// was still in force.
///
/// # Errors
///
/// [`Error::Invalid`] for an out-of-bound limit, reason or lifetime;
/// [`Error::Storage`] when the database fails.
pub async fn set_override(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    change: &NewOverride<'_>,
) -> Result<(BudgetOverride, Option<BudgetOverride>)> {
    validate_override(change)?;
    let previous = locked_override(conn, tenant_id, change.meter).await?;
    let row = sqlx::query_as!(
        OverrideRow,
        r#"
        insert into tenant_usage_budgets
            (tenant_id, meter, daily_limit, reason, set_by, set_at, expires_at)
        values ($1, $2, $3, $4, $5, statement_timestamp(),
                statement_timestamp() + make_interval(secs => $6::bigint::double precision))
        on conflict (tenant_id, meter) do update
           set daily_limit = excluded.daily_limit, reason = excluded.reason,
               set_by = excluded.set_by, set_at = excluded.set_at,
               expires_at = excluded.expires_at
        returning meter, daily_limit, reason, set_by, set_at, expires_at
        "#,
        tenant_id.as_uuid(),
        change.meter.as_str(),
        change.daily_limit,
        change.reason,
        change.set_by,
        change.ttl_secs,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok((row.try_into()?, previous))
}

/// Removes the meter's override, returning the removed row, expired or not.
///
/// # Errors
///
/// [`Error::Storage`] when the database fails.
pub async fn clear_override(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    meter: Meter,
) -> Result<Option<BudgetOverride>> {
    let row = sqlx::query_as!(
        OverrideRow,
        r#"
        delete from tenant_usage_budgets
         where tenant_id = $1 and meter = $2
        returning meter, daily_limit, reason, set_by, set_at, expires_at
        "#,
        tenant_id.as_uuid(),
        meter.as_str(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(BudgetOverride::try_from).transpose()
}

async fn locked_override(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    meter: Meter,
) -> Result<Option<BudgetOverride>> {
    sqlx::query_as!(
        OverrideRow,
        r#"
        select meter, daily_limit, reason, set_by, set_at, expires_at
          from tenant_usage_budgets
         where tenant_id = $1 and meter = $2 and expires_at > statement_timestamp()
         for update
        "#,
        tenant_id.as_uuid(),
        meter.as_str(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?
    .map(BudgetOverride::try_from)
    .transpose()
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23514") => {
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

fn validate_override(change: &NewOverride<'_>) -> Result<()> {
    let invalid = |message: &str| {
        Err(Error::Invalid {
            message: message.to_owned(),
        })
    };
    if change.daily_limit < 0 {
        return invalid("a daily limit is zero or more");
    }
    if change.reason.trim().is_empty() || change.reason.chars().count() > MAX_REASON_CHARS {
        return invalid("an override reason is 1 to 500 characters");
    }
    if change.set_by.trim().is_empty() || change.set_by.chars().count() > 255 {
        return invalid("an override needs its verified subject");
    }
    if !(60..=MAX_OVERRIDE_SECS).contains(&change.ttl_secs) {
        return invalid("an override lasts between one minute and 90 days");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encoded_bytes_is_the_compact_json_length() {
        for value in [
            json!({}),
            json!({"text": "é and \"quotes\"", "n": [1, 2.5, null, true]}),
        ] {
            assert_eq!(encoded_bytes(&value), value.to_string().len() as i64);
        }
    }

    #[test]
    fn meters_round_trip_and_unknown_meters_are_refused() {
        for meter in Meter::ALL {
            assert_eq!(meter.as_str().parse::<Meter>().expect("meter"), meter);
        }
        assert!("tokens".parse::<Meter>().is_err());
    }

    #[test]
    fn override_bounds_are_checked_before_storage() {
        let base = NewOverride {
            meter: Meter::ContextRuns,
            daily_limit: 0,
            reason: "incident 42: runaway agent",
            set_by: "ops@example.test",
            ttl_secs: 3_600,
        };
        validate_override(&base).expect("zero pauses the meter");
        for change in [
            NewOverride {
                daily_limit: -1,
                ..base.clone()
            },
            NewOverride {
                reason: " ",
                ..base.clone()
            },
            NewOverride {
                ttl_secs: 59,
                ..base.clone()
            },
            NewOverride {
                ttl_secs: MAX_OVERRIDE_SECS + 1,
                ..base.clone()
            },
        ] {
            assert!(validate_override(&change).is_err());
        }
        let long = "r".repeat(MAX_REASON_CHARS + 1);
        assert!(
            validate_override(&NewOverride {
                reason: &long,
                ..base
            })
            .is_err()
        );
    }
}
