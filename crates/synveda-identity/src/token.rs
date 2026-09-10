//! Bearer-token verification (TEN-1, ADR-0008).
//!
//! The [`TokenVerifier`] trait is the AuthN seam. The real implementation is
//! the OIDC/JWKS verifier (`oidc` module, AUTH-1/ADR-0010); [`Hs256Verifier`]
//! verifies HMAC-SHA256 JWTs signed with a shared dev secret for CLI/demo
//! bootstrap, and [`DisabledVerifier`] is the fail-closed default when
//! neither mode is configured. `verify` is async because OIDC verification
//! may fetch discovery documents or rotated keys mid-request.
//!
//! Claims contract (ADR-0008): `sub` names the subject, `tid` carries the
//! tenant UUID, `exp` is mandatory. The algorithm is pinned — the token
//! header's `alg` is checked against HS256 and never used to select a
//! scheme, so there is no algorithm-confusion surface.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use synveda_types::{Error, Result, TenantId};

type HmacSha256 = Hmac<Sha256>;

/// Which credential boundary produced verified claims.
///
/// OIDC service-audience tokens are kept distinct through tenant resolution:
/// they may name only a registered service identity. That prevents an ID
/// token minted for an accidentally interactive service client from becoming
/// a user bearer merely because its client id was configured as an accepted
/// service audience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialClass {
    /// An ID token verified inside the authorization-code callback.
    Interactive,
    /// A bearer accepted through the primary API resource audience, or the
    /// development HS256 verifier that has no OIDC audience vocabulary.
    PrimaryBearer,
    /// A bearer accepted exclusively through an additional service audience.
    ServiceBearer,
}

/// Clock skew allowed when an issuer's `iat` is marginally ahead of the
/// verifier. The same bound is used by dev/service and OIDC verification so
/// one token authority cannot admit a time shape the other refuses.
pub(crate) const FUTURE_IAT_LEEWAY: Duration = Duration::from_secs(30);

/// Validates ordered token times and returns the issued lifetime when `iat`
/// is present. Missing `iat` remains an unknown lifetime for the service-
/// identity enforcement seam; a malformed one is never disguised as a zero
/// lifetime.
pub(crate) fn issued_lifetime(exp: u64, iat: Option<u64>, now: u64) -> Result<Option<Duration>> {
    let Some(iat) = iat else {
        return Ok(None);
    };
    let seconds = exp.checked_sub(iat).ok_or_else(|| Error::Unauthenticated {
        message: "token issued-at is after its expiry".to_owned(),
    })?;
    let latest = now.saturating_add(FUTURE_IAT_LEEWAY.as_secs());
    if iat > latest {
        return Err(Error::Unauthenticated {
            message: "token issued-at is too far in the future".to_owned(),
        });
    }
    Ok(Some(Duration::from_secs(seconds)))
}

/// The verified claims a token resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    /// The token's `sub`: who is acting.
    pub subject: String,
    /// The token's `tid`: which tenant the request runs as.
    pub tenant_id: TenantId,
    /// Identity attributes for JIT provisioning (AUTH-2, ADR-0013).
    /// `Some` whenever an IdP verified the token — the OIDC verifier always
    /// sets it, even when the token names no groups — and `None` for
    /// out-of-band subjects (the HS256 dev mode). The PDP seam treats an
    /// IdP subject with no provisioned identity as quarantined (ADR-0013
    /// decision 6), so this field's presence is itself a claim.
    pub provisioning: Option<ProvisioningClaims>,
    /// The token's issued lifetime (`exp − iat`), when the token carries
    /// `iat`; `None` when it does not. The enforcement seam caps service
    /// identities' token lifetime with this, failing closed on `None`
    /// (AUTH-3, ADR-0018 decision 5). User tokens ignore it.
    pub lifetime: Option<Duration>,
    /// The verified audience boundary that admitted this credential.
    pub credential_class: CredentialClass,
}

/// What an IdP asserts about a subject beyond its name: the raw material
/// of JIT provisioning (AUTH-2, ADR-0013 decision 1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvisioningClaims {
    /// Group names from the per-issuer `groups_claim` (default `groups`);
    /// empty when the token carries none.
    pub groups: Vec<String>,
    /// The `email` claim, if present.
    pub email: Option<String>,
    /// The `name` claim, if present.
    pub display_name: Option<String>,
    /// The directory anchor from the per-issuer `external_id_claim`
    /// (default `sub`), if present — the value a SCIM mirror row's
    /// `externalId` is matched against at first login (AUTH-4, ADR-0059
    /// decision 4).
    ///
    /// Configurable per issuer because it is not `sub` everywhere and the
    /// difference is not cosmetic: Entra issues a **pairwise** `sub`,
    /// unique per (application, user), so an Entra tenant's `sub` will
    /// never equal the directory object id its provisioning agent sends.
    /// A server that assumed otherwise would give every Entra user two
    /// identities and half their memory in each.
    pub external_id: Option<String>,
}

/// Verifies a bearer token and returns its claims. Implementations must be
/// fail-closed: any doubt is an [`Error::Unauthenticated`].
#[async_trait::async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Verifies `token` (signature and expiry) and extracts [`Claims`].
    async fn verify(&self, token: &str) -> Result<Claims>;
}

