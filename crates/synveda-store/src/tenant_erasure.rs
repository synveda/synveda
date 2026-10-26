//! Tenant erasure under the interim fixture rules (ADR-0157).
//!
//! Erasure deletes a suspended tenant's rows in bounded steps. Each step is
//! one migrator transaction that holds the tenant's exclusive admission lock
//! and the `tenant_erasure` operation's lease, names the tenant in the
//! `synveda.tenant_erasure` setting so the guards and restrictive policies of
//! migration 0015 admit the deletion, deletes at most one chunk and records the
//! measured count in the operation's result in the same transaction. A crash
//! loses at most the uncommitted step; the next run reclaims the expired lease
//! and continues from the recorded cursor without counting a row twice.
//!
//! The deletion order is derived from the live foreign-key catalogue so that
//! every referencing row is gone before the row it references. Tables in a
//! reference cycle form declared units: `scopes` references itself and is
//! deleted leaves first; five parents name their current version and are
//! deleted with it after every other version; the inbound source cluster has
//! only deferrable constraints and is deleted in one step under a declared row
//! bound. An undeclared cycle refuses the plan before anything is deleted.
//!
//! What remains is the tenant row as a tombstone (ID, slug and status), the
//! content-free audit chain and the tenant erasure operations and attempts.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use serde_json::json;
use sqlx::PgConnection;
use synveda_types::{DurableOperationId, Error, Result, TenantId, TenantStatus};

use crate::tenant_inventory::{EXTERNAL_OBJECTS, Erasure, TENANT_TABLES, is_plain_identifier};

/// Erasure starts only from a suspended tenant.
pub const TENANT_NOT_SUSPENDED: &str = "tenant_not_suspended";
/// A Knowledge revision of the tenant is under a legal or retention hold.
pub const ERASURE_HELD: &str = "erasure_held";
/// Another process holds the live erasure lease.
pub const ERASURE_IN_PROGRESS: &str = "erasure_in_progress";
/// The lease expired or another process reclaimed it.
pub const ERASURE_LEASE_LOST: &str = "erasure_lease_lost";
/// The catalogue has a reference cycle no unit declares.
pub const ERASURE_PLAN_UNSUPPORTED: &str = "erasure_plan_unsupported";
/// The atomic source unit holds more rows than one step may delete.
pub const ERASURE_UNIT_TOO_LARGE: &str = "erasure_unit_too_large";
/// Rows remain that the plan should have deleted.
pub const ERASURE_INCOMPLETE: &str = "erasure_incomplete";
/// The receipt format.
pub const RECEIPT_FORMAT: &str = "synveda.tenant-erasure-receipt.v1";

/// Largest chunk one statement may delete.
pub const MAX_CHUNK_ROWS: u32 = 50_000;
/// Rows the atomic source unit may hold for its single step.
pub const ATOMIC_UNIT_MAX_ROWS: u64 = 1_000_000;
/// Longest lease a caller may request.
pub const MAX_LEASE: Duration = Duration::from_secs(300);
/// Longest statement bound a caller may request.
pub const MAX_STATEMENT_TIMEOUT: Duration = Duration::from_secs(3600);

const OPERATION_KIND: &str = "tenant_erasure";
const MAX_FOREIGN_KEYS: i64 = 4_096;
const MAX_HELD_COUNT: i64 = 1_000_000;
const RESIDUAL_COUNT_CAP: u64 = 100_000_000;

/// A self-referencing table deleted children first.
#[derive(Debug, PartialEq, Eq)]
pub struct SelfReference {
    /// The table.
    pub table: &'static str,
    /// The referenced key column.
    pub key: &'static str,
    /// The referencing parent column.
    pub parent: &'static str,
}

/// A parent whose deferred foreign key names its current version.
#[derive(Debug, PartialEq, Eq)]
pub struct VersionPair {
    /// The parent table; its key column is `id`.
    pub parent: &'static str,
    /// The parent's current-version column.
    pub current: &'static str,
    /// The version table; its key column is `id`.
    pub version: &'static str,
    /// The version's parent column.
    pub owner: &'static str,
}

const SCOPES: SelfReference = SelfReference {
    table: "scopes",
    key: "id",
    parent: "parent_scope_id",
};

const VERSION_PAIRS: [VersionPair; 5] = [
    VersionPair {
        parent: "configuration_artifacts",
        current: "current_version_id",
        version: "configuration_versions",
        owner: "artifact_id",
    },
    VersionPair {
        parent: "knowledge_items",
        current: "current_revision_id",
        version: "knowledge_revisions",
        owner: "knowledge_item_id",
    },
    VersionPair {
        parent: "policy_relaxations",
        current: "current_version_id",
        version: "policy_relaxation_versions",
        owner: "relaxation_id",
    },
    VersionPair {
        parent: "skills",
        current: "current_version_id",
        version: "skill_versions",
        owner: "skill_id",
    },
    VersionPair {
        parent: "tool_servers",
        current: "current_version_id",
        version: "tool_server_versions",
        owner: "server_id",
    },
];

/// Every foreign key among these tables is deferrable.
const SOURCE_CLUSTER: [&str; 5] = [
    "capture_batches",
    "import_jobs",
    "source_observations",
    "source_reconcile_heads",
    "source_reconcile_requests",
];

const CONSOLE_SESSION_INDEX: &str = "console_session_index";

