use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;
use topcoat::view::ViewExt;

mod common;
use common::{TestClient, body_string, seeded_db};

#[tokio::test]
async fn bulk_delete_deletes_selected() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = TestClient::new(&router);
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    assert_eq!(users.len(), 3);
    let ids: Vec<String> = users.iter().take(2).map(|u| u.id.to_string()).collect();
    let ids_param = ids.join(",");

    // Check that list page contains Bulk Delete
    let resp = client.get("/admin/users").await;
    let html = body_string(resp).await;
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
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={ids_param}&csrf_token={csrf}"),
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
    let resp2 = client.get(loc).await;
    let html2 = body_string(resp2).await;
    assert!(
        html2.contains("fixed top-4 right-4"),
        "notification should survive, got {}",
        html2
    );
}

#[tokio::test]
async fn bulk_bar_renders_checkboxes_with_row_keys() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = TestClient::new(&router);
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    assert_eq!(users.len(), 3);
    let ids: std::collections::HashSet<String> = users.iter().map(|u| u.id.to_string()).collect();

    // The list streams (skeleton first, rows in the swap payload); the
    // collected body contains both. The table paginates by 2, so the first
    // page carries exactly 2 row checkboxes.
    let resp = client.get("/admin/users").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert_eq!(
        html.matches("data-row-select").count(),
        2,
        "first page should carry 2 row checkboxes in {}",
        html
    );
    // Every rendered checkbox value is a real row key (the two visible rows;
    // delete forms carry ids in actions, never in `value=`).
    let mut found = 0;
    for u in &users {
        if html.contains(&format!("value=\"{}\"", u.id)) {
            found += 1;
        }
    }
    assert_eq!(
        found, 2,
        "both visible row keys should be checkbox values in {}",
        html
    );
    assert!(
        html.contains("data-bulk-select-all"),
        "missing select-all in {}",
        html
    );
    // Bulk form keeps the single ids transport (JS joins checked keys into
    // it; the text field is the no-JS fallback).
    assert!(
        html.contains("data-bulk-form") && html.contains("name=\"ids\""),
        "missing bulk form transport in {}",
        html
    );
    // A filtered list shows only the matching row's checkbox.
    let ada = users.iter().find(|u| u.name == "Ada Lovelace").unwrap();
    let resp = client.get("/admin/users?q=Ada").await;
    let html = body_string(resp).await;
    assert!(
        html.contains(&format!("value=\"{}\"", ada.id)),
        "filtered row checkbox missing in {}",
        html
    );
    assert!(
        !ids.iter()
            .filter(|id| *id != &ada.id.to_string())
            .any(|id| html.contains(&format!("value=\"{id}\""))),
        "only the filtered row should be selectable in {}",
        html
    );
    // Checkbox-joined POST uses the same comma format the handler parses.
    let ids_param = users
        .iter()
        .take(2)
        .map(|u| u.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={ids_param}&csrf_token={csrf}"),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "checkbox-joined bulk delete should redirect, got {}",
        resp.status()
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
    let client = TestClient::new(&router);
    let slug = PartialDenyResource::slug();
    let ids = format!("{},{}", a.id, b.id);
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/{}/bulk-delete", slug),
            format!("ids={ids}&csrf_token={csrf}"),
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
    let client = TestClient::new(&router);
    let slug = DenyViewAnyResource::slug();
    let resp = client.get(&format!("/admin/{}", slug)).await;
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
