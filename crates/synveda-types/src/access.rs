//! Membership and access assignment (CPR-5, ADR-0072): who may act on a
//! governed scope, and where that authority came from.
//!
//! One model for a person working alone, for four people sharing agent
//! context, and for a company with a directory — ADR-0068 decision 1 applied
//! to access. There is no `personal_members` table and no enterprise variant
//! of a grant: a solo deployment has one grant (the `owner` one its first
//! workspace minted) and a bank has fifty thousand, in the same four tables.
//!
//! ## Four nouns
//!
//! - A [`Group`] is a named set of principals. It grants nothing by itself.
//! - A [`GroupMember`] puts a principal in one.
//! - A [`ScopeGrant`] gives a **subject** — a principal or a group — a
//!   [`RoleKey`] at a scope. The scope's subtree inherits it, which is how a
//!   workspace-level grant reaches that workspace's projects without a second
//!   row.
//! - A [`PendingInvite`] is an expiring, one-time token that mints a grant
//!   when somebody redeems it.
//!
//! ## A role key is a key, not a permission set
//!
//! [`RoleKey`] is six words and no semantics: nothing in this module says what
//! an `owner` may do. That is deliberate and it is the whole shape of the
//! decision (ADR-0072 decision 2). The product already has exactly one place
//! that decides what somebody may do — the Cedar packs — and a second table
//! mapping roles to permissions would be a second decision point that
//! disagrees with the first the day one of them is edited. So this crate
//! stores the key and the policy layer interprets it.
//!
//! ## A principal is a token subject
//!
//! [`ScopeGrant::principal_id`] and [`GroupMember::principal_id`] hold a
//! verified token subject, not a foreign key into `identities`. That is
//! ADR-0015 decision 2's reasoning, unchanged: the PDP's principal is
//! `(tenant, subject)`, and a grant that could not precede first login could
//! not be pre-assigned, could not name a dev subject, and — the reason that
//! decided it here — could not be written without an `identities` row, which
//! in this tree still requires a node of the **old** hierarchy. A membership
//! model that needed the model it replaces would be a synchronisation between
//! the two, which this programme forbids outright.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, GrantId, GroupId, InviteId, Result, ScopeId, TenantId};

/// Longest principal id (a token subject), in characters. The same bound
/// `role_bindings.subject` and `identities.subject` carry, because it is the
/// same string.
pub const MAX_PRINCIPAL_CHARS: usize = 255;

/// Longest invited email address, in characters. RFC 5321's practical
/// maximum; the value is a label rather than an authentication factor.
pub const MAX_EMAIL_CHARS: usize = 320;

/// Longest directory reference, in characters — the external id a directory
/// knows a group by.
pub const MAX_DIRECTORY_REF_CHARS: usize = 255;

/// How long an invitation may stand, at most.
///
/// A bound rather than a preference: an invitation is a bearer credential
/// that mints access, and one that never expires is a key left under the mat
/// — AUTH-3's lifetime-cap doctrine (ADR-0018 decision 5), applied to the
/// credential a person pastes into a chat window.
pub const MAX_INVITE_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// How long an invitation stands when the caller does not say.
pub const DEFAULT_INVITE_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// The role a grant carries.
///
/// A **key**, not a permission set: this enum names authorities, and what each
/// one may actually do is decided by the policy pack in force at the scope
/// (ADR-0072 decision 2). Closed, like [`crate::Role`] and pack names, so that
/// `owner` means the same thing in every tenant and a pack written for one
/// deployment reads correctly in another.
///
/// No `Default`: what somebody is granted is always an explicit decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleKey {
    /// The scope is theirs: they may hand out access to it, including this
    /// role. Minted for whoever creates a workspace or a project.
    Owner,
    /// An ordinary participant: reads and contributes.
    Member,
    /// Reads, and does not contribute.
    Viewer,
    /// Casts verdicts on what others propose.
    Reviewer,
    /// Curates what the scope keeps — publishes, retires, organises.
    Curator,
    /// Administers the scope: policy, membership and configuration, without
    /// the ownership claim `owner` carries.
    Administrator,
}