/// One step-sized piece of the deletion plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Every row of one table.
    Rows(&'static str),
    /// Every row except those of tenant erasure operations.
    Lifecycle(&'static str),
    /// Console custody rows named by the tenant's index; the index follows.
    ConsoleSessions,
    /// A self-referencing table, children first.
    Leaves(&'static SelfReference),
    /// A parent naming its current version, and that version table.
    Versioned(&'static VersionPair),
    /// Tables whose mutual references are all deferrable, in one step.
    Atomic(&'static [&'static str]),
}

impl Unit {
    /// The tenant tables this unit empties.
    #[must_use]
    pub fn tables(&self) -> Vec<&'static str> {
        match self {
            Self::Rows(table) | Self::Lifecycle(table) => vec![table],
            Self::ConsoleSessions => vec![CONSOLE_SESSION_INDEX],
            Self::Leaves(reference) => vec![reference.table],
            Self::Versioned(pair) => vec![pair.parent, pair.version],
            Self::Atomic(tables) => tables.to_vec(),
        }
    }

    /// A stable name for progress output and the plan digest.
    #[must_use]
    pub fn label(&self) -> String {
        let kind = match self {
            Self::Rows(_) => "rows",
            Self::Lifecycle(_) => "lifecycle",
            Self::ConsoleSessions => "console_sessions",
            Self::Leaves(_) => "leaves",
            Self::Versioned(_) => "versioned",
            Self::Atomic(_) => "atomic",
        };
        format!("{kind}:{}", self.tables().join(","))
    }
}

/// The ordered deletion plan and its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    units: Vec<Unit>,
    digest: String,
}

impl Plan {
    /// Units in deletion order.
    #[must_use]
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// BLAKE3 of the unit sequence.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Reads the public foreign keys and derives the plan.
pub async fn plan(connection: &mut PgConnection) -> Result<Plan> {
    let edges: Vec<(String, String)> = sqlx::query_as(
        "select child.relname::text, parent.relname::text
           from pg_catalog.pg_constraint as fk
           join pg_catalog.pg_class as child on child.oid = fk.conrelid
           join pg_catalog.pg_class as parent on parent.oid = fk.confrelid
          where fk.contype = 'f' and fk.connamespace = 'public'::regnamespace
          order by 1, 2
          limit $1",
    )
    .bind(MAX_FOREIGN_KEYS)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| storage("read the foreign-key catalogue", &error))?;
    if i64::try_from(edges.len()).unwrap_or(i64::MAX) >= MAX_FOREIGN_KEYS {
        return Err(refused(
            ERASURE_PLAN_UNSUPPORTED,
            "the foreign-key catalogue exceeds its bound",
        ));
    }
    derive(&edges)
}

/// Orders the units so that each is deleted after every unit that references
/// it. `edges` are (referencing table, referenced table). Pure; every loop is
/// bounded by the reviewed table list or the edge list.
pub fn derive(edges: &[(String, String)]) -> Result<Plan> {
    let mut units: Vec<Unit> = Vec::with_capacity(TENANT_TABLES.len());
    let mut rank: Vec<usize> = Vec::with_capacity(TENANT_TABLES.len());
    let mut unit_of: HashMap<&'static str, usize> = HashMap::with_capacity(TENANT_TABLES.len());
    let mut retained: Vec<&'static str> = Vec::new();
    for (position, table) in TENANT_TABLES.iter().enumerate() {
        let unit = match table.erasure {
            Erasure::RetainEvidence => {
                retained.push(table.name);
                continue;
            }
            Erasure::RetainLifecycleRows => Unit::Lifecycle(table.name),
            Erasure::Erase if table.name == CONSOLE_SESSION_INDEX => Unit::ConsoleSessions,
            Erasure::Erase if table.name == SCOPES.table => Unit::Leaves(&SCOPES),
            Erasure::Erase if SOURCE_CLUSTER.contains(&table.name) => Unit::Atomic(&SOURCE_CLUSTER),
            Erasure::Erase => VERSION_PAIRS
                .iter()
                .find(|pair| pair.parent == table.name || pair.version == table.name)
                .map_or(Unit::Rows(table.name), Unit::Versioned),
        };
        let index = if let Some(index) = units.iter().position(|known| *known == unit) {
            index
        } else {
            units.push(unit);
            rank.push(position);
            units.len() - 1
        };
        rank[index] = rank[index].max(position);
        unit_of.insert(table.name, index);
    }
    for unit in &units {
        for table in unit.tables() {
            if !unit_of.contains_key(table) || !is_plain_identifier(table) {
                return Err(refused(
                    ERASURE_PLAN_UNSUPPORTED,
                    &format!("declared unit table {table} is not an erased tenant table"),
                ));
            }
        }
    }

    let mut referencers: Vec<Vec<usize>> = vec![Vec::new(); units.len()];
    for (child, parent) in edges {
        if retained.contains(&child.as_str()) && unit_of.contains_key(parent.as_str()) {
            return Err(refused(
                ERASURE_PLAN_UNSUPPORTED,
                &format!("retained {child} references erased {parent}"),
            ));
        }
        let (Some(&from), Some(&to)) = (unit_of.get(child.as_str()), unit_of.get(parent.as_str()))
        else {
            continue;
        };
        if from == to {
            if matches!(
                units[from],
                Unit::Rows(_) | Unit::Lifecycle(_) | Unit::ConsoleSessions
            ) {
                return Err(refused(
                    ERASURE_PLAN_UNSUPPORTED,
                    &format!("{child} references itself without a declared unit"),
                ));
            }
            continue;
        }
        if !referencers[to].contains(&from) {
            referencers[to].push(from);
        }
    }
    let mut blockers: Vec<usize> = referencers.iter().map(Vec::len).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); units.len()];
    for (to, from_units) in referencers.iter().enumerate() {
        for &from in from_units {
            dependents[from].push(to);
        }
    }

    let mut ordered: Vec<Unit> = Vec::with_capacity(units.len());
    let mut placed = vec![false; units.len()];
    for _ in 0..units.len() {
        let next = (0..units.len())
            .filter(|&index| !placed[index] && blockers[index] == 0)
            .max_by_key(|&index| rank[index]);
        let Some(next) = next else {
            let cycle: Vec<String> = (0..units.len())
                .filter(|&index| !placed[index])
                .map(|index| units[index].label())
                .collect();
            return Err(refused(
                ERASURE_PLAN_UNSUPPORTED,
                &format!("undeclared reference cycle among {}", cycle.join(" ")),
            ));
        };
        placed[next] = true;
        ordered.push(units[next]);
        for &to in &dependents[next] {
            blockers[to] = blockers[to].saturating_sub(1);
        }
    }
    let mut hasher = blake3::Hasher::new();
    for unit in &ordered {
        hasher.update(unit.label().as_bytes());
        hasher.update(b"\n");
    }
    Ok(Plan {
        units: ordered,
        digest: hasher.finalize().to_hex().to_string(),
    })
}

