use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

use crate::common::{
    TestClient, body_string, demo_client, response_cookies, seeded_db, set_cookie_header,
    user_count,
};

#[tokio::test]
async fn delete_requires_confirmation_and_deletes() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let before = users.len();
    let user = users.first().unwrap();
    let id = user.id.to_string();
    let delete_url = format!("/admin/users/{}/delete", id);
    let csrf = uuid::Uuid::new_v4().to_string();

    // The list renders a Delete link that opens the confirmation dialog
    // (`?delete=<key>`) — no per-row POST form, no navigation to open. The
    // row action is destructive (GH #154 §6), matching the bulk Delete and
    // the dialog's confirm.
    let resp = client.get("/admin/users").await;
    let html = body_string(resp).await;
    assert!(
        html.contains(&format!("delete={id}")),
        "list should link the delete dialog for the row, got {html}"
    );
    let row_delete = tag_with(&html, &format!("delete={id}"));
    assert!(
        row_delete.contains("bg-destructive"),
        "row Delete must be destructive, got {row_delete}"
    );
    // The control keeps the `?delete=` opener as the no-JS fallback, which
    // renders the same dialog open with the action already set.
    let href = attr_value(row_delete, "href");
    assert!(
        href.starts_with("/admin/users?") && href.ends_with(&format!("delete={id}")),
        "the control must keep its fallback href, got {href}"
    );
    // The control opens the table's one dialog in place (GH #233): it names
    // that dialog and carries this record's POST target, so the click costs no
    // navigation and the dialog's Delete posts to the clicked row.
    let dialog_id = attr_value(row_delete, "data-row-delete-trigger");
    assert_eq!(
        attr_value(row_delete, "data-row-delete-action"),
        format!("/admin/users/{id}/delete"),
        "the control must carry the row's POST target, got {row_delete}"
    );
    // The dialog itself ships closed — an ordinary list page renders no open
    // dialog — and takes its action from the control, not from the server. One
    // dialog for the page: the streamed table carries none of its own.
    assert_eq!(
        html.matches("data-row-delete-dialog").count(),
        1,
        "one row dialog per page, got {html}"
    );
    let dialog = tag_with(&html, "data-row-delete-dialog");
    assert_eq!(
        attr_value(dialog, "id"),
        dialog_id,
        "the control must name a dialog the page renders, got {dialog}"
    );
    assert!(
        !dialog.contains("open=\"\""),
        "an ordinary list page must render the row dialog closed, got {dialog}"
    );
    let form = tag_with(&html, "data-row-delete-form");
    assert!(
        !form.contains("action="),
        "the closed dialog takes its action from the row control, got {form}"
    );

    // ?delete=<id> renders the alert dialog on the list page: destructive
    // confirm, Cancel, and the confirmed POST target (GH #151).
    let resp = client.get(&format!("/admin/users?delete={id}")).await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    let dialog = tag_with(&html, "data-row-delete-dialog");
    assert!(
        dialog.contains("open=\"\""),
        "?delete= must render the row dialog open, got {dialog}"
    );
    // The open one is the only one: the streamed table's shard output carries
    // no dialog of its own (GH #233), so the live page's eager copy stands
    // alone. A second copy would duplicate the dialog's ids and give the morph
    // one to replace mid-dismissal.
    assert_eq!(
        html.matches("data-row-delete-dialog").count(),
        1,
        "one row dialog on the page, got {html}"
    );
    let action = format!("action=\"/admin/users/{id}/delete\"");
    for needle in [
        "role=\"alertdialog\"",
        "Delete this record?",
        "data-dialog-close",
        // URL-driven dialogs carry the marker dialog.js mirrors `?open=`
        // through (GH #154 §3); signal-driven dialogs do not.
        "data-dialog-open-param=\"open\"",
        "bg-destructive",
        action.as_str(),
        "name=\"confirm\"",
    ] {
        assert!(html.contains(needle), "dialog missing {needle} in {html}");
    }
    // Cancel is a button (GH #233): dismissal closes in place instead of
    // navigating to the list URL.
    let from_title = &html[html
        .find("Delete this record?")
        .expect("the row dialog's title")..];
    let cancel = tag_with(from_title, "data-dialog-close");
    assert!(
        cancel.starts_with("<button"),
        "the row dialog's Cancel must be a button, got {cancel}"
    );

    // dialog.js mirrors Escape/backdrop dismissal into the URL; the server
    // honors it so a reload stays closed.
    let resp = client
        .get(&format!("/admin/users?delete={id}&open=false"))
        .await;
    let html = body_string(resp).await;
    // The row dialog specifically: the page also carries the bulk bar's own
    // confirm dialog (GH #184), which is unrelated to `?delete=`/`?open=`.
    let dialog = tag_with(&html, "data-row-delete-dialog");
    assert!(
        !dialog.contains("open=\"\""),
        "?open=false must keep the row dialog closed, got {dialog}"
    );
    assert!(
        !dialog.contains("data-dialog-open-param"),
        "a closed dialog has no dismissal to mirror, got {dialog}"
    );

    // POST without the dialog's confirmation marker is malformed now that
    // the confirmation page is gone.
    let resp = client
        .csrf(&csrf)
        .post_form(&delete_url, format!("csrf_token={csrf}"))
        .await;
    assert_eq!(
        resp.status(),
        400,
        "an unconfirmed delete POST must refuse, got {}",
        resp.status()
    );

    // POST with confirm should delete and redirect with notification
    let resp = client
        .csrf(&csrf)
        .post_form(&delete_url, format!("confirm=1&csrf_token={csrf}"))
        .await;
    assert!(
        resp.status().is_redirection(),
        "confirmed delete should redirect, got {}",
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
    assert_eq!(resp.status(), 303, "a completed delete is a 303 PRG");
    assert!(
        !loc.contains("notification"),
        "the toast must not ride the query, got {loc}"
    );
    let flash = set_cookie_header(&resp, "__Host-argentum_notification")
        .expect("the flash cookie is set on the redirect");
    assert!(
        flash.contains("Deleted"),
        "the flash carries the action, got {flash}"
    );

    // Check DB: user should be gone, and only that one.
    assert_eq!(
        user_count(&db).await,
        before - 1,
        "deleting one of {before} must leave {}",
        before - 1
    );
    let mut db_check = db.clone();
    let gone = User::filter(User::fields().id().eq(user.id))
        .first()
        .exec(&mut db_check)
        .await
        .unwrap();
    assert!(gone.is_none(), "deleted user should be gone");

    // Follow redirect carrying the flash cookie and check the toast
    let resp2 = client.cookies(&response_cookies(&resp)).get(loc).await;
    let html2 = body_string(resp2).await;
    assert!(
        html2.contains("Deleted"),
        "notification should survive, got {}",
        html2
    );
}

