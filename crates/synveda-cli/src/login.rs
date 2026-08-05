//! `synveda login` and `synveda auth token` (ADPT-1, ADR-0027 decisions 4
//! to 6).
//!
//! The CLI is the sole credential authority. It binds an ephemeral
//! loopback listener, opens the browser at the *gateway's* `/auth/login`
//! — never the IdP's — and lets AUTH-1 run end to end: PKCE, JWKS
//! verification, TEN-1's active-tenant rule, AUTH-2 JIT provisioning. A
//! token obtained any other way would belong to an unprovisioned subject
//! that the PDP quarantines, which is why this flow goes through the
//! gateway rather than talking OAuth itself (ADR-0027 option 3).
//!
//! What comes back to the loopback listener is a one-time code, never a
//! token: tokens are redeemed over a POST and so never enter a URL, a
//! browser history, or a shell history.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::credentials::{self, Profile};

/// How long to wait for the browser round-trip before giving the terminal
/// back. Long enough for an SSO prompt with MFA, short enough that a
/// forgotten tab is not a hung shell.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Refresh this far ahead of expiry, so a token that is valid when
/// `auth token` returns it is still valid when the caller uses it.
const REFRESH_SKEW_SECS: i64 = 60;

/// Cap on the request line the loopback listener will read. A browser
/// redirect is a few hundred bytes; anything else is not our browser.
const MAX_REQUEST_BYTES: usize = 8192;

