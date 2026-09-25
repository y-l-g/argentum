//! Shared fixtures and request scaffolding for the showcase integration
//! tests (#128). Every test crate includes this module via
//! `mod common;` and uses a subset of it, so `dead_code` is expected here and
//! allowed once instead of leaking per-crate warnings.

#![allow(dead_code)]

use http::header::{CONTENT_TYPE, COOKIE};
use http_body_util::BodyExt;
use showcase::models::{DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, create_admin, seed, seed_phase2};
use tablo_core::{Resource, Tenant};
use toasty::Db;
use topcoat::{
    context::Cx,
    router::{Body, Router},
};

/// A fresh in-memory `Db` carrying the **full** showcase model set, schema
/// pushed and no rows — the one place the model list is written.
///
/// The list is the whole showcase set even though most tests touch one or two
/// tables: a lens path is resolved against the app schema, and the
/// `Panel` registers every resource regardless of which tables a given test
/// cares about. A narrower `models!(..)` made the panel's schema incomplete, so
/// a form for an unregistered model could not resolve its embedded paths — and
/// would have bound whichever model the id happened to name. An empty table
/// costs nothing; an incomplete schema misleads.
pub async fn empty_schema_db() -> Db {
    let db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment,
            showcase::models::MediaAsset,
            tablo_core::auth::AdminUser,
            tablo_core::auth::AuthSession
        ))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    db
}

/// [`empty_schema_db`] with the demo admin seeded and zero user rows.
pub async fn empty_users_db() -> Db {
    let mut db = empty_schema_db().await;
    create_admin(
        &mut db,
        DEMO_ADMIN_EMAIL,
        "Demo Admin",
        DEMO_ADMIN_PASSWORD,
        Some(showcase::models::DEMO_TENANT),
    )
    .await
    .expect("seed demo admin");
    db
}

/// `Db` with the phase-1 users seed applied.
pub async fn seeded_db() -> Db {
    let mut db = empty_schema_db().await;
    seed(&mut db).await.expect("seed");
    db
}

/// `Db` with both seed phases (users, authors, posts, comments) and the
/// shipped auth models.
pub async fn full_db() -> Db {
    let mut db = empty_schema_db().await;
    seed(&mut db).await.expect("seed");
    seed_phase2(&mut db).await.expect("seed_phase2");
    db
}

