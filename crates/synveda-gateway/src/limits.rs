//! Request admission for one gateway process.
//!
//! Before a credential is verified, login-plane requests and credential
//! failures are charged to their network source. After verification, every
//! catalogued operation is charged to the verified principal and tenant for
//! its cost class and then occupies bounded concurrency at the principal,
//! tenant and process levels before any tenant database work runs. The
//! permits live as long as the admitted request future, so success, error and
//! cancellation all release them.
//!
//! All of this state is in memory: it resets when the process restarts and is
//! not shared with any other gateway. Durable tenant usage is accounted in the
//! store, not here.

mod bucket;
mod class;
mod config;
mod source;
mod table;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use synveda_types::{Error, TenantId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::time::Instant;

use self::bucket::{Cell, Rate};
use self::source::{Source, SourceKey};
use self::table::Table;
use crate::error::ApiError;
use crate::telemetry::{
    ADMISSION_CAPACITY, ADMISSION_DECISIONS_TOTAL, ADMISSION_ENFORCED, ADMISSION_IDENTITIES,
    ADMISSION_IN_FLIGHT, ADMISSION_WAIT_SECONDS,
};

pub use self::class::OperationClass;
pub use self::config::{
    ClassLimits, IdentityCapacity, Limits, MAX_LIMITS_DOCUMENT_BYTES, Mode, RateLimit,
    SourceLimits, UsageCeilings,
};
pub use self::source::TrustedProxies;

/// The one refusal every limit returns, so a caller cannot tell which budget,
/// level or neighbour was responsible.
const REFUSAL_MESSAGE: &str = "request limit reached; retry after the indicated delay";
const MAX_RETRY_AFTER_SECS: u64 = 60;
const SWEEP_INTERVAL_NANOS: u64 = 1_000_000_000;
const LOGIN_MAX_WAIT: Duration = Duration::from_secs(2);
const LOGIN_MAX_WAITING: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PrincipalKey([u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TenantKey([u8; 16]);

impl PrincipalKey {
    /// Subjects are only unique within a tenant, and a digest keeps every key
    /// the same size whatever the issuer's subject length.
    fn new(tenant: TenantId, subject: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(tenant.as_uuid().as_bytes());
        hasher.update(subject.as_bytes());
        let mut key = [0; 16];
        key.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        Self(key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Source,
    Principal,
    Tenant,
    Process,
}

impl Level {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Principal => "principal",
            Self::Tenant => "tenant",
            Self::Process => "process",
        }
    }
}

/// Why a request was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Rate {
        level: Level,
        wait_nanos: u64,
    },
    WaitExpired {
        level: Level,
    },
    QueueFull {
        level: Level,
    },
    /// No class for the route, or a closed semaphore: refuse as unavailable.
    Unavailable,
}

impl Refusal {
    const fn outcome(self) -> &'static str {
        match self {
            Self::Rate { .. } => "rate_limited",
            Self::WaitExpired { .. } => "wait_expired",
            Self::QueueFull { .. } => "queue_full",
            Self::Unavailable => "unavailable",
        }
    }

    const fn level(self) -> Level {
        match self {
            Self::Rate { level, .. } | Self::WaitExpired { level } | Self::QueueFull { level } => {
                level
            }
            Self::Unavailable => Level::Process,
        }
    }

    fn retry_after_secs(self) -> u64 {
        match self {
            Self::Rate { wait_nanos, .. } => wait_nanos
                .div_ceil(1_000_000_000)
                .clamp(1, MAX_RETRY_AFTER_SECS),
            _ => 1,
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        if self == Self::Unavailable {
            return (StatusCode::SERVICE_UNAVAILABLE, "service unavailable").into_response();
        }
        let mut response = ApiError(Error::RateLimited {
            message: REFUSAL_MESSAGE.to_owned(),
        })
        .into_response();
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from(self.retry_after_secs()),
        );
        response
    }
}

/// What admission knows about a matched request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Classified {
    /// A catalogued operation of this class.
    Operation(OperationClass),
    /// A catalogued path without this method: the router answers 405 and no
    /// handler runs, so there is no work to admit.
    NoHandler,
    /// No catalogued path matched; refused as unavailable.
    Unknown,
}

/// The durable budget rules handlers apply when they charge accepted work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsagePolicy {
    pub(crate) ceilings: UsageCeilings,
    pub(crate) enforce: bool,
}

/// Concurrency permits for one admitted request, released when dropped.
#[derive(Debug)]
pub(crate) struct Hold {
    _permits: [Option<OwnedSemaphorePermit>; 3],
}

struct Pool {
    slots: Arc<Semaphore>,
    waiting: Arc<Semaphore>,
}

impl Pool {
    fn new(slots: usize, waiting: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            waiting: Arc::new(Semaphore::new(waiting)),
        }
    }
}

struct Identities {
    principals: Table<PrincipalKey, 6>,
    tenants: Table<TenantKey, 6>,
}

struct Sources {
    known: Table<SourceKey, 1>,
    unattributed: Cell,
}

