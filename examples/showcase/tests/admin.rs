use http_body_util::BodyExt;
use toasty::Db;
use topcoat::router::Body;

use showcase::{
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

#[tokio::test]
async fn showcase_index_lists_features() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "showcase index status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Showcase"),
        "missing Showcase heading in {html}"
    );
    for path in [
        "/admin/showcase/schema",
        "/admin/showcase/resource",
        "/admin/showcase/panel",
        "/admin/showcase/table",
        "/admin/showcase/db",
    ] {
        assert!(html.contains(path), "missing link {path} in {html}");
    }
}

#[tokio::test]
async fn showcase_schema_renders_variants() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/schema")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "schema showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // Snippets + rendered classes
    assert!(html.contains("Text::new"), "missing Text snippet in {html}");
    assert!(html.contains("ac-text"), "missing ac-text in {html}");
    assert!(html.contains("ac-section"), "missing ac-section in {html}");
    assert!(html.contains("ac-group"), "missing ac-group in {html}");
    assert!(html.contains("ac-grid"), "missing ac-grid in {html}");
    assert!(
        html.contains("ac-grid-cols-2"),
        "missing grid cols in {html}"
    );
    // TextInput field
    assert!(
        html.contains("TextInput::for"),
        "missing TextInput snippet in {html}"
    );
    assert!(html.contains("ac-field"), "missing ac-field in {html}");
    assert!(html.contains("<input"), "missing input in {html}");
    // Composition and empty
    assert!(
        html.contains("Schema::empty"),
        "missing empty snippet in {html}"
    );
}

#[tokio::test]
async fn showcase_resource_renders_derives_and_navigation() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "resource showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("BareUserResource"),
        "missing Bare snippet in {html}"
    );
    assert!(
        html.contains("only_ada"),
        "missing only_ada snippet in {html}"
    );
    assert!(
        html.contains("Bare query rows: 2"),
        "missing all count in {html}"
    );
    assert!(
        html.contains("Scoped query rows: 1"),
        "missing scoped count in {html}"
    );
    assert!(
        html.contains("Ada Lovelace"),
        "missing scoped user in {html}"
    );
    assert!(html.contains("Users"), "missing navigation label in {html}");
}

#[tokio::test]
async fn showcase_panel_renders_normalization() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/panel")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "panel showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Panel::new"),
        "missing Panel snippet in {html}"
    );
    assert!(
        html.contains("/admin") && html.contains("/showcase"),
        "missing prefix variants in {html}"
    );
}

#[tokio::test]
async fn showcase_db_renders_memoized_loader() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/db")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "db showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("db(cx)"), "missing db snippet in {html}");
    assert!(
        html.contains("#[memoize"),
        "missing memoize snippet in {html}"
    );
    assert!(
        html.contains("Ada Lovelace") || html.contains("Grace Hopper"),
        "missing user rows in {html}"
    );
}

#[tokio::test]
async fn showcase_table_renders_variants() {
    let db = seeded_db().await;
    let router = router(db);
    let response = router
        .handle(
            http::Request::builder()
                .uri("/admin/showcase/table")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        response.status().is_success(),
        "table showcase status {}",
        response.status()
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("TextColumn::for"),
        "missing TextColumn snippet in {html}"
    );
    assert!(html.contains("ac-table"), "missing ac-table in {html}");
    assert!(
        html.contains("ac-column--searchable"),
        "missing ac-column--searchable in {html}"
    );
    assert!(
        html.contains("ac-column--sortable"),
        "missing ac-column--sortable in {html}"
    );
    assert!(
        html.contains("Ada Lovelace") || html.contains("ac-column"),
        "missing table rows in {html}"
    );
}
