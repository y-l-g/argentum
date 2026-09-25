use showcase::app::router_for_tests as router;

use crate::common::{body_string, demo_client, full_db};

#[tokio::test]
async fn posts_export_bom_opt_in_prepends_bom() {
    // GH #94: `?bom=1` opts into a UTF-8 BOM for Excel; default stays BOM-free.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/export?bom=1").await;
    assert!(resp.status().is_success());
    let csv = body_string(resp).await;
    assert!(
        csv.starts_with('\u{FEFF}'),
        "bom=1 export must start with BOM, got {csv:?}"
    );

    let resp = client.get("/admin/posts/export").await;
    let csv = body_string(resp).await;
    assert!(
        !csv.starts_with('\u{FEFF}'),
        "default export must stay BOM-free, got {csv:?}"
    );
}

#[tokio::test]
async fn posts_group_by_status_shows_counts() {
    let db = full_db().await;
    let mut db_q = db.clone();
    // Derived, not literal: the page-local count is the number of
    // published rows in the fixture, so one more seeded post cannot break it.
    let published = showcase::models::Post::filter(
        showcase::models::Post::fields()
            .status()
            .eq("published".to_string()),
    )
    .exec(&mut db_q)
    .await
    .unwrap()
    .len();
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts?group_by=status").await;
    assert!(
        resp.status().is_success(),
        "group_by should be 200, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
    // The header label *and* its page-local count. The bare label is not
    // asserted separately: the status SelectFilter renders "published" and
    // "draft" as options on every list page, so a label-only check
    // passes with grouping off. `on this page` is emitted only by a group
    // header (`render.rs`), and core pins the ordering and exact
    // "draft (2 on this page)" labels in
    // `group_by_orders_each_row_under_its_own_header`.
    assert!(
        html.contains(&format!("published ({published} on this page)")),
        "missing the published group header in {html}"
    );
}

#[tokio::test]
async fn posts_export_streams_csv_with_content_disposition() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/export").await;
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
    // Hardening headers: bodies must never be sniffed as HTML.
    assert_eq!(
        resp.headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "export must carry nosniff"
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
    // Every relation-reading column declares the include its projection reads
    // so the export's narrowed query loads them all and no cell
    // falls back to the unloaded marker (the columns' `debug_assert` is the
    // other half of that contract — it panics first).
    assert!(
        !csv.contains("(unloaded)"),
        "a declared include must reach the export query, got {}",
        csv
    );
    // A filter narrows the export: only the published post survives.
    let resp = client
        .get("/admin/posts/export?filters=status:published")
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

#[tokio::test]
async fn export_over_cap_413s_at_route_level() {
    // GH #136 §4: the 413 mapping is unit-tested (`export_cap_maps_one_row…`);
    // this pins the route wiring — a table past the cap answers 413.
    use tablo_core::{Resource, Schema, Table, TextColumn, TextInput};
    use toasty::Db;

    use crate::common::TestClient;

    #[derive(Debug, toasty::Model, Clone)]
    struct Dummy {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct BigResource;
    impl Resource for BigResource {
        type Model = Dummy;
        fn slug() -> String {
            "dummies".to_string()
        }
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _record: &Dummy) -> bool {
            true
        }
        fn table(cx: &topcoat::context::Cx) -> Table<Dummy> {
            Table::r#for(cx)
                .id(|d: &Dummy| d.id.to_string())
                .pk(|d: &Dummy| d.id.to_string())
                .columns(TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                    d.name.clone()
                }))
        }
        fn form(_cx: &topcoat::context::Cx) -> Schema {
            Schema::new(TextInput::r#for(Dummy::fields().name()))
        }
    }

    let mut db = Db::builder()
        .models(toasty::models!(Dummy))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    // One row past the 10_000 cap, in one batched insert: a
    // `toasty::create!` per row spent ~2s going through the engine pipeline
    // 10,001 times, which was more than the route under test.
    let mut create = Dummy::create_many();
    for i in 0..10_001 {
        create = create.item(Dummy::create().name(format!("row-{i:05}")));
    }
    create.exec(&mut db).await.unwrap();
    let router = tablo_core::Panel::new("admin")
        .app_context(db)
        .auth(tablo_core::Auth::disabled())
        .resource::<BigResource>()
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);
    let resp = client.get("/admin/dummies/export").await;
    assert_eq!(
        resp.status(),
        413,
        "an over-cap export must be 413, got {}",
        resp.status()
    );
}
