//! Controlled HTTPS provider metadata plus the real Git HTTP backend. DNS
//! overrides and test CA trust are confined to this private unit-test module.

use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use chrono::{TimeZone, Utc};
use serde_json::json;
use synveda_types::{AssetKind, Channel, IdentityId, PackDocument, PromptTemplate, Sensitivity};
use synveda_vedaflow::{
    CommitHash, ContextPackAsset, PolicySnapshot, PromptAsset, StoredCommit, TreeEntry,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::task::JoinSet;

use crate::{
    CommitEvidence, EntryEvidence, ObjectEvidence, Snapshot, TreeEvidence, verify_repository,
};

const TOKEN: &str = "github_pat_fixture_source_secret";
const REFERENCE: &str = "refs/heads/synveda/prompt/published";

#[derive(Clone)]
enum Mode {
    Normal,
    HttpFlush,
    Public,
    Replaced,
    Archived,
    Fork,
    Redirect,
    Oversize,
    Forbidden,
    Outage,
    Stall,
    DropAfterPush,
    Race(String),
    MissingReport,
}

struct Fixture {
    _root: Arc<tempfile::TempDir>,
    repo: PathBuf,
    target: GitHubTarget,
    credential: GitHubCredential,
    transport: GitHubTransport,
    mode: Arc<Mutex<Mode>>,
    pushes: Arc<AtomicUsize>,
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .current_dir(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "native fixture Git command {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

impl Fixture {
    async fn new() -> Self {
        let root = Arc::new(tempfile::tempdir().unwrap());
        let repo = root.path().join("fixture-owner/export.git");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--bare", "--template=", "."]).await;
        git(&repo, &["config", "http.receivepack", "true"]).await;
        git(&repo, &["config", "receive.denyNonFastForwards", "true"]).await;
        let conf = root.path().join("tls.conf");
        std::fs::write(&conf, "[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=github.com\n[ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:github.com,DNS:api.github.com\n").unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        let generated = Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-config",
            ])
            .arg(&conf)
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(
            generated.success(),
            "OpenSSL fixture certificate generation failed"
        );
        let cert = std::fs::read(cert).unwrap();
        let pkcs8 = root.path().join("pkcs8.pem");
        assert!(
            Command::new("openssl")
                .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
                .arg(&key)
                .arg("-out")
                .arg(&pkcs8)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap()
                .success()
        );
        let identity =
            native_tls::Identity::from_pkcs8(&cert, &std::fs::read(pkcs8).unwrap()).unwrap();
        let acceptor =
            tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = client_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&cert).unwrap())
            .resolve("api.github.com", address)
            .resolve("github.com", address)
            .build()
            .unwrap();
        let target = GitHubTarget {
            tenant_id: TenantId::new(),
            scope_id: ScopeId::new(),
            target: "review".into(),
            owner: "fixture-owner".into(),
            repository: "export".into(),
            repository_id: 123,
            secret_reference: synveda_types::secret::tenant_secret_reference(TenantSecretId::new()),
        };
        let credential = credential(123);
        let transport = GitHubTransport {
            targets: vec![target.clone()],
            client,
        };
        let mode = Arc::new(Mutex::new(Mode::Normal));
        let pushes = Arc::new(AtomicUsize::new(0));
        let (server_root, server_repo, server_mode, server_pushes) =
            (root.clone(), repo.clone(), mode.clone(), pushes.clone());
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (stream, _) = connection.unwrap();
                        let (acceptor, root, repo, mode, pushes) = (acceptor.clone(), server_root.clone(), server_repo.clone(), server_mode.clone(), server_pushes.clone());
                        connections.spawn(async move {
                            if let Ok(stream) = acceptor.accept(stream).await {
                                tokio::time::timeout(Duration::from_secs(5), serve(stream, root.path(), &repo, mode, pushes)).await.ok();
                            }
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                }
            }
        });
        let probe = transport
            .client
            .get(target.api_url())
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("controlled provider TLS handshake");
        assert_eq!(probe.status(), StatusCode::OK);
        Self {
            _root: root,
            repo,
            target,
            credential,
            transport,
            mode,
            pushes,
            address,
            task,
        }
    }

    fn mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }
    async fn advance(
        &self,
        projection: &Projection,
        old: Option<&str>,
        completed: bool,
    ) -> Result<bool> {
        self.transport
            .advance(&self.target, &self.credential, projection, old, completed)
            .await
    }
}

