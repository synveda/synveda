//! The console session secret (CNSL-1, ADR-0056 decision 2).
//!
//! A browser holds an opaque secret in an `HttpOnly` cookie; the gateway
//! stores only its SHA-256. That split is the whole content of this module,
//! and it lives here rather than in the gateway because this crate is where
//! credentials are minted and where the rule that governs them — a secret
//! at rest is a secret somebody can steal — is already applied to the CLI
//! handoff code beside it.
//!
//! The secret is **not** a bearer and must never be presented as one. It
//! names a stored bearer, which the gateway then verifies through the same
//! [`crate::TokenVerifier`] every request goes through.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use synveda_types::{Error, Result};

/// Bytes of entropy in a session secret. 256 bits, the same budget the
/// login flow's `state`, `nonce` and PKCE verifier get: this secret is the
/// only thing standing between a stolen cookie value and a review session.
const SECRET_BYTES: usize = 32;

/// The cookie name. Prefixed `__Host-` deliberately — the prefix is a
/// promise the *browser* enforces: no `Domain` attribute, `Path=/`, and
/// `Secure` required, so a sibling subdomain cannot set a cookie this
/// gateway would then read. It is one of the few places a server can make a
/// browser refuse its own mistake.
pub const CONSOLE_COOKIE: &str = "__Host-synveda_console";

/// A freshly minted session secret and the hash to store for it.
pub struct ConsoleSecret {
    /// The value the browser gets. Never stored, never logged.
    pub secret: String,
    /// SHA-256 of [`Self::secret`] — the database key.
    pub hash: [u8; 32],
}

/// Mints a session secret.
///
/// No stretching: this is a lookup key with 256 bits of entropy from the
/// OS, not a user-chosen password, so a single SHA-256 is the right
/// primitive and anything slower would only tax the read path.
pub fn mint() -> Result<ConsoleSecret> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|err| Error::Internal {
        message: format!("failed to generate a console session secret: {err}"),
    })?;
    let secret = URL_SAFE_NO_PAD.encode(bytes);
    let hash = hash(&secret);
    Ok(ConsoleSecret { secret, hash })
}

/// Hashes a presented secret to its storage key.
pub fn hash(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Reads the console cookie out of a `Cookie` header value.
///
/// Hand-parsed rather than pulled from a cookie crate: RFC 6265 §4.2.1 is a
/// `;`-separated list of `name=value`, this needs exactly one name, and a
/// dependency whose whole job is that split is a dependency whose licence,
/// supply chain and CVE feed we would be adopting for four lines.
///
/// Returns the **first** match. A duplicate cookie name is the shape of a
/// fixation attempt — a sibling host setting a second cookie the gateway
/// might prefer — and `__Host-` already forbids the setter; taking the
/// first and ignoring the rest keeps the behaviour defined either way.
pub fn from_cookie_header(header: &str) -> Option<&str> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == CONSOLE_COOKIE).then(|| value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_secret_hashes_to_its_stored_key() {
        let minted = mint().expect("mint");
        assert_eq!(minted.hash, hash(&minted.secret));
    }

    #[test]
    fn two_secrets_are_never_the_same() {
        // Not a randomness test — a wiring test. A `mint` that returned a
        // constant would pass every other test in this module.
        let a = mint().expect("mint");
        let b = mint().expect("mint");
        assert_ne!(a.secret, b.secret);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn the_secret_never_appears_in_its_own_hash() {
        let minted = mint().expect("mint");
        assert!(!minted.hash.starts_with(minted.secret.as_bytes()));
        assert_eq!(minted.hash.len(), 32);
    }

    #[test]
    fn the_cookie_is_found_among_others_and_only_by_its_full_name() {
        let header = format!("theme=dark; {CONSOLE_COOKIE}=abc123; other=1");
        assert_eq!(from_cookie_header(&header), Some("abc123"));

        // A prefix or suffix of the name is a different cookie. Without the
        // exact compare, a page could set `x__Host-synveda_console` and be
        // read as the real one by a sloppy `contains`.
        assert_eq!(
            from_cookie_header(&format!("x{CONSOLE_COOKIE}=abc123")),
            None
        );
        assert_eq!(
            from_cookie_header(&format!("{CONSOLE_COOKIE}_other=abc123")),
            None
        );
        assert_eq!(from_cookie_header("theme=dark"), None);
        assert_eq!(from_cookie_header(""), None);
    }

    #[test]
    fn a_duplicated_cookie_resolves_to_the_first_deterministically() {
        // The fixation shape: two cookies of the same name. Whatever we do
        // must be defined, because "whichever the map iterated to" is how
        // one process reads a different session than the next.
        let header = format!("{CONSOLE_COOKIE}=first; {CONSOLE_COOKIE}=second");
        assert_eq!(from_cookie_header(&header), Some("first"));
    }
}
