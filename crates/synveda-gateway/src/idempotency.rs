//! The idempotency seam (CPR-4, ADR-0071 decision 6): what a creation route
//! wraps itself in so that a client's retry is safe.
//!
//! # The header is required, not optional
//!
//! Every creation route on this plane demands `Idempotency-Key`. Stripe makes
//! it optional and most clients then omit it, which means the guarantee exists
//! for the callers who did not need it and is absent for the one whose network
//! dropped. A required header costs a client one UUID and removes the failure
//! mode entirely — and the refusal names the header, so a caller that forgot
//! learns from a 400 rather than from two workspaces.
//!
//! # The three outcomes
//!
//! 1. **Key unseen** — the work runs, and [`crate::idempotency::Claim::remember`]
//!    records the key, a digest of the request and the resource produced,
//!    *inside the same transaction as the creation*. `201 Created`.
//! 2. **Key seen, same request** — nothing runs; the original resource is
//!    served with `200 OK`. The status is what tells a client the two apart,
//!    which is why there is no bespoke header saying the same thing.
//! 3. **Key seen, different request** — `409 Conflict`. The alternative is
//!    answering a request the caller did not make with the resource from one
//!    they did, and reporting it as success.
//!
//! # The race, and why it is not a conflict
//!
//! Two concurrent requests carrying one key both miss the lookup; the second
//! insert blocks on the primary key until the first commits, then fails. That
//! caller is not doing anything wrong — it is the timeout retry this whole
//! mechanism exists for, arriving early — so [`run`] re-reads and replays
//! rather than returning the conflict. A 409 there would make the guarantee
//! hold everywhere except under exactly the conditions that produce a retry.

use axum::http::HeaderMap;
use synveda_store::{idempotency, rls};
use synveda_types::{Error, Result, TenantId};
use uuid::Uuid;

/// The header a creation route takes its key from.
pub const HEADER: &str = "idempotency-key";

/// The caller's claim: a key, the subject that minted it, and a digest of the
/// request it belongs to.
#[derive(Debug, Clone)]
pub struct Claim {
    /// The operation the key is scoped to (`workspace.create`, …). Part of
    /// the primary key, so one client's key for one verb never shadows its
    /// key for another.
    pub operation: &'static str,
    /// The key itself.
    pub key: String,
    /// The token subject that presented it.
    pub subject: String,
    /// BLAKE3-256 of the canonical request.
    pub digest: [u8; 32],
}

impl Claim {
    /// Reads the header and hashes the request.
    ///
    /// `canonical` is the request rendered so that two byte-identical requests
    /// hash the same and two different ones do not: the route path with its
    /// parameters, then the parsed body re-serialised. Re-serialised rather
    /// than hashed as received, deliberately — a client that reformats its
    /// JSON between a request and its retry has not changed the request, and a
    /// digest over raw bytes would call that a conflict.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the header is absent, unreadable, or outside
    /// the key grammar.
    pub fn from_headers(
        headers: &HeaderMap,
        operation: &'static str,
        subject: &str,
        canonical: &serde_json::Value,
    ) -> Result<Self> {
        let key = headers
            .get(HEADER)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "creation requires an `Idempotency-Key` header: send a unique value \
                     per request (a UUID is fine) and reuse it verbatim when retrying, so \
                     a retry after a timeout cannot create a second {}",
                    operation.split('.').next().unwrap_or(operation)
                ),
            })?
            .to_str()
            .map_err(|_| Error::Invalid {
                message: "`Idempotency-Key` holds printable ASCII only".to_owned(),
            })?
            .trim()
            .to_owned();
        idempotency::validate_key(&key)?;
        Ok(Claim {
            operation,
            key,
            subject: subject.to_owned(),
            digest: *blake3::hash(canonicalise(canonical).to_string().as_bytes()).as_bytes(),
        })
    }

    /// Records the claim in the caller's transaction, beside the row it
    /// produced. Call immediately before the audit event, so all three commit
    /// together or none of them do.
    ///
    /// # Errors
    ///
    /// [`Error::Conflict`] when another transaction committed this key first
    /// — which [`run`] turns into a replay rather than surfacing.
    pub async fn remember(
        &self,
        tx: &mut sqlx::PgConnection,
        tenant_id: TenantId,
        resource_id: Uuid,
    ) -> Result<()> {
        idempotency::remember(
            tx,
            tenant_id,
            &self.subject,
            self.operation,
            &self.key,
            &self.digest,
            resource_id,
        )
        .await
    }
}

/// Sorts every object's keys, recursively, so the digest is over the request
/// rather than over how it happened to be written.
///
/// `serde_json`'s default map is already ordered, which makes this a no-op
/// today — and that is exactly why it is here rather than assumed: the
/// ordering is a *feature flag* away from becoming insertion order, and the
/// day some crate in the graph turns `preserve_order` on, a client's retry
/// would start reading as a conflict for a reason nobody would look for.
fn canonicalise(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            serde_json::Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonicalise(&map[key])))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalise).collect())
        }
        scalar => scalar.clone(),
    }
}

