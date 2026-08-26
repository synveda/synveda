//! Curator files: CODEOWNERS, generalised (tech plan §2.4; FLOW-3,
//! ADR-0032 decisions 13–15).
//!
//! A curator file names, per scope, who must **additionally** approve a
//! proposal touching matching assets. It is the per-scope half of the
//! approval matrix: the pack says what `restricted` Knowledge needs
//! anywhere, the file says that in *this* team a deployment procedure
//! also needs the person who owns deployments.
//!
//! # It adds requirements; it never grants authority
//!
//! This is the decision the whole module hangs on. Listing a subject
//! makes their approval *required*; it does not make them able to
//! approve. A named principal still has to pass `ProposalReview` at the
//! target scope, so a file naming someone the pack denies makes the
//! proposal unsatisfiable rather than making that person an approver.
//!
//! That is [`synveda_types::CompositionConfig`]'s rule (ADR-0025
//! decision 2 — "the config never grants") applied to the other side of
//! the trust boundary, and it is what keeps this file from becoming a
//! second authorisation system editable by whoever can write a file.
//!
//! # It is a VedaFlow asset, not a table
//!
//! An [`synveda_types::AssetKind::Policy`] object holding the file's
//! exact authored bytes, committed to a ref named [`CURATORS_REF`] at its
//! scope. So it inherits content addressing, immutable history, and a
//! recorded policy pack on every change: "who changed who must approve,
//! and when" is answered from the same tables as "who published this".
//!
//! Resolution up a chain is **nearest-ancestor-first** — the first scope
//! carrying a file wins outright, no union — matching pack assignment
//! (ADR-0014 decision 3). A union would make an org-wide file impossible
//! to narrow at a project, inverting how the scope tree works everywhere
//! else.
//!
//! # The format
//!
//! ```text
//! # Platform team curators (FLOW-3)
//! knowledge/*     @alice-subject role:administrator
//! skill/deploy-*  @bob-subject
//! *               @head-of-eng
//! ```
//!
//! One rule per line: a pattern, then one or more approvers. `@name` is a
//! token subject; `role:name` is a grant-key role (the one vocabulary, CPR-7).
//! `#` starts a comment. A pattern matches `{asset-kind}/{entry-name}`,
//! with `*` standing for any run of characters and no other metacharacter
//! — deliberately minimal, because entry names are record ids today and
//! real path semantics only start to mean something when SKIL-1 and
//! PRMT-1 bring path-named entries. The shape is accepted now so that
//! growth is not a format change.

use std::fmt;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::access::RoleKey;
use synveda_types::{
    ApprovalRequirement, AssetKind, Error, IdentityId, RequirementOrigin, Result, ScopeId, TenantId,
};

use crate::commits::{NewCommit, commit};
use crate::hash::{CommitHash, ObjectHash};
use crate::objects::{put_object, read_object};
use crate::policy::PolicySnapshot;
use crate::refs::{RefUpdate, create_ref, read_ref, update_ref};
use crate::signer::CommitSigner;
use crate::storage_error;
use crate::trees::{TreeEntry, put_tree};

/// The ref a scope's curator file lives under.
///
/// Not a channel name and it does not parse as one — ADR-0031 decision 1
/// kept `vedaflow_refs` generic for exactly this — so `GET /v1/channels`
/// skips it and the channel vocabulary stays two-segment.
pub const CURATORS_REF: &str = "curators";

/// The tree entry a curator file takes inside its commit.
pub const CURATORS_ENTRY: &str = "CURATORS";

/// The largest curator file accepted. A file nobody can read through is
/// not a review policy; sixteen kibibytes is thousands of rules.
pub const MAX_CURATOR_FILE_BYTES: usize = 16 * 1024;

/// The most rules one file may carry.
pub const MAX_CURATOR_RULES: usize = 200;

/// The most approvers one rule may name. Every named approver raises the
/// distinct-approver floor by one, so a rule naming more than this
/// describes a review that will never complete.
pub const MAX_RULE_APPROVERS: usize = 16;

/// Who a curator rule requires.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Approver {
    /// A named token subject — CODEOWNERS' `@user`.
    Subject(String),
    /// Anyone holding this role at the target scope.
    RoleKey(RoleKey),
}

impl fmt::Display for Approver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Approver::Subject(subject) => write!(f, "@{subject}"),
            Approver::RoleKey(role) => write!(f, "role:{role}"),
        }
    }
}

