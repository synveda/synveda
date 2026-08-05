//! `synveda scim token` — the provisioning credential from a terminal
//! (AUTH-4, ADR-0059 decision 13).
//!
//! HTTP only, like `hierarchy` and `lapse` and for their reason: issuing a
//! credential is a governed act, decided by the PDP at the tenant resource
//! and chained, and a verb that wrote the row directly would answer a
//! governed question with no decision in the trail. `synveda policy` and
//! `synveda role` are the direct-store operator plumbing; this is not one
//! of those (ADR-0058 decision 8's correction, applied on the way in).
//!
//! The issued token is printed **once**, to stdout, and never stored — the
//! gateway keeps only its SHA-256. Everything else about the credential is
//! readable forever through `list`.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::api::{Api, Origin};

/// One credential's record, as the gateway renders it.
#[derive(Deserialize)]
pub struct CredentialView {
    id: String,
    label: String,
    expires_at: DateTime<Utc>,
    #[serde(default)]
    revoked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_used_at: Option<DateTime<Utc>>,
    /// Kept for the `--json` form, where an operator reconciling a
    /// rotation wants to know when each one was issued. The table renders
    /// expiry and last use, which are what a rotation decision turns on.
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
    created_by: String,
}

/// The `GET /v1/scim/credentials` envelope.
#[derive(Deserialize)]
pub struct Listing {
    credentials: Vec<CredentialView>,
}

/// The `POST /v1/scim/credentials` response — the one and only time the
/// token itself is readable.
#[derive(Deserialize)]
pub struct Issued {
    id: String,
    label: String,
    expires_at: DateTime<Utc>,
    token: String,
}

impl CredentialView {
    /// What this credential is *doing*, resolved once so the renderer and
    /// any future consumer agree: revoked beats expired, because a
    /// credential revoked before its expiry is not "expired".
    fn state(&self, now: DateTime<Utc>) -> &'static str {
        if self.revoked_at.is_some() {
            "revoked"
        } else if self.expires_at <= now {
            "expired"
        } else {
            "live"
        }
    }
}

/// `synveda scim token issue --label <name> [--days N]`.
pub async fn issue(
    profile: &str,
    label: &str,
    days: Option<i64>,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "issuing");
    let body = match days {
        Some(days) => serde_json::json!({"label": label, "expires_in_days": days}),
        None => serde_json::json!({"label": label}),
    };
    if json_out {
        println!("{}", api.post("/v1/scim/credentials", Some(body)).await?);
        return Ok(());
    }
    let issued: Issued = api.post_as("/v1/scim/credentials", Some(body)).await?;
    println!("{}", render_issued(&issued));
    Ok(())
}

/// The issue output, as a value so a test can read it.
#[must_use]
pub fn render_issued(issued: &Issued) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "credential {} ({}) expires {}\n\n",
        &issued.id[..8.min(issued.id.len())],
        issued.label,
        issued.expires_at.format("%Y-%m-%d %H:%M UTC"),
    ));
    out.push_str(&format!("{}\n\n", issued.token));
    // The warning is the interface. A token printed without it reads like
    // something the product will show again.
    out.push_str(
        "This is the only time this token is shown — the gateway stores only its\n\
         hash. Paste it into your IdP's provisioning configuration:\n\n  \
         Entra: Provisioning → Admin Credentials → Secret Token\n  \
         Okta:  Provisioning → Integration → API Token\n\n\
         Rotate by issuing a second one before revoking this one: two may be\n\
         live at once, so provisioning never stops for a rotation.\n",
    );
    out
}

/// `synveda scim token list`.
pub async fn list(profile: &str, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    if json_out {
        println!("{}", api.get("/v1/scim/credentials").await?);
        return Ok(());
    }
    let listing: Listing = api.get_as("/v1/scim/credentials").await?;
    println!("{}", render_listing(&listing, Utc::now()));
    Ok(())
}