/// Rejects every token. Installed when no verifier is configured, so a
/// misconfigured gateway denies rather than admits (seed §2.3).
pub struct DisabledVerifier;

#[async_trait::async_trait]
impl TokenVerifier for DisabledVerifier {
    async fn verify(&self, _token: &str) -> Result<Claims> {
        Err(Error::Unauthenticated {
            message: "no token verifier is configured".to_owned(),
        })
    }
}

#[derive(Deserialize)]
struct Header {
    alg: String,
}

/// Wire claims. `deny_unknown_fields` is deliberately absent: real IdP
/// tokens carry many extra claims and must stay verifiable.
#[derive(Serialize, Deserialize)]
struct RawClaims {
    sub: String,
    tid: String,
    exp: u64,
    /// Issued-at. Minted into every dev token so `synveda token issue`d
    /// service subjects pass the seam's lifetime cap; optional on
    /// verification for pre-AUTH-3 tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<u64>,
}

/// HS256 (HMAC-SHA256) JWT verification with a shared secret — the dev/test
/// mode until AUTH-1 (ADR-0008). Also mints tokens ([`Hs256Verifier::issue`])
/// for the CLI, demos, and tests; issuance never ships beyond dev mode
/// because real deployments verify against an IdP, not a shared key.
pub struct Hs256Verifier {
    secret: Vec<u8>,
}

impl Hs256Verifier {
    /// Builds a verifier over a shared secret.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
        }
    }

    fn mac(&self) -> HmacSha256 {
        // HMAC accepts any key length; new_from_slice is infallible here.
        HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length")
    }

    /// Mints a token for `subject` acting as `tenant_id`, expiring after
    /// `ttl`. Dev/test tooling only.
    #[must_use]
    pub fn issue(&self, subject: &str, tenant_id: TenantId, ttl: Duration) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let issued_at = now();
        let claims = RawClaims {
            sub: subject.to_owned(),
            tid: tenant_id.to_string(),
            exp: (issued_at + ttl).as_secs(),
            iat: Some(issued_at.as_secs()),
        };
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims serialize"));
        let mut mac = self.mac();
        mac.update(format!("{header}.{payload}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{header}.{payload}.{signature}")
    }
}

#[async_trait::async_trait]
impl TokenVerifier for Hs256Verifier {
    async fn verify(&self, token: &str) -> Result<Claims> {
        let reject = |message: &str| Error::Unauthenticated {
            message: message.to_owned(),
        };

        let [header_b64, payload_b64, signature_b64]: [&str; 3] = token
            .split('.')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| reject("malformed token"))?;

