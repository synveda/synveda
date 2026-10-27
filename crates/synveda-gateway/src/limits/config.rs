//! Admission limits for one gateway process.
//!
//! The defaults are a conservative proposal chosen for the reference database
//! pool of eight connections. One bounded JSON document may replace any value;
//! the merged result must still satisfy the ordering that, within each class,
//! keeps one principal from filling its tenant's share and one tenant from
//! filling the process.

use std::time::Duration;

use serde::Deserialize;

use super::class::OperationClass;
use super::source::TrustedProxies;

/// Upper bound on the limits document, which is configuration, not data.
pub const MAX_LIMITS_DOCUMENT_BYTES: usize = 16_384;

/// Whether a refusal is returned or only counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Refuse over-budget work with 429.
    Enforce,
    /// Admit everything and count what enforcement would have refused.
    Observe,
}

impl Mode {
    /// Parses `enforce` or `observe`; anything else is refused.
    ///
    /// # Errors
    ///
    /// A message naming the setting and the accepted values.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "enforce" => Ok(Self::Enforce),
            "observe" => Ok(Self::Observe),
            _ => Err("SYNVEDA_ADMISSION_MODE must be `enforce` or `observe`".to_owned()),
        }
    }

    /// Stable metric and log label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Observe => "observe",
        }
    }
}

/// A sustained per-minute rate and the burst admitted from rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Units per minute.
    pub per_minute: u32,
    /// Units admitted back to back from rest.
    pub burst: u32,
}

/// Budgets for one operation class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassLimits {
    /// Each verified principal's rate.
    pub principal_rate: RateLimit,
    /// Each tenant's rate across its principals.
    pub tenant_rate: RateLimit,
    /// Requests one principal may have admitted at once.
    pub principal_in_flight: u16,
    /// Requests one tenant may have admitted at once.
    pub tenant_in_flight: u16,
    /// Requests this process admits at once across tenants.
    pub process_in_flight: u16,
    /// The longest a request waits for concurrency before refusal.
    pub max_wait: Duration,
    /// Requests that may wait at once; more are refused immediately.
    pub max_waiting: u16,
}

/// Budgets charged before a credential is verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLimits {
    /// Login-plane requests and credential failures per network source.
    pub per_source: RateLimit,
    /// One shared budget for requests that cannot be attributed to a source
    /// or arrive after the source table is full.
    pub unattributed: RateLimit,
    /// Login-plane requests this process admits at once.
    pub login_in_flight: u16,
}

/// Hard caps on remembered identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityCapacity {
    /// Verified principals.
    pub principals: u32,
    /// Verified tenants.
    pub tenants: u32,
    /// Network sources.
    pub sources: u32,
}

/// Deployment ceilings on durable tenant usage per UTC day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageCeilings {
    /// Stored Session event payload bytes.
    pub session_event_bytes: i64,
    /// Created ContextRuns.
    pub context_runs: i64,
}

/// Every admission limit for one gateway process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Refuse or only count.
    pub mode: Mode,
    /// Per-class budgets in [`OperationClass::ALL`] order.
    pub classes: [ClassLimits; 6],
    /// Pre-authentication budgets.
    pub sources: SourceLimits,
    /// Identity table caps.
    pub capacity: IdentityCapacity,
    /// Durable tenant usage ceilings.
    pub usage: UsageCeilings,
    /// Peers whose `X-Forwarded-For` names the source: loopback, private and
    /// link-local networks unless configured.
    pub trusted_proxies: TrustedProxies,
}

