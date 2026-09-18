use http::header::{LOCATION, SET_COOKIE};
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

mod common;
use common::{
    SESSION_COOKIE, TestClient, body_string, demo_client, login_next, response_cookies, seeded_db,
    session_cookie_value, set_cookie_header,
};

#[tokio::test]
async fn manual_create_check() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

    // Test GET /admin/users/create returns 200 with form HTML
    let resp = client.get("/admin/users/create").await;
    println!("GET /admin/users/create status: {}", resp.status());
    assert!(resp.status().is_success(), "GET create should be 200");
    let html = body_string(resp).await;
    // GH #136 layer rule: core (`text_input_renders_with_label_and_ac_field`)
    // owns the field detail (wrapper, Tokens, for/id, error slot); this pins
    // the HTTP wiring — the create page serves the declared fields.
    assert!(
        html.contains("<form"),
        "missing form in {}",
        &html[..html.len().min(2000)]
    );
    assert!(
        html.contains("name=\"name\"") && html.contains("name=\"email\""),
        "missing declared fields in {}",
        &html[..html.len().min(2000)]
    );

    // Test POST empty name
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=&email=not-an-email&csrf_token={csrf}"),
        )
        .await;
    let status = resp.status();
    println!("POST empty name status: {}", status);
    let html = body_string(resp).await;
    assert!(
        html.contains("is required"),
        "should contain is required error, got {}",
        html
    );
    assert!(
        html.contains("must be a valid email"),
        "should contain email error, got {}",
        html
    );
    assert!(
        status.is_success(),
        "invalid POST should re-render 200, not redirect"
    );
    // Check DB still has 3 rows
    let mut db_check = db.clone();
    let count = User::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 8, "DB should still have 8 after invalid");

    // Test POST valid
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/create",
            format!("name=New%20User&email=new%40example.com&csrf_token={csrf}"),
        )
        .await;
    println!("POST valid status: {}", resp.status());
    assert!(
        resp.status().is_redirection(),
        "valid POST should redirect, got {}",
        resp.status()
    );
    let loc = resp
        .headers()
        .get(LOCATION)
        .expect("missing Location")
        .to_str()
        .unwrap()
        .to_string();
    println!("Location: {}", loc);
    assert!(
        loc.contains("/admin/users"),
        "redirect to list, got {}",
        loc
    );
    let cookies: Vec<String> = resp
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    println!("Cookie headers: {cookies:?}");
    // Post/Redirect/Get with one-time semantics (GH #97, #126): a completed
    // create answers 303, the toast rides the flash cookie on that error
    // response, and the Location query stays clean.
    assert_eq!(resp.status(), 303, "a completed create is a 303");
    assert!(
        !loc.contains("notification"),
        "the toast must not ride the query, got {loc}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.contains("__Host-argentum_notification")),
        "the flash cookie must be set on the redirect, got {cookies:?}"
    );
    // Follow the redirect, carrying whatever cookies the POST set (the
    // flash cookie included — the Location query no longer carries it).
    let resp2 = client.cookies(&response_cookies(&resp)).get(&loc).await;
    assert!(
        resp2.status().is_success(),
        "GET list after create should be 200"
    );
    // The shell consumed the one-time flash: the follow-up response clears it.
    let cleared = set_cookie_header(&resp2, "__Host-argentum_notification")
        .expect("following the redirect must consume the flash");
    assert!(
        cleared.contains("Max-Age=0") || cleared.contains("Expires=Thu, 01 Jan 1970"),
        "the flash is one-time, got {cleared}"
    );
    let html2 = body_string(resp2).await;
    // Production page size is 25: the new user sorts onto page 1.
    // Verify DB creation directly, plus the consumed toast.
    assert!(
        html2.contains("data-sonner-toaster") && html2.contains("bottom-4"),
        "missing the bottom-right toast stack in {}",
        html2
    );
    assert!(
        html2.contains("Created"),
        "the shell must render the consumed toast in {}",
        html2
    );
    assert!(
        html2.contains("data-sonner-toast")
            && html2.contains("data-type=\"success\"")
            && html2.contains("shadow-lg"),
        "missing the shadcn/Sonner toast surface, got {}",
        html2
    );
    let mut db_check2 = db.clone();
    let count2 = User::all().exec(&mut db_check2).await.unwrap().len();
    assert_eq!(count2, 9, "DB should have 9 after valid create");
    // Also verify that new user can be found via query
    let new_user = User::filter(User::fields().email().eq("new@example.com".to_string()))
        .first()
        .exec(&mut db_check2)
        .await
        .unwrap();
    assert!(new_user.is_some(), "new user should exist");
}

#[tokio::test]
async fn create_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        email: String,
    }

    struct DenyCreateResource;
    impl Resource for DenyCreateResource {
        type Model = DummyUser;
        fn can_create(_cx: &topcoat::context::Cx) -> bool {
            false
        }
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn table(cx: &topcoat::context::Cx) -> Table<DummyUser> {
            Table::r#for(cx)
                .id(|u: &DummyUser| u.id.to_string())
                .columns(TextColumn::r#for(
                    DummyUser::fields().name(),
                    |u: &DummyUser| u.name.clone(),
                ))
        }
        fn form(_cx: &topcoat::context::Cx) -> Schema {
            Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
        }
        async fn create_record(
            _cx: &topcoat::context::Cx,
            _values: std::collections::HashMap<String, String>,
            _ex: &mut dyn toasty::Executor,
        ) -> topcoat::Result<()> {
            Ok(())
        }
    }

    let db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .auth(argentum_core::Auth::disabled())
        .resource::<DenyCreateResource>()
        .build();
    let client = TestClient::new(&router);

    let slug = DenyCreateResource::slug();
    let create_url = format!("/admin/{}/create", slug);
    // GET create should be 403
    let resp = client.get(&create_url).await;
    assert_eq!(resp.status(), 403, "GET create should be 403 when denied");

    // POST should also be 403 and not create
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(&create_url, format!("name=test&csrf_token={csrf}"))
        .await;
    assert_eq!(
        resp.status(),
        403,
        "POST create should be 403 when denied, got {}",
        resp.status()
    );
    // Check DB still empty
    let mut db_check = db.clone();
    let count = DummyUser::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 0, "should not create when denied");
}

#[tokio::test]
async fn create_post_without_csrf_is_forbidden() {
    let db = seeded_db().await;
    let router = router(db);
    // A logged-in client presenting no CSRF cookie or field: the auth gate
    // passes, the double-submit check must still 403.
    let (_, login) = login_next(
        &router,
        showcase::models::DEMO_ADMIN_EMAIL,
        showcase::models::DEMO_ADMIN_PASSWORD,
        "",
    )
    .await;
    let session = session_cookie_value(&login).expect("session cookie");
    let client = TestClient::new(&router).cookie(SESSION_COOKIE, &session);
    let response = client
        .post_form(
            "/admin/users/create",
            "name=NoToken&email=notoken%40example.com".to_string(),
        )
        .await;
    assert_eq!(response.status(), 403, "missing CSRF must be 403");
}

#[tokio::test]
async fn create_post_with_unknown_keys_is_bad_request() {
    // GH #89 allow-list: role/tenant_id smuggling is a 400 at the framework
    // layer, never silently ignored.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client.csrf(&csrf).post_form("/admin/users/create", format!(
            "name=Sneaky&email=sneaky%40example.com&role=admin&tenant_id=victim&csrf_token={csrf}"
        ))
    .await;
    assert_eq!(
        resp.status(),
        400,
        "unknown POST keys must be 400, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let count = User::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 8, "smuggled POST must not create");
}