/// What a lookup decided the route should do next.
pub enum Dispatch {
    /// This key has not been used: run the work.
    Create,
    /// This key already produced this resource: serve it, unchanged.
    Replay(Uuid),
}

/// Looks the claim up on its own connection.
///
/// A short transaction of its own rather than the creation's, because the
/// answer decides whether the creation runs at all — and because a replay must
/// not hold the creation path's locks open while it reads a row it is not
/// going to write.
///
/// # Errors
///
/// [`Error::Conflict`] when the key was used for a different request.
pub async fn dispatch(pool: &sqlx::PgPool, tenant_id: TenantId, claim: &Claim) -> Result<Dispatch> {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id).await?;
    let found = idempotency::find(
        &mut *tx,
        tenant_id,
        &claim.subject,
        claim.operation,
        &claim.key,
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit idempotency lookup: {err}"),
    })?;

    match found {
        None => Ok(Dispatch::Create),
        Some(record) if record.request_digest == claim.digest => {
            Ok(Dispatch::Replay(record.resource_id))
        }
        Some(_) => Err(Error::Conflict {
            message: format!(
                "idempotency key {:?} was already used for a different {} request; \
                 a key identifies one request, so retrying a changed body needs a new key",
                claim.key, claim.operation
            ),
        }),
    }
}

/// Decides what a creation's own [`Error::Conflict`] meant.
///
/// A creation can conflict for two unrelated reasons and the caller must not
/// see them as one. Either another transaction committed *this key* between
/// the dispatch and the insert — the timeout retry arriving early, which must
/// replay — or the creation conflicted on something of its own, a taken slug
/// or an already-attached repository, which must surface as the 409 it is.
/// Only the first leaves a record, so a second lookup tells them apart.
///
/// Returns the resource to replay, or the original conflict unchanged.
///
/// # Errors
///
/// The `conflict` it was given, when the key produced nothing.
pub async fn resolve_conflict(
    pool: &sqlx::PgPool,
    tenant_id: TenantId,
    claim: &Claim,
    conflict: Error,
) -> Result<Uuid> {
    match dispatch(pool, tenant_id, claim).await? {
        Dispatch::Replay(id) => Ok(id),
        Dispatch::Create => Err(conflict),
    }
}

/// The refusal for a key whose resource is no longer there — reachable for
/// repositories, which are the one thing on this plane that can be detached.
///
/// A 404 rather than a replay of nothing: the caller's key did produce
/// something, it is gone, and saying so is more use than either a 200 with an
/// empty body or a 201 that attaches a second copy.
#[must_use]
pub fn vanished(claim: &Claim, id: Uuid) -> Error {
    Error::NotFound {
        entity: format!(
            "the {} that idempotency key {:?} produced ({id}) no longer exists",
            claim.operation, claim.key
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    fn headers(key: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(key) = key {
            headers.insert(HEADER, HeaderValue::from_str(key).expect("header"));
        }
        headers
    }

    #[test]
    fn a_missing_key_is_refused_and_the_refusal_names_the_header() {
        let error = Claim::from_headers(&headers(None), "workspace.create", "sam", &json!({}))
            .expect_err("refused");
        let Error::Invalid { message } = error else {
            panic!("expected Invalid");
        };
        assert!(message.contains("Idempotency-Key"), "{message}");
        assert!(
            message.contains("workspace"),
            "the refusal says what would have been created twice: {message}"
        );
    }

    #[test]
    fn a_malformed_key_is_refused() {
        for bad in ["", "   ", "with space"] {
            assert!(
                Claim::from_headers(&headers(Some(bad)), "workspace.create", "sam", &json!({}))
                    .is_err(),
                "{bad:?}"
            );
        }
    }

    /// Reformatting the JSON between a request and its retry does not change
    /// the request, so it must not change the digest.
    #[test]
    fn the_digest_is_over_the_parsed_request_not_its_formatting() {
        let claim = |body: serde_json::Value| {
            Claim::from_headers(&headers(Some("k1")), "workspace.create", "sam", &body)
                .expect("claim")
                .digest
        };
        assert_eq!(
            claim(json!({"slug": "payments", "display_name": "Payments"})),
            claim(json!({"display_name": "Payments", "slug": "payments"})),
            "key order is formatting, not content"
        );
        assert_ne!(
            claim(json!({"slug": "payments"})),
            claim(json!({"slug": "ledger"})),
            "a different request is a different digest"
        );
    }

    #[test]
    fn the_key_is_trimmed_but_not_otherwise_normalised() {
        let claim = Claim::from_headers(
            &headers(Some("  K-1  ")),
            "workspace.create",
            "sam",
            &json!({}),
        )
        .expect("claim");
        assert_eq!(claim.key, "K-1", "surrounding space is not part of a key");
    }
}
