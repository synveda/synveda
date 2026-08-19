//! Generic governed scopes (CPR-3, ADR-0068 decision 4, ADR-0070).
//!
//! A scope is a named node with a parent and a subtree. It is what assets
//! attach to, what the PDP decides about, and what a role binding covers —
//! which is the whole of what the old organisational hierarchy was ever
//! load-bearing for. What it is *not* is a rank: there is no strictly
//! increasing level vocabulary, no `division` above a `department`, and no
//! rule that a tenant's root must be an organisation.
//!
//! The kind that remains is a **shape**, not a rank. Five of them, and the
//! only thing a kind decides is which kinds may be its parent
//! ([`ScopeKind::permits_parent`]): an `org_unit` nests inside itself to
//! arbitrary depth, a `project` lives in a `workspace`, a `principal` hangs
//! off the tenant root, and the tenant root has no parent at all. A person
//! running this product alone has one `tenant` scope and one `principal`, and
//! is never asked to be a company.
//!
//! ## Why this module is not re-exported at the crate root
//!
//! [`ScopeKind`] here and `crate::ScopeKind` (the `org`/`division`/
//! `department`/`team`/`user` vocabulary of the old hierarchy) are different
//! types with the same name, and both exist for exactly as long as Prompt 6 of
//! the context-platform programme takes to delete the old hierarchy. Reach
//! these through `synveda_types::scope::…` until then. Importing this module's
//! path is a compile-time reminder of which model the calling code is written
//! against, which a root re-export and a rename would both hide.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, IdentityId, Result, ScopeId, TenantId};

/// Longest slug, in characters. Same bound and grammar as a tenant slug
/// (ADR-0008): URL-, hostname- and CLI-safe, so a scope path can be typed.
pub const MAX_SLUG_CHARS: usize = 63;

/// Longest display name, in characters. Bounded because it is stored, listed
/// and logged; generous because it is prose in somebody's language.
pub const MAX_DISPLAY_NAME_CHARS: usize = 200;

/// Largest `attributes` document, in bytes of its compact JSON encoding.
///
/// The bag is deliberately open — a generic scope has no fixed schema, and
/// callers label scopes with what their deployment means — but "open" is not
/// "unbounded": this is a governed row that is read on every chain walk.
pub const MAX_ATTRIBUTES_BYTES: usize = 16 * 1024;

/// The separator in a scope path (`acme/platform/payments`).
pub const PATH_SEPARATOR: char = '/';

/// What shape of thing a scope is.
///
/// Not a rank. The vocabulary is closed so that placement is decidable
/// ([`ScopeKind::permits_parent`]) and so that a UI can say what it is looking
/// at; it carries no ordering, and nothing in the product compares two kinds.
///
/// No `Default`: what a scope *is* is always an explicit decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    /// The tenant root — exactly one per tenant, and the only kind with no
    /// parent. It is a scope like any other: assets attach to it, the PDP
    /// decides about it, and a one-person deployment may never create another.
    Tenant,
    /// An organisational grouping. Nests inside itself to arbitrary depth, so
    /// a deployment with a division, a department and a team expresses all
    /// three without the product having names for them — and one with none
    /// creates none.
    OrgUnit,
    /// A collaboration space: where a group of people keep shared context.
    Workspace,
    /// A unit of work inside a workspace.
    Project,
    /// A person's or a service identity's own scope. Hangs off the tenant
    /// root, so an individual's deployment is a tenant scope and a principal.
    Principal,
}