/// The erasure operation's lease as one process holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The `tenant_erasure` durable operation.
    pub operation: DurableOperationId,
    /// The attempt this process runs.
    pub attempt: i32,
    /// This process's lease owner name.
    pub worker: String,
}

/// What [`start`] found and did.
#[derive(Debug, Clone, PartialEq)]
pub enum Start {
    /// The tenant moved from suspended to erasing under a new operation.
    Started(Lease),
    /// An interrupted erasure continues under a reclaimed lease.
    Resumed(Lease),
    /// A hold blocked a new operation; nothing was deleted.
    Held {
        /// The blocked operation.
        operation: DurableOperationId,
        /// Held Knowledge revisions.
        revisions: i64,
    },
    /// The tenant is already erased; this is its stored receipt.
    Erased(serde_json::Value),
}

/// Bounds for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepLimits {
    /// Rows one statement deletes at most.
    pub chunk: u32,
    /// The lease each step renews.
    pub lease: Duration,
    /// The statement bound inside the step.
    pub statement_timeout: Duration,
}

/// What one step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The next unit to run.
    pub unit: usize,
    /// Units in the plan.
    pub units: usize,
    /// The unit this step ran, if any.
    pub label: Option<String>,
    /// Rows this step deleted.
    pub deleted: u64,
    /// Every unit has run.
    pub finished: bool,
}

/// The recorded walk, stored in the operation's result while it runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Cursor {
    plan: String,
    unit: usize,
    phase: u8,
    deleted: BTreeMap<String, u64>,
    console_sessions: u64,
}

impl Cursor {
    fn new(plan: &Plan) -> Self {
        Self {
            plan: plan.digest.clone(),
            unit: 0,
            phase: 0,
            deleted: BTreeMap::new(),
            console_sessions: 0,
        }
    }

    fn add(&mut self, table: &str, rows: u64) {
        let count = self.deleted.entry(table.to_owned()).or_insert(0);
        *count = count.saturating_add(rows);
    }
}

/// Starts or resumes erasure. The caller holds a migrator tenant transaction
/// and the tenant's exclusive admission lock.
pub async fn start(
    connection: &mut PgConnection,
    tenant: TenantId,
    plan: &Plan,
    worker: &str,
    lease: Duration,
) -> Result<Start> {
    let lease = interval(lease, MAX_LEASE)?;
    if worker.trim().is_empty() || worker.len() > 255 {
        return Err(Error::Invalid {
            message: "the erasure lease owner must be 1 to 255 bytes".to_owned(),
        });
    }
    let status = crate::tenants::by_id(&mut *connection, tenant)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("tenant {tenant}"),
        })?
        .status;
    match status {
        TenantStatus::Active => Err(refused(
            TENANT_NOT_SUSPENDED,
            "suspend the tenant before erasing it",
        )),
        TenantStatus::Erased => Ok(Start::Erased(stored_receipt(connection, tenant).await?)),
        TenantStatus::Suspended => begin(connection, tenant, plan, worker, &lease).await,
        TenantStatus::Erasing => reclaim(connection, tenant, plan, worker, &lease).await,
    }
}

async fn begin(
    connection: &mut PgConnection,
    tenant: TenantId,
    plan: &Plan,
    worker: &str,
    lease: &str,
) -> Result<Start> {
    let operation = DurableOperationId::new();
    let input_hash = blake3::hash(format!("{OPERATION_KIND}:1:{tenant}").as_bytes())
        .to_hex()
        .to_string();
    let held: i64 = sqlx::query_scalar(
        "select count(*) from (select 1 from public.knowledge_revision_headers
           where tenant_id = $1 and (legal_hold or retention_hold) limit $2) as held",
    )
    .bind(tenant.as_uuid())
    .bind(MAX_HELD_COUNT)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| storage("count held revisions", &error))?;
    if held > 0 {
        sqlx::query(
            "insert into public.durable_operations
               (tenant_id, id, kind, state, input_hash, next_attempt_at, last_error_code,
                completed_at, result)
             values ($1, $2, $3, 'blocked', $4, null, 'retention_hold', clock_timestamp(), $5)",
        )
        .bind(tenant.as_uuid())
        .bind(operation.as_uuid())
        .bind(OPERATION_KIND)
        .bind(&input_hash)
        .bind(json!({ "held_revisions": held }))
        .execute(&mut *connection)
        .await
        .map_err(|error| storage("record the held erasure", &error))?;
        return Ok(Start::Held {
            operation,
            revisions: held,
        });
    }
    let atomic = count_rows(connection, tenant, &SOURCE_CLUSTER, ATOMIC_UNIT_MAX_ROWS).await?;
    if atomic > ATOMIC_UNIT_MAX_ROWS {
        return Err(refused(
            ERASURE_UNIT_TOO_LARGE,
            &format!(
                "the inbound source tables hold more than {ATOMIC_UNIT_MAX_ROWS} rows; nothing was deleted"
            ),
        ));
    }
    let cursor = serde_json::to_value(Cursor::new(plan)).map_err(|error| Error::Internal {
        message: format!("encode the erasure cursor: {error}"),
    })?;
    sqlx::query(
        "insert into public.durable_operations
           (tenant_id, id, kind, state, input_hash, attempts, next_attempt_at, started_at,
            lease_owner, lease_expires_at, result)
         values ($1, $2, $3, 'running', $4, 1, null, clock_timestamp(), $5,
                 clock_timestamp() + $6::interval, $7)",
    )
    .bind(tenant.as_uuid())
    .bind(operation.as_uuid())
    .bind(OPERATION_KIND)
    .bind(&input_hash)
    .bind(worker)
    .bind(lease)
    .bind(cursor)
    .execute(&mut *connection)
    .await
    .map_err(|error| storage("record the erasure operation", &error))?;
    insert_attempt(connection, tenant, operation, 1, worker).await?;
    let moved = sqlx::query(
        "update public.tenants set status = 'erasing' where id = $1 and status = 'suspended'",
    )
    .bind(tenant.as_uuid())
    .execute(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, crate::tenant_lifecycle::TENANT_STATE_CHANGED))?;
    if moved.rows_affected() != 1 {
        return Err(refused(
            crate::tenant_lifecycle::TENANT_STATE_CHANGED,
            "the tenant left the suspended status",
        ));
    }
    Ok(Start::Started(Lease {
        operation,
        attempt: 1,
        worker: worker.to_owned(),
    }))
}