const fn class(
    principal: (u32, u32),
    tenant: (u32, u32),
    in_flight: (u16, u16, u16),
    max_wait_ms: u64,
    max_waiting: u16,
) -> ClassLimits {
    ClassLimits {
        principal_rate: RateLimit {
            per_minute: principal.0,
            burst: principal.1,
        },
        tenant_rate: RateLimit {
            per_minute: tenant.0,
            burst: tenant.1,
        },
        principal_in_flight: in_flight.0,
        tenant_in_flight: in_flight.1,
        process_in_flight: in_flight.2,
        max_wait: Duration::from_millis(max_wait_ms),
        max_waiting,
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            mode: Mode::Enforce,
            classes: [
                class((1_200, 120), (6_000, 600), (8, 12, 24), 2_000, 256),
                class((300, 30), (1_200, 120), (2, 3, 6), 2_000, 64),
                class((1_800, 180), (9_000, 900), (3, 4, 12), 2_000, 256),
                class((960, 96), (1_200, 120), (2, 3, 6), 3_000, 64),
                class((600, 60), (3_000, 300), (3, 4, 8), 2_000, 128),
                class((12, 3), (60, 6), (1, 2, 3), 1_000, 8),
            ],
            sources: SourceLimits {
                per_source: RateLimit {
                    per_minute: 300,
                    burst: 60,
                },
                unattributed: RateLimit {
                    per_minute: 1_200,
                    burst: 200,
                },
                login_in_flight: 16,
            },
            capacity: IdentityCapacity {
                principals: 16_384,
                tenants: 4_096,
                sources: 16_384,
            },
            usage: UsageCeilings {
                session_event_bytes: 1 << 30,
                context_runs: 100_000,
            },
            trusted_proxies: TrustedProxies::default(),
        }
    }
}

impl Limits {
    /// The default limits counted but never enforced, for behavior tests that
    /// exercise other contracts through the same admission path.
    #[must_use]
    pub fn observe() -> Self {
        Self {
            mode: Mode::Observe,
            ..Self::default()
        }
    }

    /// The limits for one class.
    #[must_use]
    pub const fn class(&self, class: OperationClass) -> &ClassLimits {
        &self.classes[class.index()]
    }

    /// Replaces one class's limits, for tests and tuning tools.
    pub fn set_class(&mut self, class: OperationClass, limits: ClassLimits) {
        self.classes[class.index()] = limits;
    }

    /// Builds limits from the three admission settings.
    ///
    /// # Errors
    ///
    /// A message naming the setting and bound; setting values are not echoed.
    pub fn from_settings(
        mode: Option<&str>,
        document: Option<&str>,
        trusted_proxies: Option<&str>,
    ) -> Result<Self, String> {
        let mut limits = Self::default();
        if let Some(mode) = mode {
            limits.mode = Mode::parse(mode.trim())?;
        }
        if let Some(document) = document {
            if document.len() > MAX_LIMITS_DOCUMENT_BYTES {
                return Err(format!(
                    "SYNVEDA_ADMISSION_LIMITS exceeds {MAX_LIMITS_DOCUMENT_BYTES} bytes"
                ));
            }
            let overrides: Overrides = serde_json::from_str(document).map_err(|error| {
                format!(
                    "SYNVEDA_ADMISSION_LIMITS is not a valid limits document at line {} column {}",
                    error.line(),
                    error.column()
                )
            })?;
            overrides.apply(&mut limits);
        }
        if let Some(trusted) = trusted_proxies {
            limits.trusted_proxies = TrustedProxies::parse(trusted)?;
        }
        limits.validate()?;
        Ok(limits)
    }

    /// Checks every bound and ordering invariant.
    ///
    /// # Errors
    ///
    /// The first violated bound, naming its document path.
    pub fn validate(&self) -> Result<(), String> {
        for class in OperationClass::ALL {
            validate_class(class.as_str(), self.class(class))?;
        }
        rate("sources.per_source", self.sources.per_source)?;
        rate("sources.unattributed", self.sources.unattributed)?;
        within(
            "sources.login_in_flight",
            self.sources.login_in_flight,
            1,
            1_024,
        )?;
        within(
            "identities.principals",
            self.capacity.principals,
            64,
            1_000_000,
        )?;
        within("identities.tenants", self.capacity.tenants, 16, 100_000)?;
        within("identities.sources", self.capacity.sources, 64, 1_000_000)?;
        within(
            "tenant_daily.session_event_bytes",
            self.usage.session_event_bytes,
            1,
            1 << 50,
        )?;
        within(
            "tenant_daily.context_runs",
            self.usage.context_runs,
            1,
            1_000_000_000,
        )
    }
}