impl ScopeKind {
    /// Every kind, in declaration order. For exhaustive tests and for the
    /// vocabulary a CHECK constraint mirrors.
    pub const ALL: &'static [ScopeKind] = &[
        ScopeKind::Tenant,
        ScopeKind::OrgUnit,
        ScopeKind::Workspace,
        ScopeKind::Project,
        ScopeKind::Principal,
    ];

    /// Stable wire name, identical to the serde form and to the value stored
    /// in `scopes.kind`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ScopeKind::Tenant => "tenant",
            ScopeKind::OrgUnit => "org_unit",
            ScopeKind::Workspace => "workspace",
            ScopeKind::Project => "project",
            ScopeKind::Principal => "principal",
        }
    }

    /// Whether a scope of this kind may sit directly under a `parent` of that
    /// kind. The whole of the structural rule, in one place.
    ///
    /// The tenant root permits nothing, which is the same statement as
    /// [`is_tenant_root`](Self::is_tenant_root): a scope with no permitted
    /// parent is a root, and a root is what has no parent.
    #[must_use]
    pub const fn permits_parent(&self, parent: ScopeKind) -> bool {
        matches!(
            (self, parent),
            (ScopeKind::OrgUnit, ScopeKind::Tenant | ScopeKind::OrgUnit)
                | (ScopeKind::Workspace, ScopeKind::Tenant | ScopeKind::OrgUnit)
                | (ScopeKind::Project, ScopeKind::Workspace)
                | (ScopeKind::Principal, ScopeKind::Tenant)
        )
    }

    /// The kinds a scope of this kind may sit under — the readable form of
    /// [`permits_parent`](Self::permits_parent), used to say *what would have
    /// been legal* when a placement is refused. A test asserts the two agree
    /// over the whole product of the vocabulary.
    #[must_use]
    pub const fn permitted_parents(&self) -> &'static [ScopeKind] {
        match self {
            ScopeKind::Tenant => &[],
            ScopeKind::OrgUnit | ScopeKind::Workspace => &[ScopeKind::Tenant, ScopeKind::OrgUnit],
            ScopeKind::Project => &[ScopeKind::Workspace],
            ScopeKind::Principal => &[ScopeKind::Tenant],
        }
    }

    /// Whether this kind is the tenant root — the one kind with no parent,
    /// and therefore the one kind that cannot move.
    #[must_use]
    pub const fn is_tenant_root(&self) -> bool {
        matches!(self, ScopeKind::Tenant)
    }
}

impl fmt::Display for ScopeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScopeKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "tenant" => Ok(ScopeKind::Tenant),
            "org_unit" => Ok(ScopeKind::OrgUnit),
            "workspace" => Ok(ScopeKind::Workspace),
            "project" => Ok(ScopeKind::Project),
            "principal" => Ok(ScopeKind::Principal),
            other => Err(Error::Invalid {
                message: format!("unknown scope kind: {other:?}"),
            }),
        }
    }
}

/// Whether a scope is in use.
///
/// No `Default`: a scope is created active by the one service that creates
/// scopes, and every other value is a deliberate transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeStatus {
    /// Ordinary. Assets attach, policy applies, the chain walks through it.
    Active,
    /// Retired. Kept because the history that references it is kept — a scope
    /// is what audit events, versions and bindings name, so removing one
    /// would orphan a record somebody has to be able to read.
    Archived,
}

impl ScopeStatus {
    /// Every status, in declaration order.
    pub const ALL: &'static [ScopeStatus] = &[ScopeStatus::Active, ScopeStatus::Archived];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ScopeStatus::Active => "active",
            ScopeStatus::Archived => "archived",
        }
    }
}

impl fmt::Display for ScopeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScopeStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "active" => Ok(ScopeStatus::Active),
            "archived" => Ok(ScopeStatus::Archived),
            other => Err(Error::Invalid {
                message: format!("unknown scope status: {other:?}"),
            }),
        }
    }
}

/// A node of a tenant's scope tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// The scope's identity — stable for its whole life, and what every asset,
    /// binding and audit event names.
    pub id: ScopeId,
    /// Owning tenant. Immutable: a scope never moves across tenants, which is
    /// a database fact rather than a convention (ADR-0070).
    pub tenant_id: TenantId,
    /// What shape of thing this is.
    pub kind: ScopeKind,
    /// Parent scope; `None` exactly for the tenant root.
    pub parent_scope_id: Option<ScopeId>,
    /// Human-stable handle, unique among siblings and immutable. Renaming
    /// changes [`display_name`](Self::display_name), never this — a slug is
    /// half of a path somebody may have written down.
    pub slug: String,
    /// Display name; renameable, and the only renameable field.
    pub display_name: String,
    /// Whether the scope is in use.
    pub status: ScopeStatus,
    /// The token subject this scope belongs to. `Some` exactly when
    /// [`kind`](Self::kind) is [`ScopeKind::Principal`], and immutable — a
    /// scope never changes whose it is (CPR-6, ADR-0073 decision 2).
    ///
    /// It is a column rather than a slug convention or an `attributes` entry
    /// because the anchor resolver reads it to answer "the authenticated
    /// caller's own scope", which makes it an authorisation input — and
    /// `attributes` is documented above as never being one.
    pub principal_id: Option<String>,
    /// Open labelling bag. A JSON object (never a scalar or an array), at most
    /// [`MAX_ATTRIBUTES_BYTES`] encoded. Deliberately unstructured: what a
    /// deployment means by a scope is the deployment's to say, and a fixed
    /// schema here would be the rank vocabulary returning under another name.
    ///
    /// Never an authorisation input: policy decides over the tree and the
    /// profile assigned to it, not over a label the caller wrote.
    pub attributes: serde_json::Value,
    /// The identity that created the scope, when one did. `None` means the
    /// deployment created it — a tenant root minted at admission has no
    /// author, and inventing a synthetic one would lose that distinction.
    pub created_by: Option<IdentityId>,
    /// When the scope was created.
    pub created_at: DateTime<Utc>,
    /// When the scope was last renamed or moved.
    pub updated_at: DateTime<Utc>,
}

