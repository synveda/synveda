//! FLOW-8 acceptance against native Git, including an ordinary clone.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use chrono::{Duration, TimeZone, Utc};
use synveda_git::{
    CommitEvidence, EntryEvidence, LocalTransport, ObjectEvidence, PinEvidence, Projection,
    Snapshot, Transport, TreeEvidence, verify_repository,
};
use synveda_types::{
    AssetKind, Channel, IdentityId, PackDocument, PromptTemplate, ScopeId, Sensitivity, TenantId,
};
use synveda_vedaflow::{
    CommitHash, CommitSigner, ContextPackAsset, Ed25519Signer, PolicySnapshot, PromptAsset,
    StoredCommit, TreeEntry,
};

fn private_root() -> tempfile::TempDir {
    let root = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn snapshot(asset: AssetKind, signed: bool) -> (Snapshot, BTreeMap<String, [u8; 32]>) {
    let tenant = "00000000-0000-0000-0000-000000000001"
        .parse::<TenantId>()
        .unwrap();
    let scope = "00000000-0000-0000-0000-000000000002"
        .parse::<ScopeId>()
        .unwrap();
    let author = "00000000-0000-0000-0000-000000000003"
        .parse::<IdentityId>()
        .unwrap();
    let at = Utc.timestamp_opt(1_700_000_000, 123_456_000).unwrap();
    let signer = Ed25519Signer::new([42; 32], "fixture").unwrap();
    let mut objects = Vec::new();
    let mut trees = Vec::new();
    for text in ["First source text.", "Second source text."] {
        let (name, bytes) = match asset {
            AssetKind::Prompt => {
                let source = PromptAsset {
                    scope_id: scope,
                    sensitivity: Sensitivity::Internal,
                    template: PromptTemplate {
                        name: "support/reply".parse().unwrap(),
                        description: "Fixture".into(),
                        template: text.into(),
                        variables: vec![],
                    },
                };
                (source.entry_name(), source.canonical_bytes())
            }
            AssetKind::ContextPack => {
                // Valid source names can exceed NAME_MAX after encoding.
                let source = ContextPackAsset {
                    scope_id: scope,
                    pack: "x".repeat(64).parse().unwrap(),
                    sensitivity: Sensitivity::Internal,
                    document: PackDocument {
                        name: "d".repeat(64).parse().unwrap(),
                        title: "Fixture".into(),
                        content: text.into(),
                    },
                };
                (source.entry_name(), source.canonical_bytes())
            }
            _ => unreachable!(),
        };
        let hash = synveda_vedaflow::hash::object_hash(asset, &bytes);
        let tree = synveda_vedaflow::verify::recompute_tree(&[TreeEntry::object(&name, hash)]);
        objects.push(ObjectEvidence {
            hash: hash.to_hex(),
            kind: asset,
            content: String::from_utf8(bytes).unwrap(),
        });
        trees.push(TreeEvidence {
            hash: tree.to_hex(),
            entries: vec![EntryEvidence {
                name,
                object: hash.to_hex(),
            }],
        });
    }
    let policy = PolicySnapshot::new("standard", 1).hash().unwrap();
    let mut commits = Vec::new();
    let mut add = |tree: usize, parents: Vec<CommitHash>, number: i64| {
        let mut source = StoredCommit {
            hash: CommitHash::from_bytes([0; 32]),
            tree: trees[tree].hash.parse().unwrap(),
            parents,
            author,
            message: format!("Source {number}\nExact message."),
            committed_at: at + Duration::seconds(number),
            policy_snapshot_hash: policy,
            signature: None,
        };
        source.hash = synveda_vedaflow::verify::recompute_commit(&source);
        if signed {
            source.signature = signer.sign(source.hash);
        }
        commits.push(CommitEvidence::from_stored(&source));
        source.hash
    };
    let root = add(0, vec![], 0);
    let left = add(0, vec![root], 1);
    let right = add(1, vec![root], 2);
    let merge = add(1, vec![right, left], 3);
    commits.sort_by(|a, b| a.hash.cmp(&b.hash));
    trees.sort_by(|a, b| a.hash.cmp(&b.hash));
    objects.sort_by(|a, b| a.hash.cmp(&b.hash));
    let keys = if signed {
        BTreeMap::from([("fixture".into(), signer.verifying_key())])
    } else {
        BTreeMap::new()
    };
    (
        Snapshot {
            tenant,
            scope,
            target: "review".into(),
            asset,
            channel: Channel::Published,
            head: merge.to_hex(),
            updated_at: at + Duration::seconds(5),
            updated_by: author,
            pin: Some(PinEvidence {
                commit: root.to_hex(),
                pinned_at: at + Duration::seconds(4),
                pinned_by: author,
            }),
            commits,
            trees,
            objects,
        },
        keys,
    )
}

async fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = tokio::process::Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[tokio::test]
async fn native_git_preserves_both_families_topology_signatures_and_clone() {
    for asset in [AssetKind::Prompt, AssetKind::ContextPack] {
        for signed in [false, true] {
            let (source, keys) = snapshot(asset, signed);
            let projection = Projection::render(source.clone(), None).unwrap();
            let repeated = Projection::render(source.clone(), None).unwrap();
            assert_eq!(projection.head, repeated.head);
            assert_eq!(
                serde_json::to_vec(&projection.manifest).unwrap(),
                serde_json::to_vec(&repeated.manifest).unwrap()
            );
            let root = private_root();
            let transport = LocalTransport::new(root.path().to_owned()).unwrap();
            assert!(transport.advance(&projection, None).await.unwrap());
            assert!(!transport.advance(&repeated, None).await.unwrap());
            let repo = transport
                .repository(source.tenant, source.scope, &source.target)
                .unwrap();
            git(&repo, &["fsck", "--strict", "--no-dangling"]).await;
            let actual = verify_repository(&repo, &source.git_ref(), &keys)
                .await
                .unwrap();
            assert_eq!(actual.mapping_digest, projection.manifest.mapping_digest);
            for commit in &source.commits {
                let oid = &actual.commits[&commit.hash];
                let bytes = git(&repo, &["cat-file", "commit", oid]).await;
                let text = String::from_utf8(bytes).unwrap();
                let parents = text
                    .lines()
                    .filter_map(|l| l.strip_prefix("parent "))
                    .collect::<Vec<_>>();
                assert_eq!(
                    parents,
                    commit
                        .parents
                        .iter()
                        .map(|p| actual.commits[p].as_str())
                        .collect::<Vec<_>>()
                );
                assert!(text.contains(&format!("{}@identity.synveda.invalid", commit.author)));
                assert!(text.contains(&format!(" {} +0000\n", commit.committed_at.timestamp())));
                assert!(text.ends_with(&commit.message));
                assert!(!text.contains("gpgsig "));
                let stored: CommitEvidence = serde_json::from_slice(
                    &git(&repo, &["show", &format!("{oid}:.synveda/commit.json")]).await,
                )
                .unwrap();
                assert_eq!(stored.signature, commit.signature);
                assert_eq!(stored.committed_at, commit.committed_at);
            }
            for object in &source.objects {
                assert_eq!(
                    git(&repo, &["cat-file", "blob", &actual.objects[&object.hash]]).await,
                    object.content.as_bytes()
                );
            }
            let checkout = root.path().join("clone");
            let cloned = tokio::process::Command::new("git")
                .args(["clone", "--quiet", "--no-local"])
                .arg(&repo)
                .arg(&checkout)
                .output()
                .await
                .unwrap();
            assert!(cloned.status.success(), "clone failed");
            let verified = verify_repository(&checkout, &source.git_ref(), &keys)
                .await
                .unwrap();
            assert_eq!(verified.snapshot.head, source.head);
            assert!(checkout.join(".synveda/export.json").is_file());
            if signed {
                assert!(
                    verify_repository(&repo, &source.git_ref(), &BTreeMap::new())
                        .await
                        .is_err()
                );
                let wrong = BTreeMap::from([(
                    "fixture".into(),
                    Ed25519Signer::new([24; 32], "fixture")
                        .unwrap()
                        .verifying_key(),
                )]);
                assert!(
                    verify_repository(&repo, &source.git_ref(), &wrong)
                        .await
                        .is_err()
                );
            }
        }
    }
}

#[tokio::test]
async fn rollback_pin_and_resume_advance_without_rewriting_source_history() {
    let (source, keys) = snapshot(AssetKind::Prompt, false);
    let first = Projection::render(source.clone(), None).unwrap();
    let root = private_root();
    let transport = LocalTransport::new(root.path().to_owned()).unwrap();
    transport.advance(&first, None).await.unwrap();
    let mut rolled_back = source.clone();
    rolled_back.head = source.pin.as_ref().unwrap().commit.clone();
    rolled_back.pin.as_mut().unwrap().commit = source.head.clone();
    rolled_back.pin.as_mut().unwrap().pinned_at += Duration::seconds(10);
    rolled_back.updated_at += Duration::seconds(10);
    let next = Projection::render(rolled_back, Some(first.head.clone())).unwrap();
    assert_eq!(next.manifest.commits, first.manifest.commits);
    assert!(transport.advance(&next, Some(&first.head)).await.unwrap());
    // The Git effect succeeded, but the application still holds its prepared receipt.
    assert!(!transport.advance(&next, Some(&first.head)).await.unwrap());
    let repo = transport
        .repository(source.tenant, source.scope, &source.target)
        .unwrap();
    git(
        &repo,
        &["merge-base", "--is-ancestor", &first.head, &next.head],
    )
    .await;
    let manifest = verify_repository(&repo, &source.git_ref(), &keys)
        .await
        .unwrap();
    assert_eq!(
        manifest.previous_state.as_deref(),
        Some(first.head.as_str())
    );
    assert_eq!(manifest.snapshot.pin.unwrap().commit, source.head);
    // Force-pushing a valid, retained source commit is still a divergence.
    git(
        &repo,
        &[
            "update-ref",
            &source.git_ref(),
            &first.manifest.commits[&source.head],
        ],
    )
    .await;
    assert!(matches!(
        transport.advance(&next, Some(&first.head)).await,
        Err(synveda_types::Error::Conflict { .. })
    ));
    git(&repo, &["update-ref", "-d", &source.git_ref()]).await;
    assert!(transport.advance(&next, Some(&first.head)).await.is_err());
    git(
        &repo,
        &["symbolic-ref", &source.git_ref(), "refs/heads/missing"],
    )
    .await;
    // A dangling symbolic ref has no objectname; it must still refuse even
    // for an exporter whose recorded branch is absent.
    assert!(transport.advance(&first, None).await.is_err());
    assert_eq!(
        git(&repo, &["symbolic-ref", &source.git_ref()]).await,
        b"refs/heads/missing\n"
    );
    let mut independent = source.clone();
    independent.target = "review-new".into();
    assert!(
        transport
            .advance(&Projection::render(independent, None).unwrap(), None)
            .await
            .unwrap()
    );
}

#[test]
fn malformed_or_unbounded_source_evidence_is_refused() {
    let (source, _) = snapshot(AssetKind::Prompt, false);
    let mut candidates = Vec::new();
    let mut wrong = source.clone();
    wrong.objects[0].content.push(' ');
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.commits[0].message.push('!');
    candidates.push(wrong);
    let mut wrong = source.clone();
    let hash = wrong.commits[0].hash.clone();
    wrong.commits[0].parents.push(hash);
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.trees[0].entries[0].name = "../../escape".into();
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.commits.push(wrong.commits[0].clone());
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.asset = AssetKind::Configuration;
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.objects[0].content = "x".repeat(synveda_git::MAX_BYTES + 1);
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.commits[0].signature = Some(vec![0; 64]);
    candidates.push(wrong);
    let mut wrong = source.clone();
    wrong.head = "00".repeat(32);
    candidates.push(wrong);
    for wrong in candidates {
        assert!(Projection::render(wrong, None).is_err());
    }
    for target in [
        "../escape",
        "https://token@host",
        "-option",
        "",
        "two/paths",
        "Upper",
    ] {
        assert!(synveda_git::validate_target(target).is_err());
    }
    assert!(Projection::render(source, Some("bad-parent".into())).is_err());
}

#[tokio::test]
async fn custody_refuses_symlinks_open_permissions_and_retained_non_repositories() {
    let root = private_root();
    let alias = root.path().join("alias");
    symlink(root.path(), &alias).unwrap();
    assert!(LocalTransport::new(alias).is_err());
    assert!(LocalTransport::new("relative".into()).is_err());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(LocalTransport::new(root.path().to_owned()).is_err());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let transport = LocalTransport::new(root.path().to_owned()).unwrap();
    let (source, _) = snapshot(AssetKind::Prompt, false);
    let projection = Projection::render(source.clone(), None).unwrap();
    let repo = transport
        .repository(source.tenant, source.scope, &source.target)
        .unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    for dir in [
        &repo,
        repo.parent().unwrap(),
        repo.parent().unwrap().parent().unwrap(),
    ] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(repo.join("retained"), b"retain operator data").unwrap();
    assert!(transport.advance(&projection, None).await.is_err());
    assert_eq!(
        std::fs::read(repo.join("retained")).unwrap(),
        b"retain operator data"
    );
    std::fs::remove_file(repo.join("retained")).unwrap();
    transport.advance(&projection, None).await.unwrap();
    std::fs::remove_file(repo.join("HEAD")).unwrap();
    assert!(
        transport
            .advance(&projection, Some(&projection.head))
            .await
            .is_err()
    );
    assert!(!repo.join("HEAD").exists());
}

#[tokio::test]
async fn verifier_requires_trusted_signatures_in_retained_rollback_history() {
    let (old, keys) = snapshot(AssetKind::Prompt, true);
    let first = Projection::render(old.clone(), None).unwrap();
    let root = private_root();
    let transport = LocalTransport::new(root.path().to_owned()).unwrap();
    transport.advance(&first, None).await.unwrap();
    let source = old.commits.iter().find(|c| c.parents.is_empty()).unwrap();
    let mut commit = StoredCommit {
        hash: CommitHash::from_bytes([0; 32]),
        tree: source.tree.parse().unwrap(),
        parents: vec![],
        author: source.author,
        message: "New unsigned independent source".into(),
        committed_at: source.committed_at + Duration::days(1),
        policy_snapshot_hash: source.policy_snapshot_hash.parse().unwrap(),
        signature: None,
    };
    commit.hash = synveda_vedaflow::verify::recompute_commit(&commit);
    let mut new = old.clone();
    new.head = commit.hash.to_hex();
    new.pin = None;
    new.updated_at = commit.committed_at;
    new.commits = vec![CommitEvidence::from_stored(&commit)];
    new.trees.retain(|t| t.hash == source.tree);
    new.objects
        .retain(|o| new.trees[0].entries.iter().any(|e| e.object == o.hash));
    let next = Projection::render(new, Some(first.head.clone())).unwrap();
    transport.advance(&next, Some(&first.head)).await.unwrap();
    let repo = transport
        .repository(old.tenant, old.scope, &old.target)
        .unwrap();
    assert!(
        verify_repository(&repo, &old.git_ref(), &BTreeMap::new())
            .await
            .is_err()
    );
    assert!(
        verify_repository(&repo, &old.git_ref(), &keys)
            .await
            .is_ok()
    );
}