fn validate_class(name: &str, limits: &ClassLimits) -> Result<(), String> {
    rate(&format!("classes.{name}.principal"), limits.principal_rate)?;
    rate(&format!("classes.{name}.tenant"), limits.tenant_rate)?;
    for (field, value) in [
        ("principal_in_flight", limits.principal_in_flight),
        ("tenant_in_flight", limits.tenant_in_flight),
        ("process_in_flight", limits.process_in_flight),
    ] {
        within(&format!("classes.{name}.{field}"), value, 1, 1_024)?;
    }
    within(
        &format!("classes.{name}.max_waiting"),
        limits.max_waiting,
        0,
        4_096,
    )?;
    if limits.max_wait > Duration::from_secs(30) {
        return Err(format!("classes.{name}.max_wait_ms must be at most 30000"));
    }
    if limits.principal_in_flight > limits.tenant_in_flight
        || limits.tenant_in_flight >= limits.process_in_flight
    {
        return Err(format!(
            "classes.{name} needs principal_in_flight <= tenant_in_flight < process_in_flight"
        ));
    }
    if limits.principal_rate.per_minute > limits.tenant_rate.per_minute
        || limits.principal_rate.burst > limits.tenant_rate.burst
    {
        return Err(format!(
            "classes.{name} needs the principal rate and burst at most the tenant's"
        ));
    }
    Ok(())
}

fn rate(name: &str, limit: RateLimit) -> Result<(), String> {
    within(&format!("{name}_per_minute"), limit.per_minute, 1, 600_000)?;
    within(&format!("{name}_burst"), limit.burst, 1, 10_000)
}

