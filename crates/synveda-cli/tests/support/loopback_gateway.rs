//! A loopback public API that answers exactly `requests` project/workspace
//! reads for the fixture bearer. No issuer or harness claim.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub const PROJECT: &str = "0198e4c1-0000-7000-8000-000000000002";
pub const WORKSPACE: &str = "0198e4c1-0000-7000-8000-000000000001";

/// Returns the gateway URL and a handle that yields the number served.
pub fn gateway(requests: usize) -> (String, std::thread::JoinHandle<usize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut served = 0;
        while served < requests && Instant::now() < deadline {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
                assert!(head.len() < 8192);
            }
            let request = String::from_utf8(head).unwrap();
            assert!(request.contains("authorization: Bearer fixture-bearer"));
            let body = if request.starts_with(&format!("GET /v1/projects/{PROJECT} ")) {
                serde_json::json!({"id": PROJECT, "workspace_id": WORKSPACE, "status": "active"})
            } else if request.starts_with(&format!("GET /v1/workspaces/{WORKSPACE} ")) {
                serde_json::json!({"id": WORKSPACE, "status": "active"})
            } else {
                panic!("unexpected request: {request}")
            }
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            served += 1;
        }
        served
    });
    (url, thread)
}