/// The gateway a command talks to: `--gateway`, then `SYNVEDA_GATEWAY`,
/// then the gateway's own default listen address.
pub fn gateway_url(flag: Option<String>) -> String {
    let url = flag
        .or_else(|| std::env::var("SYNVEDA_GATEWAY").ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8120".to_owned());
    url.trim_end_matches('/').to_owned()
}

/// What `/auth/cli/exchange` returns: the browser-facing session plus the
/// two things a long-lived client needs.
#[derive(serde::Deserialize)]
struct CliSession {
    subject: String,
    tenant: TenantSummary,
    identity: IdentitySummary,
    issuer: String,
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

#[derive(serde::Deserialize)]
struct TenantSummary {
    id: String,
    slug: String,
}

#[derive(serde::Deserialize)]
struct IdentitySummary {
    scope_path: String,
    quarantined: bool,
}

#[derive(serde::Deserialize)]
struct RefreshedSession {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

/// Runs the loopback login and stores the result under `profile`.
pub async fn login(
    gateway: String,
    issuer: Option<String>,
    profile_name: String,
    open_browser: bool,
) -> Result<(), String> {
    // Bind before opening anything: the port is part of the URL, and a
    // browser that arrives before the listener exists gets a refusal.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|err| format!("bind a loopback listener: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("read the listener address: {err}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = random_urlsafe()?;

    let mut url = url::Url::parse(&format!("{gateway}/auth/login"))
        .map_err(|err| format!("{gateway} is not a valid gateway URL: {err}"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("cli_redirect_uri", &redirect_uri);
        query.append_pair("cli_state", &state);
        if let Some(issuer) = &issuer {
            query.append_pair("issuer", issuer);
        }
    }
    let url = url.to_string();

    // stderr, always: stdout is for machine-readable output, and the URL
    // has to be visible whether or not the browser opens.
    eprintln!("synveda: opening your browser to log in at {gateway}");
    eprintln!("         if it does not open, visit:\n\n  {url}\n");
    if open_browser {
        open_in_browser(&url);
    }

    let callback = tokio::time::timeout(LOGIN_TIMEOUT, await_callback(&listener))
        .await
        .map_err(|_| {
            format!(
                "timed out after {}s waiting for the browser to come back",
                LOGIN_TIMEOUT.as_secs()
            )
        })??;

    // The CLI's own CSRF check: anything can connect to a loopback port,
    // so a callback that does not carry the state this process minted is
    // not this process's callback.
    if callback.state.as_deref() != Some(state.as_str()) {
        return Err("the login callback carried the wrong state; nothing was stored".to_owned());
    }
    if let Some(error) = callback.error {
        let detail = callback
            .error_description
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| error.clone());
        return Err(format!("login failed: {detail}"));
    }
    let Some(code) = callback.code else {
        return Err("the login callback carried no code".to_owned());
    };

    let http = client()?;
    let response = http
        .post(format!("{gateway}/auth/cli/exchange"))
        .json(&serde_json::json!({ "code": code, "state": state }))
        .send()
        .await
        .map_err(|err| format!("exchange the login code at {gateway}: {err}"))?;
    let session: CliSession = read_json(response, "exchange the login code").await?;

    let expires_at = session
        .expires_in
        .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
    if session.refresh_token.is_none() {
        // Decision 6's documented degradation: the login worked, but this
        // issuer will not let us keep it alive.
        eprintln!(
            "synveda: warning — this issuer granted no refresh token, so this \
             login lasts only as long as its access token"
        );
    }
    if session.identity.quarantined {
        eprintln!(
            "synveda: note — this identity is quarantined; sessions will receive \
             an empty context block until an administrator places it"
        );
    }
    credentials::store(
        &profile_name,
        Profile {
            gateway_url: gateway.clone(),
            issuer: session.issuer,
            tenant_id: session.tenant.id,
            tenant_slug: Some(session.tenant.slug.clone()),
            subject: session.subject.clone(),
            access_token: session.access_token,
            token_type: session.token_type,
            expires_at,
            refresh_token: session.refresh_token,
        },
    )?;

    eprintln!(
        "synveda: logged in as {} in tenant {} at {}",
        session.subject, session.tenant.slug, session.identity.scope_path
    );
    eprintln!(
        "         credentials written to {} (profile `{profile_name}`)",
        credentials::path()?.display()
    );
    Ok(())
}

/// The profile with a currently-valid access token, refreshing through
/// the gateway when the stored one is spent (ADR-0027 decision 4).
///
/// One implementation of expiry, skew, and refresh in this binary:
/// `synveda auth token` prints what this returns, and every governed
/// command (`synveda proposal ...`, FLOW-6) carries it as its bearer. A
/// second copy would drift, and the thing it would drift on is whether a
/// reviewer's approval reaches the gateway at all.
pub async fn resolve(profile_name: &str) -> Result<Profile, String> {
    let mut profile = credentials::profile(profile_name)?;
    let skew = chrono::Duration::seconds(REFRESH_SKEW_SECS);
    if profile.valid_for(skew) {
        return Ok(profile);
    }

    let Some(refresh_token) = profile.refresh_token.clone() else {
        return Err(format!(
            "the credentials for profile `{profile_name}` have expired and this \
             issuer granted no refresh token; run `synveda login` again"
        ));
    };
    match refresh(&profile.gateway_url, &profile.issuer, &refresh_token).await {
        Ok(refreshed) => {
            profile.access_token = refreshed.access_token;
            profile.token_type = refreshed.token_type;
            profile.expires_at = refreshed
                .expires_in
                .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
            // Issuers that rotate refresh tokens invalidate the old one;
            // keep the new one or the next refresh fails.
            if refreshed.refresh_token.is_some() {
                profile.refresh_token = refreshed.refresh_token.clone();
            }
            credentials::store(profile_name, profile.clone())?;
        }
        Err(error) => match recover(&profile, &error) {
            Recovery::UseStored => eprintln!("synveda: {error}; using the stored token"),
            Recovery::Fail(message) => return Err(message),
        },
    }
    Ok(profile)
}

/// Prints a currently-valid bearer for `profile`. This is what the Claude
/// Code adapter's hooks shell out to, so its stdout is a contract: the raw
/// token, or one JSON object.
pub async fn auth_token(profile_name: String, json: bool) -> Result<(), String> {
    let profile = resolve(&profile_name).await?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "profile": profile_name,
                "access_token": profile.access_token,
                "token_type": profile.token_type,
                "expires_at": profile.expires_at,
                "gateway_url": profile.gateway_url,
                "tenant_id": profile.tenant_id,
                "subject": profile.subject,
            })
        );
    } else {
        println!("{}", profile.access_token);
    }
    Ok(())
}

/// What to do about a refresh the issuer would not grant.
#[derive(Debug, PartialEq, Eq)]
enum Recovery {
    /// The stored access token has life left: use it and say so.
    UseStored,
    /// Nothing usable is left; tell the caller what to run.
    Fail(String),
}

