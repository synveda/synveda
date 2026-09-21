//! OPS-12: multiple native CLI processes share one rotating credential file.
//! This exercises real process locks and HTTP, not an issuer/live-account claim.

#![cfg(any(unix, windows))]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};

#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

struct Gateway {
    url: String,
    requests: mpsc::Receiver<String>,
    release: Arc<(Mutex<bool>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Gateway {
    fn new(refuse: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let (send, requests) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (released, stopped) = (release.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let seen = Arc::new(Mutex::new(BTreeSet::new()));
            let mut connections = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("mock gateway: {error}"),
                };
                let (released, send, seen) = (released.clone(), send.clone(), seen.clone());
                connections.push(thread::spawn(move || {
                    // macOS can inherit the listener's nonblocking flag. These
                    // blocking HTTP reads must wait for the child's first bytes.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    assert_eq!(line, "POST /auth/refresh HTTP/1.1\r\n");
                    let mut length = None;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = Some(value.trim().parse::<usize>().unwrap());
                        }
                    }
                    let mut bytes = vec![0; length.unwrap()];
                    reader.read_exact(&mut bytes).unwrap();
                    let request: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(request["issuer"], "https://issuer.example.test");
                    let token = request["refresh_token"].as_str().unwrap().to_owned();
                    let first_use = seen.lock().unwrap().insert(token.clone());
                    send.send(token.clone()).unwrap();
                    let (mutex, ready) = &*released;
                    let (_released, timeout) = ready
                        .wait_timeout_while(mutex.lock().unwrap(), Duration::from_secs(5), |value| !*value)
                        .unwrap();
                    assert!(!timeout.timed_out(), "test must release gateway response");
                    let (status, response) = if refuse || !first_use {
                        ("401 Unauthorized", json!({"message":"refresh was refused"}))
                    } else {
                        ("200 OK", json!({"access_token":format!("access-{token}-rotated"),
                            "refresh_token":format!("{token}-rotated"),
                            "token_type":"Bearer", "expires_in":600}))
                    };
                    let body = response.to_string();
                    // The killed-process test deliberately closes before the response.
                    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            url,
            requests,
            release,
            stop,
            thread: Some(thread),
        }
    }

    fn requested(&self) -> String {
        self.requests
            .recv_timeout(Duration::from_secs(5))
            .expect("refresh request")
    }

    fn requested_from(&self, child: &mut Child) -> String {
        self.requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|error| {
                let status = child.try_wait().unwrap();
                if status.is_none() {
                    child.kill().unwrap();
                    child.wait().unwrap();
                }
                let mut stderr = String::new();
                child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("refresh request: {error}; CLI status {status:?}: {stderr}");
            })
    }

    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.release();
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str, gateway: &Gateway, expiry_seconds: i64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "synveda credential λ {name} {}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("synveda")).unwrap();
        #[cfg(windows)]
        {
            windows_private::private(&root);
            windows_private::private(&root.join("synveda"));
        }
        #[cfg(unix)]
        std::fs::set_permissions(root.join("synveda"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let profile = |name| {
            json!({
                "gateway_url":gateway.url, "issuer":"https://issuer.example.test",
                "tenant_id":"0198f000-0000-7000-8000-000000000000", "subject":"fixture",
                "access_token":format!("access-{name}"), "refresh_token":format!("refresh-{name}"),
                "token_type":"Bearer", "expires_at":Utc::now() + TimeDelta::seconds(expiry_seconds),
            })
        };
        let fixture = Self { root };
        std::fs::write(
            fixture.path(),
            json!({"version":1, "profiles":{"default":profile("default"), "work":profile("work")}})
                .to_string(),
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        #[cfg(windows)]
        windows_private::private(&fixture.path());
        fixture
    }

    fn path(&self) -> PathBuf {
        self.root.join("synveda/credentials.json")
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.path()).unwrap()
    }
    fn profiles(&self) -> Value {
        serde_json::from_slice::<Value>(&self.bytes()).unwrap()["profiles"].clone()
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.root)
            .env("SYNVEDA_INSECURE_DEVELOPMENT_HTTP", "true")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Windows runtime/provider loading needs its system directory even
        // when the fixture excludes all ambient credential/proxy settings.
        #[cfg(windows)]
        command.env(
            "SystemRoot",
            std::env::var_os("SystemRoot").expect("Windows SystemRoot"),
        );
        command
    }
    fn token(&self, profile: &str) -> Child {
        self.command(&["auth", "token", "--profile", profile, "--json"])
            .spawn()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn success(child: Child) -> Value {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn refused(output: &Output) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("synveda login"));
    assert!(!stderr.contains("refresh-default"));
    assert!(!stderr.contains("access-default"));
}

#[test]
fn concurrent_processes_refresh_once_and_recheck_expiry_after_waiting() {
    let gateway = Gateway::new(false);
    let fixture = Fixture::new("parallel", &gateway, -60);
    let mut first = fixture.token("default");
    assert_eq!(gateway.requested_from(&mut first), "refresh-default");
    let followers: Vec<_> = (0..7).map(|_| fixture.token("default")).collect();
    assert!(
        gateway
            .requests
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "another process spent the same token while refresh held the lock"
    );
    gateway.release();
    let expected = success(first);
    for follower in followers {
        assert_eq!(success(follower), expected);
    }
    assert!(
        gateway.requests.try_recv().is_err(),
        "a waiter failed to reread fresh credentials"
    );
    assert_eq!(
        fixture.profiles()["default"]["refresh_token"],
        "refresh-default-rotated"
    );
    assert_eq!(fixture.profiles()["work"]["refresh_token"], "refresh-work");
}

#[test]
fn concurrent_profiles_keep_both_rotated_tokens() {
    let gateway = Gateway::new(false);
    let fixture = Fixture::new("profiles", &gateway, -60);
    let mut first = fixture.token("default");
    assert_eq!(gateway.requested_from(&mut first), "refresh-default");
    let second = fixture.token("work");
    gateway.release();
    success(first);
    success(second);
    assert_eq!(gateway.requested(), "refresh-work");
    assert_eq!(
        fixture.profiles()["default"]["refresh_token"],
        "refresh-default-rotated"
    );
    assert_eq!(
        fixture.profiles()["work"]["refresh_token"],
        "refresh-work-rotated"
    );
}

#[test]
fn logout_waits_for_refresh_and_cannot_be_undone_by_its_completion() {
    let gateway = Gateway::new(false);
    let fixture = Fixture::new("logout", &gateway, -60);
    let mut first = fixture.token("default");
    gateway.requested_from(&mut first);
    let mut logout = fixture
        .command(&["auth", "logout", "--all"])
        .spawn()
        .unwrap();
    assert!(
        gateway
            .requests
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    assert!(logout.try_wait().unwrap().is_none());
    gateway.release();
    success(first);
    assert!(logout.wait_with_output().unwrap().status.success());
    assert_eq!(fixture.profiles(), json!({}));
    refused(&fixture.token("default").wait_with_output().unwrap());
    assert!(gateway.requests.try_recv().is_err());
}

#[test]
fn killed_refresher_releases_lock_without_rewriting_credentials() {
    let gateway = Gateway::new(false);
    let fixture = Fixture::new("killed", &gateway, -60);
    let before = fixture.bytes();
    let mut first = fixture.token("default");
    gateway.requested_from(&mut first);
    first.kill().unwrap();
    first.wait_with_output().unwrap();
    assert_eq!(fixture.bytes(), before);
    gateway.release();
    // The issuer may already have consumed the rotating token. Refuse and
    // require login; never erase or fabricate a replacement credential.
    refused(&fixture.token("default").wait_with_output().unwrap());
    assert_eq!(gateway.requested(), "refresh-default");
    assert_eq!(fixture.bytes(), before);
}

#[test]
fn failed_refresh_preserves_bytes_and_existing_preemptive_fallback() {
    let gateway = Gateway::new(true);
    gateway.release();
    let expired = Fixture::new("expired", &gateway, -60);
    let before = expired.bytes();
    refused(&expired.token("default").wait_with_output().unwrap());
    assert_eq!(gateway.requested(), "refresh-default");
    assert_eq!(expired.bytes(), before);
    let valid = Fixture::new("preemptive", &gateway, 30);
    let before = valid.bytes();
    let output = valid.token("default").wait_with_output().unwrap();
    assert_eq!(gateway.requested(), "refresh-default");
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["access_token"],
        "access-default"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("using the stored token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("refresh-default"));
    assert_eq!(valid.bytes(), before);
}