impl RoleKey {
    /// Every role key, in declaration order — the order a listing sorts by
    /// and the vocabulary a CHECK constraint mirrors.
    pub const ALL: &'static [RoleKey] = &[
        RoleKey::Owner,
        RoleKey::Member,
        RoleKey::Viewer,
        RoleKey::Reviewer,
        RoleKey::Curator,
        RoleKey::Administrator,
    ];

    /// Stable wire name, identical to the serde form and to the value stored
    /// in `scope_grants.role_key`; also what the PDP will pass to policies.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RoleKey::Owner => "owner",
            RoleKey::Member => "member",
            RoleKey::Viewer => "viewer",
            RoleKey::Reviewer => "reviewer",
            RoleKey::Curator => "curator",
            RoleKey::Administrator => "administrator",
        }
    }
}

impl fmt::Display for RoleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RoleKey {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        RoleKey::ALL
            .iter()
            .copied()
            .find(|role| role.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown role key: {s:?} (one of {})",
                    RoleKey::ALL
                        .iter()
                        .map(|role| role.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

/// What kind of thing a grant is *for*.
///
/// Two, and there will not be a third without an ADR: a grant to a group is
/// how a deployment expresses "everyone in engineering", and a grant to
/// anything else — a service, a token, a label — would be an authorisation
/// input nobody can enumerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubjectKind {
    /// One principal, named by its verified token subject.
    Principal,
    /// Every member of a group, resolved at read time rather than expanded
    /// into rows — so adding somebody to a group grants them everything the
    /// group holds, with no fan-out to keep consistent.
    Group,
}

impl SubjectKind {
    /// Both kinds, in declaration order.
    pub const ALL: &'static [SubjectKind] = &[SubjectKind::Principal, SubjectKind::Group];

    /// Stable wire name, identical to the serde form and to the stored value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SubjectKind::Principal => "principal",
            SubjectKind::Group => "group",
        }
    }
}

impl fmt::Display for SubjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SubjectKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "principal" => Ok(SubjectKind::Principal),
            "group" => Ok(SubjectKind::Group),
            other => Err(Error::Invalid {
                message: format!("unknown grant subject kind: {other:?}"),
            }),
        }
    }
}

/// A grant's subject, as a caller names it and as a reader reads it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GrantSubject {
    /// One principal, by verified token subject.
    Principal {
        /// The subject.
        principal_id: String,
    },
    /// A group, by id.
    Group {
        /// The group.
        group_id: GroupId,
    },
}

impl GrantSubject {
    /// Which variant this is, as the stored vocabulary.
    #[must_use]
    pub const fn kind(&self) -> SubjectKind {
        match self {
            GrantSubject::Principal { .. } => SubjectKind::Principal,
            GrantSubject::Group { .. } => SubjectKind::Group,
        }
    }

    /// The principal, when this is one.
    #[must_use]
    pub fn principal_id(&self) -> Option<&str> {
        match self {
            GrantSubject::Principal { principal_id } => Some(principal_id),
            GrantSubject::Group { .. } => None,
        }
    }

    /// The group, when this is one.
    #[must_use]
    pub const fn group_id(&self) -> Option<GroupId> {
        match self {
            GrantSubject::Group { group_id } => Some(*group_id),
            GrantSubject::Principal { .. } => None,
        }
    }
}

/// Where a grant came from — the whole of "access-source visibility".
///
/// The point of storing this is that "why can this person see my project" has
/// an answer that does not require reading an audit log: a grant carries its
/// own provenance, and a listing shows it beside the role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantSource {
    /// Minted with the scope, for whoever created it. Nobody hands this out.
    Owner,
    /// Somebody with the authority granted it, deliberately.
    Direct,
    /// Somebody redeemed an invitation.
    Invite,
    /// A directory said so. **Not manually editable** — see
    /// [`GrantSource::is_directory_managed`].
    Directory,
    /// The product granted it as a consequence of something else.
    Automation,
}

