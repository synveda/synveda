//! Proposals: the governed request to move reviewed content onto a
//! channel (tech plan §2.3; FLOW-3, ADR-0032).
//!
//! # A commit plus a row
//!
//! A proposal's *content* is a commit — a tree naming every member at the
//! object address of exactly the version proposed — and its *workflow* is
//! a row that moves. That is ADR-0030's split restated: refs move,
//! history does not. The commit is immutable, content-addressed, and
//! already in `vedaflow_commits`; the row carries state, and a database
//! trigger permits it exactly one transition, open → closed.
//!
//! The proposal's commit is a **root**: it descends from nothing, because
//! it is a statement about a set, not a step in a channel's history. What
//! ties it into history is publication, whose commit takes
//! `[channel head, proposal commit]` as parents — first-parent mainline
//! as in git, so the published channel's own line is unbroken and the
//! second parent is the review (ADR-0032 decision 10).
//!
//! # No ref
//!
//! A ref names a moving head, and a proposal's head is named by its row.
//! One ref per proposal would leave a permanent pointer per closed
//! proposal in a table that deliberately holds no DELETE grant.
//!
//! # This module counts nothing and authorises nothing
//!
//! It reads and writes rows. Whether a principal may open, review, or
//! publish is a Cedar decision at the seam the caller crossed to get
//! here (ADR-0030 decision 1); whether the approvals recorded here are
//! *enough* is [`synveda_types::ApprovalRequirement`]'s arithmetic.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{
    AssetKind, CastApproval, Channel, Error, IdentityId, ProposalId, ProposalState, Result, Role,
    ScopeId, Sensitivity, TenantId, Verdict,
};
use uuid::Uuid;

use crate::channels::ChannelMember;
use crate::commits::{NewCommit, commit};
use crate::hash::{CommitHash, ObjectHash};
use crate::policy::PolicySnapshot;
use crate::signer::CommitSigner;
use crate::storage_error;
use crate::trees::{TreeEntry, put_tree};

/// Counts proposal lifecycle acts, labelled by act and asset kind.
pub const PROPOSAL_ACTS_TOTAL: &str = "synveda_vedaflow_proposal_acts_total";

/// The most members one proposal may carry.
///
/// Mirrors the publish route's cap for the same reason: a proposal is a
/// reviewed act, and two hundred records in one review is a migration
/// wearing a reviewer's hat.
pub const MAX_PROPOSAL_MEMBERS: usize = 200;

/// The most proposals that may stand open at one scope.
///
/// A review queue nobody can drain is a denial of service against the
/// reviewers, and FLOW-4's rule engine will open proposals without a
/// human deciding to. Tripping this in normal use is ADR-0032's recorded
/// reversal trigger (a) — batching or expiry, before the cap moves.
pub const MAX_OPEN_PROPOSALS: i64 = 500;

/// A proposal as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewProposal<'a> {
    /// The scope whose channel would move — where requirements resolve.
    pub target_scope: ScopeId,
    /// Where the material lives now. FLOW-3 requires it to equal the
    /// target; FLOW-5's climb is what relaxes that.
    pub source_scope: ScopeId,
    /// Which asset type.
    pub asset: AssetKind,
    /// Which channel it would join.
    pub channel: Channel,
    /// The members, as `(entry name, content address)`.
    pub members: &'a [(String, ObjectHash)],
    /// The maximum sensitivity over the members.
    pub sensitivity: Sensitivity,
    /// What this proposes, in one line — a reviewer reads it in a list.
    pub title: &'a str,
    /// Who proposed it.
    pub proposer: IdentityId,
    /// The proposer's token subject.
    pub proposer_subject: &'a str,
    /// When, in valid-time terms.
    pub committed_at: DateTime<Utc>,
    /// The pack in force, as the caller resolved it.
    pub policy_snapshot: &'a PolicySnapshot,
}

