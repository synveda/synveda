//! The key-management seam (TEN-4, ADR-0064 decisions 1 and 2).
//!
//! Two levels: a key-encryption key (KEK) the KMS holds and never releases,
//! and per-scope data keys (DEKs) that are stored only in wrapped form. The
//! KMS therefore sees exactly two operations — wrap a DEK, unwrap a DEK —
//! and never a payload.
//!
//! **The surface is deliberately not "encrypt this".** A KMS that will
//! encrypt arbitrary bytes becomes a per-row network call the first time
//! somebody is in a hurry; a KMS that only wraps keys cannot become one. The
//! shape is `Extractor`/`Embedder`/`CommitSigner` (ADR-0022/0023/0030): a
//! trait for the contract, an enum for static dispatch, and a `method()`
//! name that shows up in metrics and logs. `async` for the same reason
//! `Embedder` is — the implementations this seam exists for (AWS, GCP,
//! Vault) are network calls, and discovering that after the fact would make
//! them a rewrite instead of a variant.

use synveda_types::{Error, Result};

use crate::aad::{Purpose, RowKey};
use crate::envelope::SealingKey;
use crate::key::{DataKey, KeyScope, KeyVersion};

/// Wraps and unwraps data keys.
///
/// Implementations must fail rather than return a key on any doubt: a
/// silently wrong DEK turns into a decryption failure much later, in a place
/// with none of the context needed to diagnose it.
#[allow(async_fn_in_trait)]
pub trait KeyManagement {
    /// The stable method name recorded in metrics labels and audit payloads
    /// (`local`, and later `aws-kms`, `gcp-kms`, `vault`).
    fn method(&self) -> &'static str;

    /// Names the key-encryption key in use, as it is stored beside every
    /// wrapped DEK. This is what makes BYOK configuration rather than a
    /// redesign (decision 1): a tenant whose row names a customer's own KMS
    /// key is wrapped by that key.
    fn key_ref(&self) -> &str;

    /// Wraps a data key for `scope`.
    async fn wrap_key(&self, scope: KeyScope, key: &DataKey) -> Result<Vec<u8>>;

    /// Unwraps a data key for `scope`, refusing one wrapped for any other.
    async fn unwrap_key(&self, scope: KeyScope, wrapped: &[u8]) -> Result<DataKey>;
}

/// The configured KMS, dispatched statically.
#[derive(Debug, Default)]
pub enum Kms {
    /// No key configured: every wrap and unwrap refuses.
    ///
    /// The default, and fail-closed in the house sense — `DisabledVerifier`
    /// (ADR-0008) and `Signer::Unsigned` (ADR-0030 decision 9) are the same
    /// shape. A deployment with no KEK boots and serves `/v1` exactly as
    /// before; what stops working is precisely what needs a key, with an
    /// error that says which key is missing. The alternative — refusing to
    /// boot — would make a feature nobody has configured yet into an outage,
    /// and the alternative to *that* — a built-in default key — is not a key.
    #[default]
    Disabled,
    /// A KEK from deployment configuration. The dev default, and the
    /// single-node deployment's answer; AWS, GCP and Vault land behind the
    /// same two operations.
    Local(LocalKms),
}

fn no_key(operation: &str) -> Error {
    Error::Dependency {
        service: "kms".to_string(),
        message: format!(
            "cannot {operation}: no key-encryption key is configured \
             (set SYNVEDA_KMS_KEY — `synveda kms keygen` mints one)"
        ),
    }
}

impl KeyManagement for Kms {
    fn method(&self) -> &'static str {
        match self {
            Kms::Disabled => "disabled",
            Kms::Local(inner) => inner.method(),
        }
    }

    fn key_ref(&self) -> &str {
        match self {
            // Empty, and migration 0038's `kek_ref` check refuses it — but
            // `wrap_key` fails first, so no row ever reaches that constraint.
            Kms::Disabled => "",
            Kms::Local(inner) => inner.key_ref(),
        }
    }

    async fn wrap_key(&self, scope: KeyScope, key: &DataKey) -> Result<Vec<u8>> {
        match self {
            Kms::Disabled => Err(no_key("wrap a data key")),
            Kms::Local(inner) => inner.wrap_key(scope, key).await,
        }
    }

    async fn unwrap_key(&self, scope: KeyScope, wrapped: &[u8]) -> Result<DataKey> {
        match self {
            Kms::Disabled => Err(no_key("unwrap a data key")),
            Kms::Local(inner) => inner.unwrap_key(scope, wrapped).await,
        }
    }
}

