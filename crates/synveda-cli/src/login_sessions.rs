//! `synveda auth sessions|revoke-session|revoke-all-sessions` and
//! `synveda auth logout --revoke`: the caller's own console sessions and
//! presented credential over the public API (ADR-0130).
//!
//! Console sessions are listed by public id only. A CLI login keeps its own
//! refresh token, so it is not in that inventory; `logout --revoke family`
//! records its issuer session family instead, which also ends any console
//! session that shares the same issuer sign-in.

use serde::Deserialize;
use serde_json::json;

use crate::api::Api;

/// Requests one `revoke-all-sessions` may make before reporting partial
/// progress. Each request commits up to four bounded batches.
const MAX_REVOKE_ALL_REQUESTS: usize = 64;

#[derive(Deserialize)]
struct SessionPage {
    sessions: Vec<SessionView>,
    next_after: Option<String>,
}

#[derive(Deserialize)]
struct SessionView {
    id: String,
    issuer: String,
    created_at: String,
    last_seen_at: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct RevokeAllProgress {
    revoked: u32,
    complete: bool,
}

#[derive(Deserialize)]
struct CredentialRevocation {
    identifier: String,
    enforced: bool,
}

/// `synveda auth sessions`: one page, newest first.
pub async fn list(profile: &str, after: Option<&str>, json_out: bool) -> Result<(), String> {
    let (api, _) = Api::connect(profile).await?;
    let path = match after {
        Some(after) => format!("/v1/me/console-sessions?after={}", encode(after)),
        None => "/v1/me/console-sessions".to_owned(),
    };
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let page: SessionPage = api.get_as(&path).await?;
    if page.sessions.is_empty() {
        println!("no console sessions");
    }
    for session in &page.sessions {
        println!("{}", session.id);
        println!("  issuer     {}", session.issuer);
        println!("  signed in  {}", session.created_at);
        println!("  last seen  {}", session.last_seen_at);
        println!("  expires    {}", session.expires_at);
    }
    if let Some(next) = page.next_after {
        println!("\nmore: synveda auth sessions --after {next}");
    }
    Ok(())
}

/// `synveda auth revoke-session <id>`. The gateway answers absent, foreign
/// and already revoked ids exactly like a fresh revocation.
pub async fn revoke(profile: &str, id: &str) -> Result<(), String> {
    let id: uuid::Uuid = id
        .parse()
        .map_err(|_| "session id must be a UUID from `synveda auth sessions`".to_owned())?;
    let (api, _) = Api::connect(profile).await?;
    api.delete(&format!("/v1/me/console-sessions/{id}")).await?;
    eprintln!("synveda: console session {id} is not live");
    Ok(())
}

/// `synveda auth revoke-all-sessions`, repeated until the gateway reports
/// completion or the request bound is reached.
pub async fn revoke_all(profile: &str, json_out: bool) -> Result<(), String> {
    let (api, _) = Api::connect(profile).await?;
    let mut revoked = 0_u64;
    for _ in 0..MAX_REVOKE_ALL_REQUESTS {
        let progress: RevokeAllProgress = api
            .post_as("/v1/me/console-sessions/revoke-all", None)
            .await?;
        revoked += u64::from(progress.revoked);
        if progress.complete {
            report_revoke_all(revoked, true, json_out);
            return Ok(());
        }
    }
    report_revoke_all(revoked, false, json_out);
    Err("console sessions remain; run `synveda auth revoke-all-sessions` again".to_owned())
}

fn report_revoke_all(revoked: u64, complete: bool, json_out: bool) {
    if json_out {
        println!("{}", json!({"revoked": revoked, "complete": complete}));
    } else {
        eprintln!("synveda: revoked {revoked} console session(s)");
    }
}

/// Records the profile's current bearer, or its issuer session family, as
/// revoked before the caller forgets the local credentials.
pub async fn revoke_presented(profile: &str, identifier: &str) -> Result<(), String> {
    let (api, _) = Api::connect(profile).await?;
    let recorded: CredentialRevocation = api
        .post_as(
            "/v1/me/credential/revoke",
            Some(json!({"identifier": identifier})),
        )
        .await?;
    if recorded.enforced {
        eprintln!(
            "synveda: revoked this credential's {} at the gateway",
            recorded.identifier
        );
    } else {
        eprintln!(
            "synveda: recorded this credential's {} as revoked; this issuer's profile reports \
             but does not yet refuse revoked credentials",
            recorded.identifier
        );
    }
    Ok(())
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_cursors_are_query_safe() {
        assert_eq!(
            encode("1729700000123456.0198a3b4-0000-7000-8000-000000000001"),
            "1729700000123456.0198a3b4-0000-7000-8000-000000000001"
        );
        assert_eq!(encode("a b&c"), "a%20b%26c");
    }

    #[test]
    fn the_session_shape_is_the_one_the_gateway_serves() {
        let page: SessionPage = serde_json::from_value(json!({
            "sessions": [{
                "id": "0198a3b4-0000-7000-8000-000000000001",
                "issuer": "https://auth.example.test/realms/synveda",
                "created_at": "2026-10-24T10:00:00Z",
                "last_seen_at": "2026-10-24T10:05:00Z",
                "expires_at": "2026-10-24T22:00:00Z"
            }]
        }))
        .expect("the gateway's ConsoleSessionPage shape");
        assert_eq!(page.sessions.len(), 1);
        assert!(page.next_after.is_none());
    }
}
