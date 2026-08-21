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
use synveda_types::access::{RoleKey, inherits_into};
use synveda_types::{
    ApprovalMatrix, CompositionConfig, DedupConfig, Error, Lapse, LapseAction, LapseConfig,
    MoverConfig, PackConfig, PromotionConfig, RedactionConfig, Result, RetentionConfig, ScopeId,
    Sensitivity, SkillQualityConfig, SkillScanConfig, TenantId,
};

use synveda_types::scope::ScopeKind;

use crate::entity_store::EntityStore;
use crate::request::{
    Action, AuthzContext, AuthzDecision, Principal, Resource, ResourceEntity, ScopeNode,
};
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
/// added the content-role read grant (ADR-0015 decision 4). `@3`: AUTH-3
/// added the service-identity plane to the admin permits (ADR-0018
/// decision 3). `@4`: MEM-1 added the `MemoryWrite` own-home floor and
/// content-role write grant (ADR-0020 decision 3). `@5`: MEM-2 added the
/// quarantine review plane (ADR-0021 decision 6). `@6`: FLOW-2 added the
/// channel plane (ADR-0031 decision 12). `@7`: FLOW-3 added the proposal
/// plane and each pack's approval matrix (ADR-0032 decisions 3 and 16).
/// `@8`: FLOW-7 added the rewind and pin actions (ADR-0036 decision 3).
/// `@9`: AUTHZ-4 added the lapse plane and the base layer's first permit
/// (ADR-0037 decisions 7 and 15). `@10`: AUTHZ-5 made sensitivity a policy
/// attribute — every `MemoryRead` permit names the tiers it covers, the base
/// layer forbids `restricted` outright unless a lapse declared it, and the
/// classification plane joined (ADR-0038 decisions 4, 5 and 9). `@11`: AUD-2
/// added `AuditRead` to the read-only admin permit every pack has carried
/// since AUTHZ-2 — the line whose comment named this feature — which makes
/// `auditor` a role with a live action rather than a marker row in the
/// golden matrix (ADR-0045 decision 1). `@12`: PRMT-1 added the prompt
/// registry's two seams — `PromptRead`, mirroring each pack's own MemoryRead
/// shape tier for tier, and `PromptWrite`, mirroring its write floor — and
/// the base layer's confinement carve-out gained `PromptRead` beside
/// `MemoryRead`, because a team-anchored agent is the consumer prompts exist
/// for and the org's are on its own chain (ADR-0049 decision 4). `@13`:
/// PRMT-2 added the context-pack registry's two seams on the same shape —
/// `ContextPackRead` is what admits pack chunks into a composed block, so
/// the confinement carve-out gained it too — and re-priced
/// `regulated-strict`'s `context-pack` approval cell, which FLOW-3 had left
/// at one curator at every scope kind and nothing could reach until now
/// (ADR-0050 decisions 7, 8 and 15). `@14`: SKIL-1 added the skills
/// registry's two seams — `SkillRead`, on the context-pack plane's shape
/// and in the confinement carve-out beside it, because an agent that cannot
/// resolve the org's skills cannot do the work they were published for, and
/// `SkillWrite`, separate because a skill is executable — and corrected the
/// **invariant floor's** skill rule, which had required the
/// `security-reviewer` role at one distinct approver, so under `standard`
/// and `open-collaboration` one person holding both roles published
/// executable code alone (ADR-0051 decisions 10 and 18). `@15`: AUTH-4
/// added the base layer's second forbid — a sealed scope is nobody's to
/// act on — and the `Scope` entity attribute it stands on, which is the
/// mirror of the quarantine rule these packs have carried since ADR-0013:
/// quarantine says this caller may do nothing, a seal says nothing may be
/// done to this material. Every pack's own policies are **byte-identical**
/// across this bump, which is what makes the golden diff checkable: the
/// only cells that move are the ones a seal turns off (ADR-0059
/// decisions 8 and 9). `@16`: AUTH-5 added `DirectorySealAuthorise`, the
/// human release of a pull sync's circuit breaker, as its **own** action
/// rather than widening `DirectoryManage` — one hands out a provisioning
/// token, the other authorises irreversible bulk sealing, and a tenant that
/// wants those two held by two people could not say so while they shared an
/// action (ADR-0060 decision 10). All three packs grant it to `org-admin`,
/// so the golden diff is exactly one new row per pack per scope kind and
/// nothing else moves: the separation's value is what a *stored* pack can
/// now express, not what these three say differently. `@17`: CPR-6 re-cut the
/// entity model over governed scopes (ADR-0073) and every pack moved with it,
/// in three ways. `principal.home` became `principal.own_scope` and
/// `resource.kind != "user"` became `resource.kind != "principal"` — the same
/// rules, in the shape vocabulary that replaced the rank one. Every role list
/// gained the grant keys beside the binding roles it already named, so a
/// workspace `owner` administers their workspace and a `member` contributes to
/// it without anybody minting a legacy binding. And `standard`'s sharing
/// default stopped reading `principal.department`, which is gone: it now
/// shares within `principal.anchors`, the scopes a grant actually reaches this
/// caller at, so the default follows what somebody was given rather than where
/// an org chart put them. `@18`: CPR-7 finished the cut (ADR-0074). The
/// binding vocabulary left every role list — `steward`, `org-admin`,
/// `auditor`, `contributor`, `security-reviewer` and `compliance` are
/// gone, and the six grant keys are the whole of `context.roles` — and
/// the `Hierarchy*` actions became `ScopeCreate`/`ScopeRead`/
/// `ScopeUpdate` (no delete: retiring a scope is a status transition).
/// The base layer lost the role-binding escalation guard with the action
/// it guarded. No permit changed who it lets in beyond that rename: the
/// keys these packs already named since `@17` are the keys that remain.
/// Four rules did move, and each is a hole the re-vocabulary opened rather
/// than a widening anybody wanted: the approval matrix's SHARED cell got
/// the **tenant root** back (it fell out of both cells and made the widest
/// publication in the tenant the cheapest one); `DirectoryManage` and
/// `DirectorySealAuthorise` got `administrator` back (the old `org-admin`
/// was mapped to `owner` alone, which locked every directory operator
/// out); `ProposalOpen`'s membership floor climbs by `principal.ambit`
/// (ADR-0074 decision 8 — anchors are not entity parents, so
/// `principal in resource` no longer reached the scope above); and the
/// quarantine review plane decides at the tenant (decision 7).
pub const EMBEDDED_PACKS: [(&str, i64); 3] = [
    (REGULATED_STRICT, 18),
    (STANDARD, 18),
    (OPEN_COLLABORATION, 18),
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

/// A compiled, schema-validated policy pack (base layer included), with
/// its non-Cedar configuration: the redaction modes the observe scan
/// seam applies under this pack (MEM-2, ADR-0021 decision 3), and the
/// composition budget/channel rule the read path composes under (CTX-2,
/// ADR-0025 decisions 2–3).
struct LoadedPack {
    name: String,
    version: i64,
    policies: PolicySet,
    redaction: RedactionConfig,
    composition: CompositionConfig,
    /// Behind an `Arc` because [`Pdp::effective`] clones the whole
    /// [`EffectivePack`] per candidate scope on the inject path, and a
    /// matrix is the one config that is not `Copy` (FLOW-3, ADR-0032).
    approvals: Arc<ApprovalMatrix>,
    /// The promotion rules (FLOW-4, ADR-0033 decision 6), behind an `Arc`
    /// for the same reason as the matrix. Empty means nothing
    /// auto-promotes at scopes this pack governs.
    promotion: Arc<PromotionConfig>,
    /// The lapse ceiling (AUTHZ-4, ADR-0037 decision 5): the longest window
    /// a lapse at scopes this pack governs may run for, and — at zero —
    /// whether any may stand at all. `Copy`, so no `Arc`.
    lapse: LapseConfig,
    /// What the ingestion pipeline does with a restatement or a
    /// contradiction at scopes this pack governs (MEM-5, ADR-0039
    /// decision 12). `Copy`, so no `Arc`.
    dedup: DedupConfig,
    /// How long material at scopes this pack governs is served, kept and
    /// destroyed, and how fast it decays in ranking (MEM-6, ADR-0040).
    /// `Copy`, so no `Arc`.
    retention: RetentionConfig,
    /// What happens to a mover's own memory when the directory moves them
    /// across a policy boundary (AUTH-4, ADR-0059 decision 10). `Copy`,
    /// so no `Arc`.
    mover: MoverConfig,
    /// The severity at which a skill bundle's security scan refuses
    /// rather than reports (SKIL-2, ADR-0052 decision 9). `Copy`, so no
    /// `Arc`.
    scan: SkillScanConfig,
    /// The bar a skill bundle's quality must clear to publish without an
    /// override (SKIL-3, ADR-0053 decision 9). `Copy`, so no `Arc`.
    quality: SkillQualityConfig,
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

/// What one scope permits this principal to read, as
/// [`Pdp::permitted_read_tiers`] decided it in one pack resolution.
///
/// The two tier sets are independent answers to independent questions, and
/// that independence is the point (PRMT-2, ADR-0050 decision 8): a scope
/// may distribute conventions and glossaries to readers who hold no
/// readable memory there at all, so `context_pack` being non-empty while
/// `memory` is empty is a supported state and the composition plan must
/// keep such a scope rather than skip it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermittedTiers {
    /// The tiers `MemoryRead` permits here, ascending. Empty means no
    /// memory composes from this scope.
    pub memory: Vec<Sensitivity>,
    /// The tiers `ContextPackRead` permits here, ascending. Empty means no
    /// pack chunk composes from this scope.
    pub context_pack: Vec<Sensitivity>,
    /// The tiers `SkillRead` permits here, ascending (SKIL-4, ADR-0054
    /// decision 10). Empty means this scope's published skills are neither
    /// advertised in a block nor listed as available.
    ///
    /// The same decision the resolve route takes per scope, from the same
    /// walk — which is what keeps "the set and the by-name resolve are the
    /// same walk" (decision 2) true rather than parallel.
    pub skill: Vec<Sensitivity>,
    /// The `MemoryRead` decision — what the plan records and the audit
    /// event carries. The pack identity is the same for every ask, one
    /// resource, one resolution.
    pub decision: AuthzDecision,
    /// The pack's own configuration, so a caller planning a scope needs no
    /// second resolution to read its channel rule.
    pub effective: EffectivePack,
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
    /// The pack's redaction configuration (MEM-2, ADR-0021 decision 3):
    /// what the observe scan seam does with findings under this pack.
    pub redaction: RedactionConfig,
    /// The pack's composition configuration (CTX-2, ADR-0025): the
    /// inject budget and channel rule at scopes this pack governs.
    pub composition: CompositionConfig,
    /// The pack's approval matrix (FLOW-3, ADR-0032): what it takes to
    /// publish an asset onto a channel at scopes this pack governs.
    /// Resolving it always merges the invariant floor, so this is what
    /// the pack adds *above* the product's non-negotiables.
    pub approvals: Arc<ApprovalMatrix>,
    /// The pack's promotion rules (FLOW-4, ADR-0033): what opens a
    /// proposal at scopes this pack governs without a human deciding to.
    /// Empty in every embedded pack — a trigger nobody configured must
    /// not fire.
    pub promotion: Arc<PromotionConfig>,
    /// The pack's lapse ceiling (AUTHZ-4, ADR-0037 decision 5): how long a
    /// lapse at scopes this pack governs may run, and whether one may
    /// stand at all. The grant surface reads it to bound a window; the PDP
    /// reads it on every `MemoryRead` to gate a standing one.
    pub lapse: LapseConfig,
    /// The pack's dedup configuration (MEM-5, ADR-0039 decision 12): what
    /// the extraction worker does when a candidate restates or contradicts
    /// a record its owner's scope already holds.
    pub dedup: DedupConfig,
    /// The pack's retention configuration (MEM-6, ADR-0040): the horizons
    /// scopes this pack governs serve and keep material under, and the
    /// staleness half-life composition ranks by. Read on the read path at
    /// every planned scope, and by the sweep at the scope a record lives
    /// at (ADR-0040 decision 10).
    pub retention: RetentionConfig,
    /// The pack's mover configuration (AUTH-4, ADR-0059 decision 10):
    /// whether a person's own memory follows them across a policy
    /// boundary or is sealed where it was written. Read by the SCIM
    /// reconciler at the scope the person is moving **away from** —
    /// authority over material belongs where the material is.
    pub mover: MoverConfig,
    /// The pack's skill-scan configuration (SKIL-2, ADR-0052 decision 9):
    /// the severity at which a bundle's security scan refuses rather than
    /// reports. Read at the authoring seam and again at publication —
    /// never to decide *whether* to scan, which is not a pack's to say,
    /// only where its threshold sits above the invariant floor.
    pub scan: SkillScanConfig,
    /// The pack's skill-quality configuration (SKIL-3, ADR-0053
    /// decision 9): the automated score a bundle must reach and whether a
    /// reviewer checklist is mandatory, both read at the publish seam to
    /// decide whether the publication needs an override.
    ///
    /// Its fail-safe is the **opposite** of `scan`'s and deliberately so:
    /// an unconfigured pack gates nothing here, because quality is not an
    /// invariant and a product that began refusing publications on a
    /// rubric nobody opted into would break every tenant on an upgrade.
    pub quality: SkillQualityConfig,
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

/// One entity store, materialised for a set of chains and reused across
/// many decisions (CTX-5, ADR-0042 decision 6).
///
/// Opaque on purpose: it holds no policy and takes no decision, so there
/// is nothing here a caller could get wrong except handing it to
/// [`Pdp::authorize_with`] for a chain it does not cover — which denies.
pub struct EntityBatch {
    entities: Entities,
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
    group_type: EntityTypeName,
    grant_type: EntityTypeName,
    workspace_type: EntityTypeName,
    project_type: EntityTypeName,
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
        // Embedded packs carry compiled-in redaction configs (ADR-0021
        // decision 3): strict quarantines secrets and redacts PII (the
        // seed §6 wording); the relaxed packs redact everything. All
        // three compose under the product composition config (ADR-0025
        // decision 2: derived is readable-per-policy by design; the
        // published-only restriction is an explicit choice, never a
        // default).
        // The lapse ceilings are tech plan §2.4's SMB collapse in the one
        // place a window is a number: `regulated-strict` grants seed §6's
        // own example (30 days) and the relaxed packs the product maximum
        // (90). No pack refuses lapses outright — that is a tenant's
        // decision to make in a stored pack, not a product default.
        // The retention configs are ADR-0040 decision 13's one product
        // default that can differ per pack without destroying anything a
        // tenant expected to keep: no pack sets a record horizon, and
        // `regulated-strict` disposes of the staging plane at a week
        // against the relaxed packs' month.
        // The skill-scan thresholds are the one axis where the strict
        // reading is affordable (ADR-0052 decision 3): `regulated-strict`
        // refuses a bundle that shells out or writes outside itself,
        // where the relaxed packs report it and let the two approvers the
        // floor already requires decide. Every pack refuses the critical
        // band, which is not theirs to move.
        let sources = [
            (
                REGULATED_STRICT,
                REGULATED_STRICT_SRC,
                RedactionConfig::STRICT,
                LapseConfig::STRICT,
                RetentionConfig::STRICT,
                MoverConfig::STRICT,
                SkillScanConfig::STRICT,
                SkillQualityConfig::STRICT,
            ),
            (
                STANDARD,
                STANDARD_SRC,
                RedactionConfig::REDACT_ALL,
                LapseConfig::RELAXED,
                RetentionConfig::DEFAULT,
                MoverConfig::FOLLOWS,
                SkillScanConfig::FLOOR,
                SkillQualityConfig::MODERATE,
            ),
            (
                OPEN_COLLABORATION,
                OPEN_COLLABORATION_SRC,
                RedactionConfig::REDACT_ALL,
                LapseConfig::RELAXED,
                RetentionConfig::DEFAULT,
                MoverConfig::FOLLOWS,
                SkillScanConfig::FLOOR,
                SkillQualityConfig::OPEN,
            ),
        ];
        let mut embedded = HashMap::new();
        for ((name, version), (_, source, redaction, lapse, retention, mover, scan, quality)) in
            EMBEDDED_PACKS.iter().zip(sources)
        {
            let pack = compile(
                &schema,
                name,
                *version,
                source,
                PackConfig {
                    redaction: Some(redaction),
                    composition: Some(CompositionConfig::DEFAULT),
                    approvals: Some(crate::approvals::embedded(name)),
                    // No embedded pack auto-promotes: ADR-0033 decision
                    // 6's fail-safe is silence, and a product default
                    // that opened proposals nobody asked for would be a
                    // surprise arriving through an upgrade.
                    promotion: None,
                    lapse: Some(lapse),
                    // All three dedup identically, and the product
                    // default supersedes: seed §4.4 already resolves
                    // conflicts by "newer valid-time beats older", so a
                    // pack that let contradictions accumulate would be
                    // the one making a claim (ADR-0039 decision 12).
                    dedup: Some(DedupConfig::DEFAULT),
                    // No embedded pack names a record TTL: an upgrade
                    // that silently deletes a tenant's memory is the one
                    // surprise this product must never spring (ADR-0040
                    // decision 13). What differs is the staging plane,
                    // whose disposal ADR-0020/0021 already promised.
                    retention: Some(retention),
                    // `regulated-strict` seals a personal scope that
                    // crosses a policy boundary; the relaxed packs let it
                    // follow. Safe under those two for a reason they
                    // state themselves — neither sets a record horizon,
                    // so there is no schedule for the material to be
                    // handed to (ADR-0059 decision 10).
                    mover: Some(mover),
                    scan: Some(scan),
                    quality: Some(quality),
                },
            )
            .map_err(|err| Error::Internal {
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
            group_type: type_name("Synveda::Group")?,
            grant_type: type_name("Synveda::ScopeGrant")?,
            workspace_type: type_name("Synveda::Workspace")?,
            project_type: type_name("Synveda::Project")?,
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
        compile(
            &self.schema,
            name,
            0,
            source,
            PackConfig {
                redaction: Some(RedactionConfig::STRICT),
                composition: Some(CompositionConfig::DEFAULT),
                approvals: Some(ApprovalMatrix::empty()),
                promotion: None,
                lapse: None,
                dedup: None,
                retention: None,
                mover: None,
                scan: None,
                quality: None,
            },
        )
        .map(|_| ())
    }

    /// Compiles and installs a tenant's stored pack under its name,
    /// replacing any previous version atomically. On error the previous
    /// pack stays in force (ADR-0012 decision 5: a bad apply must not
    /// widen or brick a tenant). Reserved product names are refused —
    /// the in-process face of the store's check constraint.
    /// An unconfigured field of `config` falls back to that config's own
    /// fail-safe: strict redaction (ADR-0021 decision 3), the product
    /// composition config which only ever narrows (ADR-0025 decision 2),
    /// and the empty approval matrix — which still resolves to the
    /// invariant floor, never to "no review needed" (ADR-0032
    /// decision 4).
    pub fn install_source(
        &self,
        tenant_id: TenantId,
        name: &str,
        version: i64,
        source: &str,
        config: PackConfig,
    ) -> Result<()> {
        if is_reserved(name) {
            return Err(Error::Invalid {
                message: format!("pack name {name:?} is reserved for the product (ADR-0014)"),
            });
        }
        let pack = compile(&self.schema, name, version, source, config)?;
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
        self.decide(None, principal, action, resource, context)
    }

    /// Materialises one entity store covering every chain a sweep will
    /// decide, for reuse across many [`Self::authorize_with`] calls
    /// (CTX-5, ADR-0042 decision 6).
    ///
    /// The measurement this exists for: at 516 candidate scopes, four
    /// tiers each, re-materialising per call put the plan stage at 378ms
    /// against ADR-0029's 15ms allowance — the cost is
    /// `Entities::from_entities`, not Cedar evaluation. Building it once
    /// is what makes a universe wider than the chain affordable at all.
    ///
    /// It changes no verdict, which is the property that makes it a
    /// performance mechanism rather than a policy one: see
    /// [`Self::entities_over`].
    pub fn materialise(
        &self,
        principal: &Principal,
        chains: &[&[ScopeNode]],
        context: &AuthzContext<'_>,
    ) -> Result<EntityBatch> {
        Ok(EntityBatch {
            entities: self.entities_over(principal, chains, context)?,
        })
    }

    /// [`Self::authorize`] against a pre-materialised entity store.
    ///
    /// Everything else about the decision is unchanged and still
    /// per-call: the effective pack still resolves from *this* resource's
    /// chain and assignments, the roles from the bindings that reach it,
    /// the lapses from the grants that bear on it. Only the entity store
    /// is shared, and a caller that hands over a batch missing a chain
    /// gets a *denial*, never a wrong allow — an absent scope entity fails
    /// the membership tests every permit is built on.
    pub fn authorize_with(
        &self,
        batch: &EntityBatch,
        principal: &Principal,
        action: Action,
        resource: Resource,
        context: &AuthzContext<'_>,
    ) -> Result<AuthzDecision> {
        self.decide(Some(batch), principal, action, resource, context)
    }

    /// What one scope permits this principal to *read*, as one call
    /// (CTX-5, ADR-0042 decision 6; PRMT-2, ADR-0050 decision 8).
    ///
    /// The asks at a scope differ in exactly two context attributes — the
    /// action and the tier — and everything else about them is identical:
    /// the same effective pack, the same effective roles, the same entity
    /// store. Asking them separately re-resolved the pack and re-derived
    /// the roles per ask, which at a widened universe's scale is most of the
    /// plan stage. This resolves once and evaluates eight times.
    ///
    /// It is the *same* decision either way — `authorize_with` in a loop
    /// produces identical verdicts, which the AUTHZ-5 golden matrix keeps
    /// honest — so this is a shape, not a semantics.
    ///
    /// All three read actions are decided here rather than one, because
    /// ADR-0050 decision 8 puts `ContextPackRead` *inside* the composition
    /// plan walk and ADR-0054 decision 10 puts `SkillRead` there beside it:
    /// a scope may distribute conventions, or capabilities, to readers
    /// holding no readable memory there, so the answers are genuinely
    /// independent and a second walk to get one of them would be a second
    /// authorization path.
    pub fn permitted_read_tiers(
        &self,
        batch: &EntityBatch,
        principal: &Principal,
        scope_id: ScopeId,
        context: &AuthzContext<'_>,
    ) -> Result<PermittedTiers> {
        let resource = Resource::Scope(scope_id);
        let (pack, origin) = self.resolve_pack(principal.tenant_id, resource, context, false);
        let roles = effective_roles(resource, context);
        let mut memory = Vec::with_capacity(Sensitivity::ALL.len());
        let mut context_pack = Vec::with_capacity(Sensitivity::ALL.len());
        let mut skill = Vec::with_capacity(Sensitivity::ALL.len());
        let mut last: Option<AuthzDecision> = None;
        for (action, tiers) in [
            (Action::MemoryRead, &mut memory),
            (Action::ContextPackRead, &mut context_pack),
            (Action::SkillRead, &mut skill),
        ] {
            for tier in Sensitivity::ALL {
                let scoped = AuthzContext {
                    sensitivity: Some(tier),
                    ..*context
                };
                let decision = self.evaluate(
                    &pack,
                    origin,
                    &batch.entities,
                    principal,
                    action,
                    resource,
                    &roles,
                    &scoped,
                )?;
                if decision.allowed {
                    tiers.push(tier);
                }
                // The `MemoryRead` decision is the one the plan records and
                // the audit event carries — it is the question "may this
                // reader compose here" has always meant, and widening it to
                // whichever ask happened to run last would change what a
                // stored decision means.
                if action == Action::MemoryRead {
                    last = Some(decision);
                }
            }
        }
        let decision = last.ok_or_else(|| Error::Internal {
            message: "the sensitivity vocabulary is empty".to_owned(),
        })?;
        Ok(PermittedTiers {
            memory,
            context_pack,
            skill,
            decision,
            effective: self.effective_from(&pack, origin),
        })
    }

    fn decide(
        &self,
        batch: Option<&EntityBatch>,
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
        let roles = effective_roles(resource, context);
        let owned;
        let entities = match batch {
            Some(batch) => &batch.entities,
            None => {
                owned = self.entities_over(principal, &[context.scopes], context)?;
                &owned
            }
        };
        self.evaluate(
            &pack, origin, entities, principal, action, resource, &roles, context,
        )
    }

    /// The evaluation half of a decision, once the pack, roles and
    /// entities are in hand — shared by [`Self::decide`] and
    /// [`Self::permitted_read_tiers`] so a batched tier sweep and a single
    /// call cannot drift apart.
    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        &self,
        pack: &LoadedPack,
        origin: PackOrigin,
        entities: &Entities,
        principal: &Principal,
        action: Action,
        resource: Resource,
        roles: &[&'static str],
        context: &AuthzContext<'_>,
    ) -> Result<AuthzDecision> {
        // The standing grants that bear on *this* decision, gated by the
        // pack in force at the resource: a pack whose ceiling is zero
        // admits none of them on the very next request, standing or not
        // (ADR-0037 decision 5). Empty for every action but `MemoryRead`,
        // which is the only one the vocabulary lets a lapse relax.
        let lapsed: Vec<&Lapse> = if pack.lapse.admits_lapses() {
            lapsing(action, resource, *context).collect()
        } else {
            Vec::new()
        };
        let request = self.request(
            principal,
            action,
            resource,
            roles,
            RequestContext {
                lapsed: !lapsed.is_empty(),
                sensitivity: context.sensitivity,
            },
        )?;
        let response = self
            .authorizer
            .is_authorized(&request, &pack.policies, entities);
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
            // Which grant let this through, by id — the decision log's half
            // of "why was this in that block" (ADR-0037 decision 12). Empty
            // on every decision no lapse bore on, which is almost all of
            // them.
            authz.lapses = %lapsed
                .iter()
                .map(|lapse| lapse.id.to_string())
                .collect::<Vec<_>>()
                .join(","),
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
        self.effective_from(&pack, origin)
    }

    /// [`Self::effective`] from an already-resolved pack, so a caller that
    /// just decided a scope reads its configuration without walking the
    /// chain a second time (CTX-5, ADR-0042 decision 6).
    fn effective_from(&self, pack: &LoadedPack, origin: PackOrigin) -> EffectivePack {
        EffectivePack {
            name: pack.name.clone(),
            version: pack.version,
            origin,
            redaction: pack.redaction,
            composition: pack.composition,
            approvals: Arc::clone(&pack.approvals),
            promotion: Arc::clone(&pack.promotion),
            lapse: pack.lapse,
            dedup: pack.dedup,
            retention: pack.retention,
            mover: pack.mover,
            scan: pack.scan,
            quality: pack.quality,
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
        let nodes: HashMap<ScopeId, &ScopeNode> =
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
        // Every resource with a scope resolves its pack from that scope's
        // chain — a workspace is governed by the profile assigned to the scope
        // it owns, exactly as the scope itself is (CPR-6, ADR-0073 decision 3).
        if let Some(id) = resource.anchor_scope(context) {
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

    /// Materialises the Cedar entity graph for one or more decisions
    /// (ADR-0012 decision 4, ADR-0014 decision 5, ADR-0017; CPR-6, ADR-0073).
    ///
    /// Seven entity types, and which of them appear is decided by what the
    /// caller supplied rather than by the action:
    ///
    /// - `Tenant`, one per distinct tenant in play, so a chain from a foreign
    ///   tenant chains up to a *different* one and every membership rule fails
    ///   closed.
    /// - `Scope`, one per node of every supplied chain, parented along
    ///   `parent_id` with the root parented to its tenant. Served from the
    ///   entity store's prebuilt fragments when the chain's shape matches.
    /// - `Group`, one per group this principal is in.
    /// - `Workspace`, `Project` and `ScopeGrant`, one per
    ///   [`ResourceEntity`] the caller named, each parented to the scope it
    ///   belongs to — which is what makes every containment rule written over
    ///   scopes reach them without being restated.
    /// - `Principal`, built per request so quarantine, the caller's own scope,
    ///   their anchors and their group memberships all ride the per-request
    ///   read (ADR-0016 decision 6).
    ///
    /// A superset store cannot change a verdict: Cedar resolves only the
    /// entities a request actually names, and every scope carries the same
    /// parents whether or not its neighbours are present. So one store built
    /// over every chain a sweep will decide answers exactly as N stores built
    /// one per chain would — for the price of one `Entities::from_entities`,
    /// which is where the per-decision cost almost entirely lives (CTX-5,
    /// ADR-0042 decision 6).
    fn entities_over(
        &self,
        principal: &Principal,
        chains: &[&[ScopeNode]],
        context: &AuthzContext<'_>,
    ) -> Result<Entities> {
        use cedar_policy::RestrictedExpression;

        // Chains may share nodes (a project and its workspace, or two
        // candidates under one org unit); Cedar rejects duplicate entity
        // entries, so deduplicate by uid.
        let mut merged: HashMap<EntityUid, Entity> = HashMap::new();
        for chain in chains.iter().copied().chain([context.principal_scopes]) {
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
        // supplied at all (a tenant resource, a principal with no scope).
        let principal_tenant = self.tenant_uid(principal.tenant_id)?;
        merged
            .entry(principal_tenant.clone())
            .or_insert_with(|| Entity::with_uid(principal_tenant.clone()));

        // The groups this caller is in. Parents of the principal, so
        // `principal in Synveda::Group::"…"` is an entity-hierarchy check.
        let mut group_uids: Vec<EntityUid> = Vec::with_capacity(context.groups.len());
        for group_id in context.groups {
            let uid = self.group_uid(*group_id)?;
            merged.entry(uid.clone()).or_insert_with(|| {
                new_entity(
                    uid.clone(),
                    HashMap::from([(
                        "tenant".to_owned(),
                        RestrictedExpression::new_entity_uid(principal_tenant.clone()),
                    )]),
                    HashSet::from([principal_tenant.clone()]),
                )
                .unwrap_or_else(|_| Entity::with_uid(uid.clone()))
            });
            group_uids.push(uid);
        }

        // The subtype and access-plane entities the decision names.
        for resource in context.resources {
            let (uid, entity) = self.resource_entity(principal_tenant.clone(), *resource)?;
            merged.entry(uid).or_insert(entity);
        }

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
        let mut principal_parents: HashSet<EntityUid> = HashSet::from([principal_tenant]);
        principal_parents.extend(group_uids);
        if let Some(own) = principal.scope_id {
            // The caller's own scope makes them a member of its chain
            // (`principal in resource`), and the attribute lets packs require
            // one outright (ADR-0014 decision 5).
            let own_uid = self.scope_uid(own)?;
            principal_attrs.insert(
                "own_scope".to_owned(),
                RestrictedExpression::new_entity_uid(own_uid.clone()),
            );
            principal_parents.insert(own_uid);
        }
        if let Some(token_scope) = principal.token_scope {
            // Service identities only (AUTH-3, ADR-0018 decision 4): the
            // base layer confines every decision to this scope's subtree.
            principal_attrs.insert(
                "token_scope".to_owned(),
                RestrictedExpression::new_entity_uid(self.scope_uid(token_scope)?),
            );
        }

        // The two sets the anchor model adds (ADR-0073 decision 4).
        //
        // `anchors` is every scope this caller actually **holds** something at
        // — the downward direction, so `resource in principal.anchors` is
        // "inside something I hold" and a workspace grant reaches that
        // workspace's projects with no row written there. It deliberately
        // excludes anchors the caller holds nothing at: the tenant root is
        // applicable to every request, and putting it here unheld would make
        // that expression true for the whole tenant.
        //
        // Anchors are **not** made parents of the principal. Authority flows
        // *down* from where a grant was written and never up: a project grant
        // must not make its holder a member of the workspace above it, which
        // is exactly what an entity parent would do.
        //
        // `ambit` is the **parent** of every held anchor, minus the tenant
        // root: the neighbourhood a grant puts somebody in. It replaces the
        // `department` attribute `standard`'s sharing default used to read
        // (ADR-0073 decision 4) and reproduces that rule's *shape* — a grant
        // at one team shares its neighbours — without asking what kind of
        // thing the parent is. Excluding the root is what keeps `standard`
        // from meaning `open-collaboration`.
        let roots: HashSet<ScopeId> = context
            .anchors
            .iter()
            .filter(|anchor| anchor.kind == ScopeKind::Tenant)
            .map(|anchor| anchor.scope_id)
            .collect();
        let mut anchor_uids = Vec::new();
        let mut ambit_uids = Vec::new();
        let mut private_uids = Vec::new();
        for anchor in context.anchors {
            if anchor.is_held() {
                anchor_uids.push(RestrictedExpression::new_entity_uid(
                    self.scope_uid(anchor.scope_id)?,
                ));
                if let Some(parent) = anchor.parent_scope_id
                    && !roots.contains(&parent)
                    && anchor.kind != ScopeKind::Principal
                {
                    ambit_uids.push(RestrictedExpression::new_entity_uid(
                        self.scope_uid(parent)?,
                    ));
                }
            }
            // `private` is every `principal`-shaped scope this caller may
            // reach: their own, and any somebody granted them directly. The
            // base layer forbids every other principal scope outright, so this
            // set is the whole of what makes a private scope reachable at all.
            if anchor.is_private()
                && (anchor.is_direct() || Some(anchor.scope_id) == principal.scope_id)
            {
                private_uids.push(RestrictedExpression::new_entity_uid(
                    self.scope_uid(anchor.scope_id)?,
                ));
            }
        }
        if let Some(own) = principal.scope_id
            && context.anchors.iter().all(|anchor| anchor.scope_id != own)
        {
            // A caller's own scope is reachable by them whether or not the
            // resolver produced an anchor for it — an anchor set gathered for
            // a different selection must not be able to lock somebody out of
            // their own notes.
            private_uids.push(RestrictedExpression::new_entity_uid(self.scope_uid(own)?));
        }
        principal_attrs.insert(
            "anchors".to_owned(),
            RestrictedExpression::new_set(anchor_uids),
        );
        principal_attrs.insert(
            "ambit".to_owned(),
            RestrictedExpression::new_set(ambit_uids),
        );
        principal_attrs.insert(
            "private".to_owned(),
            RestrictedExpression::new_set(private_uids),
        );

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

    /// Builds one [`ResourceEntity`] — the subtype or access-plane object a
    /// decision names, parented to the scope it belongs to.
    fn resource_entity(
        &self,
        tenant: EntityUid,
        resource: ResourceEntity,
    ) -> Result<(EntityUid, Entity)> {
        use cedar_policy::RestrictedExpression;

        let tenant_attr = |attrs: &mut HashMap<String, RestrictedExpression>| {
            attrs.insert(
                "tenant".to_owned(),
                RestrictedExpression::new_entity_uid(tenant.clone()),
            );
        };
        match resource {
            ResourceEntity::Workspace { id, scope_id } => {
                let uid = self.uid(&self.workspace_type, &id.to_string())?;
                let scope = self.scope_uid(scope_id)?;
                let mut attrs = HashMap::new();
                tenant_attr(&mut attrs);
                attrs.insert(
                    "scope".to_owned(),
                    RestrictedExpression::new_entity_uid(scope.clone()),
                );
                let entity = new_entity(uid.clone(), attrs, HashSet::from([scope]))?;
                Ok((uid, entity))
            }
            ResourceEntity::Project {
                id,
                scope_id,
                workspace_id,
            } => {
                let uid = self.uid(&self.project_type, &id.to_string())?;
                let scope = self.scope_uid(scope_id)?;
                let workspace = self.uid(&self.workspace_type, &workspace_id.to_string())?;
                let mut attrs = HashMap::new();
                tenant_attr(&mut attrs);
                attrs.insert(
                    "scope".to_owned(),
                    RestrictedExpression::new_entity_uid(scope.clone()),
                );
                attrs.insert(
                    "workspace".to_owned(),
                    RestrictedExpression::new_entity_uid(workspace),
                );
                // Parented to its own scope only: the workspace above it is
                // already that scope's ancestor, so a second parent would be
                // the same edge said twice.
                let entity = new_entity(uid.clone(), attrs, HashSet::from([scope]))?;
                Ok((uid, entity))
            }
            ResourceEntity::Group { id } => {
                let uid = self.group_uid(id)?;
                let mut attrs = HashMap::new();
                tenant_attr(&mut attrs);
                let entity = new_entity(uid.clone(), attrs, HashSet::from([tenant]))?;
                Ok((uid, entity))
            }
            ResourceEntity::Grant {
                id,
                scope_id,
                role,
                source,
            } => {
                let uid = self.uid(&self.grant_type, &id.to_string())?;
                let scope = self.scope_uid(scope_id)?;
                let mut attrs = HashMap::new();
                tenant_attr(&mut attrs);
                attrs.insert(
                    "scope".to_owned(),
                    RestrictedExpression::new_entity_uid(scope.clone()),
                );
                attrs.insert(
                    "role".to_owned(),
                    RestrictedExpression::new_string(role.as_str().to_owned()),
                );
                attrs.insert(
                    "source".to_owned(),
                    RestrictedExpression::new_string(source.as_str().to_owned()),
                );
                let entity = new_entity(uid.clone(), attrs, HashSet::from([scope]))?;
                Ok((uid, entity))
            }
        }
    }

    /// Builds one chain's fragment entities (ADR-0017 decision 4): a
    /// `Scope` entity per node — parented along `parent_id`, the root to
    /// its tenant entity — plus the chain's distinct `Tenant` entities.
    /// Every distinct tenant in play gets its entity so a chain from a
    /// foreign tenant chains up to a *different* tenant entity and
    /// membership rules fail closed.
    fn chain_entities(&self, chain: &[ScopeNode]) -> Result<Vec<Entity>> {
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
                        // A **shape**, never a rank (ADR-0070): `tenant`,
                        // `org_unit`, `workspace`, `project` or `principal`.
                        // No pack compares two of these for order, and there
                        // is nothing here to compare.
                        "kind".to_owned(),
                        RestrictedExpression::new_string(node.kind.as_str().to_owned()),
                    ),
                    (
                        // AUTH-4, ADR-0059 decision 9. Carried on the node
                        // rather than supplied beside it, so the fragment
                        // shape covers it and no cached fragment can serve an
                        // unsealed answer for a sealed scope.
                        "sealed".to_owned(),
                        RestrictedExpression::new_bool(node.sealed),
                    ),
                ]),
                HashSet::from([parent]),
            )?);
        }
        Ok(list)
    }

    /// Builds the schema-checked request, `context.roles` included
    /// (ADR-0015 decision 3).
    fn request(
        &self,
        principal: &Principal,
        action: Action,
        resource: Resource,
        roles: &[&'static str],
        extras: RequestContext,
    ) -> Result<Request> {
        let RequestContext {
            lapsed,
            sensitivity,
        } = extras;
        use cedar_policy::RestrictedExpression;

        let resource_uid = match resource {
            Resource::Tenant(id) => self.tenant_uid(id)?,
            Resource::Scope(id) => self.scope_uid(id)?,
            Resource::Workspace(id) => self.uid(&self.workspace_type, &id.to_string())?,
            Resource::Project(id) => self.uid(&self.project_type, &id.to_string())?,
            Resource::Group(id) => self.group_uid(id)?,
            Resource::Grant(id) => self.uid(&self.grant_type, &id.to_string())?,
        };
        let mut pairs = vec![(
            "roles".to_owned(),
            RestrictedExpression::new_set(
                roles
                    .iter()
                    .map(|role| RestrictedExpression::new_string((*role).to_owned())),
            ),
        )];
        if action == Action::MemoryRead {
            // Both required by the schema, and for the reason a required
            // attribute always is: a missing attribute makes the base
            // layer's lapse permit — and, since AUTHZ-5, its `restricted`
            // forbid — error, and Cedar drops a policy that errors
            // (ADR-0015 decision 5's shape, ADR-0037 decision 9,
            // ADR-0038 decision 2).
            pairs.push(("lapsed".to_owned(), RestrictedExpression::new_bool(lapsed)));
        }
        if matches!(
            action,
            Action::MemoryRead | Action::PromptRead | Action::ContextPackRead | Action::SkillRead
        ) {
            // The tier every read seam names (ADR-0038 decision 2; ADR-0049
            // decision 4; ADR-0050 decision 7; ADR-0051 decision 10). No
            // authored-asset read takes `lapsed`: a lapse relaxes a closed
            // vocabulary, and `memory.read` is all of it.
            let sensitivity = sensitivity.ok_or_else(|| Error::Internal {
                message: format!("{action} decided without a sensitivity tier in context"),
            })?;
            pairs.push((
                "sensitivity".to_owned(),
                RestrictedExpression::new_string(sensitivity.as_str().to_owned()),
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

    fn group_uid(&self, id: synveda_types::GroupId) -> Result<EntityUid> {
        self.uid(&self.group_type, &id.to_string())
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
/// The per-action context attributes, resolved by the PDP rather than
/// supplied: whether a standing lapse covers a `MemoryRead` (ADR-0037
/// decision 9), and which tier that read is asking about (ADR-0038
/// decision 2).
///
/// A struct rather than two parameters because they arrive together and
/// are set together — and because the schema requires each of them for its
/// action, so a caller that forgets one gets a build error rather than a
/// dropped policy.
#[derive(Debug, Clone, Copy)]
struct RequestContext {
    lapsed: bool,
    sensitivity: Option<Sensitivity>,
}

/// The principal's effective **role keys** at the resource (CPR-6, ADR-0073
/// decision 5): the grants, direct and group-derived, that actually reach it.
///
/// An anchor's roles apply when the anchor's scope is on the resource's own
/// chain — which is "at or above the resource", the inheritance rule
/// [`crate::request::AuthzContext::anchors`] was resolved under. Two things
/// narrow it:
///
/// - **Principal privacy.** When the resource is somebody's own scope, only an
///   anchor *at* that scope applies ([`inherits_into`]). A tenant-root grant is
///   the widest thing the model can express and it still does not reach into
///   one person's notes.
/// - **The tenant plane.** [`Resource::Tenant`] has no chain, so only the
///   anchor at the `tenant`-shaped scope applies — a grant written at a
///   workspace confers nothing over the whole boundary.
///
/// A group is deliberately in neither list: [`Resource::Group`] is tenant-wide
/// and takes tenant-root authority, which is the same shape `DirectoryManage`
/// has and for the same reason.
///
/// Sorted and deduplicated, for [`effective_roles_at`]'s reason.
#[must_use]
pub fn effective_role_keys_at(resource: Resource, context: &AuthzContext<'_>) -> Vec<RoleKey> {
    let mut keys: Vec<RoleKey> = Vec::new();
    match resource {
        Resource::Tenant(_) | Resource::Group(_) => {
            // The root is identified by its **shape**, not by why the resolver
            // put it in the set: a scope that is both the tenant root and a
            // scope this caller was granted merges under the more specific
            // source, and reading `source` here would then miss exactly the
            // tenant-wide grant this branch exists to find.
            for anchor in context.anchors {
                if anchor.kind == ScopeKind::Tenant {
                    keys.extend(anchor.roles.iter().copied());
                }
            }
        }
        _ => {
            let Some(head) = context.scopes.first() else {
                return keys;
            };
            let target = head.id;
            let inherits = inherits_into(head.kind);
            let chain: HashSet<ScopeId> = context.scopes.iter().map(|node| node.id).collect();
            for anchor in context.anchors {
                if !chain.contains(&anchor.scope_id) {
                    continue;
                }
                if !inherits && anchor.scope_id != target {
                    continue;
                }
                keys.extend(anchor.roles.iter().copied());
            }
        }
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// `context.roles`: the grant keys in force at the resource, as one set of
/// strings.
///
/// These are the six role keys of the access plane and nothing else — a
/// Cedar `Set<String>` is what a pack reads, and since the cutover there is
/// one vocabulary and one tree (CPR-7, ADR-0074 decision 1). The words the
/// old hierarchy's bindings used are gone with it.
fn effective_roles(resource: Resource, context: &AuthzContext<'_>) -> Vec<&'static str> {
    let mut roles: Vec<&'static str> = effective_role_keys_at(resource, context)
        .into_iter()
        .map(|key| key.as_str())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    roles
}

/// The lapse vocabulary's view of one PDP action: `Some` when a standing
/// grant could relax it, `None` for everything else.
///
/// The mapping rather than a name comparison, so the closed vocabulary
/// (`synveda_types::LapseAction`) and this one grow together or not at all
/// — a lapse naming an action outside it is refused at the grant surface,
/// and one that somehow reached a row still relaxes nothing here.
#[must_use]
pub const fn lapsable(action: Action) -> Option<LapseAction> {
    match action {
        Action::MemoryRead => Some(LapseAction::MemoryRead),
        _ => None,
    }
}

/// Whether `lapse` bears on one decision: it grants the action being
/// decided at exactly the resource scope, and the principal is placed at or
/// under its grantee scope — `principal in grantee`, read off the placement
/// chain the PDP already has (ADR-0037 decision 9).
///
/// The one containment rule, shared by the decision and by the read path's
/// plan, because the failure it prevents is asymmetric: a plan that offers
/// a scope the permit refuses costs a wasted decision, and a permit that
/// allows a scope the plan never offers is a grant nobody can see.
fn covers(
    lapse: &Lapse,
    principal_scopes: &[ScopeNode],
    action: LapseAction,
    resource: ScopeId,
) -> bool {
    lapse.grants(action, resource)
        && principal_scopes
            .iter()
            .any(|node| node.id == lapse.grantee_scope_id)
}

/// The grants bearing on one decision — [`covers`] over the context's rows,
/// bounded by the tier each grant declared (AUTHZ-5, ADR-0038 decision 6),
/// and empty for every action no lapse may relax.
///
/// A decision with no tier in context relaxes nothing, which cannot happen
/// for the one lapsable action — [`Pdp::request`] refuses to build a
/// `MemoryRead` request without one — and is the fail-closed reading if the
/// vocabulary ever grows an action whose seam has no tier.
fn lapsing<'a>(
    action: Action,
    resource: Resource,
    context: AuthzContext<'a>,
) -> impl Iterator<Item = &'a Lapse> {
    let wanted = lapsable(action);
    // A lapse names a scope. Nothing else it could name is one: a workspace or
    // a project is decided at its own scope and would be reached through that,
    // and a lapse over a group or a grant is not in the closed vocabulary
    // (ADR-0037 decision 2).
    let target = match resource {
        Resource::Scope(id) => Some(id),
        Resource::Tenant(_)
        | Resource::Workspace(_)
        | Resource::Project(_)
        | Resource::Group(_)
        | Resource::Grant(_) => None,
    };
    context.lapses.iter().filter(move |lapse| {
        let (Some(wanted), Some(target), Some(sensitivity)) = (wanted, target, context.sensitivity)
        else {
            return false;
        };
        covers(lapse, context.principal_scopes, wanted, target)
            && lapse.grants_at(wanted, target, sensitivity)
    })
}

/// The scopes a caller reaches only by lapse, with the grant that reached
/// each — what the read path adds to a composition plan *after* the chain
/// (ADR-0037 decision 10).
///
/// Public because the plan and the permit must agree about who a grant
/// reaches, and one implementation is how they cannot drift. It does **not**
/// apply the pack ceiling: that is a property of the target's own effective
/// pack, which the `MemoryRead` decision the plan then takes resolves
/// anyway — so this may offer a scope the PDP goes on to refuse, which is
/// the safe direction.
///
/// Ordered by grant time then id, so a plan built twice from the same rows
/// is the same plan — CTX-2's determinism AC reaches down here.
#[must_use]
pub fn lapsed_scopes<'a>(
    principal_scopes: &[ScopeNode],
    lapses: &'a [Lapse],
    action: LapseAction,
) -> Vec<&'a Lapse> {
    let mut seen: HashSet<ScopeId> = principal_scopes.iter().map(|node| node.id).collect();
    let mut granting: Vec<&Lapse> = lapses
        .iter()
        .filter(|lapse| covers(lapse, principal_scopes, action, lapse.target_scope_id))
        .collect();
    granting.sort_unstable_by_key(|lapse| (lapse.granted_at, lapse.id.as_uuid()));
    // A target already on the caller's own chain is not reached "only by
    // lapse", and two grants naming one target are one plan entry.
    granting.retain(|lapse| seen.insert(lapse.target_scope_id));
    granting
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

/// Parse + schema-validate on top of the invariant base layer (ADR-0014
/// decision 2): a pack that compiles can never fail at decision time for
/// structural reasons (ADR-0012 decision 2), and no pack can drop the
/// base rules.
fn compile(
    schema: &Schema,
    name: &str,
    version: i64,
    source: &str,
    config: PackConfig,
) -> Result<LoadedPack> {
    let redaction = config.redaction.unwrap_or_default();
    let composition = config.composition.unwrap_or_default();
    let approvals = config.approvals.unwrap_or_default();
    let promotion = config.promotion.unwrap_or_default();
    // Unconfigured falls back to the strict 30-day window rather than to
    // zero: a lapse ceiling narrows, and a missing narrowing must not become
    // a missing mechanism (ADR-0037 decision 5).
    let lapse = config.lapse.unwrap_or_default();
    // Unconfigured is the product config — supersession on — for the reason
    // the composition config's fallback is the product one: this config
    // never grants anything, so its default is the honest fallback rather
    // than a widening (ADR-0039 decision 12).
    let dedup = config.dedup.unwrap_or_default();
    // Unconfigured is the product config, whose record horizons are all
    // unset: a stored pack that says nothing about retention must not
    // start destroying a tenant's memory (ADR-0040 decision 13).
    let retention = config.retention.unwrap_or_default();
    // Unconfigured seals on a cross-pack move, which is `retention`'s
    // fail-safe rather than `quality`'s: nothing here refuses anything —
    // the move always happens — so the honest default is the one that
    // cannot hand material to a schedule nobody wrote it under (ADR-0059
    // decision 10, on ADR-0040 decision 13's argument).
    let mover = config.mover.unwrap_or_default();
    // Unconfigured is the invariant floor rather than the strict pack's
    // threshold: `critical` refuses either way, and a pack that says
    // nothing must not start refusing bundles nobody asked it to
    // (ADR-0052 decision 9). There is no `validate` for it — the type has
    // one field, every value of it is meaningful, and the one thing a
    // configuration must not be able to say is clamped on every read
    // rather than checked once here.
    let scan = config.scan.unwrap_or_default();
    // Same shape, opposite fail-safe: an unconfigured pack gets
    // `SkillQualityConfig::OPEN`, which gates nothing. ADR-0053 decision 9
    // — a pack that has said nothing about quality has not asked for a
    // quality gate, and there is no floor here to hold, because quality is
    // not an invariant.
    let quality = config.quality.unwrap_or_default();
    // A matrix asking more of one role than it asks of people is
    // unsatisfiable at every cell it governs, and it would fail silently
    // at review time rather than loudly at install time (ADR-0032).
    approvals.validate().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} approval matrix: {err}"),
    })?;
    // Same discipline for a trigger: a rule asking for zero recalls, or
    // one naming an asset with no usage signal, can only fire on
    // everything or on nothing (ADR-0033 decision 6).
    promotion.validate().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} promotion rules: {err}"),
    })?;
    // And for a ceiling: a pack asking for a longer window than the product
    // permits is refused when it is written, not clamped in silence at the
    // first grant that noticed.
    lapse.validate().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} lapse config: {err}"),
    })?;
    // And for the thresholds: a similarity outside `0..=1` would make a
    // band unreachable, which reads as "the feature is off" without ever
    // saying so (ADR-0039 decision 12).
    dedup.validate().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} dedup config: {err}"),
    })?;
    // And for a horizon: a schedule written in seconds, or a staging
    // horizon that would spend MEM-1's idempotency guarantee for nothing,
    // is refused when it is written rather than when it deletes something
    // (ADR-0040 decision 7).
    retention.validate().map_err(|err| Error::Invalid {
        message: format!("policy pack {name}@{version} retention config: {err}"),
    })?;
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
        redaction,
        composition,
        approvals: Arc::new(approvals),
        promotion: Arc::new(promotion),
        lapse,
        dedup,
        retention,
        mover,
        scan,
        quality,
    })
}