impl GrantSource {
    /// Every source, in declaration order.
    pub const ALL: &'static [GrantSource] = &[
        GrantSource::Owner,
        GrantSource::Direct,
        GrantSource::Invite,
        GrantSource::Directory,
        GrantSource::Automation,
    ];

    /// Stable wire name, identical to the serde form and to the stored value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            GrantSource::Owner => "owner",
            GrantSource::Direct => "direct",
            GrantSource::Invite => "invite",
            GrantSource::Directory => "directory",
            GrantSource::Automation => "automation",
        }
    }

    /// Whether a directory owns this row, and the product therefore must not
    /// let a person edit or revoke it here.
    ///
    /// The rule exists because the alternative is worse than an inconvenience:
    /// a directory-managed grant a person removed comes back on the next sync,
    /// so the removal looked like it worked, did nothing, and taught somebody
    /// that revocation in this product is unreliable. Refusing it names the
    /// directory instead.
    #[must_use]
    pub const fn is_directory_managed(&self) -> bool {
        matches!(self, GrantSource::Directory)
    }
}

impl fmt::Display for GrantSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GrantSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        GrantSource::ALL
            .iter()
            .copied()
            .find(|source| source.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown grant source: {s:?}"),
            })
    }
}

/// Whether a group is the product's to edit, or a directory's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupSource {
    /// Somebody made it here, and somebody here maintains it.
    Direct,
    /// A directory owns its membership. Editing it in the product would be
    /// undone by the next sync (see [`GrantSource::is_directory_managed`]).
    Directory,
}

impl GroupSource {
    /// Both sources, in declaration order.
    pub const ALL: &'static [GroupSource] = &[GroupSource::Direct, GroupSource::Directory];

    /// Stable wire name, identical to the serde form and to the stored value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            GroupSource::Direct => "direct",
            GroupSource::Directory => "directory",
        }
    }

    /// Whether a directory owns this group's membership.
    #[must_use]
    pub const fn is_directory_managed(&self) -> bool {
        matches!(self, GroupSource::Directory)
    }
}

impl fmt::Display for GroupSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GroupSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "direct" => Ok(GroupSource::Direct),
            "directory" => Ok(GroupSource::Directory),
            other => Err(Error::Invalid {
                message: format!("unknown group source: {other:?}"),
            }),
        }
    }
}

/// A named set of principals.
///
/// A group grants nothing on its own — it is a set, and a [`ScopeGrant`] to
/// that set is what grants. Keeping the two apart is what lets a deployment
/// say "engineering" once and then price it differently at three scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Stable id.
    pub id: GroupId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Tenant-unique handle, immutable. Same grammar as a scope slug, so a
    /// group is a thing somebody can type.
    pub slug: String,
    /// Display name; renameable.
    pub display_name: String,
    /// Optional prose.
    pub description: Option<String>,
    /// Whose group it is.
    pub source: GroupSource,
    /// The external id a directory knows it by; `Some` exactly when
    /// [`source`](Self::source) is [`GroupSource::Directory`].
    pub directory_ref: Option<String>,
    /// Whether the group is in use.
    pub status: crate::workspace::LifecycleStatus,
    /// Monotonic; what an update's precondition names.
    pub revision: i64,
    /// The subject that created it, when a caller did.
    pub created_by: Option<String>,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// One principal's membership of one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The group.
    pub group_id: GroupId,
    /// The principal, by verified token subject.
    pub principal_id: String,
    /// How they came to be in it.
    pub source: GrantSource,
    /// Who put them there, when a caller did.
    pub added_by: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
}

/// One subject's role at one scope.
///
/// Grants are **additive and inherited**: a scope's subtree holds every grant
/// on its ancestry, and two grants never cancel. There is no deny row and
/// there must not be one — a denial that lives in a membership table is a
/// second policy engine, and this product has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeGrant {
    /// Stable id — the handle a revocation names.
    pub id: GrantId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The scope the grant is at. Its subtree inherits, subject to
    /// [`inherits_into`].
    pub scope_id: ScopeId,
    /// Which of the two subject columns is populated.
    pub subject_kind: SubjectKind,
    /// The principal, when [`subject_kind`](Self::subject_kind) is
    /// [`SubjectKind::Principal`].
    pub principal_id: Option<String>,
    /// The group, when [`subject_kind`](Self::subject_kind) is
    /// [`SubjectKind::Group`].
    pub group_id: Option<GroupId>,
    /// What they hold.
    pub role_key: RoleKey,
    /// Where it came from.
    pub source: GrantSource,
    /// The invitation that produced it; `Some` exactly when
    /// [`source`](Self::source) is [`GrantSource::Invite`].
    pub invite_id: Option<InviteId>,
    /// Who granted it, when a caller did.
    pub granted_by: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
}

