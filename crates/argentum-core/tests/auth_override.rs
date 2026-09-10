//! The override seam, proven end to end (spec #127, ticket #132): a
//! test-local user model implements `Authenticator`, a minimal `Panel` is
//! gated over it, and a full login round-trip runs through `Router::handle`.
//! This is the "bring your own user table" path ADR-0013 promises.

use argentum_core::auth::{AuthFuture, Authenticator, CurrentUser};
use argentum_core::{Auth, Panel, Resource, Table, TextColumn};
use http::header::{COOKIE, LOCATION, SET_COOKIE};
use toasty::Db;
use topcoat::context::Cx;
use topcoat::router::response::Response;
use topcoat::router::{Body, Router};
use uuid::Uuid;

/// A custom user table — deliberately not `AdminUser`.
#[derive(Debug, Clone, toasty::Model)]
struct Member {
    #[key]
    #[auto]
    id: Uuid,
    #[unique]
    handle: String,
    secret: String,
    display_name: String,
    active: bool,
    tenant_id: Option<Uuid>,
}

/// The one trait implementation an app with an existing user table writes.
struct MemberAuth;

impl MemberAuth {
    fn current(member: Member) -> CurrentUser {
        CurrentUser {
            id: member.id.to_string(),
            login: member.handle.clone(),
            display_name: member.display_name,
            tenant_id: member.tenant_id,
            can_access_panel: member.active,
        }
    }
}

impl Authenticator for MemberAuth {
    fn verify<'a>(
        &'a self,
        cx: &'a Cx,
        login: &'a str,
        password: &'a str,
    ) -> AuthFuture<'a, Option<CurrentUser>> {
        Box::pin(async move {
            let mut db = argentum_core::db::db(cx);
            let member = Member::filter(Member::fields().handle().eq(login.trim().to_string()))
                .first()
                .exec(&mut db)
                .await
                .map_err(topcoat::Error::from)?;
            let Some(member) = member else {
                return Ok(None);
            };
            if member.secret != password {
                return Ok(None);
            }
            Ok(Some(Self::current(member)))
        })
    }

    fn find_by_id<'a>(&'a self, cx: &'a Cx, id: &'a str) -> AuthFuture<'a, Option<CurrentUser>> {
        Box::pin(async move {
            let Ok(id) = Uuid::parse_str(id) else {
                return Ok(None);
            };
            let mut db = argentum_core::db::db(cx);
            let member = Member::filter(Member::fields().id().eq(id))
                .first()
                .exec(&mut db)
                .await
                .map_err(topcoat::Error::from)?;
            Ok(member.map(Self::current))
        })
    }
}

struct MemberResource;

impl Resource for MemberResource {
    type Model = Member;

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Member> {
        Table::r#for(cx)
            .id(|member: &Member| member.id.to_string())
            .columns(TextColumn::r#for(
                Member::fields().handle(),
                |member: &Member| member.handle.clone(),
            ))
    }
}

async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(Member, argentum_core::auth::AuthSession))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    toasty::create!(Member {
        handle: "ada".to_string(),
        secret: "opensesame".to_string(),
        display_name: "Ada Member".to_string(),
        active: true,
        tenant_id: Some(Uuid::from_u128(7)),
    })
    .exec(&mut db)
    .await
    .expect("seed member");
    db
}

fn router(db: Db) -> Router {
    Panel::new("admin")
        .app_context(db)
        .auth(Auth::custom(MemberAuth))
        .resource::<MemberResource>()
        .build()
}

async fn get(router: &Router, uri: &str, cookies: &[(&str, String)]) -> Response<Body> {
    let mut request = http::Request::builder().uri(uri);
    if !cookies.is_empty() {
        let jar = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        request = request.header(COOKIE, jar);
    }
    router.handle(request.body(Body::empty()).unwrap()).await
}

async fn post_form(
    router: &Router,
    uri: &str,
    cookies: &[(&str, String)],
    body: String,
) -> Response<Body> {
    let mut request = http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(
            http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        );
    if !cookies.is_empty() {
        let jar = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        request = request.header(COOKIE, jar);
    }
    router.handle(request.body(Body::from(body)).unwrap()).await
}

/// The `(name, value)` pairs a response's `Set-Cookie` headers carry.
fn cookies(response: &Response<Body>) -> Vec<(String, String)> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect()
}

fn cookie_value(response: &Response<Body>, name: &str) -> Option<String> {
    cookies(response)
        .into_iter()
        .find(|(cookie, _)| cookie == name)
        .map(|(_, value)| value)
}

/// The `value` of the named hidden input in rendered HTML.
fn input_value(html: &str, name: &str) -> Option<String> {
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

#[tokio::test]
async fn custom_authenticator_completes_a_full_login_round_trip() {
    let db = seeded_db().await;
    let router = router(db);

    // No session: the panel gate redirects to the login page with `next`.
    let response = get(&router, "/admin/members", &[]).await;
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers().get(LOCATION).unwrap(),
        "/admin/login?next=%2Fadmin%2Fmembers"
    );

    // Log in through the shipped login page: its CSRF pair is reused, and a
    // wrong secret gets the one generic 403.
    let page = get(&router, "/admin/login", &[]).await;
    let csrf_cookie = cookie_value(&page, argentum_core::csrf::COOKIE_NAME).expect("CSRF cookie");
    let html = String::from_utf8_lossy(
        &http_body_util::BodyExt::collect(page.into_body())
            .await
            .unwrap()
            .to_bytes(),
    )
    .into_owned();
    let csrf = input_value(&html, "csrf_token").expect("CSRF field");

    let csrf_cookies = [(argentum_core::csrf::COOKIE_NAME, csrf_cookie)];
    // The shipped login form posts `email`/`password`; the custom
    // authenticator interprets those values as its handle/secret.
    let wrong = post_form(
        &router,
        "/admin/login",
        &csrf_cookies,
        format!("email=ada&password=wrong&csrf_token={csrf}"),
    )
    .await;
    assert_eq!(wrong.status(), 403);
    assert!(
        String::from_utf8_lossy(
            &http_body_util::BodyExt::collect(wrong.into_body())
                .await
                .unwrap()
                .to_bytes()
        )
        .contains("Invalid email or password.")
    );

    let login = post_form(
        &router,
        "/admin/login",
        &csrf_cookies,
        format!("email=ada&password=opensesame&csrf_token={csrf}"),
    )
    .await;
    assert_eq!(login.status(), 303, "success stays on the Ok path");
    let session = cookie_value(&login, "__Host-session").expect("session cookie");

    // The same request now serves the gated page.
    let response = get(
        &router,
        "/admin/members",
        &[("__Host-session", session.clone())],
    )
    .await;
    assert_eq!(response.status(), 200);
    let html = String::from_utf8_lossy(
        &http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes(),
    )
    .into_owned();
    assert!(html.contains("ada"), "member list must render: {html}");

    // The session is server-side: logging out revokes it and the cookie
    // stops reaching the panel.
    let logout_csrf = Uuid::new_v4().to_string();
    let logout = router
        .handle(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/admin/logout")
                .header(
                    http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .header(
                    COOKIE,
                    format!("__Host-session={session}; argentum_csrf={logout_csrf}"),
                )
                .body(Body::from(format!("csrf_token={logout_csrf}")))
                .unwrap(),
        )
        .await;
    assert_eq!(logout.status(), 303);
    let response = get(&router, "/admin/members", &[("__Host-session", session)]).await;
    assert_eq!(response.status(), 307, "logout must revoke the session");
}
