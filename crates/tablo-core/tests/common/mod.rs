//! Shared HTTP harness for the `tablo-core` integration suite.
//!
//! One binary (`tests/it.rs`) compiles every module, so the request builders,
//! the multipart writer, the DB builder and the HTML scrapers live here
//! instead of being redeclared per module. The showcase suite's `tests/common`
//! is the same idea; this is the core-side copy.
//!
//! The auth-gated `auth_override` module is the only caller of the
//! cookie-carrying helpers, so those carry the `auth` gate too.

use http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE};
use tablo_core::{Auth, Panel, Resource};
use toasty::Db;
use topcoat::router::{Body, Router, response::Response};
use uuid::Uuid;

/// An in-memory SQLite `Db` with `models` registered and its schema pushed.
pub async fn memory_db(models: toasty::schema::ModelSet) -> Db {
    let db = Db::builder()
        .models(models)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    db
}

/// A panel mounted at `/admin` with the auth gate off — the shape every suite
/// here builds before adding its own resources.
pub fn panel(db: Db) -> Panel {
    Panel::new("admin").app_context(db).auth(Auth::disabled())
}

/// [`panel`] with one resource registered and built under `auth`.
pub fn router_with<R: Resource>(db: Db, auth: Auth) -> Router {
    Panel::new("admin")
        .app_context(db)
        .auth(auth)
        .resource::<R>()
        .build()
        .expect("panel builds")
}

/// [`router_with`] under the disabled auth gate.
pub fn router<R: Resource>(db: Db) -> Router {
    router_with::<R>(db, Auth::disabled())
}

/// A POST carrying a matching CSRF cookie + field (the double-submit pair).
pub async fn post(
    router: &Router,
    uri: &str,
    csrf: &str,
    content_type: String,
    body: String,
) -> Response<Body> {
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(CONTENT_TYPE, content_type)
        .header(COOKIE, format!("{}={csrf}", tablo_core::csrf::COOKIE_NAME))
        .body(Body::from(body))
        .expect("request builds");
    router.handle(request).await
}

/// POST a multipart form (every upload form's enctype).
pub async fn post_multipart(
    router: &Router,
    uri: &str,
    csrf: &str,
    boundary: &str,
    body: String,
) -> Response<Body> {
    post(
        router,
        uri,
        csrf,
        format!("multipart/form-data; boundary={boundary}"),
        body,
    )
    .await
}

/// A GET with no cookies.
pub async fn get(router: &Router, uri: &str) -> Response<Body> {
    let request = http::Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    router.handle(request).await
}

/// A GET carrying `cookies` as one `Cookie` header.
#[cfg(feature = "auth")]
pub async fn get_with_cookies(
    router: &Router,
    uri: &str,
    cookies: &[(&str, String)],
) -> Response<Body> {
    let mut request = http::Request::builder().uri(uri);
    if let Some(jar) = cookie_header(cookies) {
        request = request.header(COOKIE, jar);
    }
    router.handle(request.body(Body::empty()).unwrap()).await
}

/// A url-encoded POST carrying `cookies`.
#[cfg(feature = "auth")]
pub async fn post_form(
    router: &Router,
    uri: &str,
    cookies: &[(&str, String)],
    body: String,
) -> Response<Body> {
    let mut request = http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(jar) = cookie_header(cookies) {
        request = request.header(COOKIE, jar);
    }
    router.handle(request.body(Body::from(body)).unwrap()).await
}

/// A url-encoded POST minting its own CSRF pair and sending `fields`.
pub async fn post_fields(router: &Router, uri: &str, fields: &[(&str, &str)]) -> Response<Body> {
    let csrf = new_csrf();
    let mut body = format!("csrf_token={csrf}");
    for (name, value) in fields {
        body.push_str(&format!("&{name}={value}"));
    }
    post(
        router,
        uri,
        &csrf,
        "application/x-www-form-urlencoded".to_string(),
        body,
    )
    .await
}

#[cfg(feature = "auth")]
fn cookie_header(cookies: &[(&str, String)]) -> Option<String> {
    (!cookies.is_empty()).then(|| {
        cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// A multipart body, one part per entry: `None` is a text part, `Some("")` the
/// browser's "no file chosen" file part, `Some(name)` a chosen file.
pub fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &str)]) -> String {
    let mut body = String::new();
    for (name, filename, content) in parts {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""
        ));
        if let Some(filename) = filename {
            body.push_str(&format!("; filename=\"{filename}\""));
        }
        body.push_str("\r\n\r\n");
        body.push_str(content);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    body
}

/// The response body as bytes.
pub async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec()
}

/// The response body as UTF-8 lossy text.
pub async fn body_string(response: Response<Body>) -> String {
    String::from_utf8_lossy(&body_bytes(response).await).into_owned()
}

/// The `(name, value)` pairs a response's `Set-Cookie` headers carry.
#[cfg(feature = "auth")]
pub fn cookies(response: &Response<Body>) -> Vec<(String, String)> {
    response
        .headers()
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect()
}

/// The `value` of the named input in rendered HTML.
#[cfg(feature = "auth")]
pub fn input_value(html: &str, name: &str) -> Option<String> {
    let name_attr = format!("name=\"{name}\"");
    for tag in html.split('<').skip(1) {
        if !tag.contains(&name_attr) {
            continue;
        }
        let attrs = &tag[..tag.find('>')?];
        if let Some(start) = attrs.find("value=\"") {
            let rest = &attrs[start + "value=\"".len()..];
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// A new CSRF token, paired with the cookie the POST helpers send.
pub fn new_csrf() -> String {
    Uuid::new_v4().to_string()
}

/// The `Content-Security-Policy` a response carries.
pub fn csp(response: &Response<Body>) -> &str {
    response
        .headers()
        .get(CONTENT_SECURITY_POLICY)
        .expect("response carries a policy")
        .to_str()
        .expect("the policy is ASCII")
}
