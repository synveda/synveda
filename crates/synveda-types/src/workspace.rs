//! Workspaces and projects (CPR-4, ADR-0071): the two product-level
//! subtypes of a governed scope.
//!
//! A [`Workspace`] is where a person or a group keeps shared context; a
//! [`Project`] is a unit of work inside one. Neither is a second tree. Each
//! **owns** a scope of the matching shape — `workspace` under the tenant root,
//! `project` under its workspace's scope — and that scope is what policy is
//! assigned to, what a role binding covers, and what every asset attaches to.
//! The subtype row carries the product-level facts a scope has no opinion
//! about: a description, a lifecycle status, a revision, and (for a project)
//! the repositories it is about.
//!
//! ## Why a subtype at all, rather than attributes on the scope
//!
//! `scopes.attributes` is deliberately an open bag that nothing constrains
//! (ADR-0070 decision 1). A workspace's slug has to be unique, its
//! description bounded, its status a closed vocabulary and its revision
//! monotonic — four rules, none of which a JSON bag can hold. Putting them in
//! a table makes them database facts; putting them in `attributes` would make
//! them a convention every consumer re-parses, which is the failure ADR-0070
//! already refused once.
//!
//! ## One shape, three profiles
//!
//! There is no personal workspace table and no team workspace table, and
//! there must not be one (ADR-0068 decision 1). A person working alone has
//! one workspace; a company has hundreds. What differs is the policy profile
//! assigned to their scopes, never the row.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, IdentityId, ProjectId, Result, ScopeId, TenantId, WorkspaceId};

/// Longest description, in characters. Bounded because it is stored, listed
/// and rendered; generous because it is prose in somebody's language.
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;

/// Whether a workspace or a project is in use.
///
/// Shared by both subtypes rather than duplicated: the vocabulary is the same
/// two words with the same meaning, and two enums that must stay identical are
/// two enums that will not.
///
/// No `Default`: a subtype is created active by the one service that creates
/// them, and every other value is a deliberate transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleStatus {
    /// Ordinary. Work happens here; new projects may be created under it.
    Active,
    /// Retired. Kept because everything that references it is kept — a
    /// workspace is what sessions, versions and audit events name, so
    /// retiring one is a status transition rather than a row that stops
    /// existing.
    Archived,
}

impl LifecycleStatus {
    /// Every status, in declaration order.
    pub const ALL: &'static [LifecycleStatus] =
        &[LifecycleStatus::Active, LifecycleStatus::Archived];

    /// Stable wire name, identical to the serde form and to the value stored
    /// in the `status` column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            LifecycleStatus::Active => "active",
            LifecycleStatus::Archived => "archived",
        }
    }

    /// Whether work may still be added here. The one place the vocabulary is
    /// asked a question rather than displayed.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, LifecycleStatus::Active)
    }
}

impl fmt::Display for LifecycleStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LifecycleStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "active" => Ok(LifecycleStatus::Active),
            "archived" => Ok(LifecycleStatus::Archived),
            other => Err(Error::Invalid {
                message: format!("unknown lifecycle status: {other:?}"),
            }),
        }
    }
}

/// A collaboration space: the product noun a person names, owning one
/// `workspace`-shaped governed scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// The workspace's identity — stable for its whole life.
    pub id: WorkspaceId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The governed scope this workspace owns. Created in the same
    /// transaction as the workspace and never reassigned: a workspace whose
    /// scope could move under it would be a workspace whose policy could be
    /// swapped without a policy change.
    pub scope_id: ScopeId,
    /// Human-stable handle, unique in the tenant and identical to the scope's
    /// own slug — the two are one name, held together by a foreign key.
    /// Immutable, like the scope's.
    pub slug: String,
    /// Display name; renameable.
    pub display_name: String,
    /// Optional prose. `None` and `Some("")` are deliberately not the same
    /// thing on the wire, and only the former is representable: a blank
    /// description is refused rather than stored.
    pub description: Option<String>,
    /// Whether the workspace is in use. Mirrored onto the scope's own status
    /// by the transition, so a policy walk sees what the product surface says.
    pub status: LifecycleStatus,
    /// Monotonic revision, starting at 1 and incremented by every accepted
    /// update. What a caller's update precondition names (ADR-0071
    /// decision 5); a database trigger is what keeps it monotonic.
    pub revision: i64,
    /// The identity that created it, when one did. `None` records that the
    /// deployment did.
    pub created_by: Option<IdentityId>,
    /// When the workspace was created.
    pub created_at: DateTime<Utc>,
    /// When the workspace last changed.
    pub updated_at: DateTime<Utc>,
}

