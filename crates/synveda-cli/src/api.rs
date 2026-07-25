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
            },
            Origin::Profile(profile_name.to_owned()),
        ))
    }

    /// The gateway this client talks to.
    pub fn gateway(&self) -> &str {
        &self.base
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
