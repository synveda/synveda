//! Okta, through its Users and Groups APIs (AUTH-5, ADR-0060).
//!
//! The same three reads as Entra and two differences that matter.
//!
//! **Paging is in a header.** Okta returns `Link: <url>; rel="next"` rather
//! than a field in the body, and it also returns `rel="self"` — following the
//! wrong one is an infinite loop that re-reads page one and looks, from the
//! outside, exactly like a directory that never finishes. The `rel` is parsed
//! and matched rather than assumed.
//!
//! **Deactivation is a status word, not a boolean.** Okta users carry
//! `status` — `ACTIVE`, `PROVISIONED`, `SUSPENDED`, `DEPROVISIONED` and
//! others — and only some of those mean "this person is gone". Mapping it is
//! a decision rather than a parse, and it is made explicitly in
//! [`is_active`], because getting it wrong in the permissive direction leaves
//! leavers live and in the strict direction seals somebody who is merely
//! mid-onboarding.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

use super::{
    DirectoryConnector, DirectorySnapshot, DirectoryUserRecord, Enumeration, Secret, http_client,
    redact,
};

/// Okta's page ceiling for the users collection.
const PAGE_SIZE: usize = 200;

/// Reads an Okta org.
pub struct OktaConnector {
    http: reqwest::Client,
    org_url: String,
    api_token: Secret,
}

#[derive(Deserialize)]
struct OktaUser {
    id: String,
    status: String,
    profile: OktaProfile,
}

#[derive(Deserialize)]
struct OktaProfile {
    login: Option<String>,
    email: Option<String>,
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct OktaGroup {
    id: String,
    profile: OktaGroupProfile,
}

#[derive(Deserialize)]
struct OktaGroupProfile {
    name: Option<String>,
}

/// Whether an Okta lifecycle status means the person is still here.
///
/// `DEPROVISIONED` is Okta's leaver, and `SUSPENDED` is an administrator
/// switching somebody off — both are acts, and both seal on the first
/// complete pass that sees them. Everything else, including `STAGED`,
/// `PROVISIONED`, `RECOVERY` and `PASSWORD_EXPIRED`, is somebody who works
/// here and is having a normal week.
///
/// Unknown statuses count as **active**, deliberately. Okta has added
/// statuses before and will again, and the failure modes are not symmetric: a
/// new status read as active leaves somebody with access they already had,
/// while a new status read as inactive seals a personal scope that does not
/// lift (ADR-0059 decision 12).
fn is_active(status: &str) -> bool {
    !matches!(
        status.to_ascii_uppercase().as_str(),
        "DEPROVISIONED" | "SUSPENDED"
    )
}

/// The `rel="next"` URL from an RFC 8288 `Link` header, if there is one.
///
/// Okta sends `self` and `next` in the same header, comma-separated. Taking
/// the first URL rather than the one whose `rel` is `next` is an infinite
/// loop over page one.
fn next_link(header: Option<&str>) -> Option<String> {
    let header = header?;
    for link in header.split(',') {
        let mut parts = link.split(';');
        let url = parts.next()?.trim();
        let is_next = parts.any(|part| {
            let part = part.trim().replace(' ', "");
            part == "rel=\"next\"" || part == "rel=next"
        });
        if is_next {
            let url = url.trim_start_matches('<').trim_end_matches('>');
            return Some(url.to_owned());
        }
    }
    None
}

impl OktaConnector {
    /// Builds a connector for one Okta org.
    ///
    /// # Errors
    /// If the shared HTTP client cannot be constructed.
    pub fn new(org_url: String, api_token: Secret) -> synveda_types::Result<Self> {
        Ok(Self {
            http: http_client()?,
            org_url: org_url.trim_end_matches('/').to_owned(),
            api_token,
        })
    }

    /// Walks one paged collection to its end, or to its first failure,
    /// returning both — `entra::EntraConnector::walk`'s contract, over a
    /// different paging mechanism.
    async fn walk<T: serde::de::DeserializeOwned>(
        &self,
        first: String,
    ) -> (Vec<T>, Option<String>) {
        let mut collected = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next {
            let response = match self
                .http
                .get(&url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("SSWS {}", self.api_token.expose()),
                )
                .send()
                .await
            {
                Ok(response) => response,
                Err(err) => return (collected, Some(self.scrub(&format!("GET {url}: {err}")))),
            };
            if !response.status().is_success() {
                let status = response.status();
                return (collected, Some(format!("GET {url} answered {status}")));
            }
            let link = response
                .headers()
                .get(reqwest::header::LINK)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let page: Vec<T> = match response.json().await {
                Ok(page) => page,
                Err(err) => {
                    return (
                        collected,
                        Some(self.scrub(&format!("decoding {url}: {err}"))),
                    );
                }
            };
            collected.extend(page);
            next = next_link(link.as_deref());
        }
        (collected, None)
    }

