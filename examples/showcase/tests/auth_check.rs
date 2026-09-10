//! Authentication integration tests (spec #127, tickets #129/#130): the
//! shipped password auth, server-side sessions, login/logout, the shell's
//! account controls, and the fail-closed gate over the panel and runtime
//! prefixes.

use argentum_core::auth::{AdminUser, AuthSession};
use http::header::{COOKIE, LOCATION};
use showcase::app::router_for_tests as router;
use showcase::models::{DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD};
use topcoat::context::CxTestBuilder;
use topcoat::router::Body;

mod common;
use common::{
    SESSION_COOKIE, TestClient, body_string, form_body, full_db, input_value, login, login_next,
    response_cookies, session_cookie_value, set_cookie_header,
};

/// A runtime (page re-run) POST, optionally carrying a session cookie.
async fn runtime_post(
    router: &topcoat::router::Router,
    session: Option<&str>,
) -> http::Response<Body> {
    let mut request = http::Request::builder()
        .method(http::Method::POST)
        .uri("/_topcoat/runtime/pages/admin/users")
        .header(http::header::CONTENT_TYPE, "application/json");
    if let Some(session) = session {
        request = request.header(COOKIE, format!("{SESSION_COOKIE}={session}"));
    }
    router.handle(request.body(Body::from("{}")).unwrap()).await
}

#[tokio::test]
async fn login_page_is_standalone_with_csrf_and_demo_credentials() {
    let db = full_db().await;
    let router = router(db);
    let response = TestClient::new(&router).get("/admin/login").await;

    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    assert!(html.contains("Sign in"), "missing heading: {html}");
    assert!(html.contains("Showcase"), "missing brand: {html}");
    assert!(
        html.contains("Demo credentials: admin@example.com / password"),
        "missing demo hint: {html}"
    );
    assert!(
        html.contains("name=\"csrf_token\""),
        "missing CSRF hidden field: {html}"
    );
    assert!(
        html.contains("action=\"/admin/login\""),
        "missing form action: {html}"
    );
    assert!(
        !html.contains("data-sidebar=\"sidebar\""),
        "login page must not render the sidebar shell: {html}"
    );
}

#[tokio::test]
async fn login_sets_a_hardened_session_cookie() {
    let db = full_db().await;
    let router = router(db);
    let (_client, response) = login_next(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, "").await;

    assert_eq!(response.status(), 303, "success is a Post/Redirect/Get");
    assert_eq!(
        response.headers().get(http::header::LOCATION).unwrap(),
        "/admin"
    );
    let cookie = set_cookie_header(&response, SESSION_COOKIE).expect("session cookie");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
}

#[tokio::test]
async fn login_accepts_only_same_origin_relative_next_targets() {
    let db = full_db().await;
    let router = router(db);
    for (next, expected) in [
        ("/admin/users", "/admin/users"),
        ("/admin/posts?status=draft", "/admin/posts?status=draft"),
        ("//evil.example/login", "/admin"),
        ("https://evil.example/steal", "/admin"),
        ("/\\evil.example", "/admin"),
        ("/admin/users\nLocation: https://evil.example", "/admin"),
    ] {
        let (_client, response) =
            login_next(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, next).await;
        assert_eq!(response.status(), 303, "next={next:?}");
        assert_eq!(
            response.headers().get(http::header::LOCATION).unwrap(),
            expected,
            "next={next:?}"
        );
    }
}

#[tokio::test]
async fn every_login_failure_renders_one_generic_error() {
    let db = full_db().await;
    let router = router(db);
    for (email, password) in [
        (DEMO_ADMIN_EMAIL, "definitely-wrong"),
        ("nobody@example.com", DEMO_ADMIN_PASSWORD),
        ("", ""),
    ] {
        let (_, response) = login_next(&router, email, password, "").await;
        assert_eq!(response.status(), 403, "email={email:?}");
        assert!(
            session_cookie_value(&response).is_none(),
            "a failed login must not start a session"
        );
        let html = body_string(response).await;
        assert!(
            html.contains("Invalid email or password."),
            "one generic error: {html}"
        );
    }
}