/// A KEK held in this process, from configuration.
///
/// Honest about what it is: the key is in the deployment's environment, so
/// this protects a dumped table and a stolen export, not an operator who can
/// read the process's configuration. That is the AUD-1 trust boundary and
/// the whole of what ADR-0064 claims.
pub struct LocalKms {
    kek: DataKey,
    key_ref: String,
}

// Never the key.
impl std::fmt::Debug for LocalKms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalKms")
            .field("key_ref", &self.key_ref)
            .finish_non_exhaustive()
    }
}

impl LocalKms {
    /// Adopts a KEK and the name it is known by.
    ///
    /// `key_ref` is stored beside every DEK this KEK wraps, so re-keying is
    /// expressible and an operator can tell which KEK a row needs. An empty
    /// one is refused: a row whose `kek_ref` says nothing is a row nobody can
    /// route to a key later.
    pub fn new(kek: DataKey, key_ref: impl Into<String>) -> Result<Self> {
        let key_ref = key_ref.into();
        if key_ref.is_empty() || key_ref.len() > 256 {
            return Err(Error::Invalid {
                message: format!(
                    "kms key ref must be 1..=256 characters, got {}",
                    key_ref.len()
                ),
            });
        }
        Ok(LocalKms { kek, key_ref })
    }

    /// Builds one from the 64-character hex form configuration carries.
    pub fn from_hex(kek: &str, key_ref: impl Into<String>) -> Result<Self> {
        LocalKms::new(DataKey::from_hex(kek)?, key_ref)
    }

    /// The KEK as a sealing key for `scope`.
    ///
    /// The target scope goes into the sealing key rather than into a
    /// parameter, so a DEK wrapped for one tenant cannot be unwrapped as
    /// another's — decision 4's binding, one level up. Reusing the envelope
    /// here rather than writing a second construction is deliberate: one
    /// audited code path, not two.
    ///
    /// The version field is [`KeyVersion::FIRST`] because a local KEK's
    /// generation is expressed by its `key_ref`, not by a counter — rotating
    /// to a new KEK writes a new `kek_ref` on the row.
    fn kek_for(&self, scope: KeyScope) -> SealingKey {
        SealingKey::new(
            scope,
            KeyVersion::FIRST,
            DataKey::from_bytes(*self.kek.expose()),
        )
    }
}

/// Every wrapped DEK is the one singleton of its scope.
const DATA_KEY_ROW: RowKey<'static> = RowKey::Name("data-key");