impl ScopeGrant {
    /// The subject, reassembled from the two columns.
    ///
    /// # Errors
    ///
    /// [`Error::Internal`] when neither column is populated, which the
    /// database's own CHECK makes unreachable.
    pub fn subject(&self) -> Result<GrantSubject> {
        match (self.subject_kind, &self.principal_id, self.group_id) {
            (SubjectKind::Principal, Some(principal_id), _) => Ok(GrantSubject::Principal {
                principal_id: principal_id.clone(),
            }),
            (SubjectKind::Group, _, Some(group_id)) => Ok(GrantSubject::Group { group_id }),
            _ => Err(Error::Internal {
                message: format!("grant {} has no subject column populated", self.id),
            }),
        }
    }
}

/// Whether a grant at an ancestor scope reaches a descendant of the given
/// shape — **principal-private scope isolation**, in one place.
///
/// A `principal`-shaped scope is somebody's own. Nothing above it reaches into
/// it: not the tenant root, not an org unit, not a workspace owner. Only a
/// grant written directly at that scope applies, which is what makes "my own
/// notes" a thing this product can hold at all.
///
/// It is expressed as a predicate over the **descendant's** shape rather than
/// as a rule about the tree because the tree already makes a principal scope a
/// leaf ([`crate::scope::ScopeKind::permits_parent`] admits no child under
/// one). If that ever changes, this is where the rule already is.
#[must_use]
pub const fn inherits_into(descendant: crate::scope::ScopeKind) -> bool {
    !matches!(descendant, crate::scope::ScopeKind::Principal)
}

/// Where an invitation stands.
///
/// `Expired` is **derived, never stored**: expiry is a property of the
/// decision rather than of a job (ADR-0037 decision 4), so the one read that
/// loads an invitation is the one place a window ends. A stored `expired`
/// would be a status nothing writes until somebody builds the sweep, and a
/// sweep that has not run yet is an invitation that still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteStatus {
    /// Outstanding and redeemable.
    Pending,
    /// Somebody redeemed it. Terminal — an invitation is one-time.
    Accepted,
    /// Somebody withdrew it. Terminal.
    Revoked,
    /// Outstanding but past its window. Derived at read time.
    Expired,
}

impl InviteStatus {
    /// Every status, in declaration order.
    pub const ALL: &'static [InviteStatus] = &[
        InviteStatus::Pending,
        InviteStatus::Accepted,
        InviteStatus::Revoked,
        InviteStatus::Expired,
    ];

    /// The three the database stores. [`InviteStatus::Expired`] is absent
    /// because nothing writes it.
    pub const STORED: &'static [InviteStatus] = &[
        InviteStatus::Pending,
        InviteStatus::Accepted,
        InviteStatus::Revoked,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            InviteStatus::Pending => "pending",
            InviteStatus::Accepted => "accepted",
            InviteStatus::Revoked => "revoked",
            InviteStatus::Expired => "expired",
        }
    }

    /// Whether an invitation in this state can still be redeemed.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, InviteStatus::Pending)
    }
}

impl fmt::Display for InviteStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for InviteStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        InviteStatus::ALL
            .iter()
            .copied()
            .find(|status| status.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown invite status: {s:?}"),
            })
    }
}

/// An outstanding invitation to a scope.
///
/// The token itself is **not here**: what is stored is its hash, and the
/// plaintext exists once, in the response to the request that created it. A
/// dump of this table mints nothing (the AUD-1 threat model, ADR-0059
/// decision 13's shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingInvite {
    /// Stable id — the handle a revocation names.
    pub id: InviteId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The scope the invitation grants at.
    pub scope_id: ScopeId,
    /// What it grants.
    pub role_key: RoleKey,
    /// Who it was meant for, when the inviter said. Optional on purpose: a
    /// deployment with no mail path invites by copying a link, and an address
    /// nobody can send to is a label rather than a requirement.
    pub email: Option<String>,
    /// Where it stands. Never [`InviteStatus::Expired`] as stored; the read
    /// derives that from [`expires_at`](Self::expires_at).
    pub status: InviteStatus,
    /// When it stops being redeemable.
    pub expires_at: DateTime<Utc>,
    /// Who created it.
    pub created_by: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
    /// The principal that redeemed it, when somebody has.
    pub accepted_by: Option<String>,
    /// When it was redeemed.
    pub accepted_at: Option<DateTime<Utc>>,
    /// Who withdrew it, when somebody has.
    pub revoked_by: Option<String>,
    /// When it was withdrawn.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PendingInvite {
    /// The status a reader should see: the stored one, or
    /// [`InviteStatus::Expired`] when it is outstanding and past its window.
    #[must_use]
    pub fn effective_status(&self, now: DateTime<Utc>) -> InviteStatus {
        if self.status == InviteStatus::Pending && self.expires_at <= now {
            InviteStatus::Expired
        } else {
            self.status
        }
    }

    /// Whether this invitation may be redeemed at `now`.
    #[must_use]
    pub fn is_redeemable(&self, now: DateTime<Utc>) -> bool {
        self.effective_status(now).is_open()
    }
}