/// Checks a slug against the grammar `^[a-z0-9][a-z0-9-]{0,62}$` — the same
/// one tenant slugs use (ADR-0008), so a path is URL-, hostname- and
/// CLI-safe. The database carries the same rule as a CHECK; this is the
/// version that produces a message worth reading.
///
/// # Errors
///
/// [`Error::Invalid`] when the slug is empty, too long, starts with a hyphen,
/// or contains anything outside `[a-z0-9-]`.
pub fn validate_slug(slug: &str) -> Result<()> {
    let invalid = |message: String| Error::Invalid { message };
    if slug.is_empty() {
        return Err(invalid("a scope slug cannot be empty".to_owned()));
    }
    if slug.chars().count() > MAX_SLUG_CHARS {
        return Err(invalid(format!(
            "a scope slug is at most {MAX_SLUG_CHARS} characters, got {}",
            slug.chars().count()
        )));
    }
    let mut chars = slug.chars();
    let first = chars.next().unwrap_or_default();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(invalid(format!(
            "a scope slug starts with a lowercase letter or a digit, got {slug:?}"
        )));
    }
    if let Some(bad) = chars.find(|c| !c.is_ascii_lowercase() && !c.is_ascii_digit() && *c != '-') {
        return Err(invalid(format!(
            "a scope slug holds lowercase letters, digits and hyphens; {slug:?} holds {bad:?}"
        )));
    }
    Ok(())
}

/// Checks a display name: non-blank and at most [`MAX_DISPLAY_NAME_CHARS`].
///
/// # Errors
///
/// [`Error::Invalid`] when the name is blank or too long.
pub fn validate_display_name(display_name: &str) -> Result<()> {
    if display_name.trim().is_empty() {
        return Err(Error::Invalid {
            message: "a scope display name cannot be blank".to_owned(),
        });
    }
    let len = display_name.chars().count();
    if len > MAX_DISPLAY_NAME_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a scope display name is at most {MAX_DISPLAY_NAME_CHARS} characters, got {len}"
            ),
        });
    }
    Ok(())
}

/// Checks an attributes bag: a JSON object, at most [`MAX_ATTRIBUTES_BYTES`]
/// encoded.
///
/// # Errors
///
/// [`Error::Invalid`] when the value is not an object or is over the bound.
pub fn validate_attributes(attributes: &serde_json::Value) -> Result<()> {
    if !attributes.is_object() {
        return Err(Error::Invalid {
            message: "scope attributes must be a JSON object".to_owned(),
        });
    }
    let encoded = attributes.to_string().len();
    if encoded > MAX_ATTRIBUTES_BYTES {
        return Err(Error::Invalid {
            message: format!(
                "scope attributes are at most {MAX_ATTRIBUTES_BYTES} bytes encoded, got {encoded}"
            ),
        });
    }
    Ok(())
}