/// A unit of work inside a workspace, owning one `project`-shaped governed
/// scope beneath the workspace's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The project's identity — stable for its whole life.
    pub id: ProjectId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The workspace this project belongs to. Immutable: moving a project
    /// between workspaces would move its scope across a policy boundary, so
    /// it is a create and an archive rather than an update.
    pub workspace_id: WorkspaceId,
    /// The governed scope this project owns, whose parent is the workspace's
    /// scope — a foreign key, not a convention.
    pub scope_id: ScopeId,
    /// Handle, unique within the workspace and identical to the scope's slug.
    /// Immutable.
    pub slug: String,
    /// Display name; renameable.
    pub display_name: String,
    /// Optional prose.
    pub description: Option<String>,
    /// Whether the project is in use.
    pub status: LifecycleStatus,
    /// Monotonic revision — see [`Workspace::revision`].
    pub revision: i64,
    /// The identity that created it, when one did.
    pub created_by: Option<IdentityId>,
    /// When the project was created.
    pub created_at: DateTime<Utc>,
    /// When the project last changed.
    pub updated_at: DateTime<Utc>,
}

/// Checks an optional description: present means non-blank and at most
/// [`MAX_DESCRIPTION_CHARS`].
///
/// A blank string is refused rather than normalised to absent, deliberately:
/// "I sent an empty description" and "I sent no description" are different
/// requests, and silently making them the same hides a client bug.
///
/// # Errors
///
/// [`Error::Invalid`] when the description is present and blank, or too long.
pub fn validate_description(description: Option<&str>) -> Result<()> {
    let Some(description) = description else {
        return Ok(());
    };
    if description.trim().is_empty() {
        return Err(Error::Invalid {
            message: "a description cannot be blank; omit it instead".to_owned(),
        });
    }
    let len = description.chars().count();
    if len > MAX_DESCRIPTION_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a description is at most {MAX_DESCRIPTION_CHARS} characters, got {len}"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip_through_the_wire_name() {
        for status in LifecycleStatus::ALL {
            assert_eq!(status.as_str().parse::<LifecycleStatus>().unwrap(), *status);
            let json = serde_json::to_string(status).unwrap();
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
        assert!("suspended".parse::<LifecycleStatus>().is_err());
        assert!("deleted".parse::<LifecycleStatus>().is_err());
    }

    #[test]
    fn only_active_admits_new_work() {
        assert!(LifecycleStatus::Active.is_active());
        assert!(!LifecycleStatus::Archived.is_active());
    }

    #[test]
    fn descriptions_are_optional_bounded_and_never_blank() {
        validate_description(None).unwrap();
        validate_description(Some("Payments platform")).unwrap();
        validate_description(Some(&"x".repeat(MAX_DESCRIPTION_CHARS))).unwrap();
        assert!(validate_description(Some("")).is_err());
        assert!(validate_description(Some("   \n ")).is_err());
        assert!(validate_description(Some(&"x".repeat(MAX_DESCRIPTION_CHARS + 1))).is_err());
    }

    #[test]
    fn a_workspace_round_trips_through_json() {
        let workspace = Workspace {
            id: WorkspaceId::new(),
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            slug: "payments".to_owned(),
            display_name: "Payments".to_owned(),
            description: Some("Everything the payments team knows".to_owned()),
            status: LifecycleStatus::Active,
            revision: 1,
            created_by: Some(IdentityId::new()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&workspace).unwrap();
        assert_eq!(serde_json::from_str::<Workspace>(&json).unwrap(), workspace);
    }

    #[test]
    fn a_project_round_trips_through_json() {
        let project = Project {
            id: ProjectId::new(),
            tenant_id: TenantId::new(),
            workspace_id: WorkspaceId::new(),
            scope_id: ScopeId::new(),
            slug: "ledger".to_owned(),
            display_name: "Ledger".to_owned(),
            description: None,
            status: LifecycleStatus::Archived,
            revision: 7,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }
}