async fn reclaim(
    connection: &mut PgConnection,
    tenant: TenantId,
    plan: &Plan,
    worker: &str,
    lease_interval: &str,
) -> Result<Start> {
    let rows: Vec<(uuid::Uuid, i32, Option<String>, bool)> = sqlx::query_as(
        "select id, attempts, lease_owner, lease_expires_at > clock_timestamp()
           from public.durable_operations
          where tenant_id = $1 and kind = $2 and state = 'running'
          order by created_at limit 2",
    )
    .bind(tenant.as_uuid())
    .bind(OPERATION_KIND)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| storage("read the running erasure", &error))?;
    let [(operation, attempts, owner, live)] = rows.as_slice() else {
        return Err(refused(
            ERASURE_INCOMPLETE,
            "an erasing tenant must have exactly one running erasure operation",
        ));
    };
    let operation = DurableOperationId::from_uuid(*operation);
    if *live {
        if owner.as_deref() != Some(worker) {
            return Err(refused(
                ERASURE_IN_PROGRESS,
                "another process holds the live erasure lease; retry after it expires",
            ));
        }
        return Ok(Start::Resumed(Lease {
            operation,
            attempt: *attempts,
            worker: worker.to_owned(),
        }));
    }
    let attempt = attempts.saturating_add(1);
    let claimed = sqlx::query(
        "update public.durable_operations
            set attempts = $3, lease_owner = $4,
                lease_expires_at = clock_timestamp() + $5::interval,
                updated_at = clock_timestamp()
          where tenant_id = $1 and id = $2 and state = 'running' and attempts = $3 - 1
            and lease_expires_at <= clock_timestamp()",
    )
    .bind(tenant.as_uuid())
    .bind(operation.as_uuid())
    .bind(attempt)
    .bind(worker)
    .bind(lease_interval)
    .execute(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))?;
    if claimed.rows_affected() != 1 {
        return Err(refused(ERASURE_LEASE_LOST, "the erasure lease changed"));
    }
    sqlx::query(
        "update public.operation_attempts
            set state = 'retry_scheduled', safe_error_code = 'lease_expired',
                completed_at = clock_timestamp()
          where tenant_id = $1 and operation_id = $2 and attempt_number = $3
            and state = 'running'",
    )
    .bind(tenant.as_uuid())
    .bind(operation.as_uuid())
    .bind(*attempts)
    .execute(&mut *connection)
    .await
    .map_err(|error| storage("close the expired attempt", &error))?;
    insert_attempt(connection, tenant, operation, attempt, worker).await?;
    let lease = Lease {
        operation,
        attempt,
        worker: worker.to_owned(),
    };
    let mut cursor = read_cursor(connection, tenant, &lease, lease_interval).await?;
    if cursor.plan != plan.digest {
        // A migration changed the plan. Deleting is idempotent, so the walk
        // restarts and every count already measured is kept.
        cursor.plan.clone_from(&plan.digest);
        cursor.unit = 0;
        cursor.phase = 0;
        store_cursor(connection, tenant, &lease, &cursor, 0).await?;
    }
    Ok(Start::Resumed(lease))
}

/// Runs one step of the plan. The caller holds a migrator tenant transaction
/// and the tenant's exclusive admission lock, and commits after this returns.
pub async fn step(
    connection: &mut PgConnection,
    tenant: TenantId,
    plan: &Plan,
    lease: &Lease,
    limits: StepLimits,
) -> Result<Progress> {
    if limits.chunk == 0 || limits.chunk > MAX_CHUNK_ROWS {
        return Err(Error::Invalid {
            message: format!("the erasure chunk must be between 1 and {MAX_CHUNK_ROWS} rows"),
        });
    }
    let renewal = interval(limits.lease, MAX_LEASE)?;
    authorize(connection, tenant, limits.statement_timeout).await?;
    let mut cursor = read_cursor(connection, tenant, lease, &renewal).await?;
    if cursor.plan != plan.digest {
        return Err(refused(
            ERASURE_LEASE_LOST,
            "the erasure plan changed under this lease; rerun to reclaim it",
        ));
    }
    let units = plan.units.len();
    let Some(unit) = plan.units.get(cursor.unit).copied() else {
        return Ok(Progress {
            unit: cursor.unit,
            units,
            label: None,
            deleted: 0,
            finished: true,
        });
    };
    let chunk = i64::from(limits.chunk);
    let (deleted, next) = run_unit(connection, tenant, unit, cursor.phase, chunk).await?;
    let mut total = 0_u64;
    for (table, rows) in &deleted {
        if *table == "console_sessions" {
            cursor.console_sessions = cursor.console_sessions.saturating_add(*rows);
        } else {
            cursor.add(table, *rows);
            total = total.saturating_add(*rows);
        }
    }
    match next {
        Next::Done => {
            cursor.unit += 1;
            cursor.phase = 0;
        }
        Next::Phase(phase) => cursor.phase = phase,
        Next::Again => {}
    }
    let percent = i16::try_from((cursor.unit * 100 / units.max(1)).min(99)).unwrap_or(99);
    store_cursor(connection, tenant, lease, &cursor, percent).await?;
    Ok(Progress {
        unit: cursor.unit,
        units,
        label: Some(unit.label()),
        deleted: total,
        finished: cursor.unit >= units,
    })
}

