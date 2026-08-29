//! Fail-closed database-authority gate for the gateway process.
//!
//! Liveness is deliberately independent from PostgreSQL, but no application
//! route may run until one bounded proof has accepted the read path, schema
//! epoch/revision, executable ACL/RLS/routine/trigger surface, exact runtime
//! role and writable database target. The same proof repeats while the process
//! is live: outages close the gate and retry; a conclusive schema, authority or
//! target refusal makes the gate terminal.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{Acquire as _, PgPool};
use tokio::sync::watch;

use crate::runtime_config::{PoolRefusal, PoolRefusalStage};
use crate::telemetry::{
    GATEWAY_AUTHORITY_CHECKS_TOTAL, GATEWAY_AUTHORITY_READY, WORKER_AUTHORITY_CHECKS_TOTAL,
    WORKER_AUTHORITY_READY,
};

/// Cadence of the process-owned authority sentinel.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(15);
/// Whole-proof deadline, including serialization behind an in-flight probe.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// One of the bounded stages in an authority proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The gateway→retrieval→store dependency leg.
    Ping,
    /// Immutable schema epoch and baseline revision.
    Epoch,
    /// Effective session settings on the acquired physical connection.
    Session,
    /// Exact PostgreSQL runtime principal and inherited authority.
    Role,
    /// Writable database target and pinned cluster/database identity.
    Identity,
    /// The whole serialized proof exceeded its deadline.
    Probe,
    /// A previous conclusive refusal already made the gate terminal.
    Terminal,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Epoch => "epoch",
            Self::Session => "session",
            Self::Role => "role",
            Self::Identity => "identity",
            Self::Probe => "probe",
            Self::Terminal => "terminal",
        }
    }
}

/// Result of one complete, bounded authority check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// All four stages accepted and the application gate is open.
    Accepted,
    /// The proof could not reach a verdict; the gate is closed and retryable.
    Unavailable {
        /// Stage which could not complete.
        stage: Stage,
    },
    /// The whole proof timed out; the gate is closed and retryable.
    Timeout,
    /// A conclusive refusal made the gate terminal.
    Refused {
        /// Stage which refused the runtime state.
        stage: Stage,
        /// Closed operator action for an incompatible schema, when one is safe.
        guidance: Option<OperatorGuidance>,
    },
}

/// Content-free operator action attached to a schema refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorGuidance {
    /// The database predates this pre-1.0 hard cut and must be reset explicitly.
    Reset,
    /// The database is newer than the binary and the binary must be upgraded.
    Upgrade,
}

const OUTCOME_UNKNOWN: u8 = 0;
const OUTCOME_ACCEPTED: u8 = 1;
const OUTCOME_UNAVAILABLE: u8 = 2;
const OUTCOME_TIMEOUT: u8 = 3;
const OUTCOME_REFUSED: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
struct StableTarget {
    database: String,
    cluster_system_identifier: String,
    database_oid: i64,
}