fn credential(id: u64) -> GitHubCredential {
    GitHubCredential::parse(
        &serde_json::to_vec(&json!({ "format": "synveda-github-export-credential-v1",
        "repository_id": id, "token": TOKEN }))
        .unwrap(),
        id,
    )
    .unwrap()
}

async fn serve(
    mut stream: tokio_native_tls::TlsStream<TcpStream>,
    root: &Path,
    repo: &Path,
    mode: Arc<Mutex<Mode>>,
    pushes: Arc<AtomicUsize>,
) {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break at + 4;
        }
        assert!(bytes.len() <= 16384);
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .unwrap()
        .to_owned();
    let request = headers.lines().next().unwrap();
    let mut parts = request.split(' ');
    let method = parts.next().unwrap();
    let uri = parts.next().unwrap();
    let header = |name: &str| {
        headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    };
    let length: usize = header("content-length").unwrap_or("0").parse().unwrap();
    assert!(length <= crate::MAX_BYTES * 8);
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let mode = mode.lock().unwrap().clone();
    if matches!(mode, Mode::Outage) {
        return;
    }
    if matches!(mode, Mode::Stall) {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if uri == "/repos/fixture-owner/export" {
        assert_eq!(header("host"), Some("api.github.com"));
        assert_eq!(
            header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        let (status, extra, body) = match mode {
            Mode::Redirect => (302, "Location: https://untrusted.invalid/secret\r\n", b"source-secret".to_vec()),
            Mode::Oversize => (200, "", vec![b'x'; wire::MAX_RESPONSE + 1]),
            Mode::Forbidden => (403, "", b"source-secret".to_vec()),
            _ => (200, "", serde_json::to_vec(&json!({ "id": if matches!(mode, Mode::Replaced) { 124 } else { 123 },
                "full_name": "fixture-owner/export", "private": !matches!(mode, Mode::Public),
                "visibility": if matches!(mode, Mode::Public) { "public" } else { "private" },
                "archived": matches!(mode, Mode::Archived), "disabled": false, "fork": matches!(mode, Mode::Fork) })).unwrap()),
        };
        reply(&mut stream, status, "application/json", extra, &body).await;
        return;
    }
    assert_eq!(header("host"), Some("github.com"));
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{TOKEN}"))
    );
    assert_eq!(header("authorization"), Some(auth.as_str()));
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    assert!(matches!(
        (method, path, query),
        (
            "GET",
            "/fixture-owner/export.git/info/refs",
            "service=git-receive-pack"
        ) | ("POST", "/fixture-owner/export.git/git-receive-pack", "")
    ));
    if method == "POST" {
        pushes.fetch_add(1, Ordering::SeqCst);
        if let Mode::Race(ref oid) = mode {
            git(repo, &["update-ref", REFERENCE, oid]).await;
        }
    }
    let mut child = Command::new("git")
        .arg("http-backend")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", "fixture")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", header("content-type").unwrap_or(""))
        .env("CONTENT_LENGTH", length.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(&bytes[header_end..header_end + length])
        .await
        .unwrap();
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success(), "native Git HTTP backend failed");
    let end = output
        .stdout
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let cgi = std::str::from_utf8(&output.stdout[..end]).unwrap();
    let content_type = cgi
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .unwrap()
        .1
        .trim();
    if method == "POST" && matches!(mode, Mode::DropAfterPush) {
        return;
    }
    let mut body = if method == "POST" && matches!(mode, Mode::MissingReport) {
        b"0000".as_slice()
    } else {
        &output.stdout[end + 4..]
    }
    .to_vec();
    if method == "POST" && matches!(mode, Mode::HttpFlush) {
        assert!(body.ends_with(b"0000"));
        body.extend_from_slice(b"0000");
    }
    reply(&mut stream, 200, content_type, "", &body).await;
}

