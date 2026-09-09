use http::Request;
use http_body_util::BodyExt;
use showcase::{
    app::router_for_tests as router,
    models::{seed, seed_phase2},
};
use toasty::Db;
use topcoat::router::Body;

async fn full_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    seed(&mut db).await.unwrap();
    seed_phase2(&mut db).await.unwrap();
    db
}

#[tokio::test]
async fn posts_filter_widgets_render_typed_controls() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=status:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // One typed control per declared filter, composed by filters.js into the
    // single filters param (free-text input stays as the fallback).
    for name in ["status", "featured", "created_at"] {
        assert!(
            html.contains(&format!("data-filter-name=\"{name}\"")),
            "missing control for {name} in {html}",
            name = name,
            html = html
        );
    }
    assert!(
        html.contains("data-filters-form"),
        "missing filters form in {}",
        html
    );
    // Select options, ternary values, date input, current selection.
    assert!(
        html.contains("draft") && html.contains("published"),
        "missing status options in {}",
        html
    );
    assert!(
        html.contains("value=\"true\"") && html.contains("value=\"false\""),
        "missing ternary options in {}",
        html
    );
    assert!(
        html.contains("type=\"date\""),
        "missing date control in {}",
        html
    );
    assert!(
        html.contains("name=\"filters\"") && html.contains("status:published"),
        "missing free-text fallback in {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_select_status_published() {
    let db = full_db().await;
    let router = router(db);
    // filter status:published should show only Hello Toasty (published)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=status:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Hello Toasty"),
        "should contain published {}",
        html
    );
    assert!(
        !html.contains("Second Post"),
        "should not contain draft {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_ternary_featured_true() {
    let db = full_db().await;
    let router = router(db);
    // featured:true should show only Hello Toasty (featured true)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=featured:true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Hello Toasty"),
        "featured true should show Hello {}",
        html
    );
    assert!(
        !html.contains("Second Post"),
        "featured true should not show Second {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_ternary_featured_false() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=featured:false")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        !html.contains("Hello Toasty"),
        "featured false should not show Hello {}",
        html
    );
    assert!(
        html.contains("Second Post"),
        "featured false should show Second {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_date_created_at() {
    let db = full_db().await;
    let router = router(db);
    // filter by exact timestamp of Hello Toasty
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=created_at:2024-01-15T09:30:00Z")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Hello Toasty"),
        "date filter should show Hello {}",
        html
    );
    assert!(
        !html.contains("Second Post"),
        "date filter should not show Second {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_composes_and() {
    let db = full_db().await;
    let router = router(db);
    // status:published and featured:true should still show Hello Toasty (both true)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=status:published,featured:true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Hello Toasty"),
        "and filter should show Hello {}",
        html
    );
    // status:draft and featured:true should show none (draft is not featured)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=status:draft,featured:true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        !html.contains("Hello Toasty") && !html.contains("Second Post"),
        "and filter should show none {}",
        html
    );
    assert!(
        html.contains("No records") || html.contains("No results"),
        "should show empty {}",
        html
    );
}

#[tokio::test]
async fn table_state_parses_filters_and_filter_expr() {
    use argentum_core::{DateFilter, Filter, SelectFilter, TableState, TernaryFilter};
    use showcase::models::Post;
    use topcoat::context::CxTestBuilder;

    // Test parsing
    let (parts, ()) = http::Request::builder()
        .uri("/admin/posts?filters=status:published,featured:true")
        .body(())
        .unwrap()
        .into_parts();
    let cx = CxTestBuilder::new().request_context(parts).build();
    let state = TableState::from_cx(&cx);
    assert_eq!(
        state.filters.get("status").map(|s| s.as_str()),
        Some("published")
    );
    assert_eq!(
        state.filters.get("featured").map(|s| s.as_str()),
        Some("true")
    );

    // Test SelectFilter expr
    let f = SelectFilter::r#for(
        Post::fields().status(),
        vec!["draft".into(), "published".into()],
    );
    assert!(f.to_expr("published").is_some());
    assert!(f.to_expr("").is_none());
    assert!(f.to_expr("unknown").is_none());

    // Ternary
    let tf = TernaryFilter::r#for(Post::fields().featured());
    assert!(tf.to_expr("true").is_some());
    assert!(tf.to_expr("false").is_some());
    assert!(tf.to_expr("").is_none());
    assert!(tf.to_expr("all").is_none());

    // Date
    let df = DateFilter::r#for(Post::fields().created_at());
    assert!(df.to_expr("2024-01-15T09:30:00Z").is_some());
    assert!(df.to_expr("").is_none());
    assert!(df.to_expr("not-a-date").is_none());

    // Filter enum
    let filter: Filter<Post> = f.into();
    assert_eq!(filter.name(), "status");
    assert!(filter.to_expr("published").is_some());
}

#[tokio::test]
async fn typo_filter_warns_on_list_but_refuses_export() {
    // GH #93: unknown/typo'd filters warn visibly on the list (200) and fail
    // closed on export (400) instead of silently over-sharing.
    let db = full_db().await;
    let router = router(db);

    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?filters=stauts:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success(), "typo filter keeps 200");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("role=\"alert\"") && html.contains("stauts:published"),
        "typo filter must warn, got {html}"
    );

    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/export?filters=stauts:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        400,
        "typo'd export must refuse, got {}",
        resp.status()
    );

    // Rejected values (capital P) behave the same.
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/export?filters=status:Published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        400,
        "rejected-value export must refuse, got {}",
        resp.status()
    );

    // Valid filters still export fine.
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/export?filters=status:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success(), "valid export must stay 200");
}
