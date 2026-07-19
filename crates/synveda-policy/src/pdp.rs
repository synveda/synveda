//! The embedded Cedar engine behind the facade (ADR-0002, ADR-0012,
//! ADR-0014).
//!
//! Everything Cedar stays inside this crate: packs compile (parse +
//! schema-validate, on top of the invariant base layer) at install time,
//! the effective pack resolves nearest-ancestor-first from caller-supplied
//! assignment rows, decisions evaluate against entities served from the
//! entity store's prebuilt fragments (HIER-3, ADR-0017) — rebuilt from
//! the caller-supplied chains whenever a hierarchy mutation reshaped
//! them — and every call logs its decision with the policy pack version
//! in force (the AUTHZ-1 AC; an AUD-1 emission point until the
//! hash-chained log lands).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName, EntityUid,
    PolicySet, Request, Schema, ValidationMode, Validator,
};
use synveda_types::{Error, HierarchyNode, Result, Role, ScopeId, ScopeKind, TenantId};

use crate::entity_store::EntityStore;
use crate::request::{Action, AuthzContext, AuthzDecision, Principal, Resource};
use crate::{AUTHZ_DECISIONS_TOTAL, POLICY_PACK_FALLBACKS_TOTAL};

/// The default product pack (ADR-0014 decision 1): deny-first, own-chain
/// composition only. In force wherever nothing else is assigned
/// (seed §2.1: strict by default).
pub const REGULATED_STRICT: &str = "regulated-strict";

/// Team-shares-by-default within a department (seed §6).
pub const STANDARD: &str = "standard";

/// Org-wide read, personal scopes excluded (seed §6).
pub const OPEN_COLLABORATION: &str = "open-collaboration";

/// The embedded product packs and their versions — hand-bumped constants,
/// changed whenever the corresponding source changes (ADR-0014
/// decision 1). `@2`: AUTHZ-3 narrowed the admin planes to roles and
/// added the content-role read grant (ADR-0015 decision 4).
pub const EMBEDDED_PACKS: [(&str, i64); 3] = [
    (REGULATED_STRICT, 2),
    (STANDARD, 2),
    (OPEN_COLLABORATION, 2),
];

/// Whether `name` is reserved for the product (ADR-0014 decision 6): the
/// embedded packs, plus the retired `bootstrap`. Stored packs may not use
/// these names — `regulated-strict` must mean the same thing in every
/// tenant. Mirrored by the `policy_packs` check constraint; this is the
/// in-process guard for the same rule.
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    name == "bootstrap" || EMBEDDED_PACKS.iter().any(|(pack, _)| *pack == name)
}

const SCHEMA_SRC: &str = include_str!("synveda.cedarschema");
const BASE_SRC: &str = include_str!("base.cedar");
const REGULATED_STRICT_SRC: &str = include_str!("packs/regulated-strict.cedar");
const STANDARD_SRC: &str = include_str!("packs/standard.cedar");
const OPEN_COLLABORATION_SRC: &str = include_str!("packs/open-collaboration.cedar");

/// A compiled, schema-validated policy pack (base layer included).
struct LoadedPack {
    name: String,
    version: i64,
    policies: PolicySet,
}

/// Where the effective pack came from — logged with every decision so the
/// trail explains not just which pack decided but why it was in force,
/// and surfaced by the policy routes (`GET .../policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackOrigin {
    /// Assigned at this node of the resource's chain.
    Assigned(ScopeId),
    /// The tenant's stored default.
    TenantDefault,
    /// Nothing assigned anywhere: the embedded default (seed §2.1).
    Default,
    /// An assigned name had no compiled pack; fell back to the embedded
    /// default (ADR-0014 decision 7).
    Fallback,
}

/// The pack in force for a resource, as [`Pdp::effective`] resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePack {
    /// The pack's name.
    pub name: String,
    /// The pack's version.
    pub version: i64,
    /// How the pack came to be in force.
    pub origin: PackOrigin,
}

impl fmt::Display for PackOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackOrigin::Assigned(scope) => write!(f, "assigned:{scope}"),
            PackOrigin::TenantDefault => f.write_str("tenant-default"),
            PackOrigin::Default => f.write_str("default"),
            PackOrigin::Fallback => f.write_str("fallback"),
        }
    }
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
    embedded: HashMap<&'static str, Arc<LoadedPack>>,
    tenant_packs: RwLock<HashMap<TenantId, HashMap<String, Arc<LoadedPack>>>>,
    entity_store: EntityStore,
}