/// One line of a curator file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratorRule {
    /// The `{asset-kind}/{entry-name}` pattern, `*` as the one wildcard.
    pub pattern: String,
    /// Who this rule requires. Never empty — a rule requiring nobody is
    /// a typo, and parsing rejects it.
    pub approvers: Vec<Approver>,
}

impl CuratorRule {
    /// Whether this rule governs `path`.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        glob_matches(&self.pattern, path)
    }
}

/// A parsed curator file, holding the exact bytes it was authored as.
///
/// The source is kept verbatim rather than re-rendered: FLOW-6 diffs what
/// the author wrote, comments included, and a canonicalising round trip
/// would fight the author over formatting for no governance benefit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratorFile {
    source: String,
    rules: Vec<CuratorRule>,
}

impl CuratorFile {
    /// Parses a curator file.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the line, for an unparseable approver, a
    /// rule with no approvers, an empty pattern, or a file over the size
    /// or rule caps.
    pub fn parse(source: &str) -> Result<Self> {
        if source.len() > MAX_CURATOR_FILE_BYTES {
            return Err(Error::Invalid {
                message: format!(
                    "curator file is {} bytes, over the {MAX_CURATOR_FILE_BYTES} limit",
                    source.len()
                ),
            });
        }
        let mut rules = Vec::new();
        for (index, raw) in source.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let number = index + 1;
            let mut tokens = line.split_whitespace();
            let pattern = tokens.next().expect("a non-empty line has a first token");
            let approvers = tokens
                .map(|token| parse_approver(number, token))
                .collect::<Result<Vec<Approver>>>()?;
            if approvers.is_empty() {
                return Err(Error::Invalid {
                    message: format!(
                        "curator file line {number}: {pattern:?} names no approver; \
                         a rule that requires nobody requires nothing"
                    ),
                });
            }
            if approvers.len() > MAX_RULE_APPROVERS {
                return Err(Error::Invalid {
                    message: format!(
                        "curator file line {number}: {} approvers, over the \
                         {MAX_RULE_APPROVERS} limit",
                        approvers.len()
                    ),
                });
            }
            rules.push(CuratorRule {
                pattern: pattern.to_owned(),
                approvers,
            });
        }
        if rules.len() > MAX_CURATOR_RULES {
            return Err(Error::Invalid {
                message: format!(
                    "curator file holds {} rules, over the {MAX_CURATOR_RULES} limit",
                    rules.len()
                ),
            });
        }
        Ok(CuratorFile {
            source: source.to_owned(),
            rules,
        })
    }

    /// The file's exact authored bytes — what is hashed, stored, and
    /// diffed.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The parsed rules, in file order.
    #[must_use]
    pub fn rules(&self) -> &[CuratorRule] {
        &self.rules
    }

    /// Whether the file requires anything at all. An empty file is how a
    /// scope's curator requirements are cleared: refs hold no DELETE
    /// grant (migration 0018), so "no requirements here" is committed,
    /// not deleted — which also leaves the removal in the history.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Adds this file's requirements to `requirement`, for a proposal of
    /// `asset` whose tree names `entries`.
    ///
    /// Every rule matching *any* member contributes: a set is reviewed as
    /// a set, so one member owned by someone pulls that owner into the
    /// review of the whole set.
    pub fn apply(
        &self,
        scope_id: ScopeId,
        asset: AssetKind,
        entries: &[String],
        requirement: &mut ApprovalRequirement,
    ) {
        let origin = RequirementOrigin::Curators { scope_id };
        let paths: Vec<String> = entries
            .iter()
            .map(|entry| format!("{}/{entry}", asset.as_str()))
            .collect();
        for rule in &self.rules {
            if !paths.iter().any(|path| rule.matches(path)) {
                continue;
            }
            for approver in &rule.approvers {
                match approver {
                    Approver::Subject(subject) => requirement.require_subject(origin, subject),
                    Approver::RoleKey(role) => requirement.require_role(origin, *role),
                }
            }
        }
    }
}

fn parse_approver(line: usize, token: &str) -> Result<Approver> {
    if let Some(subject) = token.strip_prefix('@') {
        if subject.is_empty() || subject.len() > 255 {
            return Err(Error::Invalid {
                message: format!(
                    "curator file line {line}: {token:?} is not a subject (1..=255 characters)"
                ),
            });
        }
        return Ok(Approver::Subject(subject.to_owned()));
    }
    if let Some(role) = token.strip_prefix("role:") {
        return Ok(Approver::RoleKey(role.parse()?));
    }
    Err(Error::Invalid {
        message: format!("curator file line {line}: {token:?} is neither @subject nor role:name"),
    })
}