/// Checks a principal id (a verified token subject): non-blank and bounded.
///
/// # Errors
///
/// [`Error::Invalid`] when the id is blank or over [`MAX_PRINCIPAL_CHARS`].
pub fn validate_principal_id(principal_id: &str) -> Result<()> {
    if principal_id.trim().is_empty() {
        return Err(Error::Invalid {
            message: "a principal id cannot be blank".to_owned(),
        });
    }
    let len = principal_id.chars().count();
    if len > MAX_PRINCIPAL_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a principal id is at most {MAX_PRINCIPAL_CHARS} characters, got {len}"
            ),
        });
    }
    Ok(())
}

/// Checks an invited email address: non-blank, bounded, and shaped like one.
///
/// Deliberately permissive. The address is a label the inviter writes down so
/// a list of outstanding invitations is readable; nothing authenticates
/// against it, and a validator strict enough to be worth arguing about would
/// reject somebody's real address for no gain.
///
/// # Errors
///
/// [`Error::Invalid`] when the address is blank, over [`MAX_EMAIL_CHARS`], or
/// has no `@` with something either side of it.
pub fn validate_email(email: &str) -> Result<()> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid {
            message: "an invited email cannot be blank; omit it instead".to_owned(),
        });
    }
    let len = trimmed.chars().count();
    if len > MAX_EMAIL_CHARS {
        return Err(Error::Invalid {
            message: format!("an email is at most {MAX_EMAIL_CHARS} characters, got {len}"),
        });
    }
    let mut halves = trimmed.split('@');
    let local = halves.next().unwrap_or_default();
    let domain = halves.next().unwrap_or_default();
    if local.is_empty() || domain.is_empty() || halves.next().is_some() {
        return Err(Error::Invalid {
            message: format!("{email:?} is not an email address"),
        });
    }
    Ok(())
}

/// Checks a directory reference: non-blank and bounded.
///
/// # Errors
///
/// [`Error::Invalid`] when it is blank or over [`MAX_DIRECTORY_REF_CHARS`].
pub fn validate_directory_ref(directory_ref: &str) -> Result<()> {
    if directory_ref.trim().is_empty() {
        return Err(Error::Invalid {
            message: "a directory reference cannot be blank".to_owned(),
        });
    }
    let len = directory_ref.chars().count();
    if len > MAX_DIRECTORY_REF_CHARS {
        return Err(Error::Invalid {
            message: format!(
                "a directory reference is at most {MAX_DIRECTORY_REF_CHARS} characters, got {len}"
            ),
        });
    }
    Ok(())
}