/// Proves the residual inventory is empty, marks the tenant erased and
/// returns the stored receipt. The caller holds a migrator tenant transaction
/// and the tenant's exclusive admission lock.
pub async fn finish(
    connection: &mut PgConnection,
    tenant: TenantId,
    plan: &Plan,
    lease: &Lease,
    limits: StepLimits,
) -> Result<serde_json::Value> {
    let renewal = interval(limits.lease, MAX_LEASE)?;
    authorize(connection, tenant, limits.statement_timeout).await?;
    let cursor = read_cursor(connection, tenant, lease, &renewal).await?;
    if cursor.plan != plan.digest || cursor.unit < plan.units.len() {
        return Err(refused(
            ERASURE_INCOMPLETE,
            "erasure steps remain; rerun to continue",
        ));
    }
    let mut retained = BTreeMap::new();
    let mut residual = Vec::new();
    for table in &TENANT_TABLES {
        match table.erasure {
            Erasure::RetainEvidence => {
                retained.insert(
                    table.name.to_owned(),
                    count_rows(connection, tenant, &[table.name], RESIDUAL_COUNT_CAP).await?,
                );
            }
            Erasure::RetainLifecycleRows => {
                if exists(
                    connection,
                    tenant,
                    table.name,
                    lifecycle_predicate(table.name)?,
                )
                .await?
                {
                    residual.push(table.name);
                }
                retained.insert(
                    table.name.to_owned(),
                    count_rows(connection, tenant, &[table.name], RESIDUAL_COUNT_CAP).await?,
                );
            }
            Erasure::Erase => {
                if exists(connection, tenant, table.name, "").await? {
                    residual.push(table.name);
                }
            }
        }
    }
    if !residual.is_empty() {
        return Err(refused(
            ERASURE_INCOMPLETE,
            &format!("rows remain in {}; rerun to continue", residual.join(", ")),
        ));
    }
    retained.insert("tenants".to_owned(), 1);
    let (started_at, completed_at): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as(
            "select started_at, clock_timestamp() from public.durable_operations
              where tenant_id = $1 and id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(lease.operation.as_uuid())
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| storage("read the erasure times", &error))?;
    let mut deleted = BTreeMap::new();
    for table in &TENANT_TABLES {
        if table.erasure != Erasure::RetainEvidence {
            let rows = cursor.deleted.get(table.name).copied().unwrap_or(0);
            deleted.insert(table.name.to_owned(), rows);
        }
    }
    let deleted_rows: u64 = deleted.values().sum();
    let body = json!({
        "format": RECEIPT_FORMAT,
        "tenant": tenant,
        "operation": lease.operation,
        "operation_version": 1,
        "plan_digest": plan.digest,
        "attempts": lease.attempt,
        "started_at": started_at,
        "completed_at": completed_at,
        "deleted": deleted,
        "deleted_rows": deleted_rows,
        "deployment_deleted": { "console_sessions": cursor.console_sessions },
        "retained": retained,
        "residual_rows": 0,
        "unattributable_deployment_rows": ["cli_handoffs"],
        "unchanged_outside_database": EXTERNAL_OBJECTS.iter().map(|object| object.name).collect::<Vec<_>>(),
    });
    let encoded = serde_json::to_vec(&body).map_err(|error| Error::Internal {
        message: format!("encode the erasure receipt: {error}"),
    })?;
    let mut receipt = body;
    if let Some(object) = receipt.as_object_mut() {
        object.insert(
            "digest".to_owned(),
            blake3::hash(&encoded).to_hex().to_string().into(),
        );
    }
    let erased = sqlx::query(
        "update public.tenants set status = 'erased', name = '' where id = $1 and status = 'erasing'",
    )
    .bind(tenant.as_uuid())
    .execute(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, crate::tenant_lifecycle::TENANT_STATE_CHANGED))?;
    if erased.rows_affected() != 1 {
        return Err(refused(
            crate::tenant_lifecycle::TENANT_STATE_CHANGED,
            "the tenant left the erasing status",
        ));
    }
    let completed = sqlx::query(
        "update public.durable_operations
            set state = 'succeeded', progress_percent = 100, completed_at = clock_timestamp(),
                lease_owner = null, lease_expires_at = null, result = $3,
                updated_at = clock_timestamp()
          where tenant_id = $1 and id = $2 and state = 'running'",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(&receipt)
    .execute(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))?;
    if completed.rows_affected() != 1 {
        return Err(refused(ERASURE_LEASE_LOST, "the erasure operation changed"));
    }
    sqlx::query(
        "update public.operation_attempts set state = 'succeeded', completed_at = clock_timestamp()
          where tenant_id = $1 and operation_id = $2 and attempt_number = $3 and state = 'running'",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(lease.attempt)
    .execute(&mut *connection)
    .await
    .map_err(|error| storage("complete the erasure attempt", &error))?;
    Ok(receipt)
}

/// Ends this process's lease now so a later run reclaims it without waiting
/// for expiry. Committed steps stand; nothing is deleted.
pub async fn release(connection: &mut PgConnection, tenant: TenantId, lease: &Lease) -> Result<()> {
    sqlx::query(
        "update public.durable_operations
            set lease_expires_at = clock_timestamp(), updated_at = clock_timestamp()
          where tenant_id = $1 and id = $2 and kind = 'tenant_erasure' and state = 'running'
            and lease_owner = $3 and attempts = $4 and lease_expires_at > clock_timestamp()",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(&lease.worker)
    .bind(lease.attempt)
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))
}

