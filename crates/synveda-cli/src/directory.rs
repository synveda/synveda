//! `synveda directory` — the pull sync from a terminal (AUTH-5, ADR-0060).
//!
//! HTTP only, like `scim token` and for its reason: authorising a bulk seal
//! is a governed act, decided by the PDP at the tenant resource and chained,
//! and a verb that wrote the row directly would answer a governed question
//! with no decision in the trail.
//!
//! Two commands, and they are two halves of one act. `status` shows what the
//! last pass did — including, when the circuit breaker refused, *how many*
//! it refused. `authorise-seals` signs for that number. The read and the
//! signature share one PDP action for exactly this reason: a signer who
//! cannot see the number they are bounding is signing blind.
//!
//! The renderer leads with the refusal when there is one, because an
//! operator runs `status` at all because something is wrong, and a breaker
//! trip buried under four lines of timestamps is a breaker trip nobody acts
//! on.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::api::{Api, Origin};

/// `GET /v1/directory/sync`.
#[derive(Deserialize)]
pub struct SyncStatus {
    connector: String,
    passes_completed: i64,
    #[serde(default)]
    last_pass_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_complete_pass_at: Option<DateTime<Utc>>,
    #[serde(default)]
    breaker_tripped_at: Option<DateTime<Utc>>,
    #[serde(default)]
    breaker_would_have_sealed: Option<i32>,
    #[serde(default)]
    seal_authorisation: Option<AuthorisationView>,
}

/// A standing authorisation, as the gateway renders it.
#[derive(Deserialize)]
pub struct AuthorisationView {
    expires_at: DateTime<Utc>,
    ceiling: i32,
    granted_by: String,
    reason: String,
}

/// `synveda directory status`.
pub async fn status(profile: &str, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    if json_out {
        println!("{}", api.get("/v1/directory/sync").await?);
        return Ok(());
    }
    let status: SyncStatus = api.get_as("/v1/directory/sync").await?;
    println!("{}", render_status(&status, Utc::now()));
    Ok(())
}

/// `synveda directory authorise-seals --ceiling N --reason "…"`.
pub async fn authorise_seals(
    profile: &str,
    ceiling: i32,
    reason: &str,
    hours: Option<f64>,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "authorising");
    let mut body = serde_json::json!({"ceiling": ceiling, "reason": reason});
    if let Some(hours) = hours
        && let Some(map) = body.as_object_mut()
    {
        map.insert("expires_in_secs".to_owned(), (hours * 3600.0).into());
    }
    let path = "/v1/directory/seal-authorisations";
    if json_out {
        println!("{}", api.post(path, Some(body)).await?);
        return Ok(());
    }
    api.post(path, Some(body)).await?;
    println!(
        "authorised: the next complete pass may seal up to {ceiling} \
         {}, once.\n  \
         it is spent by the first pass that uses it, and a pass proposing \
         more than {ceiling} will refuse again rather than round down.",
        if ceiling == 1 { "person" } else { "people" }
    );
    Ok(())
}

/// What a `status` reads like.
///
/// A refusal goes **first and unindented**; everything else is context for
/// it. The other line worth a second look is a `last_pass_at` well ahead of
/// `last_complete_pass_at`, which is a connector that runs and never
/// finishes — the state in which nobody is being sealed and nothing looks
/// wrong, so the renderer says so rather than leaving two timestamps for
/// somebody to compare.
fn render_status(status: &SyncStatus, now: DateTime<Utc>) -> String {
    let mut out = String::new();

    if let (Some(tripped), Some(count)) =
        (status.breaker_tripped_at, status.breaker_would_have_sealed)
    {
        out.push_str(&format!(
            "BREAKER TRIPPED {}\n  the last complete pass declined to seal {count} \
             {}.\n  nobody was sealed. authorise with:\n    \
             synveda directory authorise-seals --ceiling {count} --reason \"…\"\n\n",
            stamp(tripped),
            if count == 1 { "person" } else { "people" }
        ));
    }

    out.push_str(&format!("connector         {}\n", status.connector));
    out.push_str(&format!("passes completed  {}\n", status.passes_completed));
    out.push_str(&format!(
        "last attempt      {}\n",
        status
            .last_pass_at
            .map_or_else(|| "never".to_owned(), stamp)
    ));
    out.push_str(&format!(
        "last completed    {}\n",
        status
            .last_complete_pass_at
            .map_or_else(|| "never".to_owned(), stamp)
    ));

    if let Some(attempted) = status.last_pass_at {
        let stalled = match status.last_complete_pass_at {
            Some(completed) => attempted > completed,
            None => true,
        };
        if stalled {
            out.push_str(
                "\n  the last attempt did not complete. absence is not being \
                 counted, so nobody is being sealed — which looks the same \
                 as a directory where nobody has left.\n",
            );
        }
    }

    match &status.seal_authorisation {
        Some(granted) if granted.expires_at > now => {
            out.push_str(&format!(
                "\nauthorisation in force\n  up to {} {}, until {}\n  signed by {}\n  \
                 reason: {}\n",
                granted.ceiling,
                if granted.ceiling == 1 {
                    "person"
                } else {
                    "people"
                },
                stamp(granted.expires_at),
                granted.granted_by,
                granted.reason,
            ));
        }
        Some(granted) => {
            // Expiry is not erasure: the row keeps it, and an operator
            // chasing "why did nothing happen" needs to see that the window
            // closed rather than that no authorisation was ever signed.
            out.push_str(&format!(
                "\nauthorisation EXPIRED at {} (up to {}, signed by {})\n",
                stamp(granted.expires_at),
                granted.ceiling,
                granted.granted_by,
            ));
        }
        None => {}
    }
    out
}

fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%SZ").to_string()
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

    fn base() -> SyncStatus {
        SyncStatus {
            connector: "entra".to_owned(),
            passes_completed: 12,
            last_pass_at: Some(Utc::now()),
            last_complete_pass_at: Some(Utc::now()),
            breaker_tripped_at: None,
            breaker_would_have_sealed: None,
            seal_authorisation: None,
        }
    }

    #[test]
    fn a_refusal_leads_and_carries_the_command_that_clears_it() {
        // An operator runs `status` because something is wrong. A trip
        // buried under four timestamps is a trip nobody acts on — and the
        // number they must pass is the number the pass refused, so the
        // renderer writes the command out rather than leaving them to
        // assemble it under pressure.
        let mut status = base();
        status.breaker_tripped_at = Some(Utc::now());
        status.breaker_would_have_sealed = Some(300);
        let rendered = render_status(&status, Utc::now());
        assert!(rendered.starts_with("BREAKER TRIPPED"));
        assert!(rendered.contains("declined to seal 300 people"));
        assert!(rendered.contains("--ceiling 300"));
    }

    #[test]
    fn a_stalled_connector_is_named_rather_than_left_as_two_timestamps() {
        // The dangerous quiet state: attempts happening, none completing,
        // so absence never accumulates and nobody is ever sealed. From the
        // outside that is indistinguishable from a company where nobody has
        // left, which is why it gets a sentence instead of arithmetic.
        let mut status = base();
        status.last_complete_pass_at = Some(Utc::now() - chrono::Duration::hours(9));
        status.last_pass_at = Some(Utc::now());
        let rendered = render_status(&status, Utc::now());
        assert!(rendered.contains("did not complete"));
        assert!(rendered.contains("nobody is being sealed"));

        // And it is not said when the last attempt did finish.
        let healthy = render_status(&base(), Utc::now());
        assert!(!healthy.contains("did not complete"));
    }

    #[test]
    fn an_expired_authorisation_reads_as_expired_rather_than_absent() {
        // "No authorisation" and "an authorisation whose window closed" send
        // an operator to different places: one to sign, one to ask why the
        // pass did not spend it in time.
        let mut status = base();
        status.seal_authorisation = Some(AuthorisationView {
            expires_at: Utc::now() - chrono::Duration::minutes(5),
            ceiling: 300,
            granted_by: "alice@example.test".to_owned(),
            reason: "Q3 restructure".to_owned(),
        });
        let rendered = render_status(&status, Utc::now());
        assert!(rendered.contains("EXPIRED"));
        assert!(!rendered.contains("in force"));

        status.seal_authorisation = Some(AuthorisationView {
            expires_at: Utc::now() + chrono::Duration::hours(1),
            ceiling: 300,
            granted_by: "alice@example.test".to_owned(),
            reason: "Q3 restructure".to_owned(),
        });
        let rendered = render_status(&status, Utc::now());
        assert!(rendered.contains("in force"));
        assert!(rendered.contains("Q3 restructure"));
        assert!(!rendered.contains("EXPIRED"));
    }

    #[test]
    fn one_person_is_not_pluralised() {
        let mut status = base();
        status.breaker_tripped_at = Some(Utc::now());
        status.breaker_would_have_sealed = Some(1);
        let rendered = render_status(&status, Utc::now());
        assert!(rendered.contains("declined to seal 1 person"));
    }
}