/// Checks an invitation's requested lifetime against the product ceiling.
///
/// # Errors
///
/// [`Error::Invalid`] when the lifetime is not positive or exceeds
/// [`MAX_INVITE_TTL_SECS`].
pub fn validate_invite_ttl(ttl_secs: i64) -> Result<()> {
    if ttl_secs <= 0 {
        return Err(Error::Invalid {
            message: "an invitation's lifetime must be positive".to_owned(),
        });
    }
    if ttl_secs > MAX_INVITE_TTL_SECS {
        return Err(Error::Invalid {
            message: format!(
                "an invitation stands for at most {MAX_INVITE_TTL_SECS} seconds \
                 ({} days), got {ttl_secs}",
                MAX_INVITE_TTL_SECS / 86_400
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeKind;

    #[test]
    fn role_keys_round_trip_through_the_wire_name() {
        for role in RoleKey::ALL {
            assert_eq!(role.as_str().parse::<RoleKey>().unwrap(), *role);
            assert_eq!(
                serde_json::to_string(role).unwrap(),
                format!("\"{}\"", role.as_str())
            );
        }
    }

    /// The vocabulary is exactly the six the feature names — no more, so a
    /// pack written against it stays exhaustive, and no fewer, so the six
    /// words in the product's documentation are the six here.
    #[test]
    fn the_role_vocabulary_is_the_six_this_product_ships() {
        let names: Vec<&str> = RoleKey::ALL.iter().map(RoleKey::as_str).collect();
        assert_eq!(
            names,
            vec![
                "owner",
                "member",
                "viewer",
                "reviewer",
                "curator",
                "administrator"
            ]
        );
    }

    /// A role key is a name and nothing else: there is no rank to compare and
    /// no permission set to look up. `Ord` exists only so a listing sorts
    /// stably, and this is what says so out loud.
    #[test]
    fn a_role_key_carries_no_privilege_ordering() {
        assert!(RoleKey::Owner < RoleKey::Viewer, "declaration order only");
        assert!(
            RoleKey::Member < RoleKey::Administrator,
            "a lower key is not a lesser authority; the pack decides"
        );
    }

    #[test]
    fn the_product_role_vocabulary_and_the_grant_role_vocabulary_are_different_things() {
        // Two closed vocabularies with two overlapping words. `curator` and
        // `viewer` appear in both and mean different things — one is a
        // binding on the old hierarchy, the other a grant on a governed
        // scope — so nothing may translate between them (ADR-0072
        // decision 3).
        let bindings: Vec<&str> = crate::Role::ALL.iter().map(crate::Role::as_str).collect();
        let grants: Vec<&str> = RoleKey::ALL.iter().map(RoleKey::as_str).collect();
        assert_ne!(bindings, grants);
        assert!(grants.contains(&"owner") && !bindings.contains(&"owner"));
        assert!(bindings.contains(&"steward") && !grants.contains(&"steward"));
    }

    #[test]
    fn grant_sources_round_trip_and_name_the_managed_one() {
        for source in GrantSource::ALL {
            assert_eq!(source.as_str().parse::<GrantSource>().unwrap(), *source);
            assert_eq!(
                source.is_directory_managed(),
                *source == GrantSource::Directory,
                "{source} is the only unmanageable source"
            );
        }
        assert!("owner".parse::<GrantSource>().is_ok());
        assert!("magic".parse::<GrantSource>().is_err());
    }

    #[test]
    fn subject_kinds_and_group_sources_round_trip() {
        for kind in SubjectKind::ALL {
            assert_eq!(kind.as_str().parse::<SubjectKind>().unwrap(), *kind);
        }
        for source in GroupSource::ALL {
            assert_eq!(source.as_str().parse::<GroupSource>().unwrap(), *source);
        }
        assert!("service".parse::<SubjectKind>().is_err());
    }

    #[test]
    fn a_grant_subject_tags_itself_on_the_wire() {
        let principal = GrantSubject::Principal {
            principal_id: "sam".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&principal).unwrap(),
            serde_json::json!({"kind": "principal", "principal_id": "sam"})
        );
        assert_eq!(principal.kind(), SubjectKind::Principal);
        assert_eq!(principal.principal_id(), Some("sam"));
        assert_eq!(principal.group_id(), None);

        let group = GrantSubject::Group {
            group_id: GroupId::new(),
        };
        assert_eq!(group.kind(), SubjectKind::Group);
        assert!(group.principal_id().is_none());
    }

    /// The isolation rule, over the whole shape vocabulary rather than the
    /// one case somebody remembered.
    #[test]
    fn only_a_principal_scope_refuses_what_its_ancestors_grant() {
        for kind in ScopeKind::ALL {
            assert_eq!(
                inherits_into(*kind),
                *kind != ScopeKind::Principal,
                "{kind} inheritance"
            );
        }
    }

    #[test]
    fn invite_statuses_round_trip_and_expired_is_not_stored() {
        for status in InviteStatus::ALL {
            assert_eq!(status.as_str().parse::<InviteStatus>().unwrap(), *status);
        }
        assert!(
            !InviteStatus::STORED.contains(&InviteStatus::Expired),
            "nothing writes `expired`; it is derived at read time"
        );
        assert_eq!(InviteStatus::STORED.len() + 1, InviteStatus::ALL.len());
    }

    #[test]
    fn an_outstanding_invitation_expires_without_anything_writing_a_row() {
        let now = Utc::now();
        let invite = PendingInvite {
            id: InviteId::new(),
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            role_key: RoleKey::Member,
            email: None,
            status: InviteStatus::Pending,
            expires_at: now - chrono::Duration::seconds(1),
            created_by: Some("sam".to_owned()),
            created_at: now - chrono::Duration::days(8),
            accepted_by: None,
            accepted_at: None,
            revoked_by: None,
            revoked_at: None,
        };
        assert_eq!(invite.effective_status(now), InviteStatus::Expired);
        assert!(!invite.is_redeemable(now));

        let live = PendingInvite {
            expires_at: now + chrono::Duration::days(1),
            ..invite.clone()
        };
        assert_eq!(live.effective_status(now), InviteStatus::Pending);
        assert!(live.is_redeemable(now));

        // A terminal status is not re-derived: an accepted invitation that
        // then passes its window is still accepted, because what happened
        // to it happened.
        let accepted = PendingInvite {
            status: InviteStatus::Accepted,
            accepted_by: Some("robin".to_owned()),
            accepted_at: Some(now),
            ..invite
        };
        assert_eq!(accepted.effective_status(now), InviteStatus::Accepted);
        assert!(!accepted.is_redeemable(now));
    }

    #[test]
    fn principal_ids_are_non_blank_and_bounded() {
        validate_principal_id("sam").unwrap();
        validate_principal_id(&"s".repeat(MAX_PRINCIPAL_CHARS)).unwrap();
        assert!(validate_principal_id("").is_err());
        assert!(validate_principal_id("   ").is_err());
        assert!(validate_principal_id(&"s".repeat(MAX_PRINCIPAL_CHARS + 1)).is_err());
    }

    #[test]
    fn emails_are_labels_but_still_have_to_be_addresses() {
        for good in ["sam@example.com", "a+b@sub.example.co.uk"] {
            validate_email(good).unwrap_or_else(|err| panic!("{good:?}: {err}"));
        }
        for bad in ["", "   ", "sam", "@example.com", "sam@", "a@b@c"] {
            assert!(validate_email(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(validate_email(&format!("{}@x.com", "a".repeat(MAX_EMAIL_CHARS))).is_err());
    }

    #[test]
    fn an_invitation_cannot_stand_forever() {
        validate_invite_ttl(DEFAULT_INVITE_TTL_SECS).unwrap();
        validate_invite_ttl(MAX_INVITE_TTL_SECS).unwrap();
        assert!(validate_invite_ttl(0).is_err());
        assert!(validate_invite_ttl(-1).is_err());
        let error = validate_invite_ttl(MAX_INVITE_TTL_SECS + 1).expect_err("refused");
        assert!(
            error.to_string().contains("30 days"),
            "the refusal says the ceiling in the unit a person thinks in: {error}"
        );
    }

    #[test]
    fn directory_refs_are_non_blank_and_bounded() {
        validate_directory_ref("00u1a2b3").unwrap();
        assert!(validate_directory_ref(" ").is_err());
        assert!(validate_directory_ref(&"x".repeat(MAX_DIRECTORY_REF_CHARS + 1)).is_err());
    }

    #[test]
    fn a_grant_round_trips_through_json_and_reassembles_its_subject() {
        let grant = ScopeGrant {
            id: GrantId::new(),
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            subject_kind: SubjectKind::Group,
            principal_id: None,
            group_id: Some(GroupId::new()),
            role_key: RoleKey::Curator,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: Some("sam".to_owned()),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&grant).unwrap();
        assert_eq!(serde_json::from_str::<ScopeGrant>(&json).unwrap(), grant);
        assert_eq!(
            grant.subject().unwrap(),
            GrantSubject::Group {
                group_id: grant.group_id.unwrap()
            }
        );

        // The unreachable case, made reachable only by constructing it by
        // hand: a row with neither column is an internal error rather than a
        // silent "nobody".
        let malformed = ScopeGrant {
            group_id: None,
            ..grant
        };
        assert!(matches!(malformed.subject(), Err(Error::Internal { .. })));
    }
}