    fn scrub(&self, message: &str) -> String {
        redact(message, self.api_token.expose())
    }
}

#[async_trait]
impl DirectoryConnector for OktaConnector {
    fn name(&self) -> &'static str {
        "okta"
    }

    #[tracing::instrument(
        name = "directory.okta.enumerate",
        skip_all,
        fields(directory.users = tracing::field::Empty, directory.complete = tracing::field::Empty)
    )]
    async fn enumerate(&self) -> Enumeration {
        let mut snapshot = DirectorySnapshot::default();

        let users_url = format!("{}/api/v1/users?limit={PAGE_SIZE}", self.org_url);
        let (users, failure) = self.walk::<OktaUser>(users_url).await;
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for user in users {
            // No login is Okta's equivalent of Entra's missing UPN: nothing
            // this product can address or match on.
            let Some(user_name) = user.profile.login else {
                continue;
            };
            by_id.insert(user.id.clone(), snapshot.users.len());
            snapshot.users.push(DirectoryUserRecord {
                external_id: user.id,
                user_name,
                active: is_active(&user.status),
                display_name: user.profile.display_name,
                given_name: user.profile.first_name,
                family_name: user.profile.last_name,
                work_email: user.profile.email,
                groups: Vec::new(),
            });
        }
        if let Some(failure) = failure {
            return partial(snapshot, failure);
        }

        let groups_url = format!("{}/api/v1/groups?limit={PAGE_SIZE}", self.org_url);
        let (groups, failure) = self.walk::<OktaGroup>(groups_url).await;
        if let Some(failure) = failure {
            return partial(snapshot, failure);
        }

        for group in groups {
            let Some(name) = group.profile.name else {
                continue;
            };
            let members_url = format!(
                "{}/api/v1/groups/{}/users?limit={PAGE_SIZE}",
                self.org_url, group.id
            );
            let (members, failure) = self.walk::<OktaUser>(members_url).await;
            for member in members {
                if let Some(index) = by_id.get(&member.id) {
                    snapshot.users[*index].groups.push(name.clone());
                }
            }
            if let Some(failure) = failure {
                return partial(snapshot, failure);
            }
        }

        complete(snapshot)
    }
}

fn partial(snapshot: DirectorySnapshot, failure: String) -> Enumeration {
    let span = tracing::Span::current();
    span.record("directory.users", snapshot.users.len());
    span.record("directory.complete", false);
    Enumeration::Partial { snapshot, failure }
}

fn complete(snapshot: DirectorySnapshot) -> Enumeration {
    let span = tracing::Span::current();
    span.record("directory.users", snapshot.users.len());
    span.record("directory.complete", true);
    Enumeration::Complete(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_link_is_the_one_whose_rel_says_so() {
        // Okta sends `self` first. Taking the first URL re-reads page one
        // forever, which from outside looks like a directory that never
        // finishes rather than like a bug.
        let header = "<https://ex.okta.com/api/v1/users?after=a>; rel=\"self\", \
                      <https://ex.okta.com/api/v1/users?after=b>; rel=\"next\"";
        assert_eq!(
            next_link(Some(header)).as_deref(),
            Some("https://ex.okta.com/api/v1/users?after=b")
        );
        // The last page carries only `self`, and that is how paging ends.
        assert_eq!(
            next_link(Some("<https://ex.okta.com/api/v1/users>; rel=\"self\"")),
            None
        );
        assert_eq!(next_link(None), None);
    }

    #[test]
    fn only_a_deliberate_status_means_gone() {
        assert!(is_active("ACTIVE"));
        assert!(is_active("PROVISIONED"));
        assert!(is_active("STAGED"));
        assert!(is_active("PASSWORD_EXPIRED"));
        assert!(!is_active("DEPROVISIONED"));
        assert!(!is_active("SUSPENDED"));
        // A status Okta has not invented yet reads as active, because a
        // wrong guess in that direction leaves access somebody already had,
        // and a wrong guess the other way seals a scope that does not lift.
        assert!(is_active("SOME_FUTURE_STATUS"));
    }
}