/// `Db` with one author and post per tenant, for tenancy tests.
pub async fn tenanted_db() -> (Db, uuid::Uuid, uuid::Uuid) {
    let t1 = uuid::Uuid::from_u128(1);
    let t2 = uuid::Uuid::from_u128(2);
    let mut db = empty_schema_db().await;
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
    let p1 = toasty::create!(showcase::models::Post {
        tenant_id: t1,
        title: "T1 Post",
        body: "body",
        status: "published".to_string(),
        featured: true,
        created_at: "2024-01-15T09:30:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t1.jpg".to_string(),
        tags: "t1".to_string(),
        seo: showcase::models::Seo {
            title: "T1 SEO".to_string(),
            description: String::new(),
        },
        publication: showcase::models::Publication::Published {
            published_at: "2024-01-15T09:30:00Z".to_string(),
            canonical_url: String::new(),
        },
        media: showcase::models::Media::Image {
            url: "/images/t1.jpg".to_string(),
            alt: String::new(),
        },
        post_stats: showcase::models::PostStats {
            word_count: 0,
            read_minutes: 0,
        },
        author_id: a1.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t1");
    let p2 = toasty::create!(showcase::models::Post {
        tenant_id: t2,
        title: "T2 Post",
        body: "body",
        status: "draft".to_string(),
        featured: false,
        created_at: "2024-06-01T12:00:00Z".parse::<jiff::Timestamp>().unwrap(),
        image_path: "/images/t2.jpg".to_string(),
        tags: "t2".to_string(),
        seo: showcase::models::Seo {
            title: "T2 SEO".to_string(),
            description: String::new(),
        },
        publication: showcase::models::Publication::Scheduled {
            scheduled_at: "2024-07-01T09:00:00Z".to_string(),
            scheduled_for: String::new(),
        },
        media: showcase::models::Media::Image {
            url: "/images/t2.jpg".to_string(),
            alt: String::new(),
        },
        post_stats: showcase::models::PostStats {
            word_count: 0,
            read_minutes: 0,
        },
        author_id: a2.id,
    })
    .exec(&mut db)
    .await
    .expect("create post t2");
    // One comment per tenant post: the inherit-through-the-relation
    // fixture for the Comments queue's tenant scoping.
    toasty::create!(showcase::models::Comment {
        body: "T1 comment",
        post_id: p1.id,
    })
    .exec(&mut db)
    .await
    .expect("create comment t1");
    toasty::create!(showcase::models::Comment {
        body: "T2 comment",
        post_id: p2.id,
    })
    .exec(&mut db)
    .await
    .expect("create comment t2");
    (db, t1, t2)
}

/// One request client for every showcase suite.
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
    /// Like a browser jar: a later value for the same name replaces the
    /// earlier one instead of appending a duplicate `Cookie` entry.
    pub fn cookies(&self, cookies: &[(String, String)]) -> Self {
        let mut client = self.clone();
        for (name, value) in cookies {
            match client.cookies.iter_mut().find(|(kept, _)| kept == name) {
                Some(existing) => existing.1 = value.clone(),
                None => client.cookies.push((name.clone(), value.clone())),
            }
        }
        client
    }

    /// Attach the CSRF cookie the form's `csrf_token` field must match.
    pub fn csrf(&self, token: &str) -> Self {
        self.cookie(tablo_core::csrf::COOKIE_NAME, token)
    }

    /// Carry a tenant as a `Tenant` request extension — the server-set
    /// override seam. It takes precedence over the logged-in user's
    /// tenant, letting a suite scope one request to another tenant.
    pub fn tenant(&self, tenant: uuid::Uuid) -> Self {
        let mut client = self.clone();
        client.tenant = Some(tenant);
        client
    }

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

    /// POST a JSON body to a runtime endpoint (a shard or procedure), with the
    /// page identity header the browser runtime sends (GH #154 §2 tests).
    pub async fn post_json(&self, uri: &str, body: String, identity: &str) -> http::Response<Body> {
        let mut request = self.request(http::Method::POST, uri);
        request.headers_mut().insert(
            CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        request.headers_mut().insert(
            topcoat::router::request::IDENTITY_HEADER,
            http::HeaderValue::from_str(identity).expect("identity header"),
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
///
/// GH #218: this is a real GET + POST + Argon2id verify (~0.4s at the shipped
/// parameters). Use it only where the login flow **is** the subject — the
/// `auth_check` suite, session revocation, deactivation, rotation, failed
/// logins, and the one test that needs a session cookie *without* the paired
/// CSRF cookie. Everywhere else, an authenticated client is setup: use
/// [`demo_client`] / [`tenantless_client`], which mint the session row instead.
pub async fn login<'a>(router: &'a Router, email: &str, password: &str) -> TestClient<'a> {
    login_next(router, email, password, "").await.0
}

/// The raw session cookie value for the seeded admin with `email`, minted
/// directly into `db`.
///
/// The login handler writes one `AuthSession` row keyed by the SHA-256 of a
/// random token and hands the client the encoded token; this does exactly that
/// and nothing else. The request path afterwards is identical — `AuthGate`
/// resolves the cookie through `auth::resolve`, which looks the row up, rejects
/// an expired one, and re-reads the user through `Authenticator::find_by_id`
/// (so `active` and `can_access_panel` still apply). What is skipped is the
/// password verification, which is the point: ~110 tests re-authenticated to
/// get an authenticated client, at ~0.4s each.
pub async fn mint_session(db: &Db, email: &str) -> String {
    use std::{fmt::Write as _, time::SystemTime};

    use tablo_core::auth::{AdminUser, AuthSession, SESSION_LIFETIME};
    use topcoat::session::Token;

    let mut db = db.clone();
    let user = AdminUser::filter(AdminUser::fields().email().eq(email.to_string()))
        .first()
        .exec(&mut db)
        .await
        .expect("look up the seeded admin")
        .unwrap_or_else(|| panic!("the seed creates the admin {email}"));
    let token = Token::random();
    let mut token_hash = String::with_capacity(64);
    for byte in token.hash().iter() {
        write!(token_hash, "{byte:02x}").expect("writing to a String cannot fail");
    }
    toasty::create!(AuthSession {
        token_hash,
        user_id: user.id.to_string(),
        expires_at: jiff::Timestamp::try_from(SystemTime::now() + SESSION_LIFETIME)
            .expect("a representable session expiry"),
        created_at: jiff::Timestamp::now(),
    })
    .exec(&mut db)
    .await
    .expect("mint the session row");
    token.encode()
}

/// A client holding a freshly minted session for `email`.
pub async fn signed_in_client<'a>(router: &'a Router, db: &Db, email: &str) -> TestClient<'a> {
    let token = mint_session(db, email).await;
    TestClient::new(router).cookie(SESSION_COOKIE, &token)
}

/// A client holding a freshly minted session for the seeded demo admin.
pub async fn demo_client<'a>(router: &'a Router, db: &Db) -> TestClient<'a> {
    signed_in_client(router, db, DEMO_ADMIN_EMAIL).await
}

