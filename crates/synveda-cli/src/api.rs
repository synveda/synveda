//! The gateway client the governed commands use (FLOW-6, ADR-0035
//! decisions 1 and 2).
//!
//! Everything a running gateway serves goes through here, under the
//! reviewer's own bearer. The store-backed commands beside it
//! (`db migrate`, `tenant create`, `policy apply`, `role bind`, ...) exist
//! for the moment before a gateway is usable and audit themselves as
//! break-glass; a review has no such moment. Approving is a governed act
//! whose authority (`ProposalReview`), whose count (the approval matrix),
//! and whose audit event all live behind the PDP, so the only honest way
//! to cast one from a terminal is to ask the gateway — which is why this
//! module opens no database connection and the `proposal` verbs take no
//! `--database-url`.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::credentials::Profile;
use crate::login;

/// A gateway, and a bearer for it.
pub struct Api {
    base: String,
    bearer: String,
    /// Who the bearer says we are — rendered so a reviewer can see which
    /// identity is about to approve something.
    pub subject: String,
    http: reqwest::Client,
    /// The W3C trace context every call from this client carries
    /// (ADR-0007's deferred clause; see [`Api::traceparent`]).
    trace: TraceContext,
}

/// The `traceparent` this client sends, minted once and reused.
///
/// # One trace per client, which is one trace per thing the user asked for
///
/// [`Api::connect`] is called once per command — and, in `synveda mcp`, once
/// per tool call, because a long-lived server resolves its bearer per call
/// so a session outliving a token refreshes instead of failing. So tying
/// the trace to the client, rather than to a process or to a request, lands
/// on exactly the right unit at both call sites for free: `synveda proposal
/// review` making five calls is one trace, and one `recall` tool call is
/// one trace rather than a session's worth.
///
/// # The root is synthetic, and that is worth knowing before you look
///
/// The CLI installs no OTel exporter — `synveda mcp` deliberately does not
/// (see `mcp::subscribe`), and the one-shot verbs have no subscriber at
/// all — so the span this names as the parent is never reported to a
/// collector. In Jaeger the gateway's spans appear under a root that is
/// not there, which renders fine and is exactly what ADPT-1's hooks have
/// always produced. Do not go looking for the missing span; nothing lost
/// it.
///
/// What this buys is real all the same: every call from one command shares
/// an id, the gateway now continues that trace rather than starting its
/// own (FND-5, ADR-0007's deferred clause), and the id is printed where a
/// person can paste it into Jaeger.
struct TraceContext {
    trace_id: String,
    parent_span_id: String,
}

impl TraceContext {
    /// A fresh context from the system CSPRNG. 16 bytes of trace id and 8
    /// of span id, lowercase hex, which is what W3C version `00` fixes.
    fn new() -> Result<Self, String> {
        Ok(Self {
            trace_id: hex(16)?,
            parent_span_id: hex(8)?,
        })
    }

    /// The header value. Sampled (`01`) because a caller that did not want
    /// this trace would not have sent one: the CLI makes a handful of calls
    /// per command, so there is nothing here worth sampling away.
    fn header(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.parent_span_id)
    }
}

fn hex(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|err| format!("system CSPRNG unavailable: {err}"))?;
    Ok(buffer.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    }))
}

/// Where a bearer came from. `synveda proposal` prints it once, because
/// "which identity am I about to approve as" is the first thing a reviewer
/// needs and the last thing they should have to guess.
pub enum Origin {
    /// A stored login (`synveda login`), refreshed if it had expired.
    Profile(String),
    /// The explicit `SYNVEDA_TOKEN` override ADPT-1 kept for CI and demos
    /// (ADR-0027).
    Environment,
}

