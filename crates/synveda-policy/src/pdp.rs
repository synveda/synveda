//! The embedded Cedar engine behind the facade (ADR-0002, ADR-0012).
//!
//! Everything Cedar stays inside this module: packs compile (parse +
//! schema-validate) at install time, decisions evaluate against per-request
//! entities materialised from caller-supplied hierarchy rows, and every
//! call logs its decision with the policy pack version in force (the
//! AUTHZ-1 AC; an AUD-1 emission point until the hash-chained log lands).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName, EntityUid,
    PolicySet, Request, Schema, ValidationMode, Validator,
};
use synveda_types::{Error, Result, ScopeId, TenantId};

use crate::AUTHZ_DECISIONS_TOTAL;
use crate::request::{Action, AuthzContext, AuthzDecision, Principal, Resource};

/// The embedded default pack's name (ADR-0012 decision 3).
pub const BOOTSTRAP_PACK: &str = "bootstrap";

/// The embedded default pack's version.
pub const BOOTSTRAP_VERSION: i64 = 1;

const SCHEMA_SRC: &str = include_str!("synveda.cedarschema");
const BOOTSTRAP_SRC: &str = include_str!("bootstrap.cedar");

/// A compiled, schema-validated policy pack.
struct LoadedPack {
    name: String,
    version: i64,
    policies: PolicySet,
}

/// The Policy Decision Point: the one `authorize` chokepoint every governed
/// action passes through (seed §2.2). Cheap to share behind an `Arc`;
/// decisions take a read lock only.
pub struct Pdp {
    schema: Schema,
    authorizer: Authorizer,
    tenant_type: EntityTypeName,
    principal_type: EntityTypeName,
    scope_type: EntityTypeName,
    action_type: EntityTypeName,
    default_pack: Arc<LoadedPack>,
    tenant_packs: RwLock<HashMap<TenantId, Arc<LoadedPack>>>,
}

impl Pdp {
    /// Builds the PDP: parses the embedded schema and compiles the
    /// embedded `bootstrap` pack. Failure means the binary itself is
    /// broken — callers treat it as fatal at startup.
    pub fn new() -> Result<Self> {
        let (schema, warnings) =
            Schema::from_cedarschema_str(SCHEMA_SRC).map_err(|err| Error::Internal {
                message: format!("embedded Cedar schema does not parse: {err}"),
            })?;
        for warning in warnings {
            tracing::warn!(warning = %warning, "embedded Cedar schema warning");
        }
        let default_pack = compile(&schema, BOOTSTRAP_PACK, BOOTSTRAP_VERSION, BOOTSTRAP_SRC)
            .map_err(|err| Error::Internal {
                message: format!("embedded bootstrap pack invalid: {err}"),
            })?;
        let type_name = |name: &str| -> Result<EntityTypeName> {
            name.parse().map_err(|err| Error::Internal {
                message: format!("entity type {name:?}: {err}"),
            })
        };
        Ok(Pdp {
            schema,
            authorizer: Authorizer::new(),
            tenant_type: type_name("Synveda::Tenant")?,
            principal_type: type_name("Synveda::Principal")?,
            scope_type: type_name("Synveda::Scope")?,
            action_type: type_name("Synveda::Action")?,
            default_pack: Arc::new(default_pack),
            tenant_packs: RwLock::new(HashMap::new()),
        })
    }

    /// Parses and schema-validates `source` without installing it — the
    /// apply-time gate (`synveda policy apply` refuses a pack the reloader
    /// would reject).
    pub fn compile_check(&self, name: &str, source: &str) -> Result<()> {
        compile(&self.schema, name, 0, source).map(|_| ())
    }

    /// Compiles and installs a tenant's pack, replacing any previous one
    /// atomically. On error the previous pack stays in force (ADR-0012
    /// decision 5: a bad apply must not widen or brick a tenant).
    pub fn install_source(
        &self,
        tenant_id: TenantId,
        name: &str,
        version: i64,
        source: &str,
    ) -> Result<()> {
        let pack = compile(&self.schema, name, version, source)?;
        self.write_packs().insert(tenant_id, Arc::new(pack));
        tracing::info!(
            tenant.id = %tenant_id,
            policy.pack = name,
            policy.pack_version = version,
            "policy pack installed"
        );
        Ok(())
    }

