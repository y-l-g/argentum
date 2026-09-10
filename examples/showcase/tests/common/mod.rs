//! Shared fixtures and request scaffolding for the showcase integration
//! tests (GH #111, #128). Every test crate includes this module via
//! `mod common;` and uses a subset of it, so `dead_code` is expected here and
//! allowed once instead of leaking per-crate warnings.

#![allow(dead_code)]

use argentum_core::Resource;
use argentum_core::Tenant;
use http::header::{CONTENT_TYPE, COOKIE};
use http_body_util::BodyExt;
use showcase::models::{DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, create_admin, seed, seed_phase2};
use toasty::Db;
use topcoat::context::Cx;
use topcoat::router::{Body, Router};

/// `Db` with the phase-1 users seed applied.
pub async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            argentum_core::auth::AdminUser,
            argentum_core::auth::AuthSession
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    db
}

/// `Db` with both seed phases (users, authors, posts, comments) and the
/// shipped auth models.
pub async fn full_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment,
            argentum_core::auth::AdminUser,
            argentum_core::auth::AuthSession
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    seed_phase2(&mut db).await.expect("seed_phase2");
    db
}

/// `Db` with one author and post per tenant, for tenancy tests.
pub async fn tenanted_db() -> (Db, uuid::Uuid, uuid::Uuid) {
    let t1 = uuid::Uuid::from_u128(1);
    let t2 = uuid::Uuid::from_u128(2);
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment,
            argentum_core::auth::AdminUser,
            argentum_core::auth::AuthSession
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    create_admin(
        &mut db,
        DEMO_ADMIN_EMAIL,
        "Demo Admin",
        DEMO_ADMIN_PASSWORD,
        Some(showcase::models::DEMO_TENANT),
    )
    .await
    .expect("seed demo admin");
    let a1 = toasty::create!(showcase::models::Author {
        tenant_id: t1,
        name: "Alice T1",
        email: "alice.t1@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create author t1");
    let a2 = toasty::create!(showcase::models::Author {
        tenant_id: t2,
        name: "Bob T2",
        email: "bob.t2@example.com",
    })
    .exec(&mut db)
    .await
    .expect("create author t2");
    toasty::create!(showcase::models::Post {
        tenant_id: t1,
        title: "T1 Post",
        body: "body",
        status: "published".to_string(),
        featured: true,
        created_at: "2024-01-15T09:30:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t1.jpg".to_string(),
        tags: "t1".to_string(),
        author_id: a1.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t1");
    toasty::create!(showcase::models::Post {
        tenant_id: t2,
        title: "T2 Post",
        body: "body",
        status: "draft".to_string(),
        featured: false,
        created_at: "2024-06-01T12:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t2.jpg".to_string(),
        tags: "t2".to_string(),
        author_id: a2.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t2");
    (db, t1, t2)
}

/// One request client for every showcase suite (GH #128).
///
/// Replaces the ad-hoc free-function helpers: every request is built here, so
/// a suite attaches cookies, a tenant, or (later) a session in one place.
/// Builder methods clone the client, leaving the base reusable:
/// `client.tenant(t).csrf(&token).post_form(uri, body)`.
#[derive(Clone)]
pub struct TestClient<'a> {
    router: &'a Router,
    cookies: Vec<(String, String)>,
    tenant: Option<uuid::Uuid>,
}

impl<'a> TestClient<'a> {
    /// A client with no cookies and no tenant.
    pub fn new(router: &'a Router) -> Self {
        Self {
            router,
            cookies: Vec::new(),
            tenant: None,
        }
    }

    /// Attach a cookie to every request this client sends. A later value for
    /// the same name replaces the earlier one, like a browser jar.
    pub fn cookie(&self, name: &str, value: &str) -> Self {
        let mut client = self.clone();
        match client.cookies.iter_mut().find(|(kept, _)| kept == name) {
            Some(existing) => existing.1 = value.to_string(),
            None => client.cookies.push((name.to_string(), value.to_string())),
        }
        client
    }

    /// Attach every `(name, value)` pair, e.g. the cookies a response set.
    pub fn cookies(&self, cookies: &[(String, String)]) -> Self {
        let mut client = self.clone();
        client.cookies.extend(
            cookies
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        client
    }

    /// Attach the CSRF cookie the form's `csrf_token` field must match.
    pub fn csrf(&self, token: &str) -> Self {
        self.cookie(argentum_core::csrf::COOKIE_NAME, token)
    }

    /// Carry a tenant as a `Tenant` request extension — the server-set
    /// override seam (GH #131). It takes precedence over the logged-in user's
    /// tenant, letting a suite scope one request to another tenant.
    pub fn tenant(&self, tenant: uuid::Uuid) -> Self {
        let mut client = self.clone();
        client.tenant = Some(tenant);
        client
    }

    /// GET `uri`.
    pub async fn get(&self, uri: &str) -> http::Response<Body> {
        self.router
            .handle(self.request(http::Method::GET, uri))
            .await
    }

    /// POST an urlencoded form.
    pub async fn post_form(&self, uri: &str, body: String) -> http::Response<Body> {
        let mut request = self.request(http::Method::POST, uri);
        request.headers_mut().insert(
            CONTENT_TYPE,
            http::HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        *request.body_mut() = Body::from(body);
        self.router.handle(request).await
    }

    /// POST a multipart body (file uploads).
    pub async fn post_multipart(
        &self,
        uri: &str,
        boundary: &str,
        body: String,
    ) -> http::Response<Body> {
        let mut request = self.request(http::Method::POST, uri);
        request.headers_mut().insert(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}")
                .parse()
                .expect("multipart content type"),
        );
        *request.body_mut() = Body::from(body);
        self.router.handle(request).await
    }

    /// Build a request carrying this client's cookies and tenant.
    fn request(&self, method: http::Method, uri: &str) -> http::Request<Body> {
        let mut builder = http::Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(COOKIE, jar);
        }
        let (mut parts, body) = builder.body(Body::empty()).unwrap().into_parts();
        if let Some(tenant) = self.tenant {
            parts.extensions.insert(Tenant(tenant));
        }
        http::Request::from_parts(parts, body)
    }
}

/// The session cookie name Topcoat's default token store writes (`__Host-`
/// prefix plus the `session` name, per its hardened cookie contract).
pub const SESSION_COOKIE: &str = "__Host-session";

/// The `(name, value)` pairs a response's `Set-Cookie` headers carry.
pub fn response_cookies(response: &http::Response<Body>) -> Vec<(String, String)> {
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

/// The full `Set-Cookie` header for `name`, so tests can assert attributes.
pub fn set_cookie_header(response: &http::Response<Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{name}=")))
        .map(str::to_string)
}

/// Log in through `{prefix}/login` like a browser: fetch the page, reuse its
/// CSRF pair, post the credentials, and keep every cookie the exchange set.
pub async fn login<'a>(router: &'a Router, email: &str, password: &str) -> TestClient<'a> {
    login_next(router, email, password, "").await.0
}

/// A client logged in as the seeded demo admin.
pub async fn demo_client(router: &Router) -> TestClient<'_> {
    login(router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await
}

/// The session cookie value a response set, if any.
pub fn session_cookie_value(response: &http::Response<Body>) -> Option<String> {
    response_cookies(response)
        .into_iter()
        .find(|(name, _)| name == SESSION_COOKIE)
        .map(|(_, value)| value)
}

/// [`login`] with an explicit `next` destination. Returns the client (CSRF +
/// session cookies) and the login POST response, so callers can assert on the
/// redirect and the `Set-Cookie` headers.
pub async fn login_next<'a>(
    router: &'a Router,
    email: &str,
    password: &str,
    next: &str,
) -> (TestClient<'a>, http::Response<Body>) {
    let page = TestClient::new(router).get("/admin/login").await;
    let cookies = response_cookies(&page);
    let html = body_string(page).await;
    let csrf = input_value(&html, "csrf_token")
        .unwrap_or_else(|| panic!("login page must embed a csrf_token input: {html}"));
    let body = form_body(&[
        ("email", email),
        ("password", password),
        ("next", next),
        ("csrf_token", &csrf),
    ]);
    let response = TestClient::new(router)
        .cookies(&cookies)
        .post_form("/admin/login", body)
        .await;
    let session = response_cookies(&response);
    (
        TestClient::new(router).cookies(&cookies).cookies(&session),
        response,
    )
}

/// URL-encode `(key, value)` pairs into an urlencoded form body.
pub fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

/// The `value` attribute of the named `<input>` in rendered HTML, in either
/// attribute order.
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

/// Collect a response body as a lossy UTF-8 string.
pub async fn body_string(response: http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Assert every hydrated key is a declared form field (GH #89): a renamed
/// lens without an updated string literal would render blank and break the
/// unique unchanged-skip.
pub fn assert_hydrate_keys_are_form_fields<R: Resource>(cx: &Cx, record: &R::Model) {
    let fields = R::form(cx).field_names();
    for key in R::hydrate_form_values(record).keys() {
        assert!(
            fields.contains(key),
            "hydrate key {key} is not a {} form field (GH #89)",
            R::slug()
        );
    }
}