/// The listing, as a value.
#[must_use]
pub fn render_listing(listing: &Listing, now: DateTime<Utc>) -> String {
    if listing.credentials.is_empty() {
        return "no provisioning credentials\n\nIssue one with `synveda scim token issue \
                --label <name>`; until then the\n/scim/v2 plane authenticates nobody.\n"
            .to_owned();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{:<10}  {:<7}  {:<24}  {:<20}  {}\n",
        "id", "state", "label", "expires", "last used"
    ));
    for credential in &listing.credentials {
        out.push_str(&format!(
            "{:<10}  {:<7}  {:<24}  {:<20}  {}\n",
            &credential.id[..8.min(credential.id.len())],
            credential.state(now),
            credential.label,
            credential.expires_at.format("%Y-%m-%d %H:%M UTC"),
            credential.last_used_at.map_or_else(
                || "never".to_owned(),
                |at| at.format("%Y-%m-%d %H:%M UTC").to_string()
            ),
        ));
    }
    let live = listing
        .credentials
        .iter()
        .filter(|credential| credential.state(now) == "live")
        .count();
    out.push_str(&format!(
        "\n{} credential(s), {live} live\n",
        listing.credentials.len()
    ));
    // Revoked and expired rows stay in the list on purpose: rotation is a
    // decision about a history, and a credential that vanished when it
    // stopped working would take with it the answer to "what was this
    // one, and who issued it".
    if let Some(issued_by) = listing.credentials.first().map(|c| &c.created_by) {
        out.push_str(&format!("most recent issued by {issued_by}\n"));
    }
    out
}

/// `synveda scim token revoke <id>`.
pub async fn revoke(profile: &str, id: &str) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "revoking");
    api.post(&format!("/v1/scim/credentials/{id}/revoke"), None)
        .await?;
    println!("credential {id} revoked");
    println!(
        "Provisioning with this token now fails at the gateway. Anything it\n\
         already did stays in the audit chain, named by this id."
    );
    Ok(())
}

fn announce(api: &Api, origin: &Origin, verb: &str) {
    match origin {
        Origin::Profile(name) => eprintln!("{verb} as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("{verb} as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(label: &str, expires_at: DateTime<Utc>, revoked: bool) -> CredentialView {
        CredentialView {
            id: "0f6f1b70-1111-2222-3333-444444444444".to_owned(),
            label: label.to_owned(),
            expires_at,
            revoked_at: revoked.then(Utc::now),
            last_used_at: None,
            created_at: Utc::now(),
            created_by: "alice@example.test".to_owned(),
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn revoked_beats_expired_in_the_state_a_row_reports() {
        // A credential revoked before its expiry is not "expired", and an
        // operator deciding what to rotate needs the difference: one was a
        // decision somebody took, the other is a clock.
        let now = at("2026-08-05T00:00:00Z");
        let past = at("2026-01-01T00:00:00Z");
        let future = at("2027-01-01T00:00:00Z");
        assert_eq!(view("a", future, false).state(now), "live");
        assert_eq!(view("a", past, false).state(now), "expired");
        assert_eq!(view("a", past, true).state(now), "revoked");
        assert_eq!(view("a", future, true).state(now), "revoked");
    }

    #[test]
    fn the_issue_output_says_the_token_is_shown_once() {
        // The sentence is load-bearing: a token printed without it reads
        // like something the product will show again, and this one never
        // will — only its hash is stored.
        let issued = Issued {
            id: "0f6f1b70-1111-2222-3333-444444444444".to_owned(),
            label: "entra".to_owned(),
            expires_at: at("2026-11-03T00:00:00Z"),
            token: "synveda_scim_v1.tenant.secret".to_owned(),
        };
        let rendered = render_issued(&issued);
        assert!(rendered.contains("synveda_scim_v1.tenant.secret"));
        assert!(rendered.contains("only time this token is shown"));
        assert!(rendered.contains("Secret Token"), "names Entra's field");
        assert!(rendered.contains("two may be\nlive at once"));
    }

    #[test]
    fn an_empty_listing_says_what_that_means_rather_than_nothing() {
        let rendered = render_listing(
            &Listing {
                credentials: Vec::new(),
            },
            at("2026-08-05T00:00:00Z"),
        );
        assert!(rendered.contains("authenticates nobody"));
        assert!(rendered.contains("scim token issue"));
    }

    #[test]
    fn a_listing_counts_the_live_ones_separately_from_the_history() {
        let now = at("2026-08-05T00:00:00Z");
        let listing = Listing {
            credentials: vec![
                view("current", at("2027-01-01T00:00:00Z"), false),
                view("rotated-out", at("2027-01-01T00:00:00Z"), true),
                view("lapsed", at("2026-01-01T00:00:00Z"), false),
            ],
        };
        let rendered = render_listing(&listing, now);
        assert!(rendered.contains("3 credential(s), 1 live"));
        assert!(rendered.contains("revoked"));
        assert!(rendered.contains("expired"));
    }
}
