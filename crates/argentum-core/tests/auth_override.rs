//! The override seam, proven end to end (spec #127, ticket #132): a
//! test-local user model implements `Authenticator`, a minimal `Panel` is
//! gated over it, and a full login round-trip runs through `Router::handle`.
//! This is the "bring your own user table" path ADR-0013 promises.

use argentum_core::{
    Auth, Resource, Table, TextColumn,
    auth::{AuthFuture, Authenticator, CurrentUser},
};
use http::header::{COOKIE, LOCATION, SET_COOKIE};
use toasty::Db;
use topcoat::{
    context::Cx,
    router::{Body, Router, response::Response},
};
use uuid::Uuid;

use crate::common::{
    body_string, cookies, get_with_cookies, input_value, memory_db, post_form, router_with,
};

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
            .pk(|member: &Member| member.id.to_string())
            .paginate(25)
            .columns(TextColumn::r#for(
                Member::fields().handle(),
                |member: &Member| member.handle.clone(),
            ))
    }
}

async fn seeded_db() -> Db {
    let mut db = memory_db(toasty::models!(Member, argentum_core::auth::AuthSession)).await;
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

fn cookie_value(response: &Response<Body>, name: &str) -> Option<String> {
    cookies(response)
        .into_iter()
        .find(|(cookie, _)| cookie == name)
        .map(|(_, value)| value)
}

/// Scrape the login page's CSRF pair (cookie + hidden token) — the shared
/// first step of every login flow in this suite.
async fn csrf_pair(router: &Router) -> (String, String) {
    let page = get_with_cookies(router, "/admin/login", &[]).await;
    let csrf_cookie = cookie_value(&page, argentum_core::csrf::COOKIE_NAME).expect("CSRF cookie");
    let html = body_string(page).await;
    let csrf = input_value(&html, "csrf_token").expect("CSRF field");
    (csrf_cookie, csrf)
}

/// Log in as the seeded member, returning the session cookie value.
async fn login_session(router: &Router) -> String {
    let (csrf_cookie, csrf) = csrf_pair(router).await;
    let login = post_form(
        router,
        "/admin/login",
        &[(argentum_core::csrf::COOKIE_NAME, csrf_cookie)],
        format!("email=ada&password=opensesame&csrf_token={csrf}"),
    )
    .await;
    assert_eq!(login.status(), 303, "login succeeds for an active member");
    cookie_value(&login, "__Host-session").expect("session cookie")
}

/// A member whose panel access is revoked mid-session must still be able to
/// log out (GH #146): the gate answers the logout route for any resolved
/// user, so the session row + cookie are cleared instead of lingering to
/// expiry behind a 403.
#[tokio::test]
async fn revoked_panel_access_can_still_log_out() {
    let db = seeded_db().await;
    let router = router_with::<MemberResource>(db.clone(), Auth::custom(MemberAuth));
    let session = login_session(&router).await;

    // Revoke the member's panel access; the live session still resolves.
    let mut db2 = db.clone();
    let mut member = Member::filter(Member::fields().handle().eq("ada".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("seeded member");
    toasty::update!(member { active: false })
        .exec(&mut db2)
        .await
        .unwrap();

    // Panel pages now 403 the de-permitted user...
    let response = get_with_cookies(
        &router,
        "/admin/members",
        &[("__Host-session", session.clone())],
    )
    .await;
    assert_eq!(response.status(), 403, "pages deny the de-permitted user");

    // ...but logout still answers: 303 to login, row deleted, cookie cleared.
    let logout_csrf = Uuid::new_v4().to_string();
    let logout = post_form(
        &router,
        "/admin/logout",
        &[
            ("__Host-session", session.clone()),
            (argentum_core::csrf::COOKIE_NAME, logout_csrf.clone()),
        ],
        format!("csrf_token={logout_csrf}"),
    )
    .await;
    assert_eq!(
        logout.status(),
        303,
        "a de-permitted user must still be able to log out"
    );
    assert_eq!(
        logout.headers().get(LOCATION).unwrap(),
        "/admin/login",
        "logout lands on the login page"
    );
    let cleared = logout
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("__Host-session="))
        .expect("logout clears the session cookie")
        .to_string();
    assert!(
        cleared.contains("Max-Age=0") || cleared.contains("Expires=Thu, 01 Jan 1970"),
        "the session cookie must be cleared: {cleared}"
    );
    let mut db2 = db.clone();
    assert!(
        argentum_core::auth::AuthSession::all()
            .exec(&mut db2)
            .await
            .unwrap()
            .is_empty(),
        "the session row must be deleted"
    );
}

#[tokio::test]
async fn custom_authenticator_completes_a_full_login_round_trip() {
    let db = seeded_db().await;
    let router = router_with::<MemberResource>(db, Auth::custom(MemberAuth));

    // No session: the panel gate redirects to the login page with `next`.
    let response = get_with_cookies(&router, "/admin/members", &[]).await;
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers().get(LOCATION).unwrap(),
        "/admin/login?next=%2Fadmin%2Fmembers"
    );

    // Log in through the shipped login page: its CSRF pair is reused, and a
    // wrong secret gets the one generic 403.
    let (csrf_cookie, csrf) = csrf_pair(&router).await;

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
        body_string(wrong)
            .await
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
    let response = get_with_cookies(
        &router,
        "/admin/members",
        &[("__Host-session", session.clone())],
    )
    .await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
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
                    format!(
                        "__Host-session={session}; {}={logout_csrf}",
                        argentum_core::csrf::COOKIE_NAME
                    ),
                )
                .body(Body::from(format!("csrf_token={logout_csrf}")))
                .unwrap(),
        )
        .await;
    assert_eq!(logout.status(), 303);
    let response =
        get_with_cookies(&router, "/admin/members", &[("__Host-session", session)]).await;
    assert_eq!(response.status(), 307, "logout must revoke the session");
}
