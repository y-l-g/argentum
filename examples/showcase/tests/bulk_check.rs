use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

mod common;
use common::{
    TestClient, body_string, demo_client, response_cookies, seeded_db, set_cookie_header,
};

#[tokio::test]
async fn bulk_delete_deletes_selected() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    assert_eq!(users.len(), 8);
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
    // Post/Redirect/Get with one-time semantics (GH #97, #126): 303, flash
    // cookie on the redirect, clean Location.
    assert_eq!(resp.status(), 303, "a completed bulk delete is a 303 PRG");
    assert!(
        !loc.contains("notification"),
        "the toast must not ride the query, got {loc}"
    );
    let flash = set_cookie_header(&resp, "__Host-argentum_notification")
        .expect("the flash cookie is set on the redirect");
    assert!(
        flash.contains("Bulk"),
        "the flash carries the action, got {flash}"
    );

    // Check DB: should have 1 left
    let mut db_check = db.clone();
    let remaining = User::all().exec(&mut db_check).await.unwrap();
    assert_eq!(
        remaining.len(),
        6,
        "should have 6 after bulk delete 2, got {}",
        remaining.len()
    );
    // Follow redirect carrying the flash cookie and check the toast
    let resp2 = client.cookies(&response_cookies(&resp)).get(loc).await;
    let html2 = body_string(resp2).await;
    assert!(
        html2.contains("Bulk deleted"),
        "notification should survive, got {}",
        html2
    );
}

#[tokio::test]
async fn bulk_delete_without_ids_redirects_with_the_reason() {
    // GH #151: the visible ids input is gone and the submit ships disabled,
    // so a hand-crafted empty POST is a validation miss — the list comes back
    // with an error toast, never the raw 400 page.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids=&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        303,
        "an empty bulk delete must redirect, not 400, got {}",
        resp.status()
    );
    let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(loc.contains("/admin/users"), "redirect to list, got {loc}");
    let flash = set_cookie_header(&resp, "__Host-argentum_notification")
        .expect("the flash cookie carries the reason");
    assert!(
        flash.contains("error") && flash.contains("Select"),
        "the flash must be the selection error, got {flash}"
    );
    // Nothing was deleted.
    let mut db_check = db.clone();
    let remaining = User::all().exec(&mut db_check).await.unwrap();
    assert_eq!(remaining.len(), 8, "an empty bulk delete deletes nothing");
}

#[tokio::test]
async fn bulk_delete_short_fetch_404s_and_deletes_nothing() {
    // GH #136 §4: a batch naming a missing id comes back short from the
    // tenancy-scoped `IN` fetch and 404s — never half-applied.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let real = users.first().unwrap().id.to_string();
    let missing = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={real},{missing}&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "short-fetch bulk delete must 404, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let remaining = User::all().exec(&mut db_check).await.unwrap();
    assert_eq!(
        remaining.len(),
        8,
        "a short-fetch batch must delete nothing, got {}",
        remaining.len()
    );
}

#[tokio::test]
async fn bulk_bar_renders_checkboxes_with_row_keys() {
    // GH #136 layer rule: core (`bulk_checkboxes_render_with_keys_and_select_all`)
    // owns the bulk-chrome detail (select-all, hidden transport, disabled
    // submit); this pins the HTTP wiring — pagination, filtering, and the
    // checkbox-joined POST format.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    assert_eq!(users.len(), 8);
    let ids: std::collections::HashSet<String> = users.iter().map(|u| u.id.to_string()).collect();

    // The list streams (skeleton first, rows in the swap payload); the
    // collected body contains both. The table paginates by 25, so the first
    // page carries all 8 seeded row checkboxes.
    let resp = client.get("/admin/users").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert_eq!(
        html.matches("data-row-select").count(),
        8,
        "first page should carry 8 row checkboxes in {}",
        html
    );
    // Every rendered checkbox value is a real row key (the three visible rows;
    // delete forms carry ids in actions, never in `value=`).
    let mut found = 0;
    for u in &users {
        if html.contains(&format!("value=\"{}\"", u.id)) {
            found += 1;
        }
    }
    assert_eq!(
        found, 8,
        "all visible row keys should be checkbox values in {}",
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
        .auth(argentum_core::Auth::disabled())
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
        .auth(argentum_core::Auth::disabled())
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