impl From<&synveda_store::runtime_role::DatabaseIdentity> for StableTarget {
    fn from(identity: &synveda_store::runtime_role::DatabaseIdentity) -> Self {
        Self {
            database: identity.database.clone(),
            cluster_system_identifier: identity.cluster_system_identifier.clone(),
            database_oid: identity.database_oid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateSnapshot {
    Closed(u64),
    Open(u64),
    Terminal(u64),
}

/// Cloneable application-plane gate shared by the HTTP router and sentinel.
#[derive(Clone)]
pub struct AuthorityGate {
    state: watch::Receiver<GateSnapshot>,
    #[cfg(test)]
    _test_keepalive: Option<Arc<watch::Sender<GateSnapshot>>>,
}

impl AuthorityGate {
    fn closed_pair() -> (Self, GateWriter) {
        let (state, receiver) = watch::channel(GateSnapshot::Closed(0));
        (
            Self {
                state: receiver,
                #[cfg(test)]
                _test_keepalive: None,
            },
            GateWriter {
                state,
                target: None,
                generation: 0,
                last_outcome: OUTCOME_UNKNOWN,
            },
        )
    }

    fn snapshot(&self) -> Option<GateSnapshot> {
        self.state.has_changed().ok()?;
        let snapshot = *self.state.borrow();
        self.state.has_changed().ok()?;
        Some(snapshot)
    }

    #[cfg(test)]
    pub(crate) fn open_for_test() -> Self {
        let (sender, state) = watch::channel(GateSnapshot::Open(1));
        Self {
            state,
            _test_keepalive: Some(Arc::new(sender)),
        }
    }

    /// Current open authority generation, when application work may run.
    pub fn open_generation(&self) -> Option<u64> {
        match self.snapshot() {
            Some(GateSnapshot::Open(generation)) => Some(generation),
            Some(GateSnapshot::Closed(_) | GateSnapshot::Terminal(_)) | None => None,
        }
    }

    /// Whether application routes may currently run.
    pub fn is_open(&self) -> bool {
        self.open_generation().is_some()
    }

    /// Whether a conclusive proof refusal made this gate permanently closed.
    pub fn is_terminal(&self) -> bool {
        // Terminal is absorbing: the writer refuses every later acceptance and
        // never replaces it during shutdown. Keep that final verdict visible
        // after the sentinel drops its sender, while open state still requires
        // the live-writer checks in `snapshot`.
        matches!(*self.state.borrow(), GateSnapshot::Terminal(_))
    }

    /// Subscribes before admission so a concurrent close cannot be missed.
    pub fn permit(&self) -> AuthorityPermit {
        let generation = self.open_generation();
        AuthorityPermit {
            state: self.state.clone(),
            generation,
        }
    }

    /// Waits until the first complete proof opens the gate, or until a
    /// terminal refusal makes opening impossible.
    pub async fn wait_until_open(&self) -> Result<u64, ()> {
        let mut state = self.state.clone();
        loop {
            if state.has_changed().is_err() {
                return Err(());
            }
            let snapshot = *state.borrow_and_update();
            if state.has_changed().is_err() {
                return Err(());
            }
            match snapshot {
                GateSnapshot::Open(generation) => return Ok(generation),
                GateSnapshot::Terminal(_) => return Err(()),
                GateSnapshot::Closed(_) => {}
            }
            if state.changed().await.is_err() {
                return Err(());
            }
        }
    }

    /// Waits until a conclusive refusal makes the gate terminal.
    pub async fn wait_until_terminal(&self) {
        let mut state = self.state.clone();
        loop {
            if matches!(*state.borrow_and_update(), GateSnapshot::Terminal(_)) {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }

    /// Waits until the sentinel has withdrawn an open generation.
    pub async fn wait_until_closed(&self) {
        let mut state = self.state.clone();
        loop {
            if !matches!(*state.borrow_and_update(), GateSnapshot::Open(_)) {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }
}

struct GateWriter {
    state: watch::Sender<GateSnapshot>,
    target: Option<StableTarget>,
    generation: u64,
    last_outcome: u8,
}

impl GateWriter {
    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    fn close(&mut self) {
        if matches!(*self.state.borrow(), GateSnapshot::Open(_)) {
            let generation = self.next_generation();
            self.state.send_replace(GateSnapshot::Closed(generation));
        }
    }

    fn refuse(&mut self) {
        if !matches!(*self.state.borrow(), GateSnapshot::Terminal(_)) {
            let generation = self.next_generation();
            self.state.send_replace(GateSnapshot::Terminal(generation));
        }
    }

    fn accept_target(
        &mut self,
        identity: &synveda_store::runtime_role::DatabaseIdentity,
    ) -> Result<(), ()> {
        if matches!(*self.state.borrow(), GateSnapshot::Terminal(_)) {
            return Err(());
        }
        let candidate = StableTarget::from(identity);
        match self.target.as_ref() {
            Some(existing) if existing != &candidate => return Err(()),
            Some(_) => {}
            None => self.target = Some(candidate),
        }
        if !matches!(*self.state.borrow(), GateSnapshot::Open(_)) {
            let generation = self.next_generation();
            self.state.send_replace(GateSnapshot::Open(generation));
        }
        Ok(())
    }

    fn observe(&mut self, outcome: u8) -> bool {
        let changed = self.last_outcome != outcome;
        self.last_outcome = outcome;
        changed
    }
}

/// One request-generation permit. Dropping the handler future after this
/// permit is revoked cancels in-flight database/external work at await seams.
pub struct AuthorityPermit {
    state: watch::Receiver<GateSnapshot>,
    generation: Option<u64>,
}

impl AuthorityPermit {
    /// Whether the exact authority generation captured at admission remains open.
    pub fn is_open(&self) -> bool {
        self.state.has_changed().is_ok()
            && self
                .generation
                .is_some_and(|generation| *self.state.borrow() == GateSnapshot::Open(generation))
            && self.state.has_changed().is_ok()
    }

    /// Whether this permit belongs to and still admits the expected generation.
    pub fn is_for(&self, generation: u64) -> bool {
        self.generation == Some(generation) && self.is_open()
    }

    /// Waits until the captured generation is no longer authoritative.
    pub async fn revoked(&mut self) {
        loop {
            if !self.is_open() {
                return;
            }
            if self.state.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Process-owned authority checker. The sentinel is its only state writer;
/// HTTP and worker planes receive only the cloneable read gate.
pub struct AuthorityMonitor {
    pool: PgPool,
    pool_refusal: PoolRefusal,
    expected_database_role: Arc<str>,
    database_roles: Arc<synveda_store::runtime_role::DatabaseRoles>,
    gate: AuthorityGate,
    writer: GateWriter,
    ready_metric: &'static str,
    checks_metric: &'static str,
    process: &'static str,
}

impl AuthorityMonitor {
    /// Creates a monitor whose application gate starts closed.
    pub fn new(
        pool: PgPool,
        pool_refusal: PoolRefusal,
        expected_database_role: String,
        database_roles: synveda_store::runtime_role::DatabaseRoles,
    ) -> Self {
        Self::new_for_process(
            pool,
            pool_refusal,
            expected_database_role,
            database_roles,
            GATEWAY_AUTHORITY_READY,
            GATEWAY_AUTHORITY_CHECKS_TOTAL,
            "synveda-gateway",
        )
    }

    /// Creates a monitor for the core-worker application plane.
    pub fn new_worker(
        pool: PgPool,
        pool_refusal: PoolRefusal,
        expected_database_role: String,
        database_roles: synveda_store::runtime_role::DatabaseRoles,
    ) -> Self {
        Self::new_for_process(
            pool,
            pool_refusal,
            expected_database_role,
            database_roles,
            WORKER_AUTHORITY_READY,
            WORKER_AUTHORITY_CHECKS_TOTAL,
            "synveda-worker",
        )
    }

    fn new_for_process(
        pool: PgPool,
        pool_refusal: PoolRefusal,
        expected_database_role: String,
        database_roles: synveda_store::runtime_role::DatabaseRoles,
        ready_metric: &'static str,
        checks_metric: &'static str,
        process: &'static str,
    ) -> Self {
        metrics::gauge!(ready_metric).set(0.0);
        let (gate, writer) = AuthorityGate::closed_pair();
        Self {
            pool,
            pool_refusal,
            expected_database_role: expected_database_role.into(),
            database_roles: Arc::new(database_roles),
            gate,
            writer,
            ready_metric,
            checks_metric,
            process,
        }
    }

    /// Returns the shared route gate.
    pub fn gate(&self) -> AuthorityGate {
        self.gate.clone()
    }

    fn close(&mut self) {
        self.writer.close();
        metrics::gauge!(self.ready_metric).set(0.0);
    }

    fn refuse(&mut self) {
        self.writer.refuse();
        metrics::gauge!(self.ready_metric).set(0.0);
    }

    fn count(&self, outcome: &'static str) {
        metrics::counter!(self.checks_metric, "outcome" => outcome).increment(1);
    }

    /// Runs and records one complete authority proof under the global bound.
    #[tracing::instrument(name = "database.authority.check", skip_all)]
    async fn check(&mut self) -> CheckOutcome {
        if self.gate.is_terminal() {
            return CheckOutcome::Refused {
                stage: Stage::Terminal,
                guidance: None,
            };
        }
        let checked = tokio::time::timeout(CHECK_TIMEOUT, self.probe()).await;
        match checked {
            Err(_) => {
                self.close();
                self.count("timeout");
                if self.writer.observe(OUTCOME_TIMEOUT) {
                    tracing::warn!(
                        process = self.process,
                        stage = Stage::Probe.label(),
                        "database authority unavailable"
                    );
                }
                CheckOutcome::Timeout
            }
            Ok(ProbeOutcome::Unavailable(stage)) => {
                self.close();
                self.count("unavailable");
                if self.writer.observe(OUTCOME_UNAVAILABLE) {
                    tracing::warn!(
                        process = self.process,
                        stage = stage.label(),
                        "database authority unavailable"
                    );
                }
                CheckOutcome::Unavailable { stage }
            }
            Ok(ProbeOutcome::Refused { stage, guidance }) => {
                self.refuse();
                self.count("refused");
                if self.writer.observe(OUTCOME_REFUSED) {
                    tracing::error!(
                        process = self.process,
                        stage = stage.label(),
                        "database authority refused runtime state"
                    );
                    match guidance {
                        Some(OperatorGuidance::Reset) => eprintln!(
                            "\n{}: incompatible pre-1.0 schema; run `{}` only if destructive reset is intended\n",
                            self.process,
                            synveda_store::epoch::RESET_COMMAND
                        ),
                        Some(OperatorGuidance::Upgrade) => eprintln!(
                            "\n{}: database baseline is newer than this binary; upgrade the installation\n",
                            self.process
                        ),
                        None => {}
                    }
                }
                CheckOutcome::Refused { stage, guidance }
            }
            Ok(ProbeOutcome::Accepted { metadata, identity }) => {
                if self.writer.accept_target(&identity).is_err() {
                    self.refuse();
                    self.count("refused");
                    if self.writer.observe(OUTCOME_REFUSED) {
                        tracing::error!(
                            process = self.process,
                            stage = Stage::Identity.label(),
                            "database authority refused runtime state"
                        );
                    }
                    return CheckOutcome::Refused {
                        stage: Stage::Identity,
                        guidance: None,
                    };
                }
                metrics::gauge!(self.ready_metric).set(1.0);
                self.count("accepted");
                if self.writer.observe(OUTCOME_ACCEPTED) {
                    tracing::info!(
                        process = self.process,
                        schema.epoch = metadata.epoch,
                        schema.baseline_revision = metadata.baseline_revision,
                        "database authority accepted"
                    );
                }
                CheckOutcome::Accepted
            }
        }
    }

    async fn probe(&self) -> ProbeOutcome {
        if let Some(stage) = self.pool_refusal.current() {
            return pool_refusal_outcome(stage);
        }
        let mut refusal = self.pool_refusal.clone();
        let acquired = tokio::select! {
            biased;
            stage = refusal.wait_until_refused() => {
                return pool_refusal_outcome(stage);
            }
            acquired = self.pool.acquire() => acquired,
        };
        let mut connection = match acquired {
            Ok(connection) => connection,
            Err(_) => return ProbeOutcome::Unavailable(Stage::Ping),
        };
        if let Some(stage) = self.pool_refusal.current() {
            return pool_refusal_outcome(stage);
        }
        let mut transaction = match connection.begin().await {
            Ok(transaction) => transaction,
            Err(_) => return ProbeOutcome::Unavailable(Stage::Session),
        };
        if synveda_store::runtime_role::configure_authority_snapshot_connection(&mut transaction)
            .await
            .is_err()
        {
            return ProbeOutcome::Unavailable(Stage::Session);
        }
        let outcome = async {
            match synveda_store::runtime_role::verify_session_safety_connection(&mut transaction)
                .await
            {
                Ok(()) => {}
                Err(synveda_types::Error::Storage { .. }) => {
                    return ProbeOutcome::Unavailable(Stage::Session);
                }
                Err(_) => {
                    return ProbeOutcome::Refused {
                        stage: Stage::Session,
                        guidance: None,
                    };
                }
            }
            if synveda_retrieval::readiness_connection(&mut transaction)
                .await
                .is_err()
            {
                return ProbeOutcome::Unavailable(Stage::Ping);
            }
            let metadata = match synveda_store::epoch::verify_connection(&mut transaction).await {
                Ok(metadata) => metadata,
                Err(error) if error.is_refusal() => {
                    return ProbeOutcome::Refused {
                        stage: Stage::Epoch,
                        guidance: schema_guidance(&error),
                    };
                }
                Err(_) => return ProbeOutcome::Unavailable(Stage::Epoch),
            };
            match synveda_store::runtime_role::verify_connection(
                &mut transaction,
                &self.expected_database_role,
                &self.database_roles,
            )
            .await
            {
                Ok(_) => {}
                Err(synveda_types::Error::Storage { .. }) => {
                    return ProbeOutcome::Unavailable(Stage::Role);
                }
                Err(_) => {
                    return ProbeOutcome::Refused {
                        stage: Stage::Role,
                        guidance: None,
                    };
                }
            }
            match synveda_store::runtime_role::database_identity_connection(&mut transaction).await
            {
                Ok(identity) => ProbeOutcome::Accepted { metadata, identity },
                Err(synveda_types::Error::Storage { .. }) => {
                    ProbeOutcome::Unavailable(Stage::Identity)
                }
                Err(_) => ProbeOutcome::Refused {
                    stage: Stage::Identity,
                    guidance: None,
                },
            }
        }
        .await;
        let outcome = outcome_after_rollback(outcome, transaction.rollback().await.is_ok());
        if let Some(stage) = self.pool_refusal.current() {
            return pool_refusal_outcome(stage);
        }
        outcome
    }
}

fn pool_refusal_outcome(stage: PoolRefusalStage) -> ProbeOutcome {
    let stage = match stage {
        PoolRefusalStage::Session => Stage::Session,
        PoolRefusalStage::Role => Stage::Role,
        PoolRefusalStage::Identity => Stage::Identity,
    };
    ProbeOutcome::Refused {
        stage,
        guidance: None,
    }
}

enum ProbeOutcome {
    Accepted {
        metadata: synveda_store::epoch::SchemaMetadata,
        identity: synveda_store::runtime_role::DatabaseIdentity,
    },
    Unavailable(Stage),
    Refused {
        stage: Stage,
        guidance: Option<OperatorGuidance>,
    },
}

fn outcome_after_rollback(outcome: ProbeOutcome, rollback_succeeded: bool) -> ProbeOutcome {
    if rollback_succeeded || matches!(&outcome, ProbeOutcome::Refused { .. }) {
        outcome
    } else {
        ProbeOutcome::Unavailable(Stage::Session)
    }
}

fn schema_guidance(error: &synveda_store::epoch::SchemaEpochError) -> Option<OperatorGuidance> {
    use synveda_store::epoch::SchemaEpochError;
    match error {
        SchemaEpochError::Newer { .. } | SchemaEpochError::NewerRevision { .. } => {
            Some(OperatorGuidance::Upgrade)
        }
        SchemaEpochError::Missing
        | SchemaEpochError::Malformed(_)
        | SchemaEpochError::Older { .. }
        | SchemaEpochError::OlderRevision { .. } => Some(OperatorGuidance::Reset),
        SchemaEpochError::Unreachable(_) | SchemaEpochError::Unreadable => None,
    }
}

/// Runs one immediate check and then repeats at the fixed cadence. Retryable
/// outages keep the task alive; a conclusive refusal returns after the gate is
/// already terminal.
pub async fn run_sentinel(
    mut monitor: AuthorityMonitor,
    mut shutdown: watch::Receiver<bool>,
) -> CheckOutcome {
    let mut ticker = tokio::time::interval(CHECK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pool_refusal = monitor.pool_refusal.clone();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    monitor.close();
                    return CheckOutcome::Unavailable { stage: Stage::Probe };
                }
            }
            _ = pool_refusal.wait_until_refused() => {
                let outcome = monitor.check().await;
                if matches!(outcome, CheckOutcome::Refused { .. }) {
                    return outcome;
                }
            }
            _ = ticker.tick() => {
                enum Step {
                    Checked(CheckOutcome),
                    Shutdown,
                    Continue,
                }
                let step = {
                    let check = monitor.check();
                    tokio::pin!(check);
                    tokio::select! {
                        biased;
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                Step::Shutdown
                            } else {
                                Step::Continue
                            }
                        }
                        outcome = &mut check => Step::Checked(outcome),
                    }
                };
                match step {
                    Step::Shutdown => {
                        monitor.close();
                        return CheckOutcome::Unavailable { stage: Stage::Probe };
                    }
                    Step::Continue => continue,
                    Step::Checked(outcome)
                        if matches!(outcome, CheckOutcome::Refused { .. }) =>
                    {
                        return outcome;
                    }
                    Step::Checked(_) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_failure_cannot_downgrade_a_conclusive_refusal() {
        let refusal = outcome_after_rollback(
            ProbeOutcome::Refused {
                stage: Stage::Identity,
                guidance: None,
            },
            false,
        );
        assert!(matches!(
            refusal,
            ProbeOutcome::Refused {
                stage: Stage::Identity,
                guidance: None,
            }
        ));
        assert!(matches!(
            outcome_after_rollback(ProbeOutcome::Unavailable(Stage::Role), false),
            ProbeOutcome::Unavailable(Stage::Session)
        ));
    }

    #[tokio::test]
    async fn terminal_state_is_visible_and_cannot_reopen() {
        let (gate, mut writer) = AuthorityGate::closed_pair();
        writer.refuse();
        assert!(!gate.is_open(), "refusal closes the route gate first");

        let identity = synveda_store::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: "1".to_owned(),
            database_oid: 16_384,
            postmaster_started_at: chrono::Utc::now(),
        };
        assert!(
            writer.accept_target(&identity).is_err(),
            "a later success cannot overwrite terminal"
        );
        drop(writer);
        assert!(
            gate.is_terminal(),
            "the published terminal verdict survives sentinel exit"
        );
        assert!(!gate.is_open());
    }

    #[tokio::test]
    async fn target_pin_ignores_restart_time_but_refuses_another_database() {
        let (_gate, mut writer) = AuthorityGate::closed_pair();
        let first = synveda_store::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: "1".to_owned(),
            database_oid: 16_384,
            postmaster_started_at: chrono::Utc::now(),
        };
        writer.accept_target(&first).expect("first target");
        let mut restarted = first.clone();
        restarted.postmaster_started_at += chrono::Duration::seconds(1);
        writer
            .accept_target(&restarted)
            .expect("same target after restart");

        let mut other = restarted;
        other.database_oid += 1;
        assert!(writer.accept_target(&other).is_err());
    }

    #[tokio::test]
    async fn a_permit_cannot_cross_a_close_and_reopen_generation() {
        let (gate, mut writer) = AuthorityGate::closed_pair();
        let identity = synveda_store::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: "1".to_owned(),
            database_oid: 16_384,
            postmaster_started_at: chrono::Utc::now(),
        };
        writer.accept_target(&identity).expect("first generation");
        let mut permit = gate.permit();
        assert!(permit.is_open());

        writer.close();
        writer
            .accept_target(&identity)
            .expect("same target may reopen under a new generation");

        tokio::time::timeout(Duration::from_millis(50), permit.revoked())
            .await
            .expect("old generation is revoked even after rapid reopen");
        assert!(!permit.is_open());
        assert!(gate.is_open());
    }

    #[tokio::test]
    async fn dropping_the_only_writer_closes_readiness_and_revokes_permits() {
        let (gate, mut writer) = AuthorityGate::closed_pair();
        let identity = synveda_store::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: "1".to_owned(),
            database_oid: 16_384,
            postmaster_started_at: chrono::Utc::now(),
        };
        writer
            .accept_target(&identity)
            .expect("open authority gate");
        let mut permit = gate.permit();
        assert!(gate.is_open());
        assert!(permit.is_open());

        drop(writer);

        assert!(!gate.is_open(), "writer loss closes synchronous readiness");
        assert_eq!(gate.open_generation(), None);
        assert!(
            !gate.is_terminal(),
            "writer loss is unavailable, not refusal"
        );
        assert!(gate.wait_until_open().await.is_err());
        tokio::time::timeout(Duration::from_millis(50), permit.revoked())
            .await
            .expect("writer loss revokes an admitted generation");
        assert!(!permit.is_open());
    }

    #[tokio::test]
    async fn pool_refusal_interrupts_a_blocked_acquire() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind silent PostgreSQL fixture");
        let address = listener.local_addr().expect("read fixture address");
        let silent_server = tokio::spawn(async move {
            let (_connection, _) = listener.accept().await.expect("accept pool connection");
            std::future::pending::<()>().await;
        });
        let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(
            r#"{"migrator":"migrator","gateway":"nobody","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#,
        )
        .expect("parse closed role contract");
        let (options, refusal) = crate::runtime_config::runtime_pool_options(
            1,
            Duration::from_secs(30),
            "nobody".to_owned(),
        );
        let pool = options
            .connect_lazy(&format!("postgres://nobody:opaque@{address}/void"))
            .expect("parse unreachable database URL");
        let trigger = refusal.clone();
        let mut monitor = AuthorityMonitor::new(pool, refusal, "nobody".to_owned(), roles);
        let trigger_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            trigger.refuse_for_test(PoolRefusalStage::Session);
        });

        let outcome = tokio::time::timeout(Duration::from_secs(1), monitor.check())
            .await
            .expect("pool refusal wakes a blocked authority acquire");
        assert_eq!(
            outcome,
            CheckOutcome::Refused {
                stage: Stage::Session,
                guidance: None,
            }
        );
        assert!(monitor.gate().is_terminal());
        trigger_task.await.expect("join refusal trigger");
        silent_server.abort();
        let _ = silent_server.await;
    }
}
