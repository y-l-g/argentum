use showcase::app::router_for_tests as router;

use crate::common::{body_string, demo_client, find_href_with, full_db};

#[tokio::test]
async fn posts_filter_widgets_render_typed_controls() {
    // GH #136 layer rule: core (`filter_widgets_render_typed_controls`)
    // owns the typed-control detail (select/ternary/date options, selected
    // state, noscript fallback); this pins the HTTP wiring — the widgets
    // arrive with the active value in the hidden transport.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts?filters=status:published").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    // One typed control per declared filter, composed by filters.js into the
    // hidden `filters` transport (the text fallback lives in `<noscript>`).
    for name in ["status", "featured", "created_at", "spotlight"] {
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
    assert!(
        html.contains("data-filters-transport")
            && html.contains("name=\"filters\"")
            && html.contains("status:published"),
        "missing hidden filters transport in {}",
        html
    );
}

#[tokio::test]
async fn posts_filter_select_status_published() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // filter status:published should show only Hello Toasty (published)
    let resp = client.get("/admin/posts?filters=status:published").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // featured:true should show only Hello Toasty (featured true)
    let resp = client.get("/admin/posts?filters=featured:true").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // Search rather than read the default page: the seed carries a pagination
    // fixture (GH #184), so with 60-odd non-featured posts the title-ordered
    // first page no longer reaches "Second Post". The search narrows to the
    // row under test, which is what this assertion is about.
    let resp = client
        .get("/admin/posts?filters=featured:false&q=Second")
        .await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // filter by exact timestamp of Hello Toasty
    let resp = client
        .get("/admin/posts?filters=created_at:2024-01-15T09:30:00Z")
        .await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // status:published and featured:true should still show Hello Toasty (both true)
    let resp = client
        .get("/admin/posts?filters=status:published,featured:true")
        .await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("Hello Toasty"),
        "and filter should show Hello {}",
        html
    );
    // status:draft and featured:true should show none (draft is not featured)
    let resp = client
        .get("/admin/posts?filters=status:draft,featured:true")
        .await;
    let html = body_string(resp).await;
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
async fn typo_filter_warns_on_list_but_refuses_export() {
    // GH #93: unknown/typo'd filters warn visibly on the list (200) and fail
    // closed on export (400) instead of silently over-sharing.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get("/admin/posts?filters=stauts:published").await;
    assert!(resp.status().is_success(), "typo filter keeps 200");
    let html = body_string(resp).await;
    assert!(
        html.contains("role=\"alert\"") && html.contains("stauts:published"),
        "typo filter must warn, got {html}"
    );

    // Colon-less segments are malformed, not silently dropped (GH #148): the
    // list banners them, export refuses with 400.
    let resp = client.get("/admin/posts?filters=foobar").await;
    assert!(resp.status().is_success(), "malformed filter keeps 200");
    let html = body_string(resp).await;
    assert!(
        html.contains("role=\"alert\"") && html.contains("foobar"),
        "malformed filter must banner, got {html}"
    );
    let resp = client.get("/admin/posts/export?filters=foobar").await;
    assert_eq!(
        resp.status(),
        400,
        "malformed export must refuse, got {}",
        resp.status()
    );

    let resp = client
        .get("/admin/posts/export?filters=stauts:published")
        .await;
    assert_eq!(
        resp.status(),
        400,
        "typo'd export must refuse, got {}",
        resp.status()
    );

    // Rejected values (capital P) behave the same.
    let resp = client
        .get("/admin/posts/export?filters=status:Published")
        .await;
    assert_eq!(
        resp.status(),
        400,
        "rejected-value export must refuse, got {}",
        resp.status()
    );

    // Valid filters still export fine.
    let resp = client
        .get("/admin/posts/export?filters=status:published")
        .await;
    assert!(resp.status().is_success(), "valid export must stay 200");
}

#[tokio::test]
async fn posts_list_renders_live_search_host() {
    // GH #104: posts table opts into the live shard (tenant header required).
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("data-live-search"),
        "posts list must render the live host, got {html}"
    );
}