        // The header is parsed only to *check* the pinned algorithm.
        let header: Header = URL_SAFE_NO_PAD
            .decode(header_b64)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| reject("malformed token header"))?;
        if header.alg != "HS256" {
            return Err(reject("unsupported token algorithm"));
        }

        // Authenticate before parsing the payload. verify_slice is
        // constant-time (RustCrypto `subtle`).
        let signature = URL_SAFE_NO_PAD
            .decode(signature_b64)
            .map_err(|_| reject("malformed token signature"))?;
        let mut mac = self.mac();
        mac.update(format!("{header_b64}.{payload_b64}").as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| reject("token signature mismatch"))?;

        let claims: RawClaims = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| reject("token is missing required claims (sub, tid, exp)"))?;
        let verified_at = now().as_secs();
        if claims.exp <= verified_at {
            return Err(reject("token expired"));
        }
        let tenant_id: TenantId = claims
            .tid
            .parse()
            .map_err(|_| reject("tid claim is not a UUID"))?;

        let lifetime = issued_lifetime(claims.exp, claims.iat, verified_at)?;

        Ok(Claims {
            subject: claims.sub,
            tenant_id,
            // Dev-mode subjects are out-of-band: no IdP stands behind them,
            // so they carry no provisioning claims (ADR-0013 decision 1).
            provisioning: None,
            lifetime,
            credential_class: CredentialClass::PrimaryBearer,
        })
    }
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verifier() -> Hs256Verifier {
        Hs256Verifier::new(b"test-secret")
    }

    fn signed(v: &Hs256Verifier, claims: &RawClaims) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims"));
        let mut mac = v.mac();
        mac.update(format!("{header}.{payload}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{header}.{payload}.{signature}")
    }

    fn assert_unauthenticated(result: Result<Claims>, containing: &str) {
        match result {
            Err(Error::Unauthenticated { message }) => assert!(
                message.contains(containing),
                "expected message containing {containing:?}, got {message:?}"
            ),
            other => panic!("expected Unauthenticated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn issue_then_verify_roundtrips() {
        let tenant = TenantId::new();
        let token = verifier().issue("alice", tenant, Duration::from_secs(60));
        let claims = verifier().verify(&token).await.expect("verify own token");
        assert_eq!(claims.subject, "alice");
        assert_eq!(claims.tenant_id, tenant);
        assert_eq!(
            claims.provisioning, None,
            "dev-mode subjects are out-of-band (ADR-0013)"
        );
        assert_eq!(
            claims.lifetime,
            Some(Duration::from_secs(60)),
            "minted tokens carry iat, so their lifetime is known (ADR-0018)"
        );
    }

    #[tokio::test]
    async fn wrong_secret_is_rejected() {
        let token = verifier().issue("alice", TenantId::new(), Duration::from_secs(60));
        let other = Hs256Verifier::new(b"different-secret");
        assert_unauthenticated(other.verify(&token).await, "signature");
    }

    #[tokio::test]
    async fn tampered_payload_is_rejected() {
        let token = verifier().issue("alice", TenantId::new(), Duration::from_secs(60));
        let [header, _, signature]: [&str; 3] =
            token.split('.').collect::<Vec<_>>().try_into().unwrap();
        let forged_payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&RawClaims {
                sub: "mallory".into(),
                tid: TenantId::new().to_string(),
                exp: now().as_secs() + 600,
                iat: None,
            })
            .unwrap(),
        );
        let forged = format!("{header}.{forged_payload}.{signature}");
        assert_unauthenticated(verifier().verify(&forged).await, "signature");
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let token = verifier().issue("alice", TenantId::new(), Duration::ZERO);
        assert_unauthenticated(verifier().verify(&token).await, "expired");
    }

    #[tokio::test]
    async fn issued_at_after_expiry_is_rejected_without_a_zero_lifetime() {
        let v = verifier();
        let current = now().as_secs();
        let token = signed(
            &v,
            &RawClaims {
                sub: "service".to_owned(),
                tid: TenantId::new().to_string(),
                exp: current + 60,
                iat: Some(current + 61),
            },
        );
        assert_unauthenticated(v.verify(&token).await, "after its expiry");
    }

    #[tokio::test]
    async fn issued_at_beyond_future_skew_is_rejected() {
        let v = verifier();
        let current = now().as_secs();
        let token = signed(
            &v,
            &RawClaims {
                sub: "service".to_owned(),
                tid: TenantId::new().to_string(),
                exp: current + 120,
                iat: Some(current + FUTURE_IAT_LEEWAY.as_secs() + 1),
            },
        );
        assert_unauthenticated(v.verify(&token).await, "future");
    }

    #[tokio::test]
    async fn issued_at_on_the_future_skew_boundary_remains_valid() {
        let v = verifier();
        let current = now().as_secs();
        let issued_at = current + FUTURE_IAT_LEEWAY.as_secs();
        let expires_at = issued_at + 60;
        let token = signed(
            &v,
            &RawClaims {
                sub: "service".to_owned(),
                tid: TenantId::new().to_string(),
                exp: expires_at,
                iat: Some(issued_at),
            },
        );
        let claims = v.verify(&token).await.expect("boundary-valid token");
        assert_eq!(claims.lifetime, Some(Duration::from_secs(60)));
        assert_eq!(
            issued_lifetime(1_090, Some(1_030), 1_000).expect("exact boundary"),
            Some(Duration::from_secs(60))
        );
    }

    #[tokio::test]
    async fn non_hs256_algorithm_is_rejected_even_with_valid_hmac() {
        // A token whose header claims another algorithm must fail on the
        // pinned-algorithm check, even if its HMAC would verify.
        let v = verifier();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&RawClaims {
                sub: "alice".into(),
                tid: TenantId::new().to_string(),
                exp: now().as_secs() + 600,
                iat: None,
            })
            .unwrap(),
        );
        let mut mac = v.mac();
        mac.update(format!("{header}.{payload}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_unauthenticated(
            v.verify(&format!("{header}.{payload}.{signature}")).await,
            "algorithm",
        );
    }

    #[tokio::test]
    async fn malformed_tokens_are_rejected() {
        for garbage in ["", "abc", "a.b", "a.b.c.d", "not base64.at.all"] {
            assert!(
                verifier().verify(garbage).await.is_err(),
                "accepted {garbage:?}"
            );
        }
    }

    #[tokio::test]
    async fn missing_claims_are_rejected() {
        let v = verifier();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        // No tid claim.
        let payload = URL_SAFE_NO_PAD.encode(format!(
            r#"{{"sub":"alice","exp":{}}}"#,
            now().as_secs() + 600
        ));
        let mut mac = v.mac();
        mac.update(format!("{header}.{payload}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_unauthenticated(
            v.verify(&format!("{header}.{payload}.{signature}")).await,
            "claims",
        );
    }

    #[tokio::test]
    async fn non_uuid_tid_is_rejected() {
        let v = verifier();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(
            r#"{{"sub":"alice","tid":"not-a-uuid","exp":{}}}"#,
            now().as_secs() + 600
        ));
        let mut mac = v.mac();
        mac.update(format!("{header}.{payload}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_unauthenticated(
            v.verify(&format!("{header}.{payload}.{signature}")).await,
            "UUID",
        );
    }

    #[tokio::test]
    async fn disabled_verifier_rejects_everything() {
        let token = verifier().issue("alice", TenantId::new(), Duration::from_secs(60));
        assert_unauthenticated(DisabledVerifier.verify(&token).await, "no token verifier");
    }
}
