use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

use crate::common::{
    TestClient, body_string, demo_client, response_cookies, seeded_db, set_cookie_header,
    user_count,
};

#[tokio::test]
async fn bulk_delete_deletes_selected() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let before = user_count(&db).await;
    assert!(
        users.len() >= 2,
        "the roster fixture must hold at least two rows to bulk-delete"
    );
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
            format!("ids={ids_param}&confirm=1&csrf_token={csrf}"),
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
    // Post/Redirect/Get with one-time semantics (#126): 303, flash
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

    // Check DB: exactly the two selected rows are gone.
    let remaining = user_count(&db).await;
    assert_eq!(
        remaining,
        before - 2,
        "bulk-deleting 2 of {before} must leave {}",
        before - 2
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
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let before = user_count(&db).await;
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids=&confirm=1&csrf_token={csrf}"),
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
    assert_eq!(
        user_count(&db).await,
        before,
        "an empty bulk delete deletes nothing"
    );
}

#[tokio::test]
async fn bulk_delete_short_fetch_404s_and_deletes_nothing() {
    // GH #136 §4: a batch naming a missing id comes back short from the
    // tenancy-scoped `IN` fetch and 404s — never half-applied.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let before = users.len();
    let real = users.first().unwrap().id.to_string();
    let missing = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={real},{missing}&confirm=1&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "short-fetch bulk delete must 404, got {}",
        resp.status()
    );
    assert_eq!(
        user_count(&db).await,
        before,
        "a short-fetch batch must delete nothing"
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
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let roster = users.len();
    let ids: std::collections::HashSet<String> = users.iter().map(|u| u.id.to_string()).collect();

    // The list streams (skeleton first, rows in the swap payload); the
    // collected body contains both. The table paginates by 25, so the first
    // page carries every seeded row's checkbox.
    let resp = client.get("/admin/users").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert_eq!(
        html.matches("data-row-select").count(),
        roster,
        "first page should carry {roster} row checkboxes in {}",
        html
    );
    // Every rendered checkbox value is a real row key (the visible rows;
    // delete forms carry ids in actions, never in `value=`).
    let mut found = 0;
    for u in &users {
        if html.contains(&format!("value=\"{}\"", u.id)) {
            found += 1;
        }
    }
    assert_eq!(
        found, roster,
        "all rendered row keys should be checkbox values in {}",
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
            format!("ids={ids_param}&confirm=1&csrf_token={csrf}"),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "checkbox-joined bulk delete should redirect, got {}",
        resp.status()
    );
}

/// GH #235: select-all over the seeded roster. Every row renders a checkbox,
/// but Ken's is disabled — the SSO guard denies his delete — so the browser's
/// select-all collects the other seven and the handler deletes them, instead of
/// refusing the whole batch over the one row the resource protects.
#[tokio::test]
async fn select_all_skips_the_denied_row_and_deletes_the_rest() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let before = users.len();
    let ken = users
        .iter()
        .find(|u| u.name == "Ken Thompson")
        .expect("the seeded SSO-managed row");
    assert!(
        before > 1,
        "the roster needs rows beyond Ken for the batch to prove a deletion"
    );

    let html = body_string(client.get("/admin/users").await).await;
    // The rendered chrome is the fix: the denied row links no edit page and no
    // delete dialog, and its checkbox is disabled, carrying the reason as its
    // accessible label.
    assert!(
        !html.contains(&format!("/admin/users/{}/edit", ken.id)),
        "the denied row must render no Edit link, got {html}"
    );
    assert!(
        !html.contains(&format!("delete={}", ken.id)),
        "the denied row must render no Delete link, got {html}"
    );
    let ken_tag = input_tag_with_value(&html, &ken.id.to_string());
    assert!(
        ken_tag.contains("disabled"),
        "the denied row's checkbox must be disabled, got {ken_tag}"
    );
    assert!(
        ken_tag.contains("You cannot delete this row"),
        "the disabled checkbox must carry the reason, got {ken_tag}"
    );
    // The allowed rows keep both links, so the absences above are not passing
    // on a page that renders no chrome at all.
    let ada = users
        .iter()
        .find(|u| u.name == "Ada Lovelace")
        .expect("the seeded allowed row");
    assert!(
        html.contains(&format!("/admin/users/{}/edit", ada.id))
            && html.contains(&format!("delete={}", ada.id)),
        "an allowed row must keep its Edit and Delete links, got {html}"
    );

    // What select-all submits: exactly the boxes `bulk.js` would check.
    let ids = selectable_row_ids(&html);
    assert_eq!(
        ids.len(),
        before - 1,
        "select-all must offer every row but Ken's, got {ids:?}"
    );
    assert!(
        !ids.contains(&ken.id.to_string()),
        "the denied key must not be selectable, got {ids:?}"
    );

    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={}&confirm=1&csrf_token={csrf}", ids.join(",")),
        )
        .await;
    assert_eq!(
        resp.status(),
        303,
        "select-all over the roster must not 403, got {}",
        resp.status()
    );
    assert_eq!(
        user_count(&db).await,
        1,
        "every allowed row must be deleted, leaving Ken"
    );
    let mut db_check = db.clone();
    let remaining = User::all().exec(&mut db_check).await.unwrap();
    assert_eq!(remaining.len(), 1, "only the denied row survives");
    assert_eq!(remaining[0].name, "Ken Thompson");
}

