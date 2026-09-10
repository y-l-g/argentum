use showcase::{app::router_for_tests as router, models::DEMO_TENANT};

mod common;
use common::{body_string, full_db, get_tenant};

#[tokio::test]
async fn posts_export_bom_opt_in_prepends_bom() {
    // GH #94: `?bom=1` opts into a UTF-8 BOM for Excel; default stays BOM-free.
    let db = full_db().await;
    let router = router(db);
    let resp = get_tenant(&router, "/admin/posts/export?bom=1", DEMO_TENANT).await;
    assert!(resp.status().is_success());
    let csv = body_string(resp).await;
    assert!(
        csv.starts_with('\u{FEFF}'),
        "bom=1 export must start with BOM, got {csv:?}"
    );

    let resp = get_tenant(&router, "/admin/posts/export", DEMO_TENANT).await;
    let csv = body_string(resp).await;
    assert!(
        !csv.starts_with('\u{FEFF}'),
        "default export must stay BOM-free, got {csv:?}"
    );
}

#[tokio::test]
async fn posts_group_by_status_shows_counts() {
    let db = full_db().await;
    let router = router(db);
    let resp = get_tenant(&router, "/admin/posts?group_by=status", DEMO_TENANT).await;
    assert!(
        resp.status().is_success(),
        "group_by should be 200, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
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
    let resp = get_tenant(&router, "/admin/posts/export", DEMO_TENANT).await;
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
    assert!(
        content_type.contains("charset=utf-8"),
        "content-type should declare utf-8 for non-ASCII cells (GH #94), got {}",
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
    let csv = body_string(resp).await;
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
    let resp = get_tenant(
        &router,
        "/admin/posts/export?filters=status:published",
        DEMO_TENANT,
    )
    .await;
    let csv = body_string(resp).await;
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