#[tokio::test]
async fn deactivated_admin_cannot_log_in() {
    let db = full_db().await;
    let mut db2 = db.clone();
    let mut admin = AdminUser::filter(AdminUser::fields().email().eq(DEMO_ADMIN_EMAIL.to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("seeded admin");
    toasty::update!(admin { active: false })
        .exec(&mut db2)
        .await
        .unwrap();

    let router = router(db);
    let (_, response) = login_next(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, "").await;
    assert_eq!(response.status(), 403);
    let html = body_string(response).await;
    assert!(html.contains("Invalid email or password."), "{html}");
}

#[tokio::test]
async fn shell_shows_the_signed_in_user_and_logout() {
    let db = full_db().await;
    let router = router(db);
    let client = login(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    let response = client.get("/admin/users").await;
    assert_eq!(response.status(), 200);
    let html = body_string(response).await;
    assert!(html.contains("Demo Admin"), "missing display name: {html}");
    assert!(html.contains("Sign out"), "missing logout control: {html}");
    assert!(
        html.contains("action=\"/admin/logout\""),
        "missing logout action: {html}"
    );
}

#[tokio::test]
async fn logout_deletes_the_session_and_clears_the_cookie() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = login(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    let page = body_string(client.get("/admin/users").await).await;
    let csrf = input_value(&page, "csrf_token").expect("logout form carries a CSRF token");
    let response = client
        .post_form("/admin/logout", form_body(&[("csrf_token", &csrf)]))
        .await;
    assert_eq!(response.status(), 303);
    assert_eq!(
        response.headers().get(http::header::LOCATION).unwrap(),
        "/admin/login"
    );
    let cleared = set_cookie_header(&response, SESSION_COOKIE).expect("cookie clearing header");
    assert!(
        cleared.contains("Max-Age=0") || cleared.contains("Expires=Thu, 01 Jan 1970"),
        "logout must clear the cookie: {cleared}"
    );

    let mut db2 = db.clone();
    assert_eq!(AuthSession::all().exec(&mut db2).await.unwrap().len(), 0);

    // The stale cookie no longer resolves a user: the gate redirects to login.
    let stale = client.get("/admin/users").await;
    assert_eq!(stale.status(), 307, "ended session still resolved");
}

#[tokio::test]
async fn login_rotates_the_session_token() {
    let db = full_db().await;
    let router = router(db.clone());
    let (client, first) = login_next(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, "").await;
    let first_session = session_cookie_value(&first).expect("first session cookie");

    // Present the first session while logging in again.
    let page = body_string(client.get("/admin/login").await).await;
    let csrf = input_value(&page, "csrf_token").expect("CSRF token");
    let response = client
        .post_form(
            "/admin/login",
            form_body(&[
                ("email", DEMO_ADMIN_EMAIL),
                ("password", DEMO_ADMIN_PASSWORD),
                ("csrf_token", &csrf),
            ]),
        )
        .await;
    assert_eq!(response.status(), 303);
    let second_session = session_cookie_value(&response).expect("rotated session cookie");
    assert_ne!(first_session, second_session, "login must mint a new token");

    let mut db2 = db.clone();
    let rows = AuthSession::all().exec(&mut db2).await.unwrap();
    assert_eq!(rows.len(), 1, "the pre-login session is revoked");

    // The pre-login token cannot be replayed: the gate redirects to login.
    let stale = TestClient::new(&router).cookie(SESSION_COOKIE, &first_session);
    assert_eq!(
        stale.get("/admin/users").await.status(),
        307,
        "pre-login token still works"
    );
}

#[tokio::test]
async fn expired_sessions_resolve_to_no_user() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = login(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    let mut db2 = db.clone();
    let mut row = AuthSession::all()
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("session row");
    toasty::update!(row {
        expires_at: "2020-01-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
    })
    .exec(&mut db2)
    .await
    .unwrap();

    assert_eq!(
        client.get("/admin/users").await.status(),
        307,
        "expired session resolved"
    );
    assert!(
        AuthSession::all().exec(&mut db2).await.unwrap().is_empty(),
        "expired rows are purged"
    );
}

#[tokio::test]
async fn revoke_sessions_for_user_ends_access() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = login(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD).await;

    let mut db2 = db.clone();
    let admin = AdminUser::filter(AdminUser::fields().email().eq(DEMO_ADMIN_EMAIL.to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("seeded admin");
    let cx = CxTestBuilder::new().app_context(db.clone()).build();
    argentum_core::auth::revoke_sessions_for_user(&cx, &admin.id.to_string())
        .await
        .unwrap();

    assert!(AuthSession::all().exec(&mut db2).await.unwrap().is_empty());
    assert_eq!(
        client.get("/admin/users").await.status(),
        307,
        "revoked session resolved"
    );
}

#[tokio::test]
async fn unauthenticated_panel_pages_redirect_to_login_with_validated_next() {
    let db = full_db().await;
    let router = router(db);
    for path in [
        "/admin/users",
        "/admin/users/create",
        "/admin/authors",
        "/admin/posts",
        "/admin/posts?filters=status%3Apublished",
        "/admin/showcase",
    ] {
        let response = TestClient::new(&router).get(path).await;
        assert_eq!(response.status(), 307, "{path}");
        let expected = format!("/admin/login?{}", form_body(&[("next", path)]));
        assert_eq!(
            response.headers().get(LOCATION).unwrap().to_str().unwrap(),
            expected,
            "{path}"
        );
    }
}

#[tokio::test]
async fn unauthenticated_runtime_requests_answer_401_not_a_redirect() {
    let db = full_db().await;
    let router = router(db);
    // The gate covers the whole `/_topcoat/runtime` prefix (shards, page
    // re-runs, procedures); page re-runs are the endpoint this binary mounts.
    assert_eq!(
        runtime_post(&router, None).await.status(),
        401,
        "page re-run"
    );
}

#[tokio::test]
async fn unauthenticated_mutations_answer_401_not_a_redirect() {
    let db = full_db().await;
    let router = router(db);
    let response = TestClient::new(&router)
        .post_form("/admin/users/create", "name=x".to_string())
        .await;
    assert_eq!(
        response.status(),
        401,
        "a mutation must not be redirected into the login POST"
    );
}

#[tokio::test]
async fn deactivating_a_user_invalidates_their_live_session() {
    let db = full_db().await;
    let router = router(db.clone());
    let (_, login_response) = login_next(&router, DEMO_ADMIN_EMAIL, DEMO_ADMIN_PASSWORD, "").await;
    let session = session_cookie_value(&login_response).expect("session cookie");

    let mut db2 = db.clone();
    let mut admin = AdminUser::filter(AdminUser::fields().email().eq(DEMO_ADMIN_EMAIL.to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("seeded admin");
    toasty::update!(admin { active: false })
        .exec(&mut db2)
        .await
        .unwrap();

    let client = TestClient::new(&router).cookie(SESSION_COOKIE, &session);
    assert_eq!(
        client.get("/admin/users").await.status(),
        307,
        "page: deactivation invalidates the session"
    );
    assert_eq!(
        runtime_post(&router, Some(&session)).await.status(),
        401,
        "runtime"
    );
    assert!(
        AuthSession::all().exec(&mut db2).await.unwrap().is_empty(),
        "dead session rows are purged"
    );
}

#[tokio::test]
async fn failed_login_preserves_the_next_destination() {
    let db = full_db().await;
    let router = router(db);
    let (_, response) = login_next(&router, DEMO_ADMIN_EMAIL, "wrong", "/admin/posts").await;
    assert_eq!(response.status(), 403);
    let html = body_string(response).await;
    assert_eq!(
        input_value(&html, "next").as_deref(),
        Some("/admin/posts"),
        "a retry must keep the original destination: {html}"
    );
}

#[tokio::test]
async fn login_returns_to_the_originally_requested_page() {
    let db = full_db().await;
    let router = router(db);

    // Follow the gate's redirect the way a browser would.
    let gated = TestClient::new(&router).get("/admin/posts").await;
    assert_eq!(gated.status(), 307);
    let login_url = gated
        .headers()
        .get(LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let page = TestClient::new(&router).get(&login_url).await;
    let cookies = response_cookies(&page);
    let html = body_string(page).await;
    assert_eq!(input_value(&html, "next").as_deref(), Some("/admin/posts"));
    let csrf = input_value(&html, "csrf_token").expect("CSRF token");

    let response = TestClient::new(&router)
        .cookies(&cookies)
        .post_form(
            "/admin/login",
            form_body(&[
                ("email", DEMO_ADMIN_EMAIL),
                ("password", DEMO_ADMIN_PASSWORD),
                ("next", "/admin/posts"),
                ("csrf_token", &csrf),
            ]),
        )
        .await;
    assert_eq!(response.status(), 303);
    assert_eq!(
        response.headers().get(LOCATION).unwrap(),
        "/admin/posts",
        "success must land where the visitor was headed"
    );
}

#[tokio::test]
async fn auth_disabled_serves_the_panel_without_login() {
    let db = full_db().await;
    let router = argentum_core::Panel::new("admin")
        .app_context(db)
        .auth(argentum_core::Auth::disabled())
        .resource::<showcase::app::UserResource>()
        .build();

    assert_eq!(
        TestClient::new(&router).get("/admin/users").await.status(),
        200,
        "disabled auth must not gate"
    );
    assert_eq!(
        TestClient::new(&router).get("/admin/login").await.status(),
        404,
        "no login routes when auth is off"
    );
}
