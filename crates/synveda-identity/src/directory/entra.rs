//! Microsoft Entra ID, through Microsoft Graph (AUTH-5, ADR-0060).
//!
//! Three reads make a pass: every user, every group, and every group's
//! members. Stable Graph object ids become the shared Group aggregate's
//! directory address; display names remain mutable presentation.
//!
//! Paging is `@odata.nextLink`, which Graph returns as an absolute URL
//! carrying its own opaque skip token. It is followed as given rather than
//! reconstructed: a client that rebuilds the next page's query is a client
//! that silently re-reads page one when the vendor changes a parameter.
//!
//! [`walk`](EntraConnector::walk) returns what it read *and* the failure that
//! stopped it, rather than discarding one for the other. That shape is
//! ADR-0060 decision 3.1 in the small: presence survives an incomplete pass,
//! and it only survives if the code does not throw away pages one to three on
//! its way out of page four.

use async_trait::async_trait;
use serde::Deserialize;
use url::Url;

use super::{
    ApiOrigin, DirectoryConnector, DirectoryGroupRecord, DirectoryItemKind, DirectorySnapshot,
    DirectoryUserRecord, Enumeration, EnumerationBudget, EnumerationLimits, Secret, bounded_json,
    http_client, redact,
};

const GRAPH_BASE: &str = "https://graph.microsoft.com";
const LOGIN_BASE: &str = "https://login.microsoftonline.com";

/// The attributes this connector asks for, and therefore the only ones it can
/// be surprised by. `$select` is explicit so a directory with a hundred
/// custom attributes still answers in a bounded shape.
const USER_SELECT: &str = "id,userPrincipalName,displayName,givenName,surname,mail,accountEnabled";

/// Graph's own page ceiling for the users collection.
const PAGE_SIZE: usize = 999;

/// Reads an Entra tenant.
pub struct EntraConnector {
    http: reqwest::Client,
    tenant_id: String,
    client_id: String,
    client_secret: Secret,
    graph_base: String,
    login_base: String,
    graph_origin: ApiOrigin,
    login_origin: ApiOrigin,
    limits: EnumerationLimits,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct Page<T> {
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct GraphUser {
    id: String,
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "givenName")]
    given_name: Option<String>,
    surname: Option<String>,
    mail: Option<String>,
    /// Absent on some directory objects. A user Graph declines to describe is
    /// treated as **enabled**, because `active: false` is an act that seals
    /// on the first pass that sees it and a missing field is not one.
    #[serde(rename = "accountEnabled")]
    account_enabled: Option<bool>,
}

#[derive(Deserialize)]
struct GraphGroup {
    id: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct GraphMember {
    id: String,
}

impl EntraConnector {
    /// Builds a connector for one Entra tenant.
    ///
    /// # Errors
    /// If the shared HTTP client cannot be constructed.
    pub fn new(
        tenant_id: String,
        client_id: String,
        client_secret: Secret,
        graph_base: Option<String>,
        login_base: Option<String>,
    ) -> synveda_types::Result<Self> {
        let graph_base = graph_base.unwrap_or_else(|| GRAPH_BASE.to_owned());
        let login_base = login_base.unwrap_or_else(|| LOGIN_BASE.to_owned());
        Ok(Self {
            http: http_client()?,
            tenant_id,
            client_id,
            client_secret,
            graph_origin: ApiOrigin::parse(&graph_base, "Entra Graph base")?,
            login_origin: ApiOrigin::parse(&login_base, "Entra login base")?,
            graph_base: graph_base.trim_end_matches('/').to_owned(),
            login_base: login_base.trim_end_matches('/').to_owned(),
            limits: EnumerationLimits::DEFAULT,
        })
    }