struct ClassRates {
    principal: Rate,
    tenant: Rate,
}

struct Inner {
    limits: Limits,
    rates: [ClassRates; 6],
    per_source: Rate,
    unattributed: Rate,
    epoch: Instant,
    identities: Mutex<Identities>,
    sources: Mutex<Sources>,
    process: [Pool; 6],
    login: Pool,
    routes: HashMap<&'static str, [Option<OperationClass>; 5]>,
}

/// The admission state shared by every request this process serves.
#[derive(Clone)]
pub struct Limiter(Arc<Inner>);

fn rate(limit: RateLimit) -> Result<Rate, String> {
    Rate::new(limit.per_minute, limit.burst).ok_or_else(|| "a rate must be positive".to_owned())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Critical sections are arithmetic and map updates that cannot panic
    // part-way, so a poisoned lock still guards consistent state.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Limiter {
    /// Builds the process limiter from validated limits.
    ///
    /// # Errors
    ///
    /// The first limit that fails validation.
    pub fn new(limits: Limits) -> Result<Self, String> {
        limits.validate()?;
        let mut rates = Vec::with_capacity(OperationClass::ALL.len());
        let mut process = Vec::with_capacity(OperationClass::ALL.len());
        for class in OperationClass::ALL {
            let limit = limits.class(class);
            rates.push(ClassRates {
                principal: rate(limit.principal_rate)?,
                tenant: rate(limit.tenant_rate)?,
            });
            process.push(Pool::new(
                usize::from(limit.process_in_flight),
                usize::from(limit.max_waiting),
            ));
            metrics::gauge!(ADMISSION_CAPACITY, "class" => class.as_str())
                .set(f64::from(limit.process_in_flight));
        }
        metrics::gauge!(ADMISSION_ENFORCED).set(if limits.mode == Mode::Enforce {
            1.0
        } else {
            0.0
        });
        let capacity = limits.capacity;
        let inner = Inner {
            rates: rates
                .try_into()
                .map_err(|_| "every class needs a rate".to_owned())?,
            process: process
                .try_into()
                .map_err(|_| "every class needs a pool".to_owned())?,
            per_source: rate(limits.sources.per_source)?,
            unattributed: rate(limits.sources.unattributed)?,
            epoch: Instant::now(),
            identities: Mutex::new(Identities {
                principals: Table::new(capacity.principals as usize, SWEEP_INTERVAL_NANOS),
                tenants: Table::new(capacity.tenants as usize, SWEEP_INTERVAL_NANOS),
            }),
            sources: Mutex::new(Sources {
                known: Table::new(capacity.sources as usize, SWEEP_INTERVAL_NANOS),
                unattributed: Cell::default(),
            }),
            login: Pool::new(
                usize::from(limits.sources.login_in_flight),
                LOGIN_MAX_WAITING,
            ),
            routes: route_classes()?,
            limits,
        };
        Ok(Self(Arc::new(inner)))
    }

    /// The default limits, counted but never enforced.
    ///
    /// # Errors
    ///
    /// Only if the built-in defaults stop validating.
    pub fn observing() -> Result<Self, String> {
        Self::new(Limits::observe())
    }

    /// The limits this process enforces or observes.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.0.limits
    }

    pub(crate) fn usage_policy(&self) -> UsagePolicy {
        UsagePolicy {
            ceilings: self.0.limits.usage,
            enforce: self.enforcing(),
        }
    }

    /// Remembered principals, tenants and sources, for capacity evidence.
    #[must_use]
    pub fn remembered(&self) -> (usize, usize, usize) {
        let identities = lock(&self.0.identities);
        let sources = lock(&self.0.sources);
        (
            identities.principals.len(),
            identities.tenants.len(),
            sources.known.len(),
        )
    }

    fn now(&self) -> u64 {
        u64::try_from(self.0.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn enforcing(&self) -> bool {
        matches!(self.0.limits.mode, Mode::Enforce)
    }

    /// The class the route catalogue declared for this request. A matched
    /// path differs from its contract path only by Axum's wildcard marker,
    /// and `HEAD` is served by the `GET` handler.
    pub(crate) fn classify(&self, request: &Request) -> Classified {
        let Some(matched) = request.extensions().get::<MatchedPath>() else {
            return Classified::Unknown;
        };
        let matched = matched.as_str();
        let path = if matched.contains("{*") {
            std::borrow::Cow::Owned(matched.replace("{*", "{"))
        } else {
            std::borrow::Cow::Borrowed(matched)
        };
        let Some(methods) = self.0.routes.get(path.as_ref()) else {
            return Classified::Unknown;
        };
        match method_index(request.method().as_str()).and_then(|index| methods[index]) {
            Some(class) => Classified::Operation(class),
            None => Classified::NoHandler,
        }
    }

    /// Charges one request to its verified principal and tenant, then waits a
    /// bounded time for concurrency at each level.
    pub(crate) async fn admit(
        &self,
        class: OperationClass,
        tenant: TenantId,
        subject: &str,
    ) -> Result<Hold, Refusal> {
        let started = Instant::now();
        let result = match self.charge(class, tenant, subject) {
            Ok(keyed) => self.occupy(class, keyed, started).await,
            Err(refusal) => Err(refusal),
        };
        let pool = &self.0.process[class.index()];
        let limit = self.0.limits.class(class).process_in_flight;
        metrics::gauge!(ADMISSION_IN_FLIGHT, "class" => class.as_str()).set(f64::from(
            u32::from(limit).saturating_sub(pool.slots.available_permits() as u32),
        ));
        let outcome = match &result {
            Ok(_) => "admitted",
            Err(refusal) => refusal.outcome(),
        };
        let level = result.as_ref().err().map_or(Level::Process, |r| r.level());
        decision(level, class.as_str(), outcome);
        result
    }

    /// Rate check and charge for both identities under one lock, so neither
    /// is charged for a request the other refuses.
    fn charge(
        &self,
        class: OperationClass,
        tenant: TenantId,
        subject: &str,
    ) -> Result<[Option<Arc<Semaphore>>; 2], Refusal> {
        let index = class.index();
        let limits = self.0.limits.class(class);
        let rates = &self.0.rates[index];
        let now = self.now();
        let mut identities = lock(&self.0.identities);
        let Identities {
            principals,
            tenants,
        } = &mut *identities;
        let mut principal = principals.entry(PrincipalKey::new(tenant, subject), now);
        let mut tenant = tenants.entry(TenantKey(*tenant.as_uuid().as_bytes()), now);
        let principal_wait = principal
            .as_ref()
            .and_then(|entry| entry.cells[index].check(rates.principal, now).err());
        let tenant_wait = tenant
            .as_ref()
            .and_then(|entry| entry.cells[index].check(rates.tenant, now).err());
        let refusal = match (principal_wait, tenant_wait) {
            (None, None) => None,
            (principal_wait, tenant_wait) => Some(Refusal::Rate {
                level: if principal_wait.is_some() {
                    Level::Principal
                } else {
                    Level::Tenant
                },
                wait_nanos: principal_wait.max(tenant_wait).unwrap_or(0),
            }),
        };
        match refusal {
            Some(refusal) if self.enforcing() => return Err(refusal),
            Some(refusal) => decision(refusal.level(), class.as_str(), "observed"),
            None => {
                if let Some(entry) = principal.as_mut() {
                    entry.cells[index].charge(rates.principal, now);
                }
                if let Some(entry) = tenant.as_mut() {
                    entry.cells[index].charge(rates.tenant, now);
                }
            }
        }
        if principal.is_none() {
            decision(Level::Principal, class.as_str(), "overflow");
        }
        if tenant.is_none() {
            decision(Level::Tenant, class.as_str(), "overflow");
        }
        let keyed = [
            principal.map(|entry| entry.slots(index, usize::from(limits.principal_in_flight))),
            tenant.map(|entry| entry.slots(index, usize::from(limits.tenant_in_flight))),
        ];
        metrics::gauge!(ADMISSION_IDENTITIES, "table" => "principal").set(principals.len() as f64);
        metrics::gauge!(ADMISSION_IDENTITIES, "table" => "tenant").set(tenants.len() as f64);
        Ok(keyed)
    }

    /// Takes principal, tenant and process slots in that order, so a queue
    /// forms at the narrowest level that is full and never holds a wider
    /// level's slot while it waits.
    async fn occupy(
        &self,
        class: OperationClass,
        keyed: [Option<Arc<Semaphore>>; 2],
        started: Instant,
    ) -> Result<Hold, Refusal> {
        let limits = self.0.limits.class(class);
        let pool = &self.0.process[class.index()];
        let deadline = started + limits.max_wait;
        let [principal, tenant] = keyed;
        let levels = [
            (Level::Principal, principal),
            (Level::Tenant, tenant),
            (Level::Process, Some(Arc::clone(&pool.slots))),
        ];
        let mut permits: [Option<OwnedSemaphorePermit>; 3] = [None, None, None];
        let mut queued = None;
        let mut waited = false;
        for (position, (level, slots)) in levels.into_iter().enumerate() {
            let Some(slots) = slots else {
                continue;
            };
            let wait = Wait {
                level,
                class: class.as_str(),
                waiting: &pool.waiting,
                deadline,
            };
            permits[position] = self.slot(slots, wait, &mut queued, &mut waited).await?;
        }
        if waited {
            metrics::histogram!(ADMISSION_WAIT_SECONDS, "class" => class.as_str())
                .record(started.elapsed().as_secs_f64());
        }
        Ok(Hold { _permits: permits })
    }

    async fn slot(
        &self,
        slots: Arc<Semaphore>,
        wait: Wait<'_>,
        queued: &mut Option<OwnedSemaphorePermit>,
        waited: &mut bool,
    ) -> Result<Option<OwnedSemaphorePermit>, Refusal> {
        match Arc::clone(&slots).try_acquire_owned() {
            Ok(permit) => return Ok(Some(permit)),
            Err(TryAcquireError::Closed) => return Err(Refusal::Unavailable),
            Err(TryAcquireError::NoPermits) => {}
        }
        if !self.enforcing() {
            decision(wait.level, wait.class, "observed");
            return Ok(None);
        }
        if queued.is_none() {
            let room = Arc::clone(wait.waiting)
                .try_acquire_owned()
                .map_err(|_| Refusal::QueueFull { level: wait.level })?;
            *queued = Some(room);
        }
        *waited = true;
        match tokio::time::timeout_at(wait.deadline, slots.acquire_owned()).await {
            Ok(Ok(permit)) => Ok(Some(permit)),
            Ok(Err(_)) => Err(Refusal::Unavailable),
            Err(_) => Err(Refusal::WaitExpired { level: wait.level }),
        }
    }

    /// Whether this source may make another unauthenticated attempt.
    fn check_source(&self, source: Source) -> Result<(), Refusal> {
        let now = self.now();
        let mut sources = lock(&self.0.sources);
        let (cell, rate) = self.source_cell(&mut sources, source, now);
        let checked = cell.check(rate, now);
        metrics::gauge!(ADMISSION_IDENTITIES, "table" => "source").set(sources.known.len() as f64);
        checked.map_err(|wait_nanos| Refusal::Rate {
            level: Level::Source,
            wait_nanos,
        })
    }

    fn charge_source(&self, source: Source) {
        let now = self.now();
        let mut sources = lock(&self.0.sources);
        let (cell, rate) = self.source_cell(&mut sources, source, now);
        cell.charge(rate, now);
    }

    /// The source's own cell, or the shared one when it cannot be named or
    /// the table is full.
    fn source_cell<'a>(
        &self,
        sources: &'a mut Sources,
        source: Source,
        now: u64,
    ) -> (&'a mut Cell, Rate) {
        let Sources {
            known,
            unattributed,
        } = sources;
        if let Source::Address(key) = source
            && let Some(entry) = known.entry(key, now)
        {
            return (&mut entry.cells[0], self.0.per_source);
        }
        (unattributed, self.0.unattributed)
    }

    async fn login_slot(&self) -> Result<Hold, Refusal> {
        let deadline = Instant::now() + LOGIN_MAX_WAIT;
        let mut queued = None;
        let mut waited = false;
        let wait = Wait {
            level: Level::Process,
            class: "login",
            waiting: &self.0.login.waiting,
            deadline,
        };
        let permit = self
            .slot(
                Arc::clone(&self.0.login.slots),
                wait,
                &mut queued,
                &mut waited,
            )
            .await?;
        Ok(Hold {
            _permits: [permit, None, None],
        })
    }
}