/// `*` matches any run of characters; every other character is literal.
///
/// Written out rather than pulled in: one wildcard is the whole language
/// (ADR-0032's accepted trade-off), and a glob crate would bring
/// filesystem semantics — `**`, character classes, path-separator rules —
/// that this format deliberately does not have.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let mut segments = pattern.split('*');
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(mut rest) = value.strip_prefix(first) else {
        return false;
    };
    let segments: Vec<&str> = segments.collect();
    let Some((last, middle)) = segments.split_last() else {
        // No wildcard at all: the prefix had to be the whole value.
        return rest.is_empty();
    };
    for segment in middle {
        match rest.find(segment) {
            Some(at) => rest = &rest[at + segment.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// A scope's curator file as stored: the file, and the history behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCuratorFile {
    /// The scope the file is committed at.
    pub scope_id: ScopeId,
    /// The parsed file.
    pub file: CuratorFile,
    /// The commit the `curators` ref points at.
    pub commit: CommitHash,
    /// The file's content address.
    pub object: ObjectHash,
    /// When the ref last moved.
    pub updated_at: DateTime<Utc>,
    /// Who last moved it.
    pub updated_by: IdentityId,
}

/// The curator file in force for a chain: the **nearest** scope on
/// `chain` that carries one, where `chain` runs nearest-first (the target
/// node, then its ancestors — HIER-2's order).
///
/// `None` means no scope on the chain has one, which requires nothing
/// extra of a proposal.
#[tracing::instrument(
    name = "vedaflow.nearest_curators",
    skip_all,
    fields(tenant.id = %tenant, scopes.count = chain.len()),
    err(Display)
)]
pub async fn nearest_curators(
    conn: &mut PgConnection,
    tenant: TenantId,
    chain: &[ScopeId],
) -> Result<Option<StoredCuratorFile>> {
    for scope in chain {
        if let Some(stored) = read_curators(conn, tenant, *scope).await? {
            return Ok(Some(stored));
        }
    }
    Ok(None)
}

/// One scope's curator file, if it has one.
#[tracing::instrument(
    name = "vedaflow.read_curators",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope),
    err(Display)
)]
pub async fn read_curators(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Option<StoredCuratorFile>> {
    let Some(reference) = read_ref(conn, tenant, scope, CURATORS_REF).await? else {
        return Ok(None);
    };
    let row = sqlx::query!(
        "select e.object_hash
         from vedaflow_commits c
         join vedaflow_tree_entries e
             on e.tenant_id = c.tenant_id and e.tree_hash = c.tree_hash
         where c.tenant_id = $1 and c.hash = $2 and e.name = $3",
        tenant.as_uuid(),
        reference.commit_hash.as_slice(),
        CURATORS_ENTRY,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read curator file entry", &err))?;

    // Only `write_curators` writes this ref, and it always writes the one
    // entry — so a commit without it means the schema and this code have
    // drifted, which is a bug to surface rather than a file to invent.
    let Some(object) = row.and_then(|row| row.object_hash) else {
        return Err(Error::Internal {
            message: format!("curators ref at scope {scope} names no {CURATORS_ENTRY} entry"),
        });
    };
    let address = ObjectHash::from_slice(&object)?;
    let stored = read_object(conn, tenant, address)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("curator file object {} is missing", address.to_hex()),
        })?;
    let source = String::from_utf8(stored.content).map_err(|err| Error::Internal {
        message: format!("curator file at scope {scope} is not UTF-8: {err}"),
    })?;
    Ok(Some(StoredCuratorFile {
        scope_id: scope,
        file: CuratorFile::parse(&source)?,
        commit: reference.commit_hash,
        object: address,
        updated_at: reference.updated_at,
        updated_by: reference.updated_by,
    }))
}

/// What a curator-file write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratorCommit {
    /// The commit the ref now points at.
    pub commit: CommitHash,
    /// What it pointed at before — `None` on the scope's first file.
    pub parent: Option<CommitHash>,
    /// The file's content address.
    pub object: ObjectHash,
    /// Whether the bytes were already stored — a re-commit of an
    /// unchanged file, which still records who re-asserted it and when.
    pub unchanged: bool,
}

/// A curator-file edit, as the caller describes it.
#[derive(Debug, Clone)]
pub struct CuratorWrite<'a> {
    /// The scope whose file this is.
    pub scope: ScopeId,
    /// The file to commit. An empty one is how requirements are cleared:
    /// refs hold no DELETE grant, so "nothing required here" is
    /// committed, which also leaves the removal in the history.
    pub file: &'a CuratorFile,
    /// Who is editing.
    pub author: IdentityId,
    /// Why — an auditor reads this.
    pub message: &'a str,
    /// When, in valid-time terms.
    pub committed_at: DateTime<Utc>,
    /// The pack in force, as the caller resolved it.
    pub policy_snapshot: &'a PolicySnapshot,
}