/// Splits a scope path (`acme/platform/payments`) into its slugs, rejecting
/// empty segments and anything outside the slug grammar.
///
/// The first segment is the tenant root's own slug: a path names a walk from
/// the root, and a path that did not include it would be ambiguous the day a
/// deployment has two roots to be wrong about.
///
/// # Errors
///
/// [`Error::Invalid`] when the path is empty, has an empty segment (a leading,
/// trailing or doubled separator), or holds a segment that is not a slug.
pub fn parse_path(path: &str) -> Result<Vec<&str>> {
    if path.is_empty() {
        return Err(Error::Invalid {
            message: "a scope path cannot be empty".to_owned(),
        });
    }
    let segments: Vec<&str> = path.split(PATH_SEPARATOR).collect();
    for segment in &segments {
        validate_slug(segment).map_err(|err| Error::Invalid {
            message: format!("scope path {path:?}: {err}"),
        })?;
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_the_wire_name() {
        for kind in ScopeKind::ALL {
            assert_eq!(kind.as_str().parse::<ScopeKind>().unwrap(), *kind);
            let json = serde_json::to_string(kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
        assert!("org".parse::<ScopeKind>().is_err());
        assert!("department".parse::<ScopeKind>().is_err());
        assert!("team".parse::<ScopeKind>().is_err());
    }

    #[test]
    fn statuses_round_trip_through_the_wire_name() {
        for status in ScopeStatus::ALL {
            assert_eq!(status.as_str().parse::<ScopeStatus>().unwrap(), *status);
            let json = serde_json::to_string(status).unwrap();
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
        assert!("suspended".parse::<ScopeStatus>().is_err());
    }

    /// The predicate and the list are two statements of one rule, and the one
    /// that produces error messages is the one nobody exercises. This is what
    /// keeps them from disagreeing.
    #[test]
    fn permitted_parents_and_permits_parent_agree_everywhere() {
        for child in ScopeKind::ALL {
            for parent in ScopeKind::ALL {
                assert_eq!(
                    child.permits_parent(*parent),
                    child.permitted_parents().contains(parent),
                    "{child} under {parent}"
                );
            }
        }
    }

    #[test]
    fn only_the_tenant_root_has_no_permitted_parent() {
        for kind in ScopeKind::ALL {
            assert_eq!(
                kind.permitted_parents().is_empty(),
                kind.is_tenant_root(),
                "{kind}"
            );
        }
    }

    /// The property the old model could not state: an org unit under an org
    /// unit, all the way down. Nothing in the vocabulary bounds the depth.
    #[test]
    fn org_units_nest_inside_themselves() {
        assert!(ScopeKind::OrgUnit.permits_parent(ScopeKind::OrgUnit));
        for kind in ScopeKind::ALL {
            if *kind != ScopeKind::OrgUnit {
                assert!(!kind.permits_parent(*kind), "{kind} nests inside itself");
            }
        }
    }

    #[test]
    fn slugs_follow_the_tenant_grammar() {
        for good in ["a", "0", "acme", "acme-emea", "a-1-2-3", &"a".repeat(63)] {
            validate_slug(good).unwrap_or_else(|err| panic!("{good:?}: {err}"));
        }
        for bad in [
            "",
            "-acme",
            "Acme",
            "acme_emea",
            "acme/emea",
            "acme ",
            &"a".repeat(64),
        ] {
            assert!(validate_slug(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn display_names_are_non_blank_and_bounded() {
        validate_display_name("Payments").unwrap();
        validate_display_name(&"x".repeat(MAX_DISPLAY_NAME_CHARS)).unwrap();
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name(&"x".repeat(MAX_DISPLAY_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn attributes_are_a_bounded_object() {
        validate_attributes(&serde_json::json!({})).unwrap();
        validate_attributes(&serde_json::json!({"cost-centre": "42"})).unwrap();
        assert!(validate_attributes(&serde_json::json!([])).is_err());
        assert!(validate_attributes(&serde_json::json!("labelled")).is_err());
        assert!(validate_attributes(&serde_json::Value::Null).is_err());
        let oversized = serde_json::json!({"blob": "x".repeat(MAX_ATTRIBUTES_BYTES)});
        assert!(validate_attributes(&oversized).is_err());
    }

    #[test]
    fn paths_split_into_slugs_and_refuse_everything_else() {
        assert_eq!(parse_path("acme").unwrap(), vec!["acme"]);
        assert_eq!(
            parse_path("acme/platform/payments").unwrap(),
            vec!["acme", "platform", "payments"]
        );
        for bad in ["", "/acme", "acme/", "acme//payments", "acme/Platform"] {
            assert!(parse_path(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_scope_round_trips_through_json() {
        let scope = Scope {
            id: ScopeId::new(),
            tenant_id: TenantId::new(),
            kind: ScopeKind::Workspace,
            parent_scope_id: Some(ScopeId::new()),
            slug: "payments".to_owned(),
            display_name: "Payments".to_owned(),
            status: ScopeStatus::Active,
            principal_id: None,
            attributes: serde_json::json!({"cost-centre": "42"}),
            created_by: Some(IdentityId::new()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(serde_json::from_str::<Scope>(&json).unwrap(), scope);
    }
}