/// A client holding a freshly minted session for the tenantless admin, for the
/// `requires_tenant` fail-closed tests.
pub async fn tenantless_client<'a>(router: &'a Router, db: &Db) -> TestClient<'a> {
    signed_in_client(router, db, showcase::models::TENANTLESS_ADMIN_EMAIL).await
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

/// A multipart body: text fields, then one file part per `(field, filename,
/// bytes)`.
///
/// The framing is what a browser sends for a form with a file input, so a
/// create can carry a real upload through the panel instead of a client-typed
/// text value.
pub fn multipart_body(
    boundary: &str,
    fields: &[(&str, &str)],
    files: &[(&str, &str, &str)],
) -> String {
    let mut body = String::new();
    for (name, value) in fields {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    for (name, filename, bytes) in files {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n{bytes}\r\n"
        ));
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    body
}

/// The `value` attribute of the named `<input>` in rendered HTML, in either
/// attribute order.
///
/// Handles both quote styles (`value="…"` and `value='…'`); the controlled
/// UUID markup only emits double quotes today, but an encoder change must
/// not silently turn every lookup into `None` (harness hardening).
pub fn input_value(html: &str, name: &str) -> Option<String> {
    let double = format!("name=\"{name}\"");
    let single = format!("name='{name}'");
    for tag in html.split('<').skip(1) {
        if !tag.contains(&double) && !tag.contains(&single) {
            continue;
        }
        let attrs = &tag[..tag.find('>')?];
        for (prefix, term) in [("value=\"", '"'), ("value='", '\'')] {
            if let Some(start) = attrs.find(prefix) {
                let rest = &attrs[start + prefix.len()..];
                if let Some(end) = rest.find(term) {
                    // Unescape the matching quote entity the encoder may emit.
                    let raw = &rest[..end];
                    return Some(raw.replace("&quot;", "\"").replace("&#x27;", "'"));
                }
            }
        }
    }
    None
}

/// The record key `kind` (`"delete"` or `"edit"`) from the first row action
/// link, which carries it as a query parameter.
///
/// Reads the control the UI actually renders rather than re-deriving identity:
/// `Table::id` is a display projection and `Table::pk` is the record key
/// so a test that guessed from the display key would be asserting
/// the wrong thing.
pub fn row_link_key(html: &str, kind: &str) -> Option<String> {
    let needle = format!("{kind}=");
    let mut rest = html;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        let end = after.find(['&', '"', '\'']).unwrap_or(after.len());
        if end > 0 {
            return Some(after[..end].to_string());
        }
        rest = &rest[at + needle.len()..];
    }
    None
}

/// The opening `<input …>` tag that carries `type="file"`.
///
/// Attributes render in no guaranteed order (topcoat#122), so callers assert
/// on the whole tag rather than a single attribute's position. Needed because
/// native validation — `required` on a file input — is exactly what broke the
/// post edit form, and only the markup can pin it.
pub fn file_input_tag(html: &str) -> String {
    let at = html.find("type=\"file\"").expect("a file input");
    let start = html[..at].rfind("<input").expect("its opening tag");
    let mut quoted = false;
    for (offset, byte) in html[start..].bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'>' if !quoted => return html[start..start + offset].to_string(),
            _ => {}
        }
    }
    panic!("unterminated <input> tag at byte {start}");
}

/// The first `href="…"` in `html` whose value contains `needle`, with the
/// entities an HTML attribute encoder emits decoded.
///
/// The result is followed as a request URI, so it must be the URL a browser
/// would send. `&amp;` is decoded *last* so `&amp;lt;` becomes the literal
/// `&lt;`, exactly as a browser reads it.
pub fn find_href_with(html: &str, needle: &str) -> Option<String> {
    let mut rest = html;
    loop {
        let start = rest.find("href=\"")?;
        rest = &rest[start + "href=\"".len()..];
        let end = rest.find('"')?;
        let href = &rest[..end];
        if href.contains(needle) {
            return Some(unescape_href(href));
        }
        rest = &rest[end..];
    }
}

