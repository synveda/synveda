//! The directory mirror (AUTH-4, ADR-0059 decision 3).
//!
//! What a provisioning agent told us, kept as it told us — separate from
//! [`crate::Identity`], which is what the product made of it. The mirror is
//! the SCIM resource of record: it answers a conformant `GET /Users/{id}`
//! for somebody who may never log in, and it survives them leaving.
//!
//! Named for the directory rather than for SCIM because the protocol is one
//! of two doors into it: AUTH-5's scheduled pull sync writes these same
//! rows from a directory read and hands them to the same reconciler.
//!
//! Only the attributes the product declares are here. A conformant server
//! answers for the schema it publishes, so an attribute that is not a
//! column is one `/Schemas` never claimed — which keeps the endpoint's
//! promises falsifiable rather than aspirational.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{DirectoryGroupId, DirectoryUserId, IdentityId, TenantId};

/// One person, as the directory describes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryUser {
    /// The resource id a provisioning agent addresses this row by.
    pub id: DirectoryUserId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The directory's own anchor (`externalId`), when it sends one.
    ///
    /// Mutable, because it is the customer's attribute mapping rather than
    /// a protocol constant — which is why reconciliation matches on an
    /// ordered fallback and never on this alone (ADR-0059 decision 4).
    pub external_id: Option<String>,
    /// `userName` — unique among live rows, case-insensitively.
    pub user_name: String,
    /// Whether the directory considers this person current. `false` is the
    /// leaver signal, and what it lands as is a seal (decision 8).
    pub active: bool,
    /// `displayName`.
    pub display_name: Option<String>,
    /// `name.givenName`.
    pub given_name: Option<String>,
    /// `name.familyName`.
    pub family_name: Option<String>,
    /// `emails[type eq "work"].value` — the one multi-valued attribute the
    /// product stores, and one of the three things reconciliation matches
    /// on.
    pub work_email: Option<String>,
    /// The identity this row projects onto, once reconciliation has run.
    pub identity_id: Option<IdentityId>,
    /// `meta.version` — the ETag, bumped on every write.
    pub version: i64,
    /// `meta.created`.
    pub created_at: DateTime<Utc>,
    /// `meta.lastModified`.
    pub updated_at: DateTime<Utc>,
}

/// One directory group. Its `display_name` is what the AUTH-2 mapping
/// resolver sees — the same `group_mappings`-then-convention resolution a
/// token's `groups` claim goes through (ADR-0013 decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryGroup {
    /// The resource id a provisioning agent addresses this row by.
    pub id: DirectoryGroupId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The directory's own anchor (`externalId`), when it sends one.
    pub external_id: Option<String>,
    /// `displayName` — the group name placement resolves against.
    pub display_name: String,
    /// `meta.version` — the ETag, bumped on every write.
    pub version: i64,
    /// `meta.created`.
    pub created_at: DateTime<Utc>,
    /// `meta.lastModified`.
    pub updated_at: DateTime<Utc>,
}

/// A provisioning credential's record — everything about it except the
/// secret, which is never stored (ADR-0059 decision 13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimCredential {
    /// The credential's own id, and what audit events name it by.
    pub id: crate::ScimCredentialId,
    /// The tenant it provisions into — named in the token itself, proven
    /// by the secret.
    pub tenant_id: TenantId,
    /// What an operator recognises it by when deciding to rotate.
    pub label: String,
    /// When it stops authenticating. Always set, always capped.
    pub expires_at: DateTime<Utc>,
    /// When it was revoked, if it was. Revocation is a stamp rather than a
    /// delete: which credential sealed which identity has to stay
    /// answerable after the credential is gone.
    pub revoked_at: Option<DateTime<Utc>>,
    /// When it last authenticated a request, on a coarse cadence.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When it was issued.
    pub created_at: DateTime<Utc>,
    /// The subject that issued it.
    pub created_by: String,
}

impl ScimCredential {
    /// Whether this credential may authenticate a request at `now` —
    /// revocation and expiry in one place, so no caller checks one and
    /// forgets the other.
    #[must_use]
    pub fn usable_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at > now
    }
}