/// How a unit continues after one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    Done,
    Again,
    Phase(u8),
}

async fn run_unit(
    connection: &mut PgConnection,
    tenant: TenantId,
    unit: Unit,
    phase: u8,
    chunk: i64,
) -> Result<(Vec<(&'static str, u64)>, Next)> {
    match unit {
        Unit::Rows(table) => {
            let rows = delete_rows(connection, tenant, table, "", chunk).await?;
            Ok((vec![(table, rows)], again_or_done(rows, chunk)))
        }
        Unit::Lifecycle(table) => {
            let predicate = lifecycle_predicate(table)?;
            let rows = delete_rows(connection, tenant, table, predicate, chunk).await?;
            Ok((vec![(table, rows)], again_or_done(rows, chunk)))
        }
        Unit::ConsoleSessions => {
            // Deleting custody cascades to the tenant's index row; the
            // custody table carries no tenant column of its own.
            let rows = execute(
                connection,
                "delete from public.console_sessions where token_hash = any(array(
                   select custody_hash from public.console_session_index
                    where tenant_id = $1 limit $2))",
                tenant,
                chunk,
            )
            .await?;
            Ok((
                vec![(CONSOLE_SESSION_INDEX, rows), ("console_sessions", rows)],
                again_or_done(rows, chunk),
            ))
        }
        Unit::Leaves(reference) => {
            let statement = format!(
                r#"delete from public."{table}" where ctid = any(array(
                     select candidate.ctid from public."{table}" as candidate
                      where candidate.tenant_id = $1
                        and not exists (select 1 from public."{table}" as child
                              where child.tenant_id = candidate.tenant_id
                                and child."{parent}" = candidate."{key}")
                      limit $2))"#,
                table = reference.table,
                parent = reference.parent,
                key = reference.key,
            );
            let rows = execute(connection, &statement, tenant, chunk).await?;
            if !exists(connection, tenant, reference.table, "").await? {
                return Ok((vec![(reference.table, rows)], Next::Done));
            }
            if rows == 0 {
                return Err(refused(
                    ERASURE_INCOMPLETE,
                    &format!("{} has rows but no leaf", reference.table),
                ));
            }
            Ok((vec![(reference.table, rows)], Next::Again))
        }
        Unit::Versioned(pair) if phase == 0 => {
            // Every version that is not its parent's current one.
            let statement = format!(
                r#"delete from public."{version}" where ctid = any(array(
                     select candidate.ctid from public."{version}" as candidate
                      where candidate.tenant_id = $1
                        and not exists (select 1 from public."{parent}" as owner
                              where owner.tenant_id = candidate.tenant_id
                                and owner.id = candidate."{owner}"
                                and owner."{current}" = candidate.id)
                      limit $2))"#,
                version = pair.version,
                parent = pair.parent,
                owner = pair.owner,
                current = pair.current,
            );
            let rows = execute(connection, &statement, tenant, chunk).await?;
            let next = if rows < u64::try_from(chunk).unwrap_or(u64::MAX) {
                Next::Phase(1)
            } else {
                Next::Again
            };
            Ok((vec![(pair.version, rows)], next))
        }
        Unit::Versioned(pair) => {
            // One chunk of parents with their current versions. The parent's
            // foreign key to its current version is initially deferred.
            let versions = format!(
                r#"delete from public."{version}" as version
                    using (select id, "{current}" as current from public."{parent}"
                            where tenant_id = $1 order by id limit $2) as chunk
                    where version.tenant_id = $1 and version."{owner}" = chunk.id
                      and version.id = chunk.current"#,
                version = pair.version,
                parent = pair.parent,
                owner = pair.owner,
                current = pair.current,
            );
            let version_rows = execute(connection, &versions, tenant, chunk).await?;
            let parents = format!(
                r#"delete from public."{parent}" where ctid = any(array(
                     select ctid from public."{parent}" where tenant_id = $1
                      order by id limit $2))"#,
                parent = pair.parent,
            );
            let parent_rows = execute(connection, &parents, tenant, chunk).await?;
            let deleted = vec![(pair.version, version_rows), (pair.parent, parent_rows)];
            if parent_rows < u64::try_from(chunk).unwrap_or(u64::MAX) {
                if exists(connection, tenant, pair.version, "").await? {
                    return Ok((deleted, Next::Phase(0)));
                }
                return Ok((deleted, Next::Done));
            }
            Ok((deleted, Next::Again))
        }
        Unit::Atomic(tables) => run_atomic(connection, tenant, tables, chunk).await,
    }
}

/// Deletes every row of a cycle whose constraints are all deferrable in one
/// transaction, then checks them before the step commits.
async fn run_atomic(
    connection: &mut PgConnection,
    tenant: TenantId,
    tables: &'static [&'static str],
    chunk: i64,
) -> Result<(Vec<(&'static str, u64)>, Next)> {
    let total = count_rows(connection, tenant, tables, ATOMIC_UNIT_MAX_ROWS).await?;
    if total > ATOMIC_UNIT_MAX_ROWS {
        return Err(refused(
            ERASURE_UNIT_TOO_LARGE,
            &format!("the inbound source tables hold more than {ATOMIC_UNIT_MAX_ROWS} rows"),
        ));
    }
    sqlx::query("set constraints all deferred")
        .execute(&mut *connection)
        .await
        .map_err(|error| storage("defer the source constraints", &error))?;
    let chunk_rows = u64::try_from(chunk).unwrap_or(1).max(1);
    let bound = total / chunk_rows + 2;
    let mut deleted = Vec::with_capacity(tables.len());
    for table in tables {
        let mut rows = 0_u64;
        let mut finished = false;
        for _ in 0..bound {
            let removed = delete_rows(connection, tenant, table, "", chunk).await?;
            rows = rows.saturating_add(removed);
            if removed < chunk_rows {
                finished = true;
                break;
            }
        }
        if !finished {
            return Err(refused(
                ERASURE_INCOMPLETE,
                &format!("{table} did not empty within its bound"),
            ));
        }
        deleted.push((*table, rows));
    }
    sqlx::query("set constraints all immediate")
        .execute(&mut *connection)
        .await
        .map_err(|error| storage("check the source constraints", &error))?;
    Ok((deleted, Next::Done))
}

fn again_or_done(rows: u64, chunk: i64) -> Next {
    if rows < u64::try_from(chunk).unwrap_or(u64::MAX) {
        Next::Done
    } else {
        Next::Again
    }
}

/// The rows of a lifecycle table that erasure deletes.
fn lifecycle_predicate(table: &str) -> Result<&'static str> {
    match table {
        "durable_operations" => Ok(" and kind <> 'tenant_erasure'"),
        "operation_attempts" => Ok(" and operation_id not in (
              select id from public.durable_operations
               where tenant_id = $1 and kind = 'tenant_erasure')"),
        other => Err(refused(
            ERASURE_PLAN_UNSUPPORTED,
            &format!("{other} has no lifecycle retention rule"),
        )),
    }
}

