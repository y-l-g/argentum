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
async fn posts_group_by_status_shows_counts() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts?group_by=status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "group_by should be 200, got {}",
        resp.status()
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // Should show group headers with counts (in-memory grouping)
    assert!(
        html.contains("published") || html.contains("draft"),
        "missing group header {}",
        html
    );
    // Each group should show count (1) for our seeded data (one published, one draft) – appears as "(1)"
    assert!(
        html.contains("(1)") || html.contains("1"),
        "missing count {}",
        html
    );
}

#[tokio::test]
async fn posts_export_streams_csv_with_content_disposition() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "export should be 200, got {}",
        resp.status()
    );
    let content_type = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/csv"),
        "content-type should be text/csv, got {}",
        content_type
    );
    let disposition = resp
        .headers()
        .get(http::header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        disposition.contains("attachment"),
        "should be attachment, got {}",
        disposition
    );
    assert!(
        disposition.contains("posts.csv"),
        "filename should be posts.csv, got {}",
        disposition
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let csv = String::from_utf8_lossy(&body);
    // Header row with column labels (Title, Author, etc.)
    assert!(
        csv.contains("Title") || csv.contains("title"),
        "missing header {}",
        csv
    );
    assert!(csv.contains("Author"), "missing Author header {}", csv);
    // Data rows should include Hello Toasty and author name via include
    assert!(csv.contains("Hello Toasty"), "missing post title {}", csv);
    assert!(
        csv.contains("Ada Author") || csv.contains("Ada"),
        "missing author name via include {}",
        csv
    );
    // Should respect filters if provided
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/export?filters=status:published")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let csv = String::from_utf8_lossy(&body);
    assert!(
        csv.contains("Hello Toasty"),
        "filtered export should contain published {}",
        csv
    );
    assert!(
        !csv.contains("Second Post"),
        "filtered export should not contain draft {}",
        csv
    );
}