struct Wait<'a> {
    level: Level,
    class: &'static str,
    waiting: &'a Arc<Semaphore>,
    deadline: Instant,
}

/// Methods the route catalogue declares, with `HEAD` served by `GET`.
fn method_index(method: &str) -> Option<usize> {
    match method {
        "GET" | "HEAD" => Some(0),
        "POST" => Some(1),
        "PATCH" => Some(2),
        "PUT" => Some(3),
        "DELETE" => Some(4),
        _ => None,
    }
}

fn route_classes() -> Result<HashMap<&'static str, [Option<OperationClass>; 5]>, String> {
    let mut routes: HashMap<&'static str, [Option<OperationClass>; 5]> = HashMap::new();
    for (method, path, class) in crate::routes::CLASSIFIED {
        let index = method_index(method).ok_or_else(|| {
            format!("the route catalogue declares an unsupported method on {path}")
        })?;
        routes.entry(path).or_default()[index] = Some(*class);
    }
    Ok(routes)
}

fn decision(level: Level, class: &'static str, outcome: &'static str) {
    metrics::counter!(
        ADMISSION_DECISIONS_TOTAL,
        "stage" => level.as_str(),
        "class" => class,
        "outcome" => outcome,
    )
    .increment(1);
}

/// Which pre-authentication rule applies to a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plane {
    /// Every request is an attempt to obtain or end a credential.
    Login,
    /// Only a failed credential is charged.
    Credentialed,
    /// Static console assets and redirects; nothing to protect here.
    Open,
}