async fn reply(
    stream: &mut tokio_native_tls::TlsStream<TcpStream>,
    status: u16,
    content_type: &str,
    extra: &str,
    body: &[u8],
) {
    let header = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    if stream.write_all(header.as_bytes()).await.is_ok() {
        stream.write_all(body).await.ok();
    }
    stream.shutdown().await.ok();
}

fn snapshot(target: &GitHubTarget, asset: AssetKind) -> Snapshot {
    let author = IdentityId::new();
    let at = Utc.timestamp_opt(1_700_000_000, 123_456_000).unwrap();
    let (name, bytes) = match asset {
        AssetKind::Prompt => {
            let object = PromptAsset {
                scope_id: target.scope_id,
                sensitivity: Sensitivity::Internal,
                template: PromptTemplate {
                    name: "reply".parse().unwrap(),
                    description: "Fixture".into(),
                    template: "Source evidence".into(),
                    variables: vec![],
                },
            };
            (object.entry_name(), object.canonical_bytes())
        }
        AssetKind::ContextPack => {
            let object = ContextPackAsset {
                scope_id: target.scope_id,
                sensitivity: Sensitivity::Internal,
                pack: "context".parse().unwrap(),
                document: PackDocument {
                    name: "guide".parse().unwrap(),
                    title: "Fixture".into(),
                    content: "Source evidence".into(),
                },
            };
            (object.entry_name(), object.canonical_bytes())
        }
        _ => unreachable!(),
    };
    let object = synveda_vedaflow::hash::object_hash(asset, &bytes);
    let tree = synveda_vedaflow::verify::recompute_tree(&[TreeEntry::object(&name, object)]);
    let mut commit = StoredCommit {
        hash: CommitHash::from_bytes([0; 32]),
        tree,
        parents: vec![],
        author,
        message: "Source commit".into(),
        committed_at: at,
        policy_snapshot_hash: PolicySnapshot::new("standard", 1).hash().unwrap(),
        signature: None,
    };
    commit.hash = synveda_vedaflow::verify::recompute_commit(&commit);
    Snapshot {
        tenant: target.tenant_id,
        scope: target.scope_id,
        target: target.target.clone(),
        asset,
        channel: Channel::Published,
        head: commit.hash.to_hex(),
        updated_at: at,
        updated_by: author,
        pin: None,
        objects: vec![ObjectEvidence {
            hash: object.to_hex(),
            kind: asset,
            content: String::from_utf8(bytes).unwrap(),
        }],
        trees: vec![TreeEvidence {
            hash: tree.to_hex(),
            entries: vec![EntryEvidence {
                name,
                object: object.to_hex(),
            }],
        }],
        commits: vec![CommitEvidence::from_stored(&commit)],
    }
}

