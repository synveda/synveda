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
use url::Url;

use super::{
    ApiOrigin, DirectoryConnector, DirectoryGroupRecord, DirectoryItemKind, DirectorySnapshot,
    DirectoryUserRecord, Enumeration, EnumerationBudget, EnumerationLimits, Secret, bounded_json,
    http_client, redact,
};

/// Okta's page ceiling for the users collection.
const PAGE_SIZE: usize = 200;

/// Reads an Okta org.
pub struct OktaConnector {
    http: reqwest::Client,
    org_url: String,
    api_origin: ApiOrigin,
    api_token: Secret,
    limits: EnumerationLimits,
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
        let org_url = org_url.trim_end_matches('/').to_owned();
        Ok(Self {
            http: http_client()?,
            api_origin: ApiOrigin::parse(&org_url, "Okta org URL")?,
            org_url,
            api_token,
            limits: EnumerationLimits::DEFAULT,
        })
    }

    #[cfg(test)]
    fn with_limits(mut self, limits: EnumerationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Walks one paged collection to its end, or to its first failure,
    /// returning both — `entra::EntraConnector::walk`'s contract, over a
    /// different paging mechanism.
    async fn walk<T: serde::de::DeserializeOwned>(
        &self,
        first: String,
        kind: DirectoryItemKind,
        budget: &mut EnumerationBudget,
    ) -> (Vec<T>, Option<String>) {
        let mut collected = Vec::new();
        let mut next = Some(first);
        let mut current_page: Option<Url> = None;
        while let Some(continuation) = next {
            let url = match self
                .api_origin
                .resolve(current_page.as_ref(), &continuation)
            {
                Ok(url) => url,
                Err(failure) => return (collected, Some(failure)),
            };
            if let Err(failure) = budget.begin_page(&url) {
                return (collected, Some(failure));
            }
            let response = match self
                .http
                .get(url.clone())
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
            let page: Vec<T> = match bounded_json(response, budget, &url).await {
                Ok(page) => page,
                Err(err) => return (collected, Some(self.scrub(&err))),
            };
            if let Err(failure) = budget.retain(kind, page.len()) {
                return (collected, Some(failure));
            }
            collected.extend(page);
            next = next_link(link.as_deref());
            current_page = Some(url);
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
        let mut budget = EnumerationBudget::with_limits(self.limits);

        let users_url = format!("{}/api/v1/users?limit={PAGE_SIZE}", self.org_url);
        let (users, failure) = self
            .walk::<OktaUser>(users_url, DirectoryItemKind::Users, &mut budget)
            .await;
        for user in users {
            // No login is Okta's equivalent of Entra's missing UPN: nothing
            // this product can address or match on.
            let Some(user_name) = user.profile.login else {
                continue;
            };
            snapshot.users.push(DirectoryUserRecord {
                external_id: user.id,
                user_name,
                active: is_active(&user.status),
                display_name: user.profile.display_name,
                given_name: user.profile.first_name,
                family_name: user.profile.last_name,
                work_email: user.profile.email,
            });
        }
        if let Some(failure) = failure {
            return partial(snapshot, failure);
        }

        let groups_url = format!("{}/api/v1/groups?limit={PAGE_SIZE}", self.org_url);
        let (groups, failure) = self
            .walk::<OktaGroup>(groups_url, DirectoryItemKind::Groups, &mut budget)
            .await;
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
            let (members, failure) = self
                .walk::<OktaUser>(members_url, DirectoryItemKind::Members, &mut budget)
                .await;
            if let Some(failure) = failure {
                return partial(snapshot, failure);
            }
            snapshot.groups.push(DirectoryGroupRecord {
                external_id: group.id,
                display_name: name,
                member_external_ids: members.into_iter().map(|member| member.id).collect(),
            });
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::response::Json;
    use axum::routing::get;
    use serde_json::{Value, json};

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

    async fn bounded_connector(
        limits: EnumerationLimits,
        large_member_body: bool,
    ) -> (OktaConnector, Arc<AtomicUsize>) {
        async fn users() -> Json<Value> {
            Json(json!([{
                "id": "u1", "status": "ACTIVE",
                "profile": {"login": "alice@example.test"}
            }]))
        }
        async fn groups() -> Json<Value> {
            Json(json!([{"id": "g1", "profile": {"name": "core"}}]))
        }
        async fn members(State((hits, large)): State<(Arc<AtomicUsize>, bool)>) -> Json<Value> {
            hits.fetch_add(1, Ordering::SeqCst);
            Json(if large {
                json!([{
                    "id": "u1", "status": "ACTIVE",
                    "profile": {
                        "login": "alice@example.test",
                        "padding": "x".repeat(2_048)
                    }
                }])
            } else {
                json!([{
                    "id": "u1", "status": "ACTIVE",
                    "profile": {"login": "alice@example.test"}
                }])
            })
        }

        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/api/v1/users", get(users))
            .route("/api/v1/groups", get(groups))
            .route("/api/v1/groups/{group}/users", get(members))
            .with_state((Arc::clone(&hits), large_member_body));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("mock serve");
        });
        let connector =
            OktaConnector::new(format!("http://{addr}"), Secret::new("bounded-test-token"))
                .expect("connector")
                .with_limits(limits);
        (connector, hits)
    }

    #[tokio::test]
    async fn the_page_bound_is_shared_across_users_groups_and_members() {
        let (connector, member_hits) = bounded_connector(
            EnumerationLimits {
                pages: 2,
                ..EnumerationLimits::DEFAULT
            },
            false,
        )
        .await;
        let Enumeration::Partial { snapshot, failure } = connector.enumerate().await else {
            panic!("the third pass-wide page must be refused");
        };
        assert_eq!(snapshot.users.len(), 1, "earlier presence is retained");
        assert!(failure.contains("page bound"), "got {failure:?}");
        assert_eq!(
            member_hits.load(Ordering::SeqCst),
            0,
            "the over-budget authenticated request is never sent"
        );
    }

    #[tokio::test]
    async fn the_total_item_bound_returns_partial_with_earlier_presence() {
        let (connector, member_hits) = bounded_connector(
            EnumerationLimits {
                items: 2,
                ..EnumerationLimits::DEFAULT
            },
            false,
        )
        .await;
        let Enumeration::Partial { snapshot, failure } = connector.enumerate().await else {
            panic!("the third retained item must make the pass partial");
        };
        assert_eq!(snapshot.users.len(), 1);
        assert!(failure.contains("item bound"), "got {failure:?}");
        assert_eq!(member_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_decoded_byte_bound_returns_partial_without_body_in_the_error() {
        let (connector, member_hits) = bounded_connector(
            EnumerationLimits {
                bytes: 512,
                ..EnumerationLimits::DEFAULT
            },
            true,
        )
        .await;
        let Enumeration::Partial { snapshot, failure } = connector.enumerate().await else {
            panic!("the oversized decoded response must make the pass partial");
        };
        assert_eq!(snapshot.users.len(), 1);
        assert!(failure.contains("byte response bound"), "got {failure:?}");
        assert!(
            !failure.contains(&"x".repeat(32)),
            "response bytes must not be copied into an error: {failure}"
        );
        assert_eq!(member_hits.load(Ordering::SeqCst), 1);
    }
}