/// A proposal as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProposal {
    /// Its identifier.
    pub id: ProposalId,
    /// The scope whose channel would move.
    pub target_scope_id: ScopeId,
    /// Where the material lives.
    pub source_scope_id: ScopeId,
    /// Which asset type.
    pub asset: AssetKind,
    /// Which channel it would join.
    pub channel: Channel,
    /// The commit holding exactly what is proposed.
    pub commit: CommitHash,
    /// The maximum sensitivity over its members.
    pub sensitivity: Sensitivity,
    /// The stored lifecycle — never `approved`, which is computed
    /// (ADR-0032 decision 11).
    pub state: ProposalState,
    /// What it proposes, in one line.
    pub title: String,
    /// Who proposed it.
    pub proposer_id: IdentityId,
    /// The proposer's token subject.
    pub proposer_subject: String,
    /// When it opened.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// When it closed.
    pub closed_at: Option<DateTime<Utc>>,
    /// Who closed it.
    pub closed_by: Option<IdentityId>,
    /// Why it closed — always present on a rejection.
    pub close_reason: Option<String>,
}

/// One recorded review act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredApproval {
    /// Who cast it.
    pub approver_id: IdentityId,
    /// Their token subject — what a curator file names.
    pub approver_subject: String,
    /// The commit they reviewed. Approvals bind bytes, so this is part of
    /// the key and can never be inherited by another content set.
    pub commit: CommitHash,
    /// Approve or reject.
    pub verdict: Verdict,
    /// The effective roles they held at the target scope when they cast
    /// it — recorded, never re-derived (ADR-0032 decision 5).
    pub roles: Vec<Role>,
    /// What they said.
    pub comment: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
}

impl StoredApproval {
    /// The matrix's view of this act. Rejections are not votes and never
    /// count toward a requirement — they close the proposal instead.
    #[must_use]
    pub fn cast(&self) -> Option<CastApproval> {
        (self.verdict == Verdict::Approve).then(|| CastApproval {
            identity: self.approver_id,
            subject: self.approver_subject.clone(),
            roles: self.roles.clone(),
        })
    }
}

/// The approvals of `approvals` that count toward a requirement, for the
/// commit under review.
#[must_use]
pub fn cast_for(approvals: &[StoredApproval], commit: CommitHash) -> Vec<CastApproval> {
    approvals
        .iter()
        .filter(|approval| approval.commit == commit)
        .filter_map(StoredApproval::cast)
        .collect()
}

/// A review act as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewApproval<'a> {
    /// Which proposal.
    pub proposal: ProposalId,
    /// The commit being reviewed — the caller passes the proposal's, and
    /// the primary key makes a stale one a distinct row rather than a
    /// silent overwrite.
    pub commit: CommitHash,
    /// Who is reviewing.
    pub approver: IdentityId,
    /// Their token subject.
    pub approver_subject: &'a str,
    /// Approve or reject.
    pub verdict: Verdict,
    /// The effective roles held at the target scope.
    pub roles: &'a [Role],
    /// What they said.
    pub comment: Option<&'a str>,
}