#[tokio::test]
async fn https_receive_pack_preserves_real_git_and_recovers_uncertain_pushes() {
    let fixture = Fixture::new().await;
    for asset in [AssetKind::Prompt, AssetKind::ContextPack] {
        let source = snapshot(&fixture.target, asset);
        let first = Projection::render(source.clone(), None).unwrap();
        assert!(fixture.advance(&first, None, false).await.unwrap());
        assert!(!fixture.advance(&first, None, true).await.unwrap());
        verify_repository(&fixture.repo, &source.git_ref(), &BTreeMap::new())
            .await
            .unwrap();
        let mut changed = source.clone();
        changed.updated_at += chrono::Duration::seconds(1);
        let second = Projection::render(changed, Some(first.head.clone())).unwrap();
        fixture.mode(Mode::DropAfterPush);
        assert!(
            fixture
                .advance(&second, Some(&first.head), false)
                .await
                .is_err()
        );
        fixture.mode(Mode::Normal);
        let count = fixture.pushes.load(Ordering::SeqCst);
        assert!(
            !fixture
                .advance(&second, Some(&first.head), false)
                .await
                .unwrap()
        );
        assert_eq!(fixture.pushes.load(Ordering::SeqCst), count);
        let manifest = verify_repository(&fixture.repo, &source.git_ref(), &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(manifest.previous_state, Some(first.head));
    }
    git(&fixture.repo, &["fsck", "--strict"]).await;
    let clone = fixture._root.path().join("clone");
    let output = Command::new("git")
        .args(["clone", "--quiet", "--bare"])
        .arg(&fixture.repo)
        .arg(&clone)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    verify_repository(&clone, REFERENCE, &BTreeMap::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn https_receive_pack_accepts_github_http_terminal_flush() {
    let fixture = Fixture::new().await;
    fixture.mode(Mode::HttpFlush);
    for asset in [AssetKind::Prompt, AssetKind::ContextPack] {
        let source = snapshot(&fixture.target, asset);
        let projection = Projection::render(source.clone(), None).unwrap();
        assert!(fixture.advance(&projection, None, false).await.unwrap());
        assert!(!fixture.advance(&projection, None, true).await.unwrap());
        let verified = verify_repository(&fixture.repo, &source.git_ref(), &BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(verified.snapshot.head, source.head);
        assert_eq!(verified.snapshot.asset, asset);
    }
    assert_eq!(fixture.pushes.load(Ordering::SeqCst), 2);
    git(&fixture.repo, &["fsck", "--strict"]).await;
}

#[tokio::test]
async fn remote_identity_privacy_tls_network_and_ref_races_fail_without_reconciliation() {
    let fixture = Fixture::new().await;
    let source = snapshot(&fixture.target, AssetKind::Prompt);
    let first = Projection::render(source.clone(), None).unwrap();
    for mode in [
        Mode::Public,
        Mode::Replaced,
        Mode::Archived,
        Mode::Fork,
        Mode::Redirect,
        Mode::Oversize,
        Mode::Forbidden,
        Mode::Outage,
    ] {
        fixture.mode(mode);
        let error = fixture.advance(&first, None, false).await.unwrap_err();
        assert!(!error.to_string().contains(TOKEN));
        assert!(!error.to_string().contains("source-secret"));
        assert_eq!(fixture.pushes.load(Ordering::SeqCst), 0);
    }
    fixture.mode(Mode::Normal);
    let untrusted = GitHubTransport {
        targets: vec![fixture.target.clone()],
        client: client_builder()
            .resolve("api.github.com", fixture.address)
            .resolve("github.com", fixture.address)
            .build()
            .unwrap(),
    };
    assert!(
        untrusted
            .advance(&fixture.target, &fixture.credential, &first, None, false)
            .await
            .is_err()
    );
    assert_eq!(fixture.pushes.load(Ordering::SeqCst), 0);
    fixture.mode(Mode::Stall);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            fixture.advance(&first, None, false)
        )
        .await
        .is_err()
    );
    fixture.mode(Mode::Normal);
    fixture.advance(&first, None, false).await.unwrap();
    let mut changed = source.clone();
    changed.updated_at += chrono::Duration::seconds(1);
    let second = Projection::render(changed, Some(first.head.clone())).unwrap();
    let source_git = first.manifest.commits[&source.head].clone();
    fixture.mode(Mode::Race(source_git.clone()));
    assert!(matches!(
        fixture.advance(&second, Some(&first.head), false).await,
        Err(Error::Conflict { .. })
    ));
    assert_eq!(
        String::from_utf8(git(&fixture.repo, &["rev-parse", REFERENCE]).await)
            .unwrap()
            .trim(),
        source_git
    );
    fixture.mode(Mode::Normal);
    assert!(matches!(
        fixture.advance(&second, Some(&first.head), false).await,
        Err(Error::Conflict { .. })
    ));
    git(&fixture.repo, &["update-ref", "-d", REFERENCE]).await;
    let count = fixture.pushes.load(Ordering::SeqCst);
    assert!(matches!(
        fixture.advance(&first, None, true).await,
        Err(Error::Conflict { .. })
    ));
    assert_eq!(fixture.pushes.load(Ordering::SeqCst), count);
    // A malformed acknowledgement after an actual write is still uncertain;
    // retry accepts only the exact prepared head without pushing again.
    fixture.mode(Mode::MissingReport);
    assert!(fixture.advance(&first, None, false).await.is_err());
    fixture.mode(Mode::Normal);
    assert!(!fixture.advance(&first, None, false).await.unwrap());
}