fn within<T: PartialOrd + std::fmt::Display>(
    name: &str,
    value: T,
    minimum: T,
    maximum: T,
) -> Result<(), String> {
    if value < minimum || value > maximum {
        return Err(format!(
            "SYNVEDA_ADMISSION_LIMITS {name} must be between {minimum} and {maximum}"
        ));
    }
    Ok(())
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Overrides {
    #[serde(default)]
    classes: ClassOverrides,
    sources: Option<SourceOverrides>,
    identities: Option<IdentityOverrides>,
    tenant_daily: Option<UsageOverrides>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ClassOverrides {
    read: Option<ClassOverride>,
    search: Option<ClassOverride>,
    session_ingest: Option<ClassOverride>,
    context_compose: Option<ClassOverride>,
    mutation: Option<ClassOverride>,
    bulk: Option<ClassOverride>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ClassOverride {
    principal_per_minute: Option<u32>,
    principal_burst: Option<u32>,
    tenant_per_minute: Option<u32>,
    tenant_burst: Option<u32>,
    principal_in_flight: Option<u16>,
    tenant_in_flight: Option<u16>,
    process_in_flight: Option<u16>,
    max_wait_ms: Option<u32>,
    max_waiting: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOverrides {
    per_source_per_minute: Option<u32>,
    per_source_burst: Option<u32>,
    unattributed_per_minute: Option<u32>,
    unattributed_burst: Option<u32>,
    login_in_flight: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityOverrides {
    principals: Option<u32>,
    tenants: Option<u32>,
    sources: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageOverrides {
    session_event_bytes: Option<i64>,
    context_runs: Option<i64>,
}

fn set<T: Copy>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

impl Overrides {
    fn apply(self, limits: &mut Limits) {
        let classes = [
            self.classes.read,
            self.classes.search,
            self.classes.session_ingest,
            self.classes.context_compose,
            self.classes.mutation,
            self.classes.bulk,
        ];
        for (target, change) in limits.classes.iter_mut().zip(classes) {
            if let Some(change) = change {
                change.apply(target);
            }
        }
        if let Some(sources) = self.sources {
            let target = &mut limits.sources;
            set(
                &mut target.per_source.per_minute,
                sources.per_source_per_minute,
            );
            set(&mut target.per_source.burst, sources.per_source_burst);
            set(
                &mut target.unattributed.per_minute,
                sources.unattributed_per_minute,
            );
            set(&mut target.unattributed.burst, sources.unattributed_burst);
            set(&mut target.login_in_flight, sources.login_in_flight);
        }
        if let Some(identities) = self.identities {
            set(&mut limits.capacity.principals, identities.principals);
            set(&mut limits.capacity.tenants, identities.tenants);
            set(&mut limits.capacity.sources, identities.sources);
        }
        if let Some(usage) = self.tenant_daily {
            set(
                &mut limits.usage.session_event_bytes,
                usage.session_event_bytes,
            );
            set(&mut limits.usage.context_runs, usage.context_runs);
        }
    }
}

impl ClassOverride {
    fn apply(self, target: &mut ClassLimits) {
        set(
            &mut target.principal_rate.per_minute,
            self.principal_per_minute,
        );
        set(&mut target.principal_rate.burst, self.principal_burst);
        set(&mut target.tenant_rate.per_minute, self.tenant_per_minute);
        set(&mut target.tenant_rate.burst, self.tenant_burst);
        set(&mut target.principal_in_flight, self.principal_in_flight);
        set(&mut target.tenant_in_flight, self.tenant_in_flight);
        set(&mut target.process_in_flight, self.process_in_flight);
        set(&mut target.max_waiting, self.max_waiting);
        if let Some(wait) = self.max_wait_ms {
            target.max_wait = Duration::from_millis(u64::from(wait));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_valid_and_enforcing() {
        let limits = Limits::default();
        limits.validate().expect("defaults");
        assert_eq!(limits.mode, Mode::Enforce);
        assert_eq!(Limits::observe().mode, Mode::Observe);
        assert_eq!(
            Limits::from_settings(None, None, None).expect("unset"),
            limits
        );
    }

    #[test]
    fn a_partial_document_replaces_only_named_values() {
        let limits = Limits::from_settings(
            Some("observe"),
            Some(
                r#"{"classes":{"context_compose":{"principal_per_minute":60,"max_wait_ms":500}},
                    "tenant_daily":{"context_runs":10}}"#,
            ),
            Some("10.0.0.0/8"),
        )
        .expect("valid");
        assert_eq!(limits.mode, Mode::Observe);
        let compose = limits.class(OperationClass::ContextCompose);
        assert_eq!(compose.principal_rate.per_minute, 60);
        assert_eq!(compose.max_wait, Duration::from_millis(500));
        assert_eq!(
            compose.tenant_rate,
            Limits::default()
                .class(OperationClass::ContextCompose)
                .tenant_rate
        );
        assert_eq!(limits.usage.context_runs, 10);
        assert_eq!(
            limits.trusted_proxies,
            TrustedProxies::parse("10.0.0.0/8").expect("trusted")
        );
    }

    #[test]
    fn unknown_fields_out_of_bound_values_and_broken_ordering_are_refused() {
        for document in [
            r#"{"classes":{"read":{"principal_rate":5}}}"#,
            r#"{"classes":{"telepathy":{}}}"#,
            r#"{"classes":{"read":{"principal_per_minute":0}}}"#,
            r#"{"classes":{"read":{"max_wait_ms":30001}}}"#,
            r#"{"classes":{"read":{"tenant_in_flight":24}}}"#,
            r#"{"classes":{"bulk":{"principal_in_flight":3}}}"#,
            r#"{"classes":{"search":{"principal_per_minute":1201}}}"#,
            r#"{"identities":{"principals":1}}"#,
            r#"{"tenant_daily":{"session_event_bytes":0}}"#,
            "[]",
        ] {
            assert!(
                Limits::from_settings(None, Some(document), None).is_err(),
                "{document} must be refused"
            );
        }
        assert!(Limits::from_settings(Some("off"), None, None).is_err());
        let oversized = " ".repeat(MAX_LIMITS_DOCUMENT_BYTES + 1);
        assert!(Limits::from_settings(None, Some(&oversized), None).is_err());
    }

    #[test]
    fn refusals_do_not_echo_the_document() {
        let error = Limits::from_settings(None, Some(r#"{"secret-looking-key":1}"#), None)
            .expect_err("refused");
        assert!(!error.contains("secret-looking-key"), "{error}");
    }
}
