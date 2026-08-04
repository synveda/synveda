//! Serving the admin console (CNSL-1, ADR-0056 decision 1).
//!
//! The console is a static bundle the gateway serves from its own origin.
//! That is one decision doing three jobs: OPS-2's chart ships one runtime
//! rather than two, there is no CORS story to get wrong, and the
//! `SameSite=Strict` session cookie is useful at all — a cookie scoped to
//! this origin is worth nothing to a page served from another one.
//!
//! Two things here are load-bearing beyond "serve some files".
//!
//! The **Content-Security-Policy** is what turns ADR-0056 decision 8's "no
//! CDN, no external font, no runtime fetch to a third party" from a claim
//! into an enforcement. `default-src 'none'` with an explicit `'self'`
//! allowlist means a bundle that grew a CDN reference would visibly fail to
//! load rather than quietly work everywhere except the air-gapped
//! deployments this product is sold into. It is also the console's main
//! defence for the session itself: a BFF cookie is proof against token
//! theft by XSS, but not against an XSS *acting* as the user, and a policy
//! with no `'unsafe-inline'` is what makes injected script hard to run.
//!
//! The **SPA fallback** is a deliberate 200-with-index rather than a 404
//! for unknown paths under the prefix, because client-side routing means
//! `/console/proposals/42` is a real page that the filesystem has never
//! heard of. It is scoped to this prefix and this prefix only: `/v1` and
//! the auth plane keep their own 404s, and a fallback that reached them
//! would turn every API typo into an HTML page.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::http::{HeaderValue, header};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

/// Where the console is mounted. Fixed rather than configurable: it is
/// baked into the bundle at build time (`base` in vite.config.ts) and it is
/// where `auth::CONSOLE_HOME` sends a completed login, so a third place to
/// change it would be a third place to get it wrong.
pub const CONSOLE_PREFIX: &str = "/console";

/// Where the built bundle is read from. `SYNVEDA_CONSOLE_DIR` overrides,
/// for a deployment that puts it somewhere other than beside the binary.
const DEFAULT_DIR: &str = "console/dist";

/// The policy every console response carries.
///
/// `default-src 'none'` and then only what the bundle actually uses. Worth
/// reading as a list of things that cannot happen: no inline script, no
/// `eval`, no third-party origin of any kind, no framing, no form post to
/// anywhere, and no `<base>` tag rewriting where relative URLs resolve.
///
/// `connect-src 'self'` is the air-gap guarantee: the page can reach this
/// gateway and nothing else, so a dependency that phoned home would be
/// stopped by the browser rather than discovered by a customer's proxy log.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; \
     script-src 'self'; \
     style-src 'self'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     form-action 'none'; \
     frame-ancestors 'none'; \
     base-uri 'none'";

/// Resolves the bundle directory, or `None` when there is nothing to serve.
///
/// A missing bundle is **not** a boot failure. The gateway's job is the
/// API; a deployment that ships no console, or a developer who has not run
/// `pnpm --filter @synveda/console build`, should get a 404 under
/// `/console/` and a working product everywhere else. Refusing to start
/// would make a static asset a dependency of the audit log.
pub fn bundle_dir() -> Option<PathBuf> {
    let configured =
        std::env::var("SYNVEDA_CONSOLE_DIR").unwrap_or_else(|_| DEFAULT_DIR.to_owned());
    resolve(PathBuf::from(configured))
}

/// The half of [`bundle_dir`] that reads no environment, so that what
/// counts as a bundle can be tested without a process-wide variable.
fn resolve(dir: PathBuf) -> Option<PathBuf> {
    // `index.html` rather than the directory: an empty `dist/` left behind
    // by a failed build is not a bundle, and serving it would answer every
    // console request with a 404 that looks like a routing bug.
    if dir.join("index.html").is_file() {
        Some(dir)
    } else {
        tracing::warn!(
            dir = %dir.display(),
            "no console bundle found; {CONSOLE_PREFIX}/ will 404 (build it with \
             `pnpm --filter @synveda/console build`, or set SYNVEDA_CONSOLE_DIR)"
        );
        None
    }
}

/// The console's routes, or an empty router when there is no bundle.
pub fn router(dir: &Path) -> Router {
    let index = dir.join("index.html");
    // Unknown paths under the prefix are the client router's, not the
    // filesystem's (see the module docs).
    let service = ServeDir::new(dir).fallback(ServeFile::new(index));

    Router::new()
        .fallback_service(service)
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        // Vite emits content-hashed asset names, so the bundle could be
        // cached hard — but `index.html` names those hashes and must never
        // be, and one rule that is always right beats two rules that are
        // right until somebody edits the first. An admin console is not
        // where bandwidth is won; `no-cache` still revalidates to a 304.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        // A bundle is served from the same origin as the API, so a file
        // whose type a browser sniffs wrongly is a file that can run as
        // script against this origin.
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // A console URL can name a proposal id; a referrer leaking one to
        // wherever a reviewer clicks next is a disclosure nobody decided.
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_permits_this_origin_and_nothing_else() {
        // The air-gap claim, asserted on the bytes that reach the browser.
        // A policy that grew a CDN would pass every other test here.
        assert!(CONTENT_SECURITY_POLICY.starts_with("default-src 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("connect-src 'self'"));
        assert!(!CONTENT_SECURITY_POLICY.contains("http://"));
        assert!(!CONTENT_SECURITY_POLICY.contains("https://"));
        assert!(!CONTENT_SECURITY_POLICY.contains('*'));
    }

    #[test]
    fn the_policy_leaves_no_room_to_run_injected_script() {
        // The BFF cookie is proof against an XSS *stealing* the session
        // (ADR-0056 decision 2). This is what makes it hard for an XSS to
        // *use* it, which is the half a cookie cannot help with.
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("base-uri 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("form-action 'none'"));
    }

    #[test]
    fn an_empty_dist_directory_is_not_a_bundle() {
        // The failed-build case: `dist/` exists, `index.html` does not.
        // Mounting it would answer every console request with a 404 that
        // reads like a routing bug rather than like a missing build.
        let dir = std::env::temp_dir().join(format!("synveda-console-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        assert!(
            resolve(dir.clone()).is_none(),
            "an empty directory is not a bundle"
        );

        std::fs::write(dir.join("index.html"), "<!doctype html>").expect("write index");
        assert_eq!(resolve(dir.clone()).as_deref(), Some(dir.as_path()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_that_does_not_exist_is_not_a_bundle() {
        // The default path when nobody has built anything, which is the
        // ordinary state of a fresh checkout.
        assert!(resolve(PathBuf::from("does/not/exist")).is_none());
    }
}
