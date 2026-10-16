//! Fictional HTTPS Git-data provider. Test CA/DNS trust stays in this fixture.
#![allow(dead_code)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const TOKEN: &str = "github_pat_fictional_read_fixture";
pub const NAME: &str = "fixture-owner/source";
#[derive(Clone, Default)]
pub enum Mode {
    #[default]
    Normal,
    Renamed,
    RenamedMovedRef,
    Replaced,
    Public,
    Redirect,
    Truncated,
    TamperedTree,
    TamperedBlob,
    MissingBlob,
    Outage,
    PauseBlob,
    MovedRef,
}
pub struct Fixture {
    _root: tempfile::TempDir,
    pub address: std::net::SocketAddr,
    pub certificate: Vec<u8>,
    pub pin: synveda_git::SourcePin,
    pub next: synveda_git::SourcePin,
    pub mode: Arc<Mutex<Mode>>,
    pub reached: Arc<tokio::sync::Notify>,
    pub release: Arc<tokio::sync::Notify>,
    pub requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn git(work: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Fictional fixture",
            "-c",
            "user.email=fixture@example.test",
        ])
        .args(args)
        .current_dir(work)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture Git command failed");
    output.stdout
}
fn text(work: &Path, args: &[&str]) -> String {
    String::from_utf8(git(work, args))
        .unwrap()
        .trim()
        .to_owned()
}
fn collect_tree(work: &Path, oid: &str, routes: &mut BTreeMap<String, Value>) {
    if routes.contains_key(&format!("/git/trees/{oid}")) {
        return;
    }
    let mut entries = Vec::new();
    for entry in git(work, &["ls-tree", "-z", oid])
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let entry = std::str::from_utf8(entry).unwrap();
        let (header, path) = entry.split_once('\t').unwrap();
        let fields: Vec<_> = header.split(' ').collect();
        let (mode, kind, sha) = (fields[0], fields[1], fields[2]);
        let size = if kind == "blob" {
            let bytes = git(work, &["cat-file", "blob", sha]);
            routes.insert(format!("/git/blobs/{sha}"),json!({"sha":sha,"size":bytes.len(),"encoding":"base64","content":format!("{}\n",STANDARD.encode(&bytes))}));
            Some(bytes.len())
        } else {
            collect_tree(work, sha, routes);
            None
        };
        entries.push(json!({"path":path,"mode":mode,"type":kind,"sha":sha,"size":size}));
    }
    routes.insert(
        format!("/git/trees/{oid}"),
        json!({"sha":oid,"truncated":false,"tree":entries}),
    );
}
fn snapshot(work: &Path, routes: &mut BTreeMap<String, Value>) -> synveda_git::SourcePin {
    let commit = text(work, &["rev-parse", "HEAD"]);
    let tree = text(work, &["rev-parse", "HEAD^{tree}"]);
    routes.insert(
        format!("/git/commits/{commit}"),
        json!({"sha":commit,"tree":{"sha":tree}}),
    );
    collect_tree(work, &tree, routes);
    synveda_git::SourcePin { commit, tree }
}
impl Fixture {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "--template=", "--initial-branch=main"]);
        std::fs::create_dir(work.join("procedures")).unwrap();
        std::fs::write(work.join("procedures/retries.md"),"---\ntype: Procedure\ntitle: Iris retry policy\ndescription: Fictional retry procedure.\n---\n\nThe fictional Iris retry cap is 3 attempts.\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "three"]);
        let mut routes = BTreeMap::new();
        let pin = snapshot(&work, &mut routes);
        std::fs::write(work.join("procedures/retries.md"),"---\ntype: Procedure\ntitle: Iris retry policy\ndescription: Fictional retry procedure.\n---\n\nThe fictional Iris retry cap is 5 attempts.\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "five"]);
        let next = snapshot(&work, &mut routes);
        let config = root.path().join("tls.conf");
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        std::fs::write(&config,"[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=api.github.com\n[ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:api.github.com\n").unwrap();
        assert!(
            Command::new("openssl")
                .args([
                    "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-config"
                ])
                .arg(config)
                .arg("-keyout")
                .arg(&key)
                .arg("-out")
                .arg(&cert)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let pkcs8 = root.path().join("pkcs8.pem");
        assert!(
            Command::new("openssl")
                .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
                .arg(key)
                .arg("-out")
                .arg(&pkcs8)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let certificate = std::fs::read(cert).unwrap();
        let identity =
            native_tls::Identity::from_pkcs8(&certificate, &std::fs::read(pkcs8).unwrap()).unwrap();
        let acceptor =
            tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mode = Arc::new(Mutex::new(Mode::Normal));
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (server_mode, server_reached, server_release, server_requests) = (
            mode.clone(),
            reached.clone(),
            release.clone(),
            requests.clone(),
        );
        let (first_commit, next_commit) = (pin.commit.clone(), next.commit.clone());
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let Ok((socket,_))=accepted else {return};
                        let (acceptor,mode,reached,release,requests,routes,first,next)=(acceptor.clone(),server_mode.clone(),server_reached.clone(),server_release.clone(),server_requests.clone(),routes.clone(),first_commit.clone(),next_commit.clone());
                        connections.spawn(async move {
                            let Ok(mut socket)=acceptor.accept(socket).await else {return};let mut header=Vec::new();
                            while !header.ends_with(b"\r\n\r\n") {
                                if header.len()>16_384 {return;}let mut byte=[0];if socket.read_exact(&mut byte).await.is_err(){return;}header.push(byte[0]);
                            }
                            let request=String::from_utf8(header).unwrap();let line=request.lines().next().unwrap();
                            requests.lock().unwrap().push(line.to_owned());
                            assert!(line.starts_with("GET "),"inbound fixture received a write request");
                            assert!(request.contains(&format!("authorization: Bearer {TOKEN}")));
                            let uri=line.split(' ').nth(1).unwrap();assert!(!uri.contains('?'),"nonrecursive contract must omit query parameters");
                            let suffix=uri.strip_prefix("/repos/fixture-owner/source").or_else(||uri.strip_prefix("/repos/fixture-owner/renamed")).unwrap();
                            let mode=mode.lock().unwrap().clone();
                            if matches!(mode,Mode::PauseBlob)&&suffix.starts_with("/git/blobs/") {reached.notify_one();release.notified().await;}
                            let mut status="200 OK";let mut extra="";
                            let mut body=if suffix.is_empty() {
                                json!({"id":123,"full_name":if matches!(mode,Mode::Renamed|Mode::RenamedMovedRef){"fixture-owner/renamed"}else{NAME},"private":true,"visibility":"private","archived":false,"disabled":false,"fork":false})
                            }else if suffix=="/git/ref/heads/main" {
                                json!({"ref":"refs/heads/main","object":{"type":"commit","sha":if matches!(mode,Mode::MovedRef|Mode::RenamedMovedRef){next}else{first}}})
                            }else {routes.get(suffix).cloned().unwrap_or_else(||{status="404 Not Found";json!({})})};
                            match mode {
                                Mode::Replaced if suffix.is_empty()=>body["id"]=json!(124),
                                Mode::Public if suffix.is_empty()=>body["private"]=json!(false),
                                Mode::Redirect=>{status="302 Found";extra="Location: https://outside.invalid/\r\n";},
                                Mode::Outage=>status="503 Unavailable",
                                Mode::Truncated if suffix.starts_with("/git/trees/")=>body["truncated"]=json!(true),
                                Mode::TamperedTree if suffix.starts_with("/git/trees/")=>body["tree"]=json!([]),
                                Mode::TamperedBlob if suffix.starts_with("/git/blobs/")=>body["content"]=json!(STANDARD.encode(b"fabricated source")),
                                Mode::MissingBlob if suffix.starts_with("/git/blobs/")=>status="404 Not Found",
                                _=>{},
                            }
                            let bytes=body.to_string();let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{bytes}",bytes.len());
                            let _=socket.write_all(response.as_bytes()).await;let _=socket.shutdown().await;
                        });
                    },
                    _=connections.join_next(),if !connections.is_empty()=>{},
                }
            }
        });
        Self {
            _root: root,
            address,
            certificate,
            pin,
            next,
            mode,
            reached,
            release,
            requests,
            task,
        }
    }
    pub fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }
    pub fn reader(
        &self,
        tenant: synveda_types::TenantId,
        scope: synveda_types::ScopeId,
        secret: synveda_types::TenantSecretId,
        renamed: bool,
    ) -> synveda_git::GitHubSourceReader {
        synveda_git::GitHubSourceReader::for_controlled_https(
            vec![synveda_git::GitHubSourceTarget {
                tenant_id: tenant,
                scope_id: scope,
                target: "private-fixture".to_owned(),
                owner: "fixture-owner".to_owned(),
                repository: if renamed { "renamed" } else { "source" }.to_owned(),
                repository_id: 123,
                secret_reference: synveda_types::secret::tenant_secret_reference(secret),
            }],
            &self.certificate,
            self.address,
        )
        .unwrap()
    }
}
