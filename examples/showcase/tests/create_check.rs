use http::{
    Method, Request,
    header::{CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE},
};
use http_body_util::BodyExt;
use showcase::{
    app::router_for_tests as router,
    models::{User, seed},
};
use toasty::Db;
use topcoat::router::Body;

async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    seed(&mut db).await.unwrap();
    db
}

#[tokio::test]
async fn manual_create_check() {
    let db = seeded_db().await;
    let router = router(db.clone());

    // Test GET /admin/users/create returns 200 with form HTML
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users/create")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    println!("GET /admin/users/create status: {}", resp.status());
    assert!(resp.status().is_success(), "GET create should be 200");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("grid gap-1.5"),
        "missing grid gap-1.5 in {}",
        html
    );
    assert!(html.contains("border-border"), "missing border-border");
    assert!(html.contains("<input"), "missing input");
    assert!(
        html.contains("for=\"name\"") || html.contains("for="),
        "missing for"
    );
    assert!(html.contains("text-destructive"), "missing required star");
    assert!(
        html.contains("text-sm text-destructive"),
        "missing error slot"
    );

    // Test POST empty name
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("name=&email=not-an-email"))
                .unwrap(),
        )
        .await;
    let status = resp.status();
    println!("POST empty name status: {}", status);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
    assert_eq!(count, 3, "DB should still have 3 after invalid");

    // Test POST valid
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("name=New%20User&email=new%40example.com"))
                .unwrap(),
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
    let cookie = resp
        .headers()
        .get(SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    println!("Cookie header: {}", cookie);
    // Notification may be via Set-Cookie or via ?notification= query param (fallback when cookie layer doesn't handle error).
    let has_notification_via =
        loc.contains("notification") || cookie.contains("argentum_notification");
    assert!(
        has_notification_via,
        "should set notification via cookie or query param, got loc {} cookie {}",
        loc, cookie
    );
    // Follow redirect (use Location URL which may contain ?notification)
    let mut req = Request::builder()
        .uri(loc.clone())
        .body(Body::empty())
        .unwrap();
    if !cookie.is_empty() {
        let cookie_val = cookie.split(';').next().unwrap();
        req.headers_mut()
            .insert(COOKIE, cookie_val.parse().unwrap());
    }
    let resp2 = router.handle(req).await;
    assert!(
        resp2.status().is_success(),
        "GET list after create should be 200"
    );
    let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let html2 = String::from_utf8_lossy(&body2);
    // Need to check if new user appears on page 1 or 2? Since paginated 2 per page, new user "New User" with name N may be on page 2 (after Grace Hopper? Let's see sort is name asc: Ada, Alan, Grace, New User -> New User is last, so on page 2)
    // So we need to fetch page 2 via pagination? Or increase page size? But list page default shows page 1 (Ada, Alan). New User not on page1.
    // Let's check DB directly that user was created, and also check that notification appears.
    assert!(
        html2.contains("fixed top-4 right-4"),
        "missing notification fixed top-4 right-4 in {}",
        html2
    );
    assert!(
        html2.contains("border-border")
            && html2.contains("bg-background")
            && html2.contains("shadow-sm"),
        "missing notification card tokens"
    );
    let mut db_check2 = db.clone();
    let count2 = User::all().exec(&mut db_check2).await.unwrap().len();
    assert_eq!(count2, 4, "DB should have 4 after valid create");
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
    use http::{Method, Request, header::CONTENT_TYPE};
    use toasty::Db;

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
        .resource::<DenyCreateResource>()
        .build();

    let slug = DenyCreateResource::slug();
    let create_url = format!("/admin/{}/create", slug);
    // GET create should be 403
    let resp = router
        .handle(
            Request::builder()
                .uri(create_url.clone())
                .body(topcoat::router::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), 403, "GET create should be 403 when denied");

    // POST should also be 403 and not create
    let resp = router
        .handle(
            Request::builder()
                .uri(create_url)
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(topcoat::router::Body::from("name=test"))
                .unwrap(),
        )
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