    /// Client-credentials grant for the Graph `.default` scope.
    async fn token(&self, budget: &mut EnumerationBudget) -> Result<String, String> {
        let candidate = format!("{}/{}/oauth2/v2.0/token", self.login_base, self.tenant_id);
        let url = self
            .login_origin
            .resolve(None, &candidate)
            .map_err(|failure| self.scrub(&failure))?;
        budget.begin_page(&url)?;
        let response = self
            .http
            .post(url.clone())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.expose()),
                ("scope", "https://graph.microsoft.com/.default"),
            ])
            .send()
            .await
            .map_err(|err| self.scrub(&format!("token request: {err}")))?;
        if !response.status().is_success() {
            // The status and nothing else. An Entra token error echoes the
            // request it refused, and this is the one request in a pass whose
            // body carries the client secret.
            return Err(format!("token endpoint answered {}", response.status()));
        }
        let token: TokenResponse = bounded_json(response, budget, &url)
            .await
            .map_err(|err| self.scrub(&format!("token response: {err}")))?;
        Ok(token.access_token)
    }

    /// Walks one paged collection to its end, or to its first failure.
    ///
    /// Returns both: everything read, and `Some(failure)` if it stopped
    /// early. A caller that wants only the happy path can check the second
    /// element, but it cannot get it without also being handed the first.
    async fn walk<T: serde::de::DeserializeOwned>(
        &self,
        token: &str,
        first: String,
        kind: DirectoryItemKind,
        budget: &mut EnumerationBudget,
    ) -> (Vec<T>, Option<String>) {
        let mut collected = Vec::new();
        let mut next = Some(first);
        let mut current_page: Option<Url> = None;
        while let Some(continuation) = next {
            // Resolve and account before building the authenticated request:
            // a hostile continuation never receives the bearer even once.
            let url = match self
                .graph_origin
                .resolve(current_page.as_ref(), &continuation)
            {
                Ok(url) => url,
                Err(failure) => return (collected, Some(failure)),
            };
            if let Err(failure) = budget.begin_page(&url) {
                return (collected, Some(failure));
            }
            let response = match self.http.get(url.clone()).bearer_auth(token).send().await {
                Ok(response) => response,
                Err(err) => return (collected, Some(self.scrub(&format!("GET {url}: {err}")))),
            };
            if !response.status().is_success() {
                let status = response.status();
                return (collected, Some(format!("GET {url} answered {status}")));
            }
            let page: Page<T> = match bounded_json(response, budget, &url).await {
                Ok(page) => page,
                Err(err) => return (collected, Some(self.scrub(&err))),
            };
            if let Err(failure) = budget.retain(kind, page.value.len()) {
                return (collected, Some(failure));
            }
            collected.extend(page.value);
            // Followed as given: the skip token is opaque, and a client that
            // rebuilds this URL is one that re-reads page one forever.
            next = page.next_link;
            current_page = Some(url);
        }
        (collected, None)
    }

    fn scrub(&self, message: &str) -> String {
        redact(message, self.client_secret.expose())
    }
}

#[async_trait]
impl DirectoryConnector for EntraConnector {
    fn name(&self) -> &'static str {
        "entra"
    }

    #[tracing::instrument(
        name = "directory.entra.enumerate",
        skip_all,
        fields(directory.users = tracing::field::Empty, directory.complete = tracing::field::Empty)
    )]
    async fn enumerate(&self) -> Enumeration {
        let mut snapshot = DirectorySnapshot::default();
        let mut budget = EnumerationBudget::with_limits(self.limits);

        let token = match self.token(&mut budget).await {
            Ok(token) => token,
            Err(failure) => return partial(snapshot, failure),
        };

        let users_url = format!(
            "{}/v1.0/users?$select={USER_SELECT}&$top={PAGE_SIZE}",
            self.graph_base
        );
        let (users, failure) = self
            .walk::<GraphUser>(&token, users_url, DirectoryItemKind::Users, &mut budget)
            .await;
        for user in users {
            // Somebody with no UPN cannot be matched to an identity or
            // addressed over SCIM, so there is nothing this product could do
            // with them. Skipped rather than invented.
            let Some(user_name) = user.user_principal_name else {
                continue;
            };
            snapshot.users.push(DirectoryUserRecord {
                external_id: user.id,
                user_name,
                active: user.account_enabled.unwrap_or(true),
                display_name: user.display_name,
                given_name: user.given_name,
                family_name: user.surname,
                work_email: user.mail,
            });
        }
        if let Some(failure) = failure {
            return partial(snapshot, failure);
        }

        let groups_url = format!("{}/v1.0/groups?$select=id,displayName", self.graph_base);
        let (groups, failure) = self
            .walk::<GraphGroup>(&token, groups_url, DirectoryItemKind::Groups, &mut budget)
            .await;
        if let Some(failure) = failure {
            return partial(snapshot, failure);
        }

        for group in groups {
            let Some(display_name) = group.display_name else {
                continue;
            };
            let members_url = format!(
                "{}/v1.0/groups/{}/members?$select=id",
                self.graph_base, group.id
            );
            let (members, failure) = self
                .walk::<GraphMember>(&token, members_url, DirectoryItemKind::Members, &mut budget)
                .await;
            if let Some(failure) = failure {
                return partial(snapshot, failure);
            }
            snapshot.groups.push(DirectoryGroupRecord {
                external_id: group.id,
                display_name,
                member_external_ids: members.into_iter().map(|member| member.id).collect(),
            });
        }

        complete(snapshot)
    }
}

/// Records the outcome on the span before returning it, so a pass's shape is
/// visible in a trace without the caller having to log it twice.
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