async fn delete_rows(
    connection: &mut PgConnection,
    tenant: TenantId,
    table: &str,
    predicate: &str,
    chunk: i64,
) -> Result<u64> {
    debug_assert!(is_plain_identifier(table));
    // Identifiers come only from the reviewed constants, never from input.
    let statement = format!(
        r#"delete from public."{table}" where ctid = any(array(
             select ctid from public."{table}" where tenant_id = $1{predicate} limit $2))"#
    );
    execute(connection, &statement, tenant, chunk).await
}

async fn execute(
    connection: &mut PgConnection,
    statement: &str,
    tenant: TenantId,
    chunk: i64,
) -> Result<u64> {
    sqlx::query(statement)
        .bind(tenant.as_uuid())
        .bind(chunk)
        .execute(&mut *connection)
        .await
        .map(|done| done.rows_affected())
        .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))
}

async fn exists(
    connection: &mut PgConnection,
    tenant: TenantId,
    table: &str,
    predicate: &str,
) -> Result<bool> {
    debug_assert!(is_plain_identifier(table));
    let statement = format!(
        r#"select exists (select 1 from public."{table}" where tenant_id = $1{predicate})"#
    );
    sqlx::query_scalar(&statement)
        .bind(tenant.as_uuid())
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| storage(&format!("check {table} for rows"), &error))
}

async fn count_rows(
    connection: &mut PgConnection,
    tenant: TenantId,
    tables: &[&str],
    cap: u64,
) -> Result<u64> {
    let limit = i64::try_from(cap.saturating_add(1)).unwrap_or(i64::MAX);
    let mut total = 0_u64;
    for table in tables {
        debug_assert!(is_plain_identifier(table));
        let statement = format!(
            r#"select count(*) from (select 1 from public."{table}" where tenant_id = $1 limit $2) as bounded"#
        );
        let rows: i64 = sqlx::query_scalar(&statement)
            .bind(tenant.as_uuid())
            .bind(limit)
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| storage(&format!("count {table}"), &error))?;
        total = total.saturating_add(u64::try_from(rows).unwrap_or(0));
    }
    Ok(total)
}

/// Names the tenant as erasing for this transaction only and bounds its
/// statements. Only the database owner benefits; ordinary roles never pass the
/// owner check in `synveda_tenant_erasure_authorized`.
async fn authorize(
    connection: &mut PgConnection,
    tenant: TenantId,
    statement_timeout: Duration,
) -> Result<()> {
    interval(statement_timeout, MAX_STATEMENT_TIMEOUT)?;
    let timeout = format!("{}ms", statement_timeout.as_millis());
    sqlx::query(
        "select set_config('synveda.tenant_erasure', $1, true), set_config('statement_timeout', $2, true)",
    )
    .bind(tenant.as_uuid().to_string())
    .bind(timeout)
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| storage("authorize the erasure step", &error))
}

/// Renews the lease this process holds and reads the cursor.
async fn read_cursor(
    connection: &mut PgConnection,
    tenant: TenantId,
    lease: &Lease,
    renewal: &str,
) -> Result<Cursor> {
    let result: Option<serde_json::Value> = sqlx::query_scalar(
        "update public.durable_operations
            set lease_expires_at = clock_timestamp() + $5::interval, updated_at = clock_timestamp()
          where tenant_id = $1 and id = $2 and kind = 'tenant_erasure' and state = 'running'
            and lease_owner = $3 and attempts = $4 and lease_expires_at > clock_timestamp()
          returning result",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(&lease.worker)
    .bind(lease.attempt)
    .bind(renewal)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))?;
    let Some(result) = result else {
        return Err(refused(
            ERASURE_LEASE_LOST,
            "this process no longer holds the erasure lease; rerun to reclaim it",
        ));
    };
    let beat = sqlx::query(
        "update public.operation_attempts set heartbeat_at = clock_timestamp()
          where tenant_id = $1 and operation_id = $2 and attempt_number = $3
            and worker_id = $4 and state = 'running'",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(lease.attempt)
    .bind(&lease.worker)
    .execute(&mut *connection)
    .await
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))?;
    if beat.rows_affected() != 1 {
        return Err(refused(
            ERASURE_LEASE_LOST,
            "the erasure attempt is no longer running",
        ));
    }
    serde_json::from_value(result).map_err(|error| Error::Internal {
        message: format!("decode the erasure cursor: {error}"),
    })
}

async fn store_cursor(
    connection: &mut PgConnection,
    tenant: TenantId,
    lease: &Lease,
    cursor: &Cursor,
    percent: i16,
) -> Result<()> {
    let encoded = serde_json::to_value(cursor).map_err(|error| Error::Internal {
        message: format!("encode the erasure cursor: {error}"),
    })?;
    sqlx::query(
        "update public.durable_operations
            set result = $3, progress_percent = greatest(progress_percent, $4),
                updated_at = clock_timestamp()
          where tenant_id = $1 and id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(lease.operation.as_uuid())
    .bind(encoded)
    .bind(percent)
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| conflict_or_storage(error, ERASURE_LEASE_LOST))
}