/// A refused *pre-emptive* refresh is not a reason to withhold a token
/// that still works.
///
/// [`REFRESH_SKEW_SECS`] exists so a bearer handed out is still valid when
/// it is used, which means the refresh fires a minute before expiry — and
/// an issuer may not honour a refresh token that early. Rauthy, the
/// bundled SMB IdP, is exactly such an issuer: its refresh tokens are
/// not-before `issued_at + access_token_lifetime - 60s`, the same instant
/// this skew fires, so whether the first attempt lands depends on which of
/// the two clocks is ahead. Failing there would cost a session its memory
/// and tell the user to log in again while a perfectly good token sat in
/// the file.
fn recover(profile: &Profile, error: &str) -> Recovery {
    if profile.valid_for(chrono::Duration::zero()) {
        return Recovery::UseStored;
    }
    Recovery::Fail(format!("{error}; run `synveda login` again"))
}

async fn refresh(
    gateway: &str,
    issuer: &str,
    refresh_token: &str,
) -> Result<RefreshedSession, String> {
    let response = client()?
        .post(format!("{gateway}/auth/refresh"))
        .json(&serde_json::json!({
            "refresh_token": refresh_token,
            "issuer": issuer,
        }))
        .send()
        .await
        .map_err(|err| format!("refresh the access token at {gateway}: {err}"))?;
    read_json(response, "refresh the access token").await
}

/// One HTTP client, with the same no-redirect posture the gateway's own
/// OIDC client takes: an auth response must come from the host we asked.
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| format!("build the HTTP client: {err}"))
}

/// Reads a gateway response, turning its taxonomy body into the message
/// the user sees. A 401 here means the login or the refresh was refused,
/// and saying so beats printing a status code.
async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    what: &str,
) -> Result<T, String> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("entity"))
                    .and_then(|field| field.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(format!("could not {what}: {detail}"));
    }
    serde_json::from_str(&body).map_err(|err| format!("could not {what}: invalid response: {err}"))
}

/// What the browser handed back to the loopback listener.
#[derive(Debug, Default)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Serves the loopback listener until the browser hits `/callback`.
/// Anything else gets a 404 and the wait continues — a stray probe on an
/// ephemeral port must not end a login.
async fn await_callback(listener: &tokio::net::TcpListener) -> Result<Callback, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|err| format!("accept the login callback: {err}"))?;

        let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_owned();

        let Some(callback) = parse_callback(&target) else {
            respond(&mut stream, "404 Not Found", "not found").await;
            continue;
        };
        let page = if callback.error.is_some() {
            "<h1>Login failed</h1><p>Return to your terminal for the reason.</p>"
        } else {
            "<h1>Signed in to Synveda</h1><p>You can close this tab and return to your terminal.</p>"
        };
        respond(&mut stream, "200 OK", page).await;
        return Ok(callback);
    }
}

/// Parses `/callback?...`, returning `None` for any other path so the
/// listener keeps waiting.
fn parse_callback(target: &str) -> Option<Callback> {
    // A relative request target has no base; any absolute loopback base
    // parses it identically, and only the path and query are read.
    let url = url::Url::parse("http://127.0.0.1/")
        .ok()?
        .join(target)
        .ok()?;
    if url.path() != "/callback" {
        return None;
    }
    let mut callback = Callback::default();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => callback.code = Some(value.into_owned()),
            "state" => callback.state = Some(value.into_owned()),
            "error" => callback.error = Some(value.into_owned()),
            "error_description" => callback.error_description = Some(value.into_owned()),
            _ => {}
        }
    }
    Some(callback)
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Synveda</title>\
         <body style=\"font:16px system-ui;margin:4rem auto;max-width:32rem\">{body}</body>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    );
    // Best effort: the login's outcome does not depend on the browser
    // rendering anything.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Opens the system browser. Failure is not an error: the URL is already
/// on stderr, and pasting it works exactly as well.
fn open_in_browser(url: &str) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    match std::process::Command::new(program)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {}
        Err(err) => eprintln!("synveda: could not open a browser ({err}); open the URL above"),
    }
}