    /// Drops a tenant's stored pack; the tenant falls back to `bootstrap`.
    /// Returns whether a pack was actually removed.
    pub fn remove_pack(&self, tenant_id: TenantId) -> bool {
        let removed = self.write_packs().remove(&tenant_id).is_some();
        if removed {
            tracing::info!(tenant.id = %tenant_id, "policy pack removed; bootstrap in force");
        }
        removed
    }

    /// The `(name, version)` of the tenant's installed pack, if any — the
    /// reloader's unchanged-skip check.
    #[must_use]
    pub fn installed_version(&self, tenant_id: TenantId) -> Option<(String, i64)> {
        self.read_packs()
            .get(&tenant_id)
            .map(|pack| (pack.name.clone(), pack.version))
    }

    /// The facade (seed §6): evaluates `principal` doing `action` on
    /// `resource` under the tenant's pack (or `bootstrap`), against
    /// entities materialised from `context`. Every call — allow and deny —
    /// logs the decision with the pack version in force and increments
    /// [`AUTHZ_DECISIONS_TOTAL`].
    #[tracing::instrument(
        name = "policy.authorize",
        skip_all,
        fields(tenant.id = %principal.tenant_id, authz.action = %action, authz.resource = %resource)
    )]
    pub fn authorize(
        &self,
        principal: &Principal,
        action: Action,
        resource: Resource,
        context: &AuthzContext<'_>,
    ) -> Result<AuthzDecision> {
        let pack = self.pack_for(principal.tenant_id);
        let entities = self.entities(principal, context)?;
        let request = self.request(principal, action, resource)?;
        let response = self
            .authorizer
            .is_authorized(&request, &pack.policies, &entities);
        // Evaluation errors mean a policy did not evaluate (e.g. a missing
        // attribute); Cedar's semantics keep the outcome fail-closed, and
        // the error belongs in the trace.
        for error in response.diagnostics().errors() {
            tracing::warn!(error = %error, "policy evaluation error");
        }
        let allowed = response.decision() == Decision::Allow;
        let decision = AuthzDecision {
            allowed,
            pack_name: pack.name.clone(),
            pack_version: pack.version,
            determining: response
                .diagnostics()
                .reason()
                .map(ToString::to_string)
                .collect(),
        };
        let verdict = if allowed { "allow" } else { "deny" };
        // The AUTHZ-1 AC: decision + policy version logged for every call.
        // AUD-1 emission point: this call site emits the audit event once
        // the hash-chained log lands (ADR-0012 decision 6).
        tracing::info!(
            tenant.id = %principal.tenant_id,
            principal.subject = %principal.subject,
            authz.action = %action,
            authz.resource = %resource,
            authz.decision = verdict,
            policy.pack = %decision.pack_name,
            policy.pack_version = decision.pack_version,
            policy.determining = %decision.determining.join(","),
            "authorization decision"
        );
        metrics::counter!(
            AUTHZ_DECISIONS_TOTAL,
            "action" => action.as_str(),
            "decision" => verdict,
            "pack" => decision.pack_name.clone(),
        )
        .increment(1);
        Ok(decision)
    }

    /// [`Self::authorize`] collapsed into the taxonomy: `Ok(())` on allow,
    /// [`Error::PolicyDenied`] on deny — the shape enforcement call sites
    /// want.
    pub fn require(
        &self,
        principal: &Principal,
        action: Action,
        resource: Resource,
        context: &AuthzContext<'_>,
    ) -> Result<()> {
        self.authorize(principal, action, resource, context)?
            .require(action, resource)
    }

    /// The tenant's installed pack, or the embedded default.
    fn pack_for(&self, tenant_id: TenantId) -> Arc<LoadedPack> {
        self.read_packs()
            .get(&tenant_id)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.default_pack))
    }

    // Lock poisoning would mean a panic mid-`HashMap` operation; the map
    // is still structurally sound, so both sides recover the guard rather
    // than propagating an unactionable error.
    fn read_packs(&self) -> std::sync::RwLockReadGuard<'_, HashMap<TenantId, Arc<LoadedPack>>> {
        self.tenant_packs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write_packs(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<TenantId, Arc<LoadedPack>>> {
        self.tenant_packs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Materialises the Cedar entity graph for one decision (ADR-0012
    /// decision 4): tenant entities, the principal parented to its tenant,
    /// and the supplied scope chain parented along `parent_id` up to the
    /// org, whose parent is its tenant entity.
    fn entities(&self, principal: &Principal, context: &AuthzContext<'_>) -> Result<Entities> {
        use cedar_policy::RestrictedExpression;

        let mut list = Vec::with_capacity(context.scopes.len() + 2);

        // Every distinct tenant in play gets its entity; a chain from a
        // foreign tenant therefore chains up to a *different* tenant
        // entity and membership rules fail closed.
        let mut tenant_ids = vec![principal.tenant_id];
        for node in context.scopes {
            if !tenant_ids.contains(&node.tenant_id) {
                tenant_ids.push(node.tenant_id);
            }
        }
        for tenant_id in &tenant_ids {
            list.push(Entity::with_uid(self.tenant_uid(*tenant_id)?));
        }

        let entity = |uid: EntityUid,
                      attrs: HashMap<String, RestrictedExpression>,
                      parents: HashSet<EntityUid>|
         -> Result<Entity> {
            Entity::new(uid, attrs, parents).map_err(|err| Error::Internal {
                message: format!("materialise entity: {err}"),
            })
        };

        let principal_tenant = self.tenant_uid(principal.tenant_id)?;
        list.push(entity(
            self.principal_uid(principal)?,
            HashMap::from([(
                "tenant".to_owned(),
                RestrictedExpression::new_entity_uid(principal_tenant.clone()),
            )]),
            HashSet::from([principal_tenant]),
        )?);

        for node in context.scopes {
            let node_tenant = self.tenant_uid(node.tenant_id)?;
            let parent = match node.parent_id {
                Some(parent_id) => self.scope_uid(parent_id)?,
                None => node_tenant.clone(),
            };
            list.push(entity(
                self.scope_uid(node.id)?,
                HashMap::from([
                    (
                        "tenant".to_owned(),
                        RestrictedExpression::new_entity_uid(node_tenant),
                    ),
                    (
                        "kind".to_owned(),
                        RestrictedExpression::new_string(node.kind.as_str().to_owned()),
                    ),
                ]),
                HashSet::from([parent]),
            )?);
        }

        Entities::from_entities(list, Some(&self.schema)).map_err(|err| Error::Internal {
            message: format!("build entity store: {err}"),
        })
    }

    fn request(
        &self,
        principal: &Principal,
        action: Action,
        resource: Resource,
    ) -> Result<Request> {
        let resource_uid = match resource {
            Resource::Tenant(id) => self.tenant_uid(id)?,
            Resource::Scope(id) => self.scope_uid(id)?,
        };
        Request::new(
            self.principal_uid(principal)?,
            self.uid(&self.action_type, action.cedar_id())?,
            resource_uid,
            Context::empty(),
            Some(&self.schema),
        )
        .map_err(|err| Error::Internal {
            message: format!("build authorization request: {err}"),
        })
    }

    fn tenant_uid(&self, id: TenantId) -> Result<EntityUid> {
        self.uid(&self.tenant_type, &id.to_string())
    }

    fn scope_uid(&self, id: ScopeId) -> Result<EntityUid> {
        self.uid(&self.scope_type, &id.to_string())
    }

    /// Tenant-qualified so equal subjects from different IdPs/tenants can
    /// never alias to one entity.
    fn principal_uid(&self, principal: &Principal) -> Result<EntityUid> {
        self.uid(
            &self.principal_type,
            &format!("{}/{}", principal.tenant_id, principal.subject),
        )
    }

    fn uid(&self, type_name: &EntityTypeName, id: &str) -> Result<EntityUid> {
        let eid: EntityId = id.parse().map_err(|err| Error::Internal {
            message: format!("entity id {id:?}: {err}"),
        })?;
        Ok(EntityUid::from_type_name_and_id(type_name.clone(), eid))
    }
}

/// Parse + schema-validate: a pack that compiles can never fail at
/// decision time for structural reasons (ADR-0012 decision 2).
fn compile(schema: &Schema, name: &str, version: i64, source: &str) -> Result<LoadedPack> {
    let policies: PolicySet = source.parse().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} does not parse: {err}"),
    })?;
    let validation = Validator::new(schema.clone()).validate(&policies, ValidationMode::default());
    if !validation.validation_passed() {
        let errors: Vec<String> = validation
            .validation_errors()
            .map(ToString::to_string)
            .collect();
        return Err(Error::Invalid {
            message: format!(
                "policy pack {name}@{version} failed schema validation: {}",
                errors.join("; ")
            ),
        });
    }
    Ok(LoadedPack {
        name: name.to_owned(),
        version,
        policies,
    })
}