async fn stored_receipt(
    connection: &mut PgConnection,
    tenant: TenantId,
) -> Result<serde_json::Value> {
    let receipt: Option<serde_json::Value> = sqlx::query_scalar(
        "select result from public.durable_operations
          where tenant_id = $1 and kind = $2 and state = 'succeeded'
          order by completed_at desc limit 1",
    )
    .bind(tenant.as_uuid())
    .bind(OPERATION_KIND)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| storage("read the erasure receipt", &error))?;
    receipt.ok_or_else(|| {
        refused(
            ERASURE_INCOMPLETE,
            "the erased tenant has no completed erasure operation",
        )
    })
}

async fn insert_attempt(
    connection: &mut PgConnection,
    tenant: TenantId,
    operation: DurableOperationId,
    attempt: i32,
    worker: &str,
) -> Result<()> {
    sqlx::query(
        "insert into public.operation_attempts (tenant_id, operation_id, attempt_number, worker_id)
         values ($1, $2, $3, $4)",
    )
    .bind(tenant.as_uuid())
    .bind(operation.as_uuid())
    .bind(attempt)
    .bind(worker)
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|error| storage("record the erasure attempt", &error))
}

/// A PostgreSQL interval within `(0, max]`.
fn interval(duration: Duration, max: Duration) -> Result<String> {
    if duration.is_zero() || duration > max {
        return Err(Error::Invalid {
            message: format!(
                "an erasure bound must be between 1 ms and {} s",
                max.as_secs()
            ),
        });
    }
    Ok(format!("{} milliseconds", duration.as_millis()))
}

fn refused(reason: &str, detail: &str) -> Error {
    Error::Conflict {
        message: format!("{reason}: {detail}"),
    }
}

fn conflict_or_storage(error: sqlx::Error, reason: &str) -> Error {
    match &error {
        sqlx::Error::Database(db) if db.code().is_some_and(|code| code == "40001") => {
            refused(reason, "a concurrent change moved the tenant or its lease")
        }
        _ => storage("tenant erasure", &error),
    }
}

fn storage(context: &str, error: &sqlx::Error) -> Error {
    Error::Storage {
        message: format!("{context}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(child, parent)| ((*child).to_owned(), (*parent).to_owned()))
            .collect()
    }

    fn position(plan: &Plan, table: &str) -> usize {
        plan.units()
            .iter()
            .position(|unit| unit.tables().contains(&table))
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn every_erased_table_belongs_to_exactly_one_unit() {
        let plan = derive(&[]).expect("an empty catalogue plans");
        let mut seen: Vec<&str> = plan.units().iter().flat_map(Unit::tables).collect();
        let expected = TENANT_TABLES
            .iter()
            .filter(|table| table.erasure != Erasure::RetainEvidence)
            .count();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), expected);
        assert!(!seen.contains(&"audit_log"));
        assert!(!seen.contains(&"audit_chain_heads"));
    }

    #[test]
    fn a_referencing_table_is_erased_before_its_parent() {
        let plan = derive(&edges(&[
            ("session_events", "sessions"),
            ("knowledge_revision_headers", "knowledge_revisions"),
            ("operation_attempts", "durable_operations"),
            ("durable_operations", "skill_versions"),
        ]))
        .expect("plan");
        assert!(position(&plan, "session_events") < position(&plan, "sessions"));
        assert!(
            position(&plan, "knowledge_revision_headers") < position(&plan, "knowledge_revisions")
        );
        assert!(position(&plan, "operation_attempts") < position(&plan, "durable_operations"));
        assert!(position(&plan, "durable_operations") < position(&plan, "skills"));
    }

    #[test]
    fn declared_cycles_plan_and_undeclared_cycles_refuse() {
        derive(&edges(&[
            ("scopes", "scopes"),
            ("skills", "skill_versions"),
            ("skill_versions", "skills"),
            ("import_jobs", "capture_batches"),
            ("capture_batches", "import_jobs"),
        ]))
        .expect("declared cycles plan");
        let cycle = derive(&edges(&[
            ("sessions", "session_events"),
            ("session_events", "sessions"),
        ]))
        .expect_err("an undeclared cycle refuses");
        assert!(
            cycle.to_string().contains(ERASURE_PLAN_UNSUPPORTED),
            "{cycle}"
        );
        let itself = derive(&edges(&[("sessions", "sessions")])).expect_err("self reference");
        assert!(
            itself.to_string().contains(ERASURE_PLAN_UNSUPPORTED),
            "{itself}"
        );
        let retained = derive(&edges(&[("audit_log", "sessions")])).expect_err("retained edge");
        assert!(
            retained.to_string().contains(ERASURE_PLAN_UNSUPPORTED),
            "{retained}"
        );
    }

    #[test]
    fn the_plan_digest_is_stable_and_order_sensitive() {
        let one = derive(&edges(&[("session_events", "sessions")])).expect("plan");
        let again = derive(&edges(&[("session_events", "sessions")])).expect("plan");
        assert_eq!(one.digest(), again.digest());
        let other = derive(&edges(&[("sessions", "session_events")])).expect("plan");
        assert_ne!(one.digest(), other.digest());
    }

    #[test]
    fn bounds_refuse_zero_and_oversized_values() {
        assert!(interval(Duration::ZERO, MAX_LEASE).is_err());
        assert!(interval(MAX_LEASE + Duration::from_secs(1), MAX_LEASE).is_err());
        assert_eq!(
            interval(Duration::from_secs(30), MAX_LEASE).expect("valid"),
            "30000 milliseconds"
        );
        assert!(lifecycle_predicate("sessions").is_err());
    }
}