/// Commits a curator file, fast-forwarding the scope's `curators` ref.
///
/// Compare-and-swap once, not in a loop: two administrators editing one scope's
/// curator file in the same instant is a collision to report, not a race
/// to paper over — unlike the derived channel, where concurrent writers
/// are the steady state.
#[tracing::instrument(
    name = "vedaflow.write_curators",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %write.scope,
        vedaflow.rules = write.file.rules().len(),
        vedaflow.commit = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn write_curators(
    conn: &mut PgConnection,
    tenant: TenantId,
    write: &CuratorWrite<'_>,
    signer: &impl CommitSigner,
) -> Result<CuratorCommit> {
    let CuratorWrite {
        scope,
        file,
        author,
        message,
        committed_at,
        policy_snapshot,
    } = *write;
    let head = read_ref(conn, tenant, scope, CURATORS_REF).await?;
    let object = put_object(conn, tenant, AssetKind::Policy, file.source().as_bytes()).await?;
    let tree = put_tree(
        conn,
        tenant,
        &[TreeEntry::object(CURATORS_ENTRY, object.hash)],
    )
    .await?;
    let minted = commit(
        conn,
        tenant,
        &NewCommit {
            tree: tree.hash,
            parents: head.iter().map(|head| head.commit_hash).collect(),
            author,
            message: message.to_owned(),
            committed_at,
            policy_snapshot: policy_snapshot.clone(),
        },
        signer,
    )
    .await?;

    let outcome = match &head {
        None => create_ref(conn, tenant, scope, CURATORS_REF, minted.hash, author).await?,
        Some(head) => {
            update_ref(
                conn,
                tenant,
                scope,
                CURATORS_REF,
                head.commit_hash,
                minted.hash,
                author,
            )
            .await?
        }
    };
    if outcome != RefUpdate::Updated {
        return Err(Error::Conflict {
            message: format!(
                "the curator file at scope {scope} moved while this edit was being written; \
                 re-read it and retry"
            ),
        });
    }
    tracing::Span::current().record("vedaflow.commit", minted.hash.to_hex());
    Ok(CuratorCommit {
        commit: minted.hash,
        parent: head.map(|head| head.commit_hash),
        object: object.hash,
        unchanged: object.deduplicated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::scope::ScopeKind;
    use synveda_types::{ApprovalMatrix, Sensitivity};
    use uuid::Uuid;

    const SAMPLE: &str = "\
# Platform team curators (FLOW-3)
knowledge/*      @alice role:administrator
skill/deploy-*   @bob

*                @head-of-eng   # everything, always
";

    fn scope() -> ScopeId {
        ScopeId::from_uuid(Uuid::from_bytes([4; 16]))
    }

    #[test]
    fn a_file_parses_into_rules_and_keeps_its_bytes() {
        let file = CuratorFile::parse(SAMPLE).unwrap();
        assert_eq!(file.source(), SAMPLE, "the authored bytes are the asset");
        assert_eq!(file.rules().len(), 3);
        assert_eq!(file.rules()[0].pattern, "knowledge/*");
        assert_eq!(
            file.rules()[0].approvers,
            vec![
                Approver::Subject("alice".to_owned()),
                Approver::RoleKey(RoleKey::Administrator)
            ]
        );
        assert_eq!(file.rules()[2].pattern, "*");
    }

    #[test]
    fn comments_and_blank_lines_are_not_rules() {
        let file = CuratorFile::parse("# nothing here\n\n   \n").unwrap();
        assert!(file.is_empty(), "an empty file requires nothing");
    }

    #[test]
    fn a_rule_naming_nobody_is_a_parse_error() {
        assert!(CuratorFile::parse("knowledge/*").is_err());
        assert!(CuratorFile::parse("knowledge/*   # @alice").is_err());
    }

    #[test]
    fn approvers_are_subjects_or_roles_and_nothing_else() {
        assert!(CuratorFile::parse("knowledge/* alice").is_err(), "no sigil");
        assert!(
            CuratorFile::parse("knowledge/* role:wizard").is_err(),
            "no such role"
        );
        assert!(
            CuratorFile::parse("knowledge/* @").is_err(),
            "empty subject"
        );
        assert!(CuratorFile::parse("knowledge/* role:administrator").is_ok());
        // The binding vocabulary fails by name (CPR-7, ADR-0074 decision 6).
        assert!(CuratorFile::parse("knowledge/* role:steward").is_err());
    }

    #[test]
    fn caps_are_enforced_on_size_rules_and_approvers() {
        let big = "x".repeat(MAX_CURATOR_FILE_BYTES + 1);
        assert!(CuratorFile::parse(&big).is_err());

        let many_rules = (0..=MAX_CURATOR_RULES)
            .map(|index| format!("knowledge/{index} @alice"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(CuratorFile::parse(&many_rules).is_err());

        let many_approvers = format!(
            "knowledge/* {}",
            (0..=MAX_RULE_APPROVERS)
                .map(|index| format!("@person-{index}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(CuratorFile::parse(&many_approvers).is_err());
    }

    #[test]
    fn the_one_wildcard_anchors_at_both_ends() {
        assert!(glob_matches("knowledge/*", "knowledge/abc"));
        assert!(glob_matches("knowledge/*", "knowledge/"));
        assert!(!glob_matches("knowledge/*", "prompt/abc"));
        assert!(glob_matches("*", "anything/at/all"));
        assert!(glob_matches("skill/deploy-*", "skill/deploy-prod"));
        assert!(!glob_matches("skill/deploy-*", "skill/build-prod"));
        assert!(glob_matches("knowledge/*-prod", "knowledge/deploy-prod"));
        assert!(!glob_matches("knowledge/*-prod", "knowledge/deploy-prod-2"));
        assert!(glob_matches("a*b*c", "axxbyyc"));
        assert!(!glob_matches("a*b*c", "axxbyy"));
        // No wildcard: exact match, not prefix match.
        assert!(glob_matches("knowledge/abc", "knowledge/abc"));
        assert!(!glob_matches("knowledge/abc", "knowledge/abcd"));
        // Overlapping segments must not be counted twice.
        assert!(!glob_matches("a*a*a", "aa"));
    }

    /// The decision-13 property: the file adds to the resolved
    /// requirement and never replaces it.
    #[test]
    fn a_file_adds_to_the_matrix_it_composes_with() {
        let mut requirement = ApprovalMatrix::empty().resolve(
            AssetKind::Knowledge,
            Sensitivity::Restricted,
            ScopeKind::OrgUnit,
        );
        let floor_roles = requirement.roles.clone();
        CuratorFile::parse(SAMPLE).unwrap().apply(
            scope(),
            AssetKind::Knowledge,
            &["abc".to_owned()],
            &mut requirement,
        );
        for required in &floor_roles {
            assert!(requirement.roles.contains(required), "the floor survives");
        }
        assert_eq!(
            requirement.subjects,
            vec!["alice".to_owned(), "head-of-eng".to_owned()],
            "both matching rules contributed"
        );
        assert!(
            requirement
                .origins
                .contains(&RequirementOrigin::Curators { scope_id: scope() })
        );
        // Two named subjects means at least two distinct approvers.
        assert!(requirement.distinct_approvers >= 2);
    }

    /// A rule matching any one member governs the whole set: a set is
    /// reviewed as a set.
    #[test]
    fn one_matching_member_pulls_its_owner_into_the_whole_review() {
        let file = CuratorFile::parse("knowledge/deploy-* @bob\n").unwrap();
        let mut requirement = ApprovalRequirement::default();
        file.apply(
            scope(),
            AssetKind::Knowledge,
            &["unrelated".to_owned(), "deploy-runbook".to_owned()],
            &mut requirement,
        );
        assert_eq!(requirement.subjects, vec!["bob".to_owned()]);

        let mut none = ApprovalRequirement::default();
        file.apply(
            scope(),
            AssetKind::Knowledge,
            &["unrelated".to_owned()],
            &mut none,
        );
        assert!(none.is_empty());
    }

    /// Patterns are matched against `{asset-kind}/{entry}`, so a rule for
    /// skills never governs Knowledge.
    #[test]
    fn patterns_are_scoped_by_asset_kind() {
        let file = CuratorFile::parse("skill/* @security\n").unwrap();
        let mut requirement = ApprovalRequirement::default();
        file.apply(
            scope(),
            AssetKind::Knowledge,
            &["anything".to_owned()],
            &mut requirement,
        );
        assert!(requirement.is_empty());

        let mut skills = ApprovalRequirement::default();
        file.apply(
            scope(),
            AssetKind::Skill,
            &["anything".to_owned()],
            &mut skills,
        );
        assert_eq!(skills.subjects, vec!["security".to_owned()]);
    }
}
