use http_body_util::BodyExt;
use toasty::Db;
use topcoat::router::Body;

use admin::{
    app::router,
    models::{User, seed},
};

async fn seeded_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(User))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    db.push_schema().await.expect("push_schema");
    seed(&mut db).await.expect("seed");
    db
}

#[tokio::test]
async fn admin_layout_and_list_page_serve_seeded_users() {
    let db = seeded_db().await;
    let router = router(db);

    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert!(
        response.status().is_success(),
        "status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);

    // Layout shell
    assert!(
        html.contains("ac-admin") || html.contains("ac-sidebar"),
        "missing admin layout in {html}"
    );
    // NavigationItem derived from UserResource
    assert!(html.contains("Users"), "missing navigation label in {html}");
    assert!(
        html.contains("href=\"/admin\"") || html.contains("/admin"),
        "missing navigation url in {html}"
    );
    // List page content
    assert!(
        html.contains("<h1>Users</h1>") || html.contains("Users"),
        "missing heading in {html}"
    );
    for (name, email) in [
        ("Ada Lovelace", "ada@example.com"),
        ("Grace Hopper", "grace@example.com"),
    ] {
        assert!(html.contains(name), "missing {name} in {html}");
        assert!(html.contains(email), "missing {email} in {html}");
    }
}

#[tokio::test]
async fn admin_unknown_route_is_not_found() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), 404);
}