/// Opens a proposal: mints the root commit holding its members, then
/// records the row.
///
/// Both land in the caller's transaction, so a proposal whose commit
/// exists but whose row does not is unrepresentable.
#[tracing::instrument(
    name = "vedaflow.open_proposal",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %new.target_scope,
        vedaflow.asset = new.asset.as_str(),
        vedaflow.members = new.members.len(),
        vedaflow.proposal = tracing::field::Empty,
        vedaflow.commit = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn open(
    conn: &mut PgConnection,
    tenant: TenantId,
    new: &NewProposal<'_>,
    signer: &impl CommitSigner,
) -> Result<StoredProposal> {
    if new.members.is_empty() {
        return Err(Error::Invalid {
            message: "a proposal with no members proposes nothing".to_owned(),
        });
    }
    if new.members.len() > MAX_PROPOSAL_MEMBERS {
        return Err(Error::Invalid {
            message: format!(
                "a proposal carries at most {MAX_PROPOSAL_MEMBERS} members, got {}",
                new.members.len()
            ),
        });
    }
    let entries: Vec<TreeEntry> = new
        .members
        .iter()
        .map(|(name, hash)| TreeEntry::object(name, *hash))
        .collect();
    let tree = put_tree(conn, tenant, &entries).await?;
    let minted = commit(
        conn,
        tenant,
        &NewCommit {
            tree: tree.hash,
            // A root commit: a proposal is a statement about a set, not a
            // step in a channel's history. Publication is what joins it.
            parents: Vec::new(),
            author: new.proposer,
            message: new.title.to_owned(),
            committed_at: new.committed_at,
            policy_snapshot: new.policy_snapshot.clone(),
        },
        signer,
    )
    .await?;

    let id = ProposalId::new();
    let row = sqlx::query!(
        r#"insert into vedaflow_proposals
               (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
                target_channel, commit_hash, sensitivity, title, proposer_id,
                proposer_subject)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           returning created_at, updated_at"#,
        tenant.as_uuid(),
        id.as_uuid(),
        new.target_scope.as_uuid(),
        new.source_scope.as_uuid(),
        new.asset.as_str(),
        new.channel.as_str(),
        minted.hash.as_slice(),
        new.sensitivity.as_str(),
        new.title,
        new.proposer.as_uuid(),
        new.proposer_subject,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("open proposal", &err))?;

    let span = tracing::Span::current();
    span.record("vedaflow.proposal", id.to_string());
    span.record("vedaflow.commit", minted.hash.to_hex());
    act("opened", new.asset);
    Ok(StoredProposal {
        id,
        target_scope_id: new.target_scope,
        source_scope_id: new.source_scope,
        asset: new.asset,
        channel: new.channel,
        commit: minted.hash,
        sensitivity: new.sensitivity,
        state: ProposalState::Open,
        title: new.title.to_owned(),
        proposer_id: new.proposer,
        proposer_subject: new.proposer_subject.to_owned(),
        created_at: row.created_at,
        updated_at: row.updated_at,
        closed_at: None,
        closed_by: None,
        close_reason: None,
    })
}

/// Reads one proposal. `None` = no such proposal in this tenant.
#[tracing::instrument(name = "vedaflow.read_proposal", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn read(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: ProposalId,
) -> Result<Option<StoredProposal>> {
    sqlx::query_as!(
        ProposalRow,
        r#"select id, target_scope_id, source_scope_id, asset_kind, target_channel,
                  commit_hash, sensitivity, state, title, proposer_id, proposer_subject,
                  created_at, updated_at, closed_at, closed_by, close_reason
           from vedaflow_proposals
           where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read proposal", &err))?
    .map(StoredProposal::try_from)
    .transpose()
}

/// Which proposals a listing wants.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProposalFilter {
    /// Only proposals targeting this scope.
    pub target_scope: Option<ScopeId>,
    /// Only proposals in this state.
    pub state: Option<ProposalState>,
    /// At most this many, newest first.
    pub limit: i64,
}

/// Proposals matching `filter`, newest first.
#[tracing::instrument(name = "vedaflow.list_proposals", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn list(
    conn: &mut PgConnection,
    tenant: TenantId,
    filter: ProposalFilter,
) -> Result<Vec<StoredProposal>> {
    sqlx::query_as!(
        ProposalRow,
        r#"select id, target_scope_id, source_scope_id, asset_kind, target_channel,
                  commit_hash, sensitivity, state, title, proposer_id, proposer_subject,
                  created_at, updated_at, closed_at, closed_by, close_reason
           from vedaflow_proposals
           where tenant_id = $1
             and ($2::uuid is null or target_scope_id = $2)
             and ($3::text is null or state = $3)
           order by created_at desc, id desc
           limit $4"#,
        tenant.as_uuid(),
        filter.target_scope.map(|scope| scope.as_uuid()),
        filter.state.map(|state| state.as_str()),
        filter.limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("list proposals", &err))?
    .into_iter()
    .map(StoredProposal::try_from)
    .collect()
}

/// How many proposals stand open at one scope.
pub async fn count_open(conn: &mut PgConnection, tenant: TenantId, scope: ScopeId) -> Result<i64> {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from vedaflow_proposals
           where tenant_id = $1 and target_scope_id = $2 and state = 'open'"#,
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("count open proposals", &err))
}