/// 32 bytes of CSPRNG entropy, base64url — the same shape the gateway's
/// login state uses.
fn random_urlsafe() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| format!("system CSPRNG unavailable: {err}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_callback_path_is_the_only_one_that_ends_the_wait() {
        assert!(parse_callback("/callback?code=c&state=s").is_some());
        assert!(parse_callback("/callback").is_some());
        for other in ["/", "/favicon.ico", "/callback/extra", "/CALLBACK"] {
            assert!(parse_callback(other).is_none(), "{other} must be ignored");
        }
    }

    #[test]
    fn callback_parameters_are_read_and_extras_ignored() {
        let callback = parse_callback("/callback?code=c1&state=s1&nonsense=x").expect("parse");
        assert_eq!(callback.code.as_deref(), Some("c1"));
        assert_eq!(callback.state.as_deref(), Some("s1"));
        assert!(callback.error.is_none());

        let failed =
            parse_callback("/callback?error=login_failed&error_description=denied&state=s1")
                .expect("parse");
        assert_eq!(failed.error.as_deref(), Some("login_failed"));
        assert_eq!(failed.error_description.as_deref(), Some("denied"));
        assert!(failed.code.is_none());

        // Percent-encoded values arrive decoded, as the gateway sent them.
        let encoded = parse_callback("/callback?code=a%26b&state=c%3Dd").expect("parse");
        assert_eq!(encoded.code.as_deref(), Some("a&b"));
        assert_eq!(encoded.state.as_deref(), Some("c=d"));
    }

    #[test]
    fn gateway_url_precedence_and_trailing_slash() {
        // The binary's one environment lock. This used to take none at
        // all, on a comment asserting "no other thread reads this variable
        // in this test binary" — true when written, false the moment
        // `api::tests` started setting `SYNVEDA_GATEWAY` and reading it
        // back through `Api::connect`. See `crate::testing`.
        let _guard = crate::testing::ENV.blocking_lock();
        // SAFETY: the lock above makes this the only thread touching the
        // environment for the duration of the test.
        unsafe { std::env::remove_var("SYNVEDA_GATEWAY") };
        assert_eq!(gateway_url(None), "http://127.0.0.1:8120");
        assert_eq!(
            gateway_url(Some("https://synveda.corp.test/".to_owned())),
            "https://synveda.corp.test"
        );
        unsafe { std::env::set_var("SYNVEDA_GATEWAY", "http://gw.test:9000/") };
        assert_eq!(gateway_url(None), "http://gw.test:9000");
        // The flag still wins over the environment.
        assert_eq!(
            gateway_url(Some("http://other.test".to_owned())),
            "http://other.test"
        );
        unsafe { std::env::remove_var("SYNVEDA_GATEWAY") };
    }

    fn profile_expiring_in(seconds: i64) -> Profile {
        Profile {
            gateway_url: "http://127.0.0.1:8120".to_owned(),
            issuer: "http://idp.test".to_owned(),
            tenant_id: "0198f000-0000-7000-8000-000000000000".to_owned(),
            tenant_slug: None,
            subject: "alice".to_owned(),
            access_token: "at".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(seconds)),
            refresh_token: Some("rt".to_owned()),
        }
    }

    #[test]
    fn a_refused_pre_emptive_refresh_keeps_a_token_that_still_works() {
        // Inside the refresh skew but not yet expired: this is the window
        // Rauthy's not-before sits in, and the stored token is still good.
        assert_eq!(
            recover(&profile_expiring_in(30), "issuer said no"),
            Recovery::UseStored
        );
        // Actually expired: there is nothing to fall back to.
        match recover(&profile_expiring_in(-1), "issuer said no") {
            Recovery::Fail(message) => assert!(
                message.contains("synveda login"),
                "unhelpful message: {message}"
            ),
            other => panic!("an expired token must not be used: {other:?}"),
        }
        // An issuer that reported no lifetime: the gateway is the
        // authority on validity, so try the token rather than refuse.
        let mut unknown = profile_expiring_in(0);
        unknown.expires_at = None;
        assert_eq!(recover(&unknown, "issuer said no"), Recovery::UseStored);
    }

    #[test]
    fn login_state_is_random_and_urlsafe() {
        let a = random_urlsafe().expect("entropy");
        let b = random_urlsafe().expect("entropy");
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
