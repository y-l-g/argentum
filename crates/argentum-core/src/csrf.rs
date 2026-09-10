//! CSRF protection via double-submit cookie (GH #99).
//!
//! Every state-changing form embeds `csrf_token`, and every POST handler
//! verifies the form value matches the `argentum_csrf` cookie. No server-side
//! session is needed: the token is a random UUID the server sets (and reads)
//! via the cookie layer, and the browser's same-origin policy keeps an
//! attacker from reading the token to forge the form field. `confirm=1` stays
//! a UX step, never a security boundary.

use topcoat::context::{Cx, try_request_context};
use topcoat::cookie::{Cookie, CookieJarCell, Cookies, cookies};

/// Cookie carrying the CSRF token.
pub const COOKIE_NAME: &str = "argentum_csrf";
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

/// Verify the submitted form token matches the cookie (GH #99).
///
/// Fails closed: missing cookie, missing field, or mismatch all yield 403.
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
    if !is_valid_token(&expected) || submitted != &expected {
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
    use topcoat::context::CxTestBuilder;

    fn cx_with_cookie(value: Option<&str>) -> Cx {
        let mut parts = http::Request::builder()
            .uri("/")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        if let Some(v) = value {
            parts.headers.insert(
                http::header::COOKIE,
                format!("{COOKIE_NAME}={v}").parse().unwrap(),
            );
        }
        CxTestBuilder::new()
            .request_context(parts)
            .request_context(CookieJarCell::new())
            .build()
    }

    #[test]
    fn valid_token_format() {
        assert!(is_valid_token(&uuid::Uuid::new_v4().to_string()));
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("not-a-uuid"));
    }

    #[test]
    fn verify_matches_cookie_and_rejects_mismatch() {
        let token = uuid::Uuid::new_v4().to_string();
        let cx = cx_with_cookie(Some(&token));
        let mut values = std::collections::HashMap::new();
        values.insert(FIELD_NAME.to_string(), token.clone());
        assert!(verify(&cx, &values).is_ok());
        values.insert(FIELD_NAME.to_string(), uuid::Uuid::new_v4().to_string());
        assert!(verify(&cx, &values).is_err());
        assert!(verify(&cx, &std::collections::HashMap::new()).is_err());
        assert!(
            verify(
                &cx_with_cookie(None),
                &std::collections::HashMap::from([(FIELD_NAME.to_string(), token)])
            )
            .is_err()
        );
    }
}