/// The pager's `after=`/`before=` link: the first href carrying `needle` that
/// is not the Delete dialog opener.
///
/// The Delete confirmation opener is built from the list URL, so it carries the
/// whole query state — including the cursor — and appends `&delete=<key>`
/// (`TableState::delete_dialog`). Following it opens a dialog instead of
/// advancing the page, and on a page holding one row that href comes first.
/// The View and Edit links are bare `{prefix}/{key}/…` paths with no query.
pub fn find_pager_href(html: &str, needle: &str) -> Option<String> {
    let mut rest = html;
    loop {
        let start = rest.find("href=\"")?;
        rest = &rest[start + "href=\"".len()..];
        let end = rest.find('"')?;
        let href = &rest[..end];
        if href.contains(needle) && !href.contains("delete=") {
            return Some(unescape_href(href));
        }
        rest = &rest[end..];
    }
}

/// Decode the entities an HTML attribute encoder emits in a URL attribute.
fn unescape_href(href: &str) -> String {
    href.replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The row titles rendered into a table table, in document order.
///
/// Each row's first cell is the title projection, so this reads the table the
/// list handlers build (skeleton rows carry no `data-row-select` and are
/// skipped). Used by pagination and comments assertions that care about which
/// rows a page actually holds.
pub fn row_titles(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-row-select") {
        rest = &rest[at..];
        if let Some(td) = rest.find("<td")
            && let Some(gt) = rest[td..].find('>')
        {
            let after = &rest[td + gt + 1..];
            if let Some(end) = after.find("</td>") {
                let text = after[..end].split('<').next().unwrap_or("").trim();
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
        }
        rest = &rest[1..];
    }
    out
}

/// The record key of every rendered row, in document order.
///
/// The bulk checkbox carries the record key as its `value` (`Table::pk`), so a
/// pagination walk can assert the exact rows a page holds: tied display values
/// cannot be told apart by their first cell. Attributes render in no
/// guaranteed order (topcoat#122), so this reads each `<input>` tag whole
/// rather than assuming `value` and the marker sit in a fixed order.
pub fn row_keys(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<input") {
        rest = &rest[at..];
        // The tag ends at the first `>` outside quotes; attribute values carry
        // `>` when topcoat escapes an expression (`=&gt;`).
        let mut quoted = false;
        let mut end = rest.len();
        for (offset, byte) in rest.bytes().enumerate() {
            match byte {
                b'"' => quoted = !quoted,
                b'>' if !quoted => {
                    end = offset;
                    break;
                }
                _ => {}
            }
        }
        let tag = &rest[..end];
        if tag.contains("data-row-select")
            && let Some(value_at) = tag.find("value=\"")
        {
            let after = &tag[value_at + "value=\"".len()..];
            if let Some(close) = after.find('"') {
                out.push(after[..close].to_string());
            }
        }
        rest = &rest[end..];
        if rest.is_empty() {
            break;
        }
        rest = &rest[1..];
    }
    out
}

/// How many `Post` rows the database holds.
///
/// Rejected submissions assert "nothing was created" by comparing this before
/// and after, rather than against a literal row count: a literal asserts the
/// fixture's size instead of the handler's behaviour.
pub async fn post_count(db: &Db) -> usize {
    let mut db = db.clone();
    showcase::models::Post::all()
        .exec(&mut db)
        .await
        .unwrap()
        .len()
}

/// How many `User` rows the database holds.
///
/// [`post_count`]'s pattern for the user list: a seeded-row literal like
/// `8` asserts the fixture's size. Write/delete tests compare this before and
/// after instead.
pub async fn user_count(db: &Db) -> usize {
    let mut db = db.clone();
    showcase::models::User::all()
        .exec(&mut db)
        .await
        .unwrap()
        .len()
}

/// How many `Comment` rows the database holds.
pub async fn comment_count(db: &Db) -> usize {
    let mut db = db.clone();
    showcase::models::Comment::all()
        .exec(&mut db)
        .await
        .unwrap()
        .len()
}

/// Collect a response body as a lossy UTF-8 string.
pub async fn body_string(response: http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Assert every hydrated key is a declared form field: a renamed
/// lens without an updated string literal would render blank and break the
/// unique unchanged-skip.
pub fn assert_hydrate_keys_are_form_fields<R: Resource>(cx: &Cx, record: &R::Model) {
    let fields = R::form(cx).field_names();
    for key in R::hydrate_form_values(cx, record).keys() {
        assert!(
            fields.contains(key),
            "hydrate key {key} is not a {} form field (GH #89)",
            R::slug()
        );
    }
}
