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
async fn delete_requires_confirmation_and_deletes() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap();
    let id = user.id.to_string();
    let delete_url = format!("/admin/users/{}/delete", id);

    // Check that list page contains Delete button
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Delete"),
        "list should contain Delete button, got {}",
        html
    );
    assert!(
        html.contains(&format!("/admin/users/{}/delete", id)),
        "Delete form action should contain id"
    );

    // POST without confirm should re-render confirmation (200 with Confirm)
    let resp = router
        .handle(
            Request::builder()
                .uri(delete_url.clone())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(""))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "POST without confirm should be 200 confirmation, got {}",
        resp.status()
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Confirm") || html.contains("Are you sure"),
        "confirmation page should have Confirm, got {}",
        html
    );

    // POST with confirm should delete and redirect with notification
    let resp = router
        .handle(
            Request::builder()
                .uri(delete_url.clone())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("confirm=1"))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "confirmed delete should redirect, got {}",
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

    // Check DB: user should be gone
    let mut db_check = db.clone();
    let count = User::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 2, "should have 2 after delete, got {}", count);
    let gone = User::filter(User::fields().id().eq(user.id))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap();
    assert!(gone.is_none(), "deleted user should be gone");

    // Follow redirect and check notification
    let resp2 = router
        .handle(Request::builder().uri(loc).body(Body::empty()).unwrap())
        .await;
    let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let html2 = String::from_utf8_lossy(&body2);
    assert!(
        html2.contains("fixed top-4 right-4"),
        "notification should survive, got {}",
        html2
    );
}

#[tokio::test]
async fn delete_404_for_missing_or_wrong_tenant() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let fake_id = uuid::Uuid::new_v4().to_string();
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/users/{}/delete", fake_id))
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("confirm=1"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "unknown id should be 404, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn delete_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};
    use http::{Method, Request, header::CONTENT_TYPE};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct DenyDeleteResource;
    impl Resource for DenyDeleteResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            true
        }
        fn can_delete(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
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
            Schema::new(TextInput::r#for(DummyUser::fields().name()))
        }
    }

    let mut db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let rec = toasty::create!(DummyUser {
        name: "x".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .resource::<DenyDeleteResource>()
        .build();
    let slug = DenyDeleteResource::slug();
    let delete_url = format!("/admin/{}/{}/delete", slug, rec.id);
    let resp = router
        .handle(
            Request::builder()
                .uri(delete_url)
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("confirm=1"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "delete should be 403 when denied, got {}",
        resp.status()
    );
    // Check not deleted
    let mut db_check = db.clone();
    let count = DummyUser::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 1, "should not delete when denied");
}