impl Pdp {
    /// Builds the PDP: parses the embedded schema and compiles the
    /// embedded product packs. Failure means the binary itself is
    /// broken — callers treat it as fatal at startup.
    pub fn new() -> Result<Self> {
        let (schema, warnings) =
            Schema::from_cedarschema_str(SCHEMA_SRC).map_err(|err| Error::Internal {
                message: format!("embedded Cedar schema does not parse: {err}"),
            })?;
        for warning in warnings {
            tracing::warn!(warning = %warning, "embedded Cedar schema warning");
        }
        let sources = [
            (REGULATED_STRICT, REGULATED_STRICT_SRC),
            (STANDARD, STANDARD_SRC),
            (OPEN_COLLABORATION, OPEN_COLLABORATION_SRC),
        ];
        let mut embedded = HashMap::new();
        for ((name, version), (_, source)) in EMBEDDED_PACKS.iter().zip(sources) {
            let pack = compile(&schema, name, *version, source).map_err(|err| Error::Internal {
                message: format!("embedded pack invalid: {err}"),
            })?;
            embedded.insert(*name, Arc::new(pack));
        }
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
            embedded,
            tenant_packs: RwLock::new(HashMap::new()),
            entity_store: EntityStore::default(),
        })
    }

    /// Drops the tenant's cached Cedar entity fragments (HIER-3,
    /// ADR-0017 decision 5). The gateway's unified
    /// hierarchy-invalidation seam calls this beside the scope-chain
    /// flush after committing any hierarchy mutation. Hygiene, not
    /// correctness: a fragment is served only for a chain whose shape it
    /// matches, so a stale fragment can never decide — this call just
    /// keeps deleted scopes from lingering as dead entries.
    pub fn flush_entities(&self, tenant_id: TenantId) {
        self.entity_store.flush(tenant_id);
    }

    /// Parses and schema-validates `source` (on top of the base layer)
    /// without installing it — the apply-time gate (`synveda policy
    /// apply` refuses a pack the reloader would reject).
    pub fn compile_check(&self, name: &str, source: &str) -> Result<()> {
        compile(&self.schema, name, 0, source).map(|_| ())
    }

    /// Compiles and installs a tenant's stored pack under its name,
    /// replacing any previous version atomically. On error the previous
    /// pack stays in force (ADR-0012 decision 5: a bad apply must not
    /// widen or brick a tenant). Reserved product names are refused —
    /// the in-process face of the store's check constraint.
    pub fn install_source(
        &self,
        tenant_id: TenantId,
        name: &str,
        version: i64,
        source: &str,
    ) -> Result<()> {
        if is_reserved(name) {
            return Err(Error::Invalid {
                message: format!("pack name {name:?} is reserved for the product (ADR-0014)"),
            });
        }
        let pack = compile(&self.schema, name, version, source)?;
        self.write_packs()
            .entry(tenant_id)
            .or_default()
            .insert(name.to_owned(), Arc::new(pack));
        tracing::info!(
            tenant.id = %tenant_id,
            policy.pack = name,
            policy.pack_version = version,
            "policy pack installed"
        );
        Ok(())
    }

    /// Drops one of the tenant's stored packs. Scopes assigned to it fall
    /// back to the embedded default at decision time (ADR-0014
    /// decision 7). Returns whether a pack was actually removed.
    pub fn remove_pack(&self, tenant_id: TenantId, name: &str) -> bool {
        let mut packs = self.write_packs();
        let Some(tenant) = packs.get_mut(&tenant_id) else {
            return false;
        };
        let removed = tenant.remove(name).is_some();
        if tenant.is_empty() {
            packs.remove(&tenant_id);
        }
        if removed {
            tracing::info!(tenant.id = %tenant_id, policy.pack = name, "policy pack removed");
        }
        removed
    }

    /// The `(name, version)` of every stored pack installed for the
    /// tenant — the refresher's reconciliation input.
    #[must_use]
    pub fn installed_versions(&self, tenant_id: TenantId) -> Vec<(String, i64)> {
        self.read_packs()
            .get(&tenant_id)
            .map(|packs| {
                packs
                    .values()
                    .map(|pack| (pack.name.clone(), pack.version))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The facade (seed §6): evaluates `principal` doing `action` on
    /// `resource` under the resource's effective pack (nearest assigned
    /// ancestor → tenant default → `regulated-strict`, ADR-0014
    /// decision 3), against entities materialised from `context`. Every
    /// call — allow and deny — logs the decision with the pack version in
    /// force and increments [`AUTHZ_DECISIONS_TOTAL`].
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
        // Assignment mutations are decided under the pack the node
        // *inherits*, skipping its own assignment (ADR-0014 decision 4):
        // changing a node's governance is authorized by the surrounding
        // governance, never by the pack being replaced — so a restrictive
        // custom pack cannot seal its own node against reassignment.
        let skip_self = action == Action::PolicyAssign;
        let (pack, origin) = self.resolve_pack(principal.tenant_id, resource, context, skip_self);
        let roles = effective_roles(principal, resource, context);
        let entities = self.entities(principal, context)?;
        let request = self.request(principal, action, resource, &roles, context.grant)?;
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
            authz.roles = %roles.join(","),
            policy.pack = %decision.pack_name,
            policy.pack_version = decision.pack_version,
            policy.pack_origin = %origin,
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

    /// The pack in force for `resource` under `context` — the resolution
    /// [`Self::authorize`] applies to every action but `PolicyAssign`,
    /// exposed for the policy routes to display (never for callers to
    /// enforce with).
    #[must_use]
    pub fn effective(
        &self,
        tenant_id: TenantId,
        resource: Resource,
        context: &AuthzContext<'_>,
    ) -> EffectivePack {
        let (pack, origin) = self.resolve_pack(tenant_id, resource, context, false);
        EffectivePack {
            name: pack.name.clone(),
            version: pack.version,
            origin,
        }
    }

    /// Resolves the effective pack for `resource` (ADR-0014 decision 3):
    /// walk the resource's chain from the node upward, nearest assignment
    /// first; then the tenant default; then the embedded default. An
    /// assigned name with no compiled pack falls back to the embedded
    /// default — strict, never dark (decision 7). With `skip_self` the
    /// walk ignores the resource node's own assignment — the
    /// `PolicyAssign` rule (decision 4).
    fn resolve_pack(
        &self,
        tenant_id: TenantId,
        resource: Resource,
        context: &AuthzContext<'_>,
        skip_self: bool,
    ) -> (Arc<LoadedPack>, PackOrigin) {
        let assigned: HashMap<ScopeId, &str> = context
            .assignments
            .iter()
            .map(|assignment| (assignment.scope_id, assignment.pack_name.as_str()))
            .collect();
        let nodes: HashMap<ScopeId, &HierarchyNode> =
            context.scopes.iter().map(|node| (node.id, node)).collect();
        let named = |name: &str, origin: PackOrigin| -> (Arc<LoadedPack>, PackOrigin) {
            match self.lookup(tenant_id, name) {
                Some(pack) => (pack, origin),
                None => {
                    tracing::warn!(
                        tenant.id = %tenant_id,
                        policy.pack = name,
                        "assigned pack has no compiled source; falling back to {REGULATED_STRICT}"
                    );
                    metrics::counter!(POLICY_PACK_FALLBACKS_TOTAL).increment(1);
                    (self.default_pack(), PackOrigin::Fallback)
                }
            }
        };
        if let Resource::Scope(id) = resource {
            let mut current = id;
            let mut skip = skip_self;
            // Bounded by the chain length: a malformed chain cannot loop.
            for _ in 0..=nodes.len() {
                if !skip && let Some(name) = assigned.get(&current) {
                    return named(name, PackOrigin::Assigned(current));
                }
                skip = false;
                match nodes.get(&current).and_then(|node| node.parent_id) {
                    Some(parent) => current = parent,
                    None => break,
                }
            }
        }
        match context.default_pack {
            Some(name) => named(name, PackOrigin::TenantDefault),
            None => (self.default_pack(), PackOrigin::Default),
        }
    }

    /// A pack by name: the tenant's stored packs first, then the embedded
    /// product packs. Reserved names can never be stored, so shadowing is
    /// impossible (ADR-0014 decision 6).
    fn lookup(&self, tenant_id: TenantId, name: &str) -> Option<Arc<LoadedPack>> {
        if let Some(pack) = self
            .read_packs()
            .get(&tenant_id)
            .and_then(|packs| packs.get(name))
        {
            return Some(Arc::clone(pack));
        }
        self.embedded.get(name).map(Arc::clone)
    }

    fn default_pack(&self) -> Arc<LoadedPack> {
        Arc::clone(&self.embedded[REGULATED_STRICT])
    }

    // Lock poisoning would mean a panic mid-`HashMap` operation; the map
    // is still structurally sound, so both sides recover the guard rather
    // than propagating an unactionable error.
    fn read_packs(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<TenantId, HashMap<String, Arc<LoadedPack>>>> {
        self.tenant_packs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write_packs(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashMap<TenantId, HashMap<String, Arc<LoadedPack>>>> {
        self.tenant_packs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Materialises the Cedar entity graph for one decision (ADR-0012
    /// decision 4, ADR-0014 decision 5, ADR-0017): both supplied chains
    /// arrive as prebuilt fragments from the entity store — served when
    /// the chain's shape matches, rebuilt from the chain otherwise — and
    /// the principal entity is built per request, so `quarantined`,
    /// `home`, and `department` keep riding the per-request identity
    /// read (ADR-0016 decision 6).
    fn entities(&self, principal: &Principal, context: &AuthzContext<'_>) -> Result<Entities> {
        use cedar_policy::RestrictedExpression;

        // Both chains may share nodes (a team reading its own scope);
        // Cedar rejects duplicate entity entries, so deduplicate by uid.
        let mut merged: HashMap<EntityUid, Entity> = HashMap::new();
        for chain in [context.scopes, context.principal_scopes] {
            if chain.is_empty() {
                continue;
            }
            let fragment = self
                .entity_store
                .resolve(chain, || self.chain_entities(chain))?;
            for entity in fragment.entities() {
                merged.entry(entity.uid()).or_insert_with(|| entity.clone());
            }
        }

        // The principal's tenant entity exists even when no chain was
        // supplied at all (a tenant resource, an unplaced principal).
        let principal_tenant = self.tenant_uid(principal.tenant_id)?;
        merged
            .entry(principal_tenant.clone())
            .or_insert_with(|| Entity::with_uid(principal_tenant.clone()));
        let mut principal_attrs = HashMap::from([
            (
                "tenant".to_owned(),
                RestrictedExpression::new_entity_uid(principal_tenant.clone()),
            ),
            (
                "quarantined".to_owned(),
                RestrictedExpression::new_bool(principal.quarantined),
            ),
        ]);
        let mut principal_parents = HashSet::from([principal_tenant]);
        if let Some(home) = principal.scope_id {
            // The placement makes the principal a member of its own chain
            // (`principal in resource`), and the `home` attribute lets
            // packs require placement outright (ADR-0014 decision 5).
            let home_uid = self.scope_uid(home)?;
            principal_attrs.insert(
                "home".to_owned(),
                RestrictedExpression::new_entity_uid(home_uid.clone()),
            );
            principal_parents.insert(home_uid);
        }
        if let Some(department) = nearest_department(context.principal_scopes) {
            principal_attrs.insert(
                "department".to_owned(),
                RestrictedExpression::new_entity_uid(self.scope_uid(department)?),
            );
        }
        let mut list: Vec<Entity> = merged.into_values().collect();
        list.push(new_entity(
            self.principal_uid(principal)?,
            principal_attrs,
            principal_parents,
        )?);

        Entities::from_entities(list, Some(&self.schema)).map_err(|err| Error::Internal {
            message: format!("build entity store: {err}"),
        })
    }

    /// Builds one chain's fragment entities (ADR-0017 decision 4): a
    /// `Scope` entity per node — parented along `parent_id`, the root to
    /// its tenant entity — plus the chain's distinct `Tenant` entities.
    /// Every distinct tenant in play gets its entity so a chain from a
    /// foreign tenant chains up to a *different* tenant entity and
    /// membership rules fail closed.
    fn chain_entities(&self, chain: &[HierarchyNode]) -> Result<Vec<Entity>> {
        use cedar_policy::RestrictedExpression;

        let mut list = Vec::with_capacity(chain.len() + 1);
        let mut tenant_ids: Vec<TenantId> = Vec::new();
        for node in chain {
            if !tenant_ids.contains(&node.tenant_id) {
                tenant_ids.push(node.tenant_id);
            }
        }
        for tenant_id in &tenant_ids {
            list.push(Entity::with_uid(self.tenant_uid(*tenant_id)?));
        }
        for node in chain {
            let node_tenant = self.tenant_uid(node.tenant_id)?;
            let parent = match node.parent_id {
                Some(parent_id) => self.scope_uid(parent_id)?,
                None => node_tenant.clone(),
            };
            list.push(new_entity(
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
        Ok(list)
    }

    /// Builds the schema-checked request, `context.roles` included
    /// (ADR-0015 decision 3). `RoleAssign` additionally requires the
    /// grant role (`context.grant`): absent, the request cannot be built
    /// and the decision fails closed (decision 5).
    fn request(
        &self,
        principal: &Principal,
        action: Action,
        resource: Resource,
        roles: &[&'static str],
        grant: Option<Role>,
    ) -> Result<Request> {
        use cedar_policy::RestrictedExpression;

        let resource_uid = match resource {
            Resource::Tenant(id) => self.tenant_uid(id)?,
            Resource::Scope(id) => self.scope_uid(id)?,
        };
        let mut pairs = vec![(
            "roles".to_owned(),
            RestrictedExpression::new_set(
                roles
                    .iter()
                    .map(|role| RestrictedExpression::new_string((*role).to_owned())),
            ),
        )];
        if action == Action::RoleAssign {
            let grant = grant.ok_or_else(|| Error::Internal {
                message: "RoleAssign decided without the grant role in context".to_owned(),
            })?;
            pairs.push((
                "grant".to_owned(),
                RestrictedExpression::new_string(grant.as_str().to_owned()),
            ));
        }
        let context = Context::from_pairs(pairs).map_err(|err| Error::Internal {
            message: format!("build authorization context: {err}"),
        })?;
        Request::new(
            self.principal_uid(principal)?,
            self.uid(&self.action_type, action.cedar_id())?,
            resource_uid,
            context,
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

/// The principal's effective roles at the resource (AUTHZ-3, ADR-0015
/// decision 3): from the caller-supplied binding rows, tenant-wide
/// bindings always apply; a node binding applies iff the bound node is on
/// the resource's chain — that one rule is "inherited downward". For a
/// tenant resource the chain is empty, so only tenant-wide bindings
/// apply: a root-scoped steward manages nodes, never the tenant plane.
/// Foreign-tenant rows are dropped defensively (the store's RLS already
/// makes them unrepresentable). Sorted for a deterministic decision log.
fn effective_roles(
    principal: &Principal,
    resource: Resource,
    context: &AuthzContext<'_>,
) -> Vec<&'static str> {
    let chain: HashSet<ScopeId> = match resource {
        Resource::Scope(_) => context.scopes.iter().map(|node| node.id).collect(),
        Resource::Tenant(_) => HashSet::new(),
    };
    let mut roles: Vec<&'static str> = context
        .role_bindings
        .iter()
        .filter(|binding| binding.tenant_id == principal.tenant_id)
        .filter(|binding| match binding.scope_id {
            None => true,
            Some(scope) => chain.contains(&scope),
        })
        .map(|binding| binding.role.as_str())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    roles
}

/// [`Entity::new`] mapped onto the taxonomy — shared by the fragment
/// builder and the per-request principal entity.
fn new_entity(
    uid: EntityUid,
    attrs: HashMap<String, cedar_policy::RestrictedExpression>,
    parents: HashSet<EntityUid>,
) -> Result<Entity> {
    Entity::new(uid, attrs, parents).map_err(|err| Error::Internal {
        message: format!("materialise entity: {err}"),
    })
}

/// The principal's department: the deepest department-kind node of its
/// placement chain. A chain is a path with strictly increasing ranks
/// (ADR-0011), so more than one can only mean malformed caller data —
/// deepest is then the conservative pick (the narrower subtree).
fn nearest_department(chain: &[HierarchyNode]) -> Option<ScopeId> {
    chain
        .iter()
        .filter(|node| node.kind == ScopeKind::Department)
        .max_by_key(|node| node.depth)
        .map(|node| node.id)
}

/// Parse + schema-validate on top of the invariant base layer (ADR-0014
/// decision 2): a pack that compiles can never fail at decision time for
/// structural reasons (ADR-0012 decision 2), and no pack can drop the
/// base rules.
fn compile(schema: &Schema, name: &str, version: i64, source: &str) -> Result<LoadedPack> {
    let combined = format!("{BASE_SRC}\n{source}");
    let policies: PolicySet = combined.parse().map_err(|err| Error::Invalid {
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
