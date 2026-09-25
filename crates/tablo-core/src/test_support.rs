//! Fixtures shared by the crate's unit tests.
//!
//! Each test module builds the same bare `Cx`, the same cookie-bearing `Cx`
//! and the same two-column `User` model, so one copy lives here: a change to
//! any of them is one edit instead of one per module.

use topcoat::context::{Cx, CxTestBuilder};

/// A `Cx` with no request or app context.
pub(crate) fn cx() -> Cx {
    CxTestBuilder::new().build()
}

/// A `Cx` carrying `name=value` when `value` is `Some`.
///
/// The `CookieJarCell` is what the cookie layer installs on a real request;
/// the test builder has to insert it or `cookies()` panics.
pub(crate) fn cx_with_cookie(name: &str, value: Option<&str>) -> Cx {
    let mut parts = http::Request::builder()
        .uri("/")
        .body(())
        .unwrap()
        .into_parts()
        .0;
    if let Some(value) = value {
        parts.headers.insert(
            http::header::COOKIE,
            format!("{name}={value}").parse().unwrap(),
        );
    }
    CxTestBuilder::new()
        .request_context(parts)
        .request_context(topcoat::cookie::CookieJarCell::new())
        .build()
}

/// A model with a primary key and one projection, for the resource and schema
/// tests that only need a row to name.
#[derive(Debug, Clone, toasty::Model)]
pub(crate) struct User {
    #[key]
    #[auto]
    pub(crate) id: uuid::Uuid,
    pub(crate) name: String,
}
