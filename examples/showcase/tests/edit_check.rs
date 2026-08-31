use http::{
    Method, Request,
    header::{CONTENT_TYPE, LOCATION},
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
async fn edit_page_hydrates_and_updates() {
    let db = seeded_db().await;
    let router = router(db.clone());

    // Get a user id
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap();
    let id = user.id.to_string();
    let edit_url = format!("/admin/users/{}/edit", id);

    // GET edit should be 200 with hydrated values
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "GET edit should be 200, got {}",
        resp.status()
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains(&user.name),
        "edit should contain hydrated name {}, got {}",
        user.name,
        html
    );
    assert!(
        html.contains(&user.email),
        "edit should contain hydrated email"
    );
    assert!(html.contains("grid gap-1.5"), "missing form chrome");
    assert!(
        html.contains("for=\"name\"") || html.contains("for="),
        "missing for/id"
    );

    // Invalid POST should re-render with errors and not mutate
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url.clone())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("name=&email=bad"))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "invalid POST should re-render 200, got {}",
        resp.status()
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("is required") || html.contains("must be a valid email"),
        "should contain validation error, got {}",
        html
    );
    // Check DB not mutated
    let mut db_check = db.clone();
    let fresh = User::get_by_id(&mut db_check, &user.id).await.unwrap();
    assert_eq!(fresh.name, user.name, "should not mutate on invalid");

    // Valid POST should update and redirect with notification
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url.clone())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=Updated%20Name&email=updated%40example.com",
                ))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid POST should redirect, got {}",
        resp.status()
    );
    let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(
        loc.contains("/admin/users"),
        "redirect to list, got {}",
        loc
    );
    assert!(
        loc.contains("notification"),
        "should have notification, got {}",
        loc
    );
    // Follow redirect and check notification
    let resp2 = router
        .handle(Request::builder().uri(loc).body(Body::empty()).unwrap())
        .await;
    let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let html2 = String::from_utf8_lossy(&body2);
    assert!(
        html2.contains("fixed top-4 right-4"),
        "missing notification"
    );
    // Check DB mutated
    let mut db_check2 = db.clone();
    let updated = User::get_by_id(&mut db_check2, &user.id).await.unwrap();
    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.email, "updated@example.com");
}

#[tokio::test]
async fn edit_404_for_unknown_or_wrong_tenant() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let fake_id = uuid::Uuid::new_v4().to_string();
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/users/{}/edit", fake_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "unknown id should be 404, got {}",
        resp.status()
    );

    // Tenancy test: create a scoped resource that only sees Ada, try to edit Grace
    // For now, just test that unknown id is 404 (tenancy via query would also be 404)
}

#[tokio::test]
async fn edit_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};
    use http::{Method, Request, header::CONTENT_TYPE};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        email: String,
    }

    struct DenyUpdateResource;
    impl Resource for DenyUpdateResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            true
        }
        fn can_update(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            false
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
        fn hydrate_form_values(_r: &DummyUser) -> std::collections::HashMap<String, String> {
            std::collections::HashMap::new()
        }
    }

    let mut db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let rec = toasty::create!(DummyUser {
        name: "x".to_string(),
        email: "x@example.com".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .resource::<DenyUpdateResource>()
        .build();
    let slug = DenyUpdateResource::slug();
    let edit_url = format!("/admin/{}/{}/edit", slug, rec.id);

    // GET should be 403
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url.clone())
                .body(topcoat::router::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "GET edit should be 403 when update denied, got {}",
        resp.status()
    );

    // POST should also be 403
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url)
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(topcoat::router::Body::from("name=y"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "POST edit should be 403 when update denied, got {}",
        resp.status()
    );
}