#[tokio::test]
async fn posts_filter_with_cursor_paginates_filtered_rows() {
    // GH #136 extension: `admin.rs` walked cursors unfiltered and
    // `filter_check.rs` asserted filtered lists without following
    // `after=`/`before=` — no `?filters=` + cursor test existed.
    use showcase::models::{Author, Post};

    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    // The posts table paginates by 25: seed 25 more published rows so the
    // `status:published` result spans two pages (Hello + 25 new).
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let author_id = authors[0].id;
    let tenant = authors[0].tenant_id;
    for i in 0..25 {
        let title = format!("Published {:02}", i);
        toasty::create!(Post {
            tenant_id: tenant,
            title: title,
            body: "extra",
            status: "published".to_string(),
            featured: false,
            created_at: "2024-02-01T00:00:00Z".parse::<jiff::Timestamp>().unwrap(),
            image_path: "/images/extra.jpg".to_string(),
            tags: "extra".to_string(),
            seo: showcase::models::Seo {
                title: "Extra".to_string(),
                description: String::new(),
            },
            publication: showcase::models::Publication::Published {
                published_at: "2024-02-01T00:00:00Z".to_string(),
                canonical_url: String::new(),
            },
            media: showcase::models::Media::Image {
                url: "extra.jpg".to_string(),
                alt: String::new(),
            },
            post_stats: showcase::models::PostStats {
                word_count: 0,
                read_minutes: 0,
            },
            author_id: author_id,
        })
        .exec(&mut db_q)
        .await
        .unwrap();
    }
    let resp = client.get("/admin/posts?filters=status:published").await;
    assert!(resp.status().is_success());
    let page1 = body_string(resp).await;
    assert!(
        !page1.contains("Second Post"),
        "filtered page 1 must not show drafts: {page1}"
    );
    let next = find_href_with(&page1, "after=").expect("filtered page 1 needs a Next link");
    assert!(
        next.contains("filters="),
        "the pager must preserve filters, got {next}"
    );
    // GH #217: this href is fed straight back as a request URI, so it must be
    // the decoded URL a browser would send.
    assert!(
        !next.contains("&amp;"),
        "the Next link must be followed decoded, got {next}"
    );
    let resp = client.get(&next).await;
    assert!(resp.status().is_success());
    let page2 = body_string(resp).await;
    assert!(
        !page2.contains("Second Post"),
        "filtered page 2 must not show drafts: {page2}"
    );
    let prev = find_href_with(&page2, "before=").expect("filtered page 2 needs a Previous link");
    assert!(
        prev.contains("filters="),
        "the Previous link must preserve filters, got {prev}"
    );
    assert!(
        !prev.contains("&amp;"),
        "the Previous link must be followed decoded, got {prev}"
    );
    let resp = client.get(&prev).await;
    assert!(resp.status().is_success());
    let back = body_string(resp).await;
    assert!(
        !back.contains("Second Post"),
        "walking back must stay filtered: {back}"
    );
}

#[tokio::test]
async fn posts_filter_variant_spotlight_splits_featured() {
    // The fourth filter kind: prebuilt-expression VariantFilter over the
    // featured flag, no embedded enum required.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    // Both halves narrow by search so the row under test is on the page
    // regardless of where the pagination fixture (GH #184) puts it.
    let resp = client
        .get("/admin/posts?filters=spotlight:Featured&q=Toasty")
        .await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("Hello Toasty"),
        "Featured must show the spotlight post: {html}"
    );
    assert!(
        !html.contains("Second Post"),
        "Featured must hide regular posts: {html}"
    );

    let resp = client
        .get("/admin/posts?filters=spotlight:Standard&q=Second")
        .await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("Second Post"),
        "Standard must show non-spotlight posts: {html}"
    );
    assert!(
        !html.contains("Hello Toasty"),
        "Standard must hide the spotlight post: {html}"
    );
}