/// The opening `<input …>` tag whose `value` attribute is `value`.
///
/// Attributes render in no guaranteed order (topcoat#122), so callers assert on
/// the whole tag rather than on a single attribute's position.
fn input_tag_with_value(html: &str, value: &str) -> String {
    let at = html
        .find(&format!("value=\"{value}\""))
        .unwrap_or_else(|| panic!("missing an input with value {value} in {html}"));
    let start = html[..at].rfind("<input").expect("its opening tag");
    input_tag_at(&html[start..])
}

/// The `<input …>` tag `html` starts with, up to the `>` that closes it.
fn input_tag_at(html: &str) -> String {
    let mut quoted = false;
    for (offset, byte) in html.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'>' if !quoted => return html[..offset].to_string(),
            _ => {}
        }
    }
    panic!("unterminated <input> tag in {html}");
}

/// The row ids the page offers for bulk selection, in document order: every
/// `data-row-select` checkbox a user can check. A row the per-record policy
/// denies delete renders `disabled`, and `bulk.js`'s `boxesIn` skips
/// exactly those — so this is what select-all submits.
fn selectable_row_ids(html: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-row-select") {
        let start = rest[..at]
            .rfind("<input")
            .expect("the marker's opening tag");
        let tag = input_tag_at(&rest[start..]);
        if !tag.contains("disabled") {
            let value_at = tag.find("value=\"").expect("a checkbox value");
            let after = &tag[value_at + "value=\"".len()..];
            let end = after.find('"').expect("a closed value");
            ids.push(after[..end].to_string());
        }
        rest = &rest[at + 1..];
    }
    ids
}

/// The server-side safety net, after GH #235 moved the visible decision into
/// the row policy: a hand-crafted POST naming a row the resource refuses is
/// still 403. The check is all-or-nothing (GH #168: `can_view` then
/// `can_delete` on every row, before any write), so the batch aborts with zero
/// deletions — which is why the rendered checkbox must never offer that row.
///
/// `can_view` allows every row here, so only the partial `can_delete` deny can
/// produce the 403: with the default-deny `can_view` in place, dropping the
/// handler's own `can_delete` check would leave this test green.
#[tokio::test]
async fn bulk_delete_hand_crafted_partial_deny_is_refused() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model, Clone)]
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
        fn can_view(_cx: &topcoat::context::Cx, _rec: &DummyUser) -> bool {
            true
        }
        fn can_delete(_cx: &topcoat::context::Cx, rec: &DummyUser) -> bool {
            // Deny second record (name == "b")
            rec.name != "b"
        }
        fn table(cx: &topcoat::context::Cx) -> Table<DummyUser> {
            Table::r#for(cx)
                .id(|u: &DummyUser| u.id.to_string())
                .pk(|u: &DummyUser| u.id.to_string())
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
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);
    let slug = PartialDenyResource::slug();
    let ids = format!("{},{}", a.id, b.id);
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/{}/bulk-delete", slug),
            format!("ids={ids}&confirm=1&csrf_token={csrf}"),
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
/// GH #184: the batch asks before it acts, and the guarantee is the server's.
/// A POST that does not carry the confirming control's marker is refused —
/// otherwise the dialog would be decoration that a crafted request skips.
#[tokio::test]
async fn bulk_delete_without_confirmation_is_refused() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let before = users.len();
    let id = users[0].id.to_string();
    let csrf = uuid::Uuid::new_v4().to_string();

    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/users/bulk-delete",
            format!("ids={id}&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        400,
        "an unconfirmed bulk delete must be refused, got {}",
        resp.status()
    );

    assert_eq!(
        user_count(&db).await,
        before,
        "a refused batch deletes nothing"
    );
}
