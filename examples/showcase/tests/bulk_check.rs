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
use topcoat::view::ViewExt;

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
async fn bulk_delete_deletes_selected() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    assert_eq!(users.len(), 3);
    let ids: Vec<String> = users.iter().take(2).map(|u| u.id.to_string()).collect();
    let ids_param = ids.join(",");

    // Check that list page contains Bulk Delete
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
        html.contains("Bulk Delete"),
        "list should contain Bulk Delete, got {}",
        html
    );
    assert!(
        html.contains("data-boundary=\"table\""),
        "Table should be a Boundary, got {}",
        html
    );

    // Bulk delete
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users/bulk-delete")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("ids={}", ids_param)))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "bulk delete should redirect, got {}",
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

    // Check DB: should have 1 left
    let mut db_check = db.clone();
    let remaining = User::all().exec(&mut db_check).await.unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "should have 1 after bulk delete 2, got {}",
        remaining.len()
    );
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
async fn bulk_delete_partial_deny_aborts() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct PartialDenyResource;
    impl Resource for PartialDenyResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_delete(_cx: &topcoat::context::Cx, rec: &DummyUser) -> bool {
            // Deny second record (name == "b")
            rec.name != "b"
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
    let a = toasty::create!(DummyUser {
        name: "a".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let b = toasty::create!(DummyUser {
        name: "b".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .resource::<PartialDenyResource>()
        .build();
    let slug = PartialDenyResource::slug();
    let ids = format!("{},{}", a.id, b.id);
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/{}/bulk-delete", slug))
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("ids={}", ids)))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "partial deny should be 403, got {}",
        resp.status()
    );
    // Check no deletions happened
    let mut db_check = db.clone();
    let remaining = DummyUser::all().exec(&mut db_check).await.unwrap();
    assert_eq!(
        remaining.len(),
        2,
        "should have 2, no deletions, got {}",
        remaining.len()
    );
}

#[tokio::test]
async fn view_any_deny_blocks_list() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct DenyViewAnyResource;
    impl Resource for DenyViewAnyResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
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

    let db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .resource::<DenyViewAnyResource>()
        .build();
    let slug = DenyViewAnyResource::slug();
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/{}", slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "viewAny deny should be 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn table_boundary_and_memoize() {
    use argentum_core::{Table, TextColumn};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use topcoat::context::{CxTestBuilder, memoize};

    #[derive(Debug, Clone, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    // Test Table is a Boundary by default
    let cx = CxTestBuilder::new().build();
    let table = Table::<DummyUser>::r#for(&cx)
        .id(|u: &DummyUser| u.id.to_string())
        .columns(TextColumn::r#for(
            DummyUser::fields().name(),
            |u: &DummyUser| u.name.clone(),
        ));
    assert!(table.is_boundary(), "Table should be a Boundary by default");
    assert!(!table.is_defer(), "Table should not defer by default");
    let table2 = Table::<DummyUser>::r#for(&cx)
        .id(|u: &DummyUser| u.id.to_string())
        .columns(TextColumn::r#for(
            DummyUser::fields().name(),
            |u: &DummyUser| u.name.clone(),
        ))
        .boundary(false);
    assert!(!table2.is_boundary(), "boundary(false) should disable");
    let table3 = Table::<DummyUser>::r#for(&cx)
        .id(|u: &DummyUser| u.id.to_string())
        .columns(TextColumn::r#for(
            DummyUser::fields().name(),
            |u: &DummyUser| u.name.clone(),
        ))
        .defer(true);
    assert!(table3.is_defer(), "defer(true) should enable");
    // Render with boundary should contain data-boundary
    let page = argentum_core::TablePage::<DummyUser>::from(vec![]);
    let html = table
        .render(&cx, page.clone())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        html.contains("data-boundary=\"table\""),
        "boundary should be in HTML, got {}",
        html
    );
    let html2 = table2
        .render(&cx, page)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        !html2.contains("data-boundary=\"table\""),
        "boundary false should not be in HTML"
    );

    // Test memoize dedup
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    #[memoize]
    async fn counted(cx: &Cx, n: usize) -> usize {
        COUNTER.fetch_add(1, Ordering::SeqCst);
        n * 2
    }

    let cx = CxTestBuilder::new().build();
    COUNTER.store(0, Ordering::SeqCst);
    let a = *counted(&cx, 5).await;
    let b = *counted(&cx, 5).await;
    assert_eq!(a, 10);
    assert_eq!(b, 10);
    assert_eq!(
        COUNTER.load(Ordering::SeqCst),
        1,
        "memoize should dedup concurrent calls"
    );
}