impl KeyManagement for LocalKms {
    fn method(&self) -> &'static str {
        "local"
    }

    fn key_ref(&self) -> &str {
        &self.key_ref
    }

    #[tracing::instrument(name = "crypto.kms.wrap", skip_all, fields(kms.method = self.method(), key.scope = scope.label()), err(Display))]
    async fn wrap_key(&self, scope: KeyScope, key: &DataKey) -> Result<Vec<u8>> {
        self.kek_for(scope)
            .seal(Purpose::DataKey, DATA_KEY_ROW, key.expose())
    }

    #[tracing::instrument(name = "crypto.kms.unwrap", skip_all, fields(kms.method = self.method(), key.scope = scope.label()), err(Display))]
    async fn unwrap_key(&self, scope: KeyScope, wrapped: &[u8]) -> Result<DataKey> {
        let opened = self
            .kek_for(scope)
            .open(Purpose::DataKey, DATA_KEY_ROW, wrapped)?;
        let bytes: [u8; crate::key::KEY_LEN] =
            opened.as_slice().try_into().map_err(|_| Error::Invalid {
                message: format!(
                    "unwrapped data key is {} bytes, not {}",
                    opened.len(),
                    crate::key::KEY_LEN
                ),
            })?;
        Ok(DataKey::from_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use synveda_types::TenantId;

    use super::*;

    fn kms() -> LocalKms {
        LocalKms::new(DataKey::generate().expect("kek"), "local:test").expect("kms")
    }

    #[tokio::test]
    async fn wrap_and_unwrap_round_trip() {
        let kms = kms();
        let scope = KeyScope::Tenant(TenantId::new());
        let dek = DataKey::generate().expect("dek");
        let expected = *dek.expose();
        let wrapped = kms.wrap_key(scope, &dek).await.expect("wrap");
        let unwrapped = kms.unwrap_key(scope, &wrapped).await.expect("unwrap");
        assert_eq!(unwrapped.expose(), &expected);
    }

    #[tokio::test]
    async fn a_wrapped_key_does_not_unwrap_for_another_tenant() {
        let kms = kms();
        let one = KeyScope::Tenant(TenantId::new());
        let two = KeyScope::Tenant(TenantId::new());
        let wrapped = kms
            .wrap_key(one, &DataKey::generate().expect("dek"))
            .await
            .expect("wrap");
        assert!(kms.unwrap_key(two, &wrapped).await.is_err());
    }

    #[tokio::test]
    async fn a_tenant_key_does_not_unwrap_as_the_deployment_key() {
        let kms = kms();
        let wrapped = kms
            .wrap_key(
                KeyScope::Tenant(TenantId::new()),
                &DataKey::generate().expect("dek"),
            )
            .await
            .expect("wrap");
        assert!(
            kms.unwrap_key(KeyScope::Deployment, &wrapped)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn another_kek_does_not_unwrap() {
        let scope = KeyScope::Deployment;
        let wrapped = kms()
            .wrap_key(scope, &DataKey::generate().expect("dek"))
            .await
            .expect("wrap");
        assert!(kms().unwrap_key(scope, &wrapped).await.is_err());
    }

    #[tokio::test]
    async fn the_wrapped_form_does_not_contain_the_key() {
        let kms = kms();
        let dek = DataKey::from_bytes([0xAB; crate::key::KEY_LEN]);
        let wrapped = kms
            .wrap_key(KeyScope::Deployment, &dek)
            .await
            .expect("wrap");
        assert!(
            !wrapped.windows(8).any(|window| window == [0xAB_u8; 8]),
            "key material must not survive the wrap"
        );
    }

    #[test]
    fn key_ref_is_bounded_and_required() {
        assert!(LocalKms::new(DataKey::generate().expect("kek"), "").is_err());
        assert!(LocalKms::new(DataKey::generate().expect("kek"), "x".repeat(257)).is_err());
    }

    #[test]
    fn from_hex_rejects_a_malformed_kek() {
        assert!(LocalKms::from_hex("nothex", "local:test").is_err());
        assert!(LocalKms::from_hex(&"ab".repeat(32), "local:test").is_ok());
    }

    #[test]
    fn debug_does_not_print_the_kek() {
        let rendered = format!("{:?}", kms());
        assert!(rendered.contains("local:test"));
        assert!(!rendered.contains("kek"));
    }

    #[tokio::test]
    async fn the_disabled_default_refuses_and_names_the_missing_key() {
        let kms = Kms::default();
        assert_eq!(kms.method(), "disabled");
        let err = kms
            .wrap_key(KeyScope::Deployment, &DataKey::generate().expect("dek"))
            .await
            .expect_err("must refuse");
        assert!(
            err.to_string().contains("SYNVEDA_KMS_KEY"),
            "an operator reading this needs to know what to set: {err}"
        );
        assert!(
            kms.unwrap_key(KeyScope::Deployment, &[0_u8; 82])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_enum_dispatches_to_the_local_impl() {
        let kms = Kms::Local(kms());
        assert_eq!(kms.method(), "local");
        assert_eq!(kms.key_ref(), "local:test");
        let scope = KeyScope::Deployment;
        let wrapped = kms
            .wrap_key(scope, &DataKey::generate().expect("dek"))
            .await
            .expect("wrap");
        assert!(kms.unwrap_key(scope, &wrapped).await.is_ok());
    }
}
