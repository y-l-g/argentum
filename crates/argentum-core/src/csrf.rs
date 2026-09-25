//! CSRF protection via double-submit cookie (GH #99).
//!
//! Every state-changing form embeds `csrf_token`, and every POST handler
//! verifies the form value matches the cookie. No server-side session is
//! needed: the token is a random UUID the server sets (and reads) via the
//! cookie layer, and the browser's same-origin policy keeps an attacker
//! from reading the token to forge the form field. `confirm=1` stays
//! a UX step, never a security boundary.
//!
//! The cookie is `__Host-`-prefixed and `Secure` (GH #149, matching the
//! session cookie's hardened contract): a cookie-writable position
//! (subdomain, cleartext HTTP) cannot pin a known token to the jar. The
//! compare is constant-time so a mismatch cannot be probed byte-by-byte.
//! The token deliberately stays a bare random UUID — binding it to the
//! server (HMAC via Topcoat's signed jar) would need a `Key` app context
//! `Panel::build` does not register; that stays an upstream-gap decision
//! (#139), not a hand-rolled one.

use subtle::ConstantTimeEq;
use topcoat::{
    context::{Cx, try_request_context},
    cookie::{Cookie, CookieJarCell, Cookies, cookies},
};

/// Cookie carrying the CSRF token (`__Host-` prefix: `Secure` + `Path=/` +
/// no `Domain` are required by the prefix contract, GH #149).
pub const COOKIE_NAME: &str = "__Host-argentum_csrf";
/// Hidden form field carrying the CSRF token.
pub const FIELD_NAME: &str = "csrf_token";

/// Ensure a token exists for this request, setting the cookie when needed.
///
/// Must be called before response headers are sent (i.e. in the page handler,
/// not inside a streamed `suspense` child — setting cookies after headers
/// panics). Returns the token to embed in forms. When no cookie layer is
/// present (bare unit renders), returns an empty string so renders never panic;
/// POST handlers behind the real router always have the layer and enforce.
pub fn ensure_token(cx: &Cx) -> String {
    if try_request_context::<CookieJarCell>(cx).is_none() {
        return String::new();
    }
    let jar = cookies(cx);
    if let Some(cookie) = jar.get(COOKIE_NAME) {
        let value = cookie.value().to_string();
        if is_valid_token(&value) {
            return value;
        }
    }
    let token = uuid::Uuid::new_v4().to_string();
    let cookie = Cookie::build((COOKIE_NAME, token.clone()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(topcoat::cookie::SameSite::Lax)
        .build();
    jar.add(cookie);
    token
}

/// Read the current token without setting one (GH #99).
///
/// Safe inside streamed `suspense` children that outlive header send: renders
/// embed the already-ensured token, or `""` when none was ensured.
pub fn current_token(cx: &Cx) -> String {
    if try_request_context::<CookieJarCell>(cx).is_none() {
        return String::new();
    }
    cookies(cx)
        .get(COOKIE_NAME)
        .map(|c| c.value().to_string())
        .filter(|v| is_valid_token(v))
        .unwrap_or_default()
}

/// The hidden field every state-changing form embeds.
///
/// The token is passed in, never resolved here: whether a site calls
/// [`ensure_token`] (which sets the cookie and must run before response headers
/// are sent) or [`current_token`] (the only one safe inside a streamed
/// `suspense` child) is the site's decision, and a helper that guessed would
/// either panic after header send or silently embed nothing. What the helper
/// owns is the spelling — [`FIELD_NAME`] is what [`verify`] reads, so a rename
/// that missed a form would be a silent 403 on every POST (GH #212).
pub fn field<'a>(cx: &'a Cx, token: &str) -> topcoat::view::BoxView<'a> {
    use topcoat::view::ViewExt;

    // Own the token before the `view!` block: the emitted view must borrow the
    // request context and nothing else, or a caller's local `String` would have
    // to outlive the page.
    let token = token.to_string();
    topcoat::view::view! { cx => <input type="hidden" name=(FIELD_NAME) value=(token)> }.boxed()
}

/// Verify the submitted form token matches the cookie (GH #99).
///
/// Fails closed: missing cookie, missing field, or mismatch all yield 403.
/// The mismatch compare is constant-time (GH #149) so a failed double-submit
/// cannot be probed byte-by-byte; the token itself stays a random UUID, so
/// a length difference is not a secret.
pub fn verify(
    cx: &Cx,
    values: &std::collections::HashMap<String, String>,
) -> Result<(), topcoat::Error> {
    let cookie_ok = try_request_context::<CookieJarCell>(cx)
        .map(|_| cookies(cx).get(COOKIE_NAME).map(|c| c.value().to_string()))
        .unwrap_or(None);
    let Some(expected) = cookie_ok else {
        return Err(topcoat::router::error::forbidden().into());
    };
    let Some(submitted) = values.get(FIELD_NAME) else {
        return Err(topcoat::router::error::forbidden().into());
    };
    if !is_valid_token(&expected) || submitted.as_bytes().ct_ne(expected.as_bytes()).into() {
        return Err(topcoat::router::error::forbidden().into());
    }
    Ok(())
}

fn is_valid_token(value: &str) -> bool {
    value.parse::<uuid::Uuid>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::cx_with_cookie;

    #[test]
    fn valid_token_format() {
        assert!(is_valid_token(&uuid::Uuid::new_v4().to_string()));
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("not-a-uuid"));
    }

    #[test]
    fn verify_matches_cookie_and_rejects_mismatch() {
        let token = uuid::Uuid::new_v4().to_string();
        let cx = cx_with_cookie(COOKIE_NAME, Some(&token));
        let mut values = std::collections::HashMap::new();
        values.insert(FIELD_NAME.to_string(), token.clone());
        assert!(verify(&cx, &values).is_ok());
        values.insert(FIELD_NAME.to_string(), uuid::Uuid::new_v4().to_string());
        assert!(verify(&cx, &values).is_err());
        assert!(verify(&cx, &std::collections::HashMap::new()).is_err());
        assert!(
            verify(
                &cx_with_cookie(COOKIE_NAME, None),
                &std::collections::HashMap::from([(FIELD_NAME.to_string(), token)])
            )
            .is_err()
        );
    }

    /// The issued cookie carries the hardened `__Host-` contract (GH #149),
    /// matching the session cookie: `Secure`, `HttpOnly`, `SameSite=Lax`,
    /// `Path=/`, no `Domain`.
    #[test]
    fn ensured_cookie_is_host_prefixed_and_secure() {
        let cx = cx_with_cookie(COOKIE_NAME, None);
        let token = ensure_token(&cx);
        assert!(is_valid_token(&token));
        let cookie = cookies(&cx)
            .get(COOKIE_NAME)
            .expect("ensure_token set the cookie");
        assert_eq!(cookie.value(), token);
        assert_eq!(cookie.name(), COOKIE_NAME);
        assert!(cookie.secure().unwrap_or(false), "{cookie:?}");
        assert!(cookie.http_only().unwrap_or(false), "{cookie:?}");
        assert_eq!(cookie.path(), Some("/"));
        assert!(cookie.domain().is_none(), "{cookie:?}");
        assert!(
            matches!(
                cookie.same_site(),
                Some(topcoat::cookie::SameSite::Lax) | None
            ),
            "{cookie:?}"
        );
    }
}