impl Api {
    /// Resolves the gateway and a currently-valid bearer for `profile`.
    ///
    /// `SYNVEDA_TOKEN` wins, and then `SYNVEDA_GATEWAY` (or the default
    /// listen address) chooses the host: an operator who supplies a raw
    /// token has supplied the gateway too. Otherwise the profile decides
    /// **both** — a stored credential's own `gateway_url` is where its
    /// bearer goes, which is the ADPT-1 rule (ADR-0027), and it is why
    /// these commands carry no `--gateway` flag: pointing a bearer at a
    /// host of the caller's choosing is not a convenience.
    pub async fn connect(profile_name: &str) -> Result<(Self, Origin), String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| format!("build the HTTP client: {err}"))?;

        let trace = TraceContext::new()?;

        if let Some(token) = std::env::var("SYNVEDA_TOKEN")
            .ok()
            .filter(|token| !token.is_empty())
        {
            return Ok((
                Self {
                    base: login::gateway_url(None),
                    bearer: token,
                    subject: "SYNVEDA_TOKEN".to_owned(),
                    http,
                    trace,
                },
                Origin::Environment,
            ));
        }

        let profile: Profile = login::resolve(profile_name).await?;
        Ok((
            Self {
                base: profile.gateway_url.clone(),
                bearer: profile.access_token.clone(),
                subject: profile.subject.clone(),
                http,
                trace,
            },
            Origin::Profile(profile_name.to_owned()),
        ))
    }

    /// The gateway this client talks to.
    pub fn gateway(&self) -> &str {
        &self.base
    }

    /// The trace id every call from this client carries — the one to paste
    /// into Jaeger to see what the gateway did with them.
    pub fn trace_id(&self) -> &str {
        &self.trace.trace_id
    }

    /// `GET path`, as a JSON value.
    pub async fn get(&self, path: &str) -> Result<Value, String> {
        self.send(self.http.get(format!("{}{path}", self.base)), "GET", path)
            .await
    }

    /// `POST path` with an optional JSON body, as a JSON value.
    pub async fn post(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let mut request = self.http.post(format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.send(request, "POST", path).await
    }

    /// [`Api::get`] into a typed view.
    pub async fn get_as<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        decode(self.get(path).await?, path)
    }

    /// [`Api::post`] into a typed view.
    pub async fn post_as<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, String> {
        decode(self.post(path, body).await?, path)
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        method: &str,
        path: &str,
    ) -> Result<Value, String> {
        let response = request
            .bearer_auth(&self.bearer)
            // Every governed verb goes through here, so this one line is
            // what makes a `synveda proposal publish` and the gateway work
            // it triggers one trace instead of two unconnected halves.
            .header("traceparent", self.trace.header())
            .send()
            .await
            .map_err(|err| format!("{method} {}{path}: {err}", self.base))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(refusal(status, &body));
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body)
            .map_err(|err| format!("{method} {path}: the gateway's answer is not JSON: {err}"))
    }
}

/// Renders a refusal in the gateway's own words.
///
/// The body is the shared taxonomy (`{"kind": ..., ...}`), so it is parsed
/// as [`synveda_types::Error`] and rendered by its own `Display` rather
/// than by a second string-shaped copy of the same vocabulary here. A
/// denial says which policy denied it; a conflict says what moved.
fn refusal(status: reqwest::StatusCode, body: &str) -> String {
    match serde_json::from_str::<synveda_types::Error>(body) {
        Ok(error) => error.to_string(),
        // Not the taxonomy: a proxy, a body limit, an empty 5xx. Say the
        // status rather than pretend to know more.
        Err(_) if body.trim().is_empty() => format!("HTTP {status}"),
        Err(_) => format!("HTTP {status}: {}", body.trim()),
    }
}