fn plane(path: &str) -> Plane {
    if path.starts_with("/auth/") {
        Plane::Login
    } else if path.starts_with("/v1/") || path.starts_with("/scim/") {
        Plane::Credentialed
    } else {
        Plane::Open
    }
}

/// Pre-authentication screening: a source that has spent its budget is
/// refused before any credential is read, so valid and invalid credentials
/// from it receive the same answer.
pub(crate) async fn screen(
    State(limiter): State<Limiter>,
    request: Request,
    next: Next,
) -> Response {
    let plane = plane(request.uri().path());
    if plane == Plane::Open {
        return next.run(request).await;
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connection| connection.0.ip());
    let source = source::attribute(peer, request.headers(), &limiter.0.limits.trusted_proxies);
    let class = if plane == Plane::Login {
        "login"
    } else {
        "credential"
    };
    if let Err(refusal) = limiter.check_source(source) {
        decision(
            Level::Source,
            class,
            if limiter.enforcing() {
                refusal.outcome()
            } else {
                "observed"
            },
        );
        if limiter.enforcing() {
            return refusal.into_response();
        }
    }
    if plane == Plane::Credentialed {
        let response = next.run(request).await;
        if response.status() == StatusCode::UNAUTHORIZED {
            limiter.charge_source(source);
        }
        return response;
    }
    limiter.charge_source(source);
    let hold = match limiter.login_slot().await {
        Ok(hold) => hold,
        Err(refusal) => {
            decision(Level::Process, class, refusal.outcome());
            return refusal.into_response();
        }
    };
    let response = next.run(request).await;
    drop(hold);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn tight(class: OperationClass, rate: (u32, u32), in_flight: (u16, u16, u16)) -> Limiter {
        let mut limits = Limits::default();
        limits.set_class(
            class,
            ClassLimits {
                principal_rate: RateLimit {
                    per_minute: rate.0,
                    burst: rate.1,
                },
                tenant_rate: RateLimit {
                    per_minute: rate.0 * 2,
                    burst: rate.1 * 2,
                },
                principal_in_flight: in_flight.0,
                tenant_in_flight: in_flight.1,
                process_in_flight: in_flight.2,
                max_wait: Duration::from_millis(500),
                max_waiting: 2,
            },
        );
        Limiter::new(limits).expect("limits")
    }

    #[test]
    fn every_catalogued_operation_and_its_router_spelling_has_a_class() {
        let limiter = Limiter::observing().expect("limiter");
        for operation in crate::routes::OPERATIONS {
            let index = method_index(operation.method).expect("supported method");
            assert!(
                limiter
                    .0
                    .routes
                    .get(operation.path)
                    .and_then(|m| m[index])
                    .is_some(),
                "{} {} has no admission class",
                operation.method,
                operation.path
            );
        }
        assert_eq!(
            crate::routes::CLASSIFIED.len(),
            crate::routes::OPERATIONS.len()
        );
    }

    #[tokio::test]
    async fn a_matched_wildcard_route_head_and_an_absent_method_are_classified() {
        let limiter = Limiter::observing().expect("limiter");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = {
            let (limiter, seen) = (limiter.clone(), Arc::clone(&seen));
            Router::new()
                .route("/v1/prompts/{*name}", get(|| async { "ok" }))
                .route("/v1/audit/export", get(|| async { "ok" }))
                .route("/v1/unclassified", get(|| async { "ok" }))
                .route_layer(axum::middleware::from_fn(
                    move |request: Request, next: Next| {
                        let (limiter, seen) = (limiter.clone(), Arc::clone(&seen));
                        async move {
                            lock(&seen).push(limiter.classify(&request));
                            next.run(request).await
                        }
                    },
                ))
        };
        let mut statuses = Vec::new();
        for (method, uri) in [
            ("GET", "/v1/prompts/team/release-notes"),
            ("HEAD", "/v1/audit/export"),
            ("DELETE", "/v1/audit/export"),
            ("GET", "/v1/unclassified"),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request");
            statuses.push(
                app.clone()
                    .oneshot(request)
                    .await
                    .expect("response")
                    .status(),
            );
        }
        assert_eq!(
            *lock(&seen),
            vec![
                Classified::Operation(OperationClass::Read),
                Classified::Operation(OperationClass::Bulk),
                Classified::NoHandler,
                Classified::Unknown,
            ]
        );
        assert_eq!(
            statuses[2],
            StatusCode::METHOD_NOT_ALLOWED,
            "a catalogued path without the method still reaches the router's 405"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn principal_and_tenant_rates_are_independent_budgets() {
        let limiter = tight(OperationClass::ContextCompose, (60, 2), (4, 8, 16));
        let class = OperationClass::ContextCompose;
        let (acme, globex) = (TenantId::new(), TenantId::new());
        for _ in 0..2 {
            limiter.admit(class, acme, "ana").await.expect("ana burst");
        }
        let refusal = limiter
            .admit(class, acme, "ana")
            .await
            .expect_err("ana spent");
        assert_eq!(
            refusal,
            Refusal::Rate {
                level: Level::Principal,
                wait_nanos: 1_000_000_000
            }
        );
        for _ in 0..2 {
            limiter
                .admit(class, acme, "bo")
                .await
                .expect("bo has its own budget");
        }
        let refusal = limiter
            .admit(class, acme, "cy")
            .await
            .expect_err("acme spent");
        assert!(matches!(
            refusal,
            Refusal::Rate {
                level: Level::Tenant,
                ..
            }
        ));
        limiter
            .admit(class, globex, "ana")
            .await
            .expect("the same subject in another tenant is another principal");
        limiter
            .admit(OperationClass::Read, acme, "ana")
            .await
            .expect("another class is another budget");
        tokio::time::advance(Duration::from_secs(1)).await;
        limiter.admit(class, acme, "cy").await.expect("refilled");
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_level_queues_until_its_deadline_and_then_refuses() {
        let class = OperationClass::Search;
        let limiter = tight(class, (6_000, 100), (1, 2, 3));
        let tenant = TenantId::new();
        let held = limiter.admit(class, tenant, "ana").await.expect("first");
        let started = Instant::now();
        let refusal = limiter
            .admit(class, tenant, "ana")
            .await
            .expect_err("queued");
        assert_eq!(
            refusal,
            Refusal::WaitExpired {
                level: Level::Principal
            }
        );
        assert_eq!(started.elapsed(), Duration::from_millis(500));

        let waiting = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.admit(class, tenant, "ana").await.map(|_| ()) }
        });
        tokio::task::yield_now().await;
        drop(held);
        waiting
            .await
            .expect("joined")
            .expect("admitted when the slot freed");
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_is_bounded_and_overflow_is_refused_at_once() {
        let class = OperationClass::Search;
        let limiter = tight(class, (6_000, 100), (1, 2, 3));
        let tenant = TenantId::new();
        let _held = limiter.admit(class, tenant, "ana").await.expect("first");
        let mut queued = Vec::new();
        for _ in 0..2 {
            let limiter = limiter.clone();
            queued.push(tokio::spawn(async move {
                limiter.admit(class, tenant, "ana").await.map(|_| ())
            }));
        }
        tokio::task::yield_now().await;
        let started = Instant::now();
        let refusal = limiter
            .admit(class, tenant, "ana")
            .await
            .expect_err("room full");
        assert_eq!(
            refusal,
            Refusal::QueueFull {
                level: Level::Principal
            }
        );
        assert_eq!(started.elapsed(), Duration::ZERO);
        for waiter in queued {
            assert!(waiter.await.expect("joined").is_err());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn one_tenant_cannot_take_the_process_share_of_a_class() {
        let class = OperationClass::Mutation;
        let limiter = tight(class, (6_000, 100), (2, 2, 3));
        let (noisy, quiet) = (TenantId::new(), TenantId::new());
        let _a = limiter.admit(class, noisy, "a").await.expect("a");
        let _b = limiter.admit(class, noisy, "b").await.expect("b");
        let refusal = limiter
            .admit(class, noisy, "c")
            .await
            .expect_err("tenant full");
        assert_eq!(
            refusal,
            Refusal::WaitExpired {
                level: Level::Tenant
            }
        );
        let started = Instant::now();
        limiter
            .admit(class, quiet, "d")
            .await
            .expect("another tenant is admitted without waiting");
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_an_admitted_or_waiting_request_releases_its_slots() {
        let class = OperationClass::Bulk;
        let limiter = tight(class, (6_000, 100), (1, 1, 2));
        let tenant = TenantId::new();
        let admitted = tokio::spawn({
            let limiter = limiter.clone();
            async move {
                let _hold = limiter.admit(class, tenant, "ana").await;
                std::future::pending::<()>().await;
            }
        });
        tokio::task::yield_now().await;
        let waiting = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.admit(class, tenant, "ana").await.map(|_| ()) }
        });
        tokio::task::yield_now().await;
        waiting.abort();
        admitted.abort();
        assert!(admitted.await.is_err() && waiting.await.is_err());
        let started = Instant::now();
        limiter.admit(class, tenant, "ana").await.expect("released");
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(
            limiter.0.process[class.index()].waiting.available_permits(),
            2
        );
    }

    #[tokio::test(start_paused = true)]
    async fn identity_churn_stays_within_the_table_caps() {
        let limits = Limits {
            capacity: IdentityCapacity {
                principals: 64,
                tenants: 16,
                sources: 64,
            },
            ..Limits::default()
        };
        let limiter = Limiter::new(limits).expect("limits");
        let tenants: Vec<TenantId> = (0..32).map(|_| TenantId::new()).collect();
        for round in 0..2_000_u32 {
            let tenant = tenants[(round % 32) as usize];
            let hold = limiter
                .admit(OperationClass::Read, tenant, &format!("subject-{round}"))
                .await
                .expect("admitted, if necessary without a remembered identity");
            drop(hold);
        }
        let (principals, tenants, _) = limiter.remembered();
        assert!(principals <= 64 && tenants <= 16, "{principals} {tenants}");
        tokio::time::advance(Duration::from_secs(10)).await;
        limiter
            .admit(OperationClass::Read, TenantId::new(), "later")
            .await
            .expect("rested identities make room");
        let (principals, tenants, _) = limiter.remembered();
        assert_eq!((principals, tenants), (1, 1));
    }

    /// One principal running `agents` concurrent agents, each repeating an
    /// event append and a ContextRun and then pausing; returns every refusal.
    async fn agents_under_one_principal(
        limiter: &Limiter,
        agents: usize,
        iterations: usize,
        run: Duration,
        pause: Duration,
    ) -> Vec<Refusal> {
        let tenant = TenantId::new();
        let mut tasks = Vec::new();
        for _ in 0..agents {
            let limiter = limiter.clone();
            tasks.push(tokio::spawn(async move {
                let mut refused = Vec::new();
                for _ in 0..iterations {
                    for (class, held) in [
                        (OperationClass::SessionIngest, Duration::from_millis(20)),
                        (OperationClass::ContextCompose, run),
                    ] {
                        match limiter.admit(class, tenant, "agent").await {
                            Ok(hold) => {
                                tokio::time::sleep(held).await;
                                drop(hold);
                            }
                            Err(refusal) => refused.push(refusal),
                        }
                    }
                    tokio::time::sleep(pause).await;
                }
                refused
            }));
        }
        let mut refused = Vec::new();
        for task in tasks {
            refused.extend(task.await.expect("joined"));
        }
        refused
    }

    /// The Helm starter's measured load: four agents sharing one agent token,
    /// each 20 rounds of an append and a ContextRun 500 ms apart. It must be
    /// admitted at the defaults however long a run takes up to the class
    /// wait, and twice its ContextRun rate must still fit the principal budget.
    #[tokio::test(start_paused = true)]
    async fn the_starter_reference_load_is_admitted_at_the_defaults() {
        for run_ms in [0_u64, 100, 200, 400, 1_000, 2_000] {
            let limiter = Limiter::new(Limits::default()).expect("limits");
            let refused = agents_under_one_principal(
                &limiter,
                4,
                20,
                Duration::from_millis(run_ms),
                Duration::from_millis(500),
            )
            .await;
            assert!(refused.is_empty(), "{run_ms} ms runs: {refused:?}");
        }
        let limiter = Limiter::new(Limits::default()).expect("limits");
        let refused =
            agents_under_one_principal(&limiter, 8, 20, Duration::ZERO, Duration::from_millis(500))
                .await;
        assert!(refused.is_empty(), "twice the starter rate: {refused:?}");
    }

    /// The deterministic evaluation freezes about 25 capture batches back to
    /// back under one identity, as an agent ending sessions in sequence does;
    /// twice that is admitted at the defaults.
    #[tokio::test(start_paused = true)]
    async fn back_to_back_capture_batches_from_one_identity_are_admitted() {
        let class = crate::routes::CLASSIFIED
            .iter()
            .find(|(method, path, _)| {
                *method == "POST" && *path == "/v1/sessions/{session_id}/capture-batches"
            })
            .map(|(_, _, class)| *class)
            .expect("catalogued");
        let limiter = Limiter::new(Limits::default()).expect("limits");
        let tenant = TenantId::new();
        for batch in 0..50 {
            let hold = limiter
                .admit(class, tenant, "eval")
                .await
                .unwrap_or_else(|refusal| panic!("batch {batch}: {refusal:?}"));
            drop(hold);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn observe_mode_counts_without_refusing_or_waiting() {
        let class = OperationClass::Bulk;
        let mut limits = Limits::observe();
        limits.set_class(
            class,
            ClassLimits {
                principal_rate: RateLimit {
                    per_minute: 1,
                    burst: 1,
                },
                ..*Limits::default().class(class)
            },
        );
        let limiter = Limiter::new(limits).expect("limits");
        let tenant = TenantId::new();
        let mut holds = Vec::new();
        for _ in 0..10 {
            holds.push(limiter.admit(class, tenant, "ana").await.expect("observed"));
        }
        assert_eq!(limiter.check_source(Source::Unattributed), Ok(()));
    }

    #[tokio::test]
    async fn a_refusal_is_the_uniform_rate_limited_envelope_with_bounded_retry() {
        for (refusal, retry) in [
            (
                Refusal::Rate {
                    level: Level::Tenant,
                    wait_nanos: 1_500_000_000,
                },
                "2",
            ),
            (
                Refusal::Rate {
                    level: Level::Principal,
                    wait_nanos: 3_600_000_000_000,
                },
                "60",
            ),
            (
                Refusal::WaitExpired {
                    level: Level::Process,
                },
                "1",
            ),
            (
                Refusal::QueueFull {
                    level: Level::Source,
                },
                "1",
            ),
        ] {
            let response = refusal.into_response();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()[header::RETRY_AFTER], retry);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
            assert_eq!(
                body,
                serde_json::json!({"kind": "rate_limited", "message": REFUSAL_MESSAGE})
            );
        }
        assert_eq!(
            Refusal::Unavailable.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    fn screened(limiter: &Limiter, status: StatusCode, calls: Arc<Mutex<u32>>) -> Router {
        let inner = move || {
            let calls = Arc::clone(&calls);
            async move {
                *lock(&calls) += 1;
                status
            }
        };
        Router::new()
            .route("/v1/me", get(inner.clone()))
            .route("/auth/login", get(inner))
            .route("/console/app.js", get(|| async { "asset" }))
            .route_layer(axum::middleware::from_fn_with_state(
                limiter.clone(),
                screen,
            ))
    }

    fn from(peer: &str, uri: &str, forwarded: Option<&str>) -> Request {
        let mut request = Request::get(uri).body(Body::empty()).expect("request");
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            peer.parse().expect("ip"),
            4_000,
        )));
        if let Some(forwarded) = forwarded {
            request
                .headers_mut()
                .insert("x-forwarded-for", forwarded.parse().expect("header"));
        }
        request
    }

    fn sources(per_minute: u32, burst: u32, trusted: &str) -> Limiter {
        let mut limits = Limits::default();
        limits.sources.per_source = RateLimit { per_minute, burst };
        limits.trusted_proxies = TrustedProxies::parse(trusted).expect("trusted");
        Limiter::new(limits).expect("limits")
    }

    #[tokio::test(start_paused = true)]
    async fn credential_failures_spend_a_source_budget_before_any_credential_is_read() {
        let limiter = sources(60, 3, "");
        let calls = Arc::new(Mutex::new(0));
        let failing = screened(&limiter, StatusCode::UNAUTHORIZED, Arc::clone(&calls));
        for _ in 0..3 {
            let response = failing
                .clone()
                .oneshot(from("203.0.113.9", "/v1/me", None))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let refused = failing
            .clone()
            .oneshot(from("203.0.113.9", "/v1/me", None))
            .await
            .expect("response");
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            *lock(&calls),
            3,
            "the refused request never reached resolution"
        );
        let other = failing
            .clone()
            .oneshot(from("198.51.100.4", "/v1/me", None))
            .await
            .expect("response");
        assert_eq!(
            other.status(),
            StatusCode::UNAUTHORIZED,
            "another source is unaffected"
        );
        let asset = failing
            .oneshot(from("203.0.113.9", "/console/app.js", None))
            .await
            .expect("response");
        assert_eq!(
            asset.status(),
            StatusCode::OK,
            "static assets are not screened"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn successful_credentials_spend_nothing_and_login_requests_spend_one_each() {
        let limiter = sources(60, 2, "");
        let calls = Arc::new(Mutex::new(0));
        let ok = screened(&limiter, StatusCode::OK, Arc::clone(&calls));
        for _ in 0..20 {
            let response = ok
                .clone()
                .oneshot(from("203.0.113.9", "/v1/me", None))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
        }
        for expected in [
            StatusCode::OK,
            StatusCode::OK,
            StatusCode::TOO_MANY_REQUESTS,
        ] {
            let response = ok
                .clone()
                .oneshot(from("203.0.113.9", "/auth/login", None))
                .await
                .expect("response");
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_forwarded_source_is_believed_only_from_a_trusted_proxy() {
        let limiter = sources(60, 1, "10.0.0.0/8");
        let calls = Arc::new(Mutex::new(0));
        let login = screened(&limiter, StatusCode::OK, Arc::clone(&calls));
        let send = |peer: &'static str, forwarded: &'static str| {
            let login = login.clone();
            async move {
                login
                    .oneshot(from(peer, "/auth/login", Some(forwarded)))
                    .await
                    .expect("response")
                    .status()
            }
        };
        assert_eq!(send("10.1.1.1", "198.51.100.1").await, StatusCode::OK);
        assert_eq!(send("10.1.1.1", "198.51.100.2").await, StatusCode::OK);
        assert_eq!(
            send("10.1.1.1", "198.51.100.1").await,
            StatusCode::TOO_MANY_REQUESTS,
            "each forwarded client has its own budget behind the proxy"
        );
        assert_eq!(send("203.0.113.9", "198.51.100.3").await, StatusCode::OK);
        assert_eq!(
            send("203.0.113.9", "198.51.100.4").await,
            StatusCode::TOO_MANY_REQUESTS,
            "an untrusted peer cannot choose its source by forging the header"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn private_clients_behind_the_default_trusted_proxy_keep_separate_budgets() {
        let mut limits = Limits::default();
        limits.sources.per_source = RateLimit {
            per_minute: 60,
            burst: 2,
        };
        let limiter = Limiter::new(limits).expect("limits");
        let failing = screened(&limiter, StatusCode::UNAUTHORIZED, Arc::default());
        let send = |forwarded: &'static str| {
            let failing = failing.clone();
            async move {
                failing
                    .oneshot(from("172.20.0.2", "/v1/me", Some(forwarded)))
                    .await
                    .expect("response")
                    .status()
            }
        };
        for _ in 0..2 {
            assert_eq!(send("192.168.1.10").await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(send("192.168.1.10").await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            send("192.168.1.11").await,
            StatusCode::UNAUTHORIZED,
            "a colleague on the same private network is unaffected"
        );
        assert_eq!(
            send("192.168.1.11, 192.168.1.10").await,
            StatusCode::TOO_MANY_REQUESTS,
            "naming a colleague behind an appending proxy does not escape the budget"
        );
    }
}