/// One proposal's review acts, oldest first.
#[tracing::instrument(name = "vedaflow.read_approvals", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn approvals(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: ProposalId,
) -> Result<Vec<StoredApproval>> {
    let rows = sqlx::query!(
        "select approver_id, approver_subject, commit_hash, verdict, roles, comment, created_at
         from vedaflow_proposal_approvals
         where tenant_id = $1 and proposal_id = $2
         order by created_at, approver_id",
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read proposal approvals", &err))?;

    rows.into_iter()
        .map(|row| {
            Ok(StoredApproval {
                approver_id: IdentityId::from_uuid(row.approver_id),
                approver_subject: row.approver_subject,
                commit: CommitHash::from_slice(&row.commit_hash)?,
                verdict: row.verdict.parse().map_err(vocabulary)?,
                roles: row
                    .roles
                    .iter()
                    .map(|role| role.parse().map_err(vocabulary))
                    .collect::<Result<Vec<Role>>>()?,
                comment: row.comment,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// Records one review act.
///
/// A second act by the same identity on the same commit is a
/// [`Error::Conflict`], not an overwrite: a review log where a verdict
/// can be replaced is not a review log.
#[tracing::instrument(
    name = "vedaflow.record_approval",
    skip_all,
    fields(tenant.id = %tenant, vedaflow.verdict = new.verdict.as_str()),
    err(Display)
)]
pub async fn record_approval(
    conn: &mut PgConnection,
    tenant: TenantId,
    new: &NewApproval<'_>,
) -> Result<StoredApproval> {
    let roles: Vec<String> = new
        .roles
        .iter()
        .map(|role| role.as_str().to_owned())
        .collect();
    let row = sqlx::query!(
        r#"insert into vedaflow_proposal_approvals
               (tenant_id, proposal_id, approver_id, commit_hash, verdict, roles,
                approver_subject, comment)
           values ($1, $2, $3, $4, $5, $6, $7, $8)
           on conflict do nothing
           returning created_at"#,
        tenant.as_uuid(),
        new.proposal.as_uuid(),
        new.approver.as_uuid(),
        new.commit.as_slice(),
        new.verdict.as_str(),
        &roles,
        new.approver_subject,
        new.comment,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("record proposal approval", &err))?;

    let Some(row) = row else {
        return Err(Error::Conflict {
            message: format!(
                "this principal has already reviewed proposal {} at this commit",
                new.proposal
            ),
        });
    };
    Ok(StoredApproval {
        approver_id: new.approver,
        approver_subject: new.approver_subject.to_owned(),
        commit: new.commit,
        verdict: new.verdict,
        roles: new.roles.to_vec(),
        comment: new.comment.map(ToOwned::to_owned),
        created_at: row.created_at,
    })
}

/// Closes an open proposal.
///
/// Returns `false` when it was already closed — the compare-and-swap
/// shape the rest of this crate uses: losing to a concurrent reviewer is
/// a result the caller reports, not an error to log past. `state` must be
/// terminal; the database trigger refuses anything else regardless.
#[tracing::instrument(
    name = "vedaflow.close_proposal",
    skip_all,
    fields(tenant.id = %tenant, vedaflow.state = state.as_str()),
    err(Display)
)]
pub async fn close(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: ProposalId,
    state: ProposalState,
    by: IdentityId,
    reason: Option<&str>,
) -> Result<bool> {
    if !state.is_terminal() {
        return Err(Error::Invalid {
            message: format!("{state} is not a closed state"),
        });
    }
    let updated = sqlx::query!(
        "update vedaflow_proposals
         set state = $3, closed_at = now(), closed_by = $4, close_reason = $5,
             updated_at = now()
         where tenant_id = $1 and id = $2 and state = 'open'",
        tenant.as_uuid(),
        id.as_uuid(),
        state.as_str(),
        by.as_uuid(),
        reason,
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("close proposal", &err))?
    .rows_affected();
    Ok(updated == 1)
}

/// A proposal commit's members, in name order — what was proposed, at the
/// addresses it was proposed at.
#[tracing::instrument(name = "vedaflow.proposal_members", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn members(
    conn: &mut PgConnection,
    tenant: TenantId,
    commit: CommitHash,
) -> Result<Vec<ChannelMember>> {
    let rows = sqlx::query!(
        "select e.name, e.object_hash
         from vedaflow_commits c
         join vedaflow_tree_entries e
             on e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash
         where c.tenant_id = $1 and c.hash = $2
         order by e.name",
        tenant.as_uuid(),
        commit.as_slice(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read proposal members", &err))?;

    rows.into_iter()
        .map(|row| {
            let Some(object) = row.object_hash else {
                return Err(Error::Internal {
                    message: format!(
                        "proposal entry {:?} points at a subtree; this reader predates sharding",
                        row.name
                    ),
                });
            };
            Ok(ChannelMember {
                name: row.name,
                object: ObjectHash::from_slice(&object)?,
            })
        })
        .collect()
}

/// Counts one lifecycle act.
pub fn act(kind: &'static str, asset: AssetKind) {
    metrics::counter!(PROPOSAL_ACTS_TOTAL, "act" => kind, "asset" => asset.as_str()).increment(1);
}

/// The raw row every proposal query shares.
struct ProposalRow {
    id: Uuid,
    target_scope_id: Uuid,
    source_scope_id: Uuid,
    asset_kind: String,
    target_channel: String,
    commit_hash: Vec<u8>,
    sensitivity: String,
    state: String,
    title: String,
    proposer_id: Uuid,
    proposer_subject: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    closed_by: Option<Uuid>,
    close_reason: Option<String>,
}

impl TryFrom<ProposalRow> for StoredProposal {
    type Error = Error;

    fn try_from(row: ProposalRow) -> Result<Self> {
        Ok(StoredProposal {
            id: ProposalId::from_uuid(row.id),
            target_scope_id: ScopeId::from_uuid(row.target_scope_id),
            source_scope_id: ScopeId::from_uuid(row.source_scope_id),
            asset: row.asset_kind.parse().map_err(vocabulary)?,
            channel: row.target_channel.parse().map_err(vocabulary)?,
            commit: CommitHash::from_slice(&row.commit_hash)?,
            sensitivity: row.sensitivity.parse().map_err(vocabulary)?,
            state: row.state.parse().map_err(vocabulary)?,
            title: row.title,
            proposer_id: IdentityId::from_uuid(row.proposer_id),
            proposer_subject: row.proposer_subject,
            created_at: row.created_at,
            updated_at: row.updated_at,
            closed_at: row.closed_at,
            closed_by: row.closed_by.map(IdentityId::from_uuid),
            close_reason: row.close_reason,
        })
    }
}

/// The CHECK constraints keep these columns inside their vocabularies, so
/// a parse failure means schema and code have drifted — a bug to name,
/// not a row to skip.
fn vocabulary(err: Error) -> Error {
    Error::Internal {
        message: format!("stored value outside vocabulary: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(byte: u8, verdict: Verdict, commit: CommitHash) -> StoredApproval {
        StoredApproval {
            approver_id: IdentityId::from_uuid(Uuid::from_bytes([byte; 16])),
            approver_subject: format!("subject-{byte}"),
            commit,
            verdict,
            roles: vec![Role::Curator],
            comment: None,
            created_at: Utc::now(),
        }
    }

    fn hash(byte: u8) -> CommitHash {
        CommitHash::from_slice(&[byte; 32]).unwrap()
    }

    /// A rejection is not a vote: it closes the proposal, and counting it
    /// toward a requirement would let a "no" complete a review.
    #[test]
    fn only_approvals_count_toward_a_requirement() {
        assert!(approval(1, Verdict::Approve, hash(9)).cast().is_some());
        assert!(approval(1, Verdict::Reject, hash(9)).cast().is_none());
    }

    /// Approvals bind bytes: an approval of another commit is evidence
    /// about other content and never carries over.
    #[test]
    fn approvals_of_another_commit_do_not_count() {
        let recorded = vec![
            approval(1, Verdict::Approve, hash(1)),
            approval(2, Verdict::Approve, hash(2)),
            approval(3, Verdict::Reject, hash(1)),
        ];
        let counted = cast_for(&recorded, hash(1));
        assert_eq!(counted.len(), 1);
        assert_eq!(counted[0].subject, "subject-1");
    }

    #[test]
    fn a_close_must_name_a_terminal_state() {
        // The typed guard; the database trigger is the backstop that
        // holds for out-of-band writers too.
        assert!(!ProposalState::Open.is_terminal());
        for state in [
            ProposalState::Rejected,
            ProposalState::Withdrawn,
            ProposalState::Published,
        ] {
            assert!(state.is_terminal());
        }
    }
}