fn decode<T: DeserializeOwned>(value: Value, path: &str) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|err| format!("{path}: the gateway's answer is not the shape expected: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header shape W3C version `00` fixes: four hyphen-separated
    /// fields, 32 lowercase hex of trace id, 16 of parent span id, and
    /// flags. The gateway's propagator rejects anything else outright, so
    /// getting this wrong means every call silently stops being traced —
    /// which looks exactly like nothing being wrong.
    #[test]
    fn the_traceparent_is_the_shape_the_gateway_will_accept() {
        let trace = TraceContext::new().expect("a context");
        let header = trace.header();
        let parts: Vec<&str> = header.split('-').collect();
        assert_eq!(parts.len(), 4, "{header}");
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1].len(), 32, "trace id is 16 bytes of hex: {header}");
        assert_eq!(parts[2].len(), 16, "span id is 8 bytes of hex: {header}");
        assert_eq!(parts[3], "01");
        assert!(
            header
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase() || c == '-'),
            "W3C requires lowercase hex; uppercase is rejected outright: {header}",
        );
        // All-zero ids are the spec's own "invalid" values and extract to
        // nothing. A CSPRNG will not produce them, but a future refactor
        // that forgot to fill the buffer would.
        assert_ne!(parts[1], "0".repeat(32));
        assert_ne!(parts[2], "0".repeat(16));
    }

    /// One trace per client, which is one trace per thing the user asked
    /// for: every call a command makes shares an id, and two commands do
    /// not. Reusing across clients would merge unrelated work into one
    /// Jaeger view; minting per *call* would scatter one command across
    /// several.
    #[test]
    fn one_client_is_one_trace_and_two_clients_are_two() {
        let first = TraceContext::new().expect("a context");
        assert_eq!(first.header(), first.header(), "stable within a client");

        let second = TraceContext::new().expect("a context");
        assert_ne!(
            first.trace_id, second.trace_id,
            "a second command must not land in the first command trace",
        );
    }

    /// The header shape tests above prove a string; this proves it reaches
    /// a socket, on **every** call, from the choke point every governed verb
    /// goes through. Without it, deleting one line in `send` leaves all the
    /// unit tests green and every trace silently unjoined.
    ///
    /// A real listener rather than a mock: the subject is what a gateway
    /// receives, and `reqwest` is between here and that.
    #[tokio::test]
    async fn every_call_carries_the_same_traceparent_to_the_wire() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        // `Api::connect` reads the environment; this is the credentials
        // tests' lock discipline, in the async-aware form this test needs —
        // a `std::sync::MutexGuard` held across an await is what clippy
        // rejects, and rightly: it parks a thread the runtime wanted.
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = LOCK.lock().await;

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind a loopback listener");
        let port = listener.local_addr().expect("addr").port();

        // Two requests, so the test can say whether the trace is stable
        // across a command rather than minted per call.
        let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let server = tokio::spawn({
            let seen = std::sync::Arc::clone(&seen);
            async move {
                for _ in 0..2 {
                    let (mut stream, _) = listener.accept().await.expect("accept");
                    let mut buffer = vec![0u8; 4096];
                    let read = stream.read(&mut buffer).await.expect("read");
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let header = request
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("traceparent: ")
                                .map(str::trim)
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| format!("ABSENT in:\n{request}"));
                    seen.lock().await.push(header);
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                        .await
                        .expect("write");
                }
            }
        });

        // SAFETY: the lock above makes this the only thread touching the
        // environment for the duration of the test.
        unsafe {
            std::env::set_var("SYNVEDA_TOKEN", "test-bearer");
            std::env::set_var("SYNVEDA_GATEWAY", format!("http://127.0.0.1:{port}"));
        }
        let (api, _origin) = Api::connect("default").await.expect("connect");
        api.get("/v1/whoami").await.expect("first call");
        api.post("/v1/recall", None).await.expect("second call");
        unsafe {
            std::env::remove_var("SYNVEDA_TOKEN");
            std::env::remove_var("SYNVEDA_GATEWAY");
        }
        server.await.expect("server task");

        let seen = seen.lock().await;
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0].starts_with("00-") && seen[0].ends_with("-01"),
            "no usable traceparent on the wire: {:?}",
            seen[0],
        );
        assert_eq!(
            seen[0], seen[1],
            "both calls of one command must land in one trace",
        );
        assert!(
            seen[0].contains(api.trace_id()),
            "the id the header carries must be the one `trace_id()` reports, or the \
             number printed for a human to paste into Jaeger names a different trace",
        );
    }

    #[test]
    fn a_refusal_is_rendered_in_the_gateways_own_words() {
        let denied = serde_json::json!({
            "kind": "policy_denied",
            "action": "ProposalReview",
            "resource": "scope 0198f000-0000-7000-8000-000000000000",
            "reason": "no policy permits it",
        })
        .to_string();
        let message = refusal(reqwest::StatusCode::FORBIDDEN, &denied);
        assert!(message.contains("ProposalReview"), "{message}");
        assert!(message.contains("no policy permits it"), "{message}");

        let conflict = serde_json::json!({
            "kind": "conflict",
            "message": "record 0198 changed after this proposal was approved",
        })
        .to_string();
        assert!(
            refusal(reqwest::StatusCode::CONFLICT, &conflict).contains("changed after"),
            "a conflict must say what moved"
        );
    }

    #[test]
    fn a_body_that_is_not_the_taxonomy_still_says_something_useful() {
        assert_eq!(
            refusal(reqwest::StatusCode::BAD_GATEWAY, ""),
            "HTTP 502 Bad Gateway"
        );
        let html = refusal(reqwest::StatusCode::NOT_FOUND, "<html>nginx</html>");
        assert!(html.starts_with("HTTP 404"), "{html}");
        assert!(html.contains("nginx"), "{html}");
    }
}