#[tokio::test]
async fn delete_404_for_missing_or_wrong_tenant() {
    // GH #136 layer rule: core owns the loader unit; this pins the HTTP
    // route for unknown ids (wrong-tenant scoping rides the same seam — see
    // `tenancy_check.rs` for the edit path and the bulk/export extension
    // below for the batch paths).
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let fake_id = uuid::Uuid::new_v4().to_string();
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/users/{}/delete", fake_id),
            format!("confirm=1&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        404,
        "unknown id should be 404, got {}",
        resp.status()
    );
}

/// The real record is untouched too: a forged POST on an existing id must
/// not reach the delete either.
#[tokio::test]
async fn forged_delete_runs_no_record_query() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Every load (find_by_key, the tx fetch) starts from `scoped_query`,
    // which calls the resource's `query` — so a counter on that override
    // proves "no find_by_key query observed" (GH #144 acceptance) instead of
    // inferring it from a status.
    static QUERIES: AtomicUsize = AtomicUsize::new(0);
    fn counted_query(_cx: &topcoat::context::Cx) -> toasty::stmt::Query<toasty::stmt::List<Dummy>> {
        QUERIES.fetch_add(1, Ordering::SeqCst);
        toasty::stmt::Query::<toasty::stmt::List<Dummy>>::all()
    }

    #[derive(Debug, toasty::Model, Clone)]
    struct Dummy {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct CountingResource;
    impl Resource for CountingResource {
        type Model = Dummy;
        fn query(cx: &topcoat::context::Cx) -> toasty::stmt::Query<toasty::stmt::List<Dummy>> {
            counted_query(cx)
        }
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _r: &Dummy) -> bool {
            true
        }
        fn can_delete(_cx: &topcoat::context::Cx, _r: &Dummy) -> bool {
            true
        }
        fn table(cx: &topcoat::context::Cx) -> Table<Dummy> {
            Table::r#for(cx)
                .id(|d: &Dummy| d.id.to_string())
                .columns(TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
                    d.name.clone()
                }))
        }
        fn form(_cx: &topcoat::context::Cx) -> Schema {
            Schema::new(TextInput::r#for(Dummy::fields().name()))
        }
        async fn delete_record(
            _cx: &topcoat::context::Cx,
            record: Dummy,
            ex: &mut dyn toasty::Executor,
        ) -> topcoat::Result<()> {
            Dummy::filter(Dummy::fields().id().eq(record.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    let mut db = Db::builder()
        .models(toasty::models!(Dummy))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let rec = toasty::create!(Dummy {
        name: "x".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .auth(argentum_core::Auth::disabled())
        .resource::<CountingResource>()
        .build()
        .expect("panel builds");
    let client = TestClient::new(&router);
    let delete_url = format!("/admin/{}/{}/delete", CountingResource::slug(), rec.id);
    let csrf = uuid::Uuid::new_v4().to_string();
    let cookie_mismatch = uuid::Uuid::new_v4().to_string();

    // A valid flow consults the query seam (the counter is live).
    let resp = client
        .csrf(&csrf)
        .post_form(&delete_url, format!("confirm=1&csrf_token={csrf}"))
        .await;
    assert!(resp.status().is_redirection(), "valid delete redirects");
    assert!(
        QUERIES.load(Ordering::SeqCst) > 0,
        "a confirmed delete must load the record (counter wired)"
    );

    // A forged POST answers 403 without a single record query: the CSRF
    // check runs before the record seam is ever consulted (no find_by_key,
    // no existence oracle). The transaction-open half of GH #144 is pinned
    // by the handler ordering (parse/verify/confirm textually precede
    // `db.transaction()`); a regression that reopened a tx before the fetch
    // would deadlock the edit path's `validate_async` pool discipline loudly
    // rather than silently pass.
    QUERIES.store(0, Ordering::SeqCst);
    for body in [
        format!("confirm=1&csrf_token={csrf}"),
        "confirm=1".to_string(),
    ] {
        let resp = client
            .csrf(&cookie_mismatch)
            .post_form(&delete_url, body)
            .await;
        assert_eq!(resp.status(), 403, "forged delete must 403");
        assert_eq!(
            QUERIES.load(Ordering::SeqCst),
            0,
            "a forged delete must not observe a record query"
        );
    }
}
#[tokio::test]
async fn delete_sso_managed_user_is_forbidden() {
    // Row-level Policy over HTTP: Ken's SSO-managed account cannot be
    // deleted from the panel, while every other row still can.
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let ken = User::filter(User::fields().name().eq("Ken Thompson".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("Ken seed");
    let before = user_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/users/{}/delete", ken.id),
            format!("confirm=1&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(
        resp.status(),
        403,
        "protected row delete must be forbidden, got {}",
        resp.status()
    );
    assert_eq!(
        user_count(&db).await,
        before,
        "forbidden delete must remove nothing"
    );
}

/// The in-place delete path is client-side only (GH #234): both confirms opt
/// in through `data-mutation-submit`, and without JavaScript the markup is the
/// ordinary POST it always was — same method, same action, same 303 the
/// redirect test above pins.
#[tokio::test]
async fn delete_forms_opt_in_without_changing_the_post() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let resp = client.get("/admin/users").await;
    let html = body_string(resp).await;

    let row_form = tag_with(&html, "data-row-delete-form");
    assert!(
        row_form.contains("method=\"post\"") && row_form.contains("data-mutation-submit"),
        "the row confirm must stay a POST that opts into the client path, got {row_form}"
    );
    assert!(
        !row_form.contains("action="),
        "the closed row dialog takes its action from the row control, got {row_form}"
    );

    let bulk_form = tag_with(&html, "data-bulk-form");
    assert!(
        bulk_form.contains("method=\"post\"") && bulk_form.contains("data-mutation-submit"),
        "the bulk confirm must stay a POST that opts into the client path, got {bulk_form}"
    );
    assert_eq!(
        attr_value(bulk_form, "action"),
        "/admin/users/bulk-delete",
        "the bulk form posts to the batch route, got {bulk_form}"
    );

    // The confirmation the handlers require rides inside the form either way:
    // the client path is an affordance, never the safeguard (GH #184).
    assert_eq!(
        html.matches("name=\"confirm\" value=\"1\"").count(),
        2,
        "both confirms must carry the marker the handlers require, got {html}"
    );
}

/// The opening tag of the element carrying `marker`: from the nearest `<`
/// before it to its closing `>`.
fn tag_with<'a>(html: &'a str, marker: &str) -> &'a str {
    let at = html
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker} in {html}"));
    let start = html[..at].rfind('<').expect("the marker's opening tag");
    let end = html[start..].find('>').expect("the tag's end") + start;
    &html[start..=end]
}

/// The value of `name="…"` inside `tag`.
fn attr_value<'a>(tag: &'a str, name: &str) -> &'a str {
    let at = tag
        .find(&format!("{name}=\""))
        .unwrap_or_else(|| panic!("missing {name} in {tag}"));
    let start = at + name.len() + 2;
    let end = tag[start..].find('"').expect("the value's end") + start;
    &tag[start..end]
}
