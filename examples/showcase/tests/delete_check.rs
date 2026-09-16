use http::header::LOCATION;
use showcase::{app::router_for_tests as router, models::User};
use toasty::Db;

mod common;
use common::{
    TestClient, body_string, demo_client, response_cookies, seeded_db, set_cookie_header,
};

#[tokio::test]
async fn delete_requires_confirmation_and_deletes() {
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
    let mut db_q = db.clone();
    let users = User::all().exec(&mut db_q).await.unwrap();
    let user = users.first().unwrap();
    let id = user.id.to_string();
    let delete_url = format!("/admin/users/{}/delete", id);
    let csrf = uuid::Uuid::new_v4().to_string();

    // The list renders a Delete link that opens the confirmation dialog
    // (`?delete=<key>`) — no per-row POST form, no dialog until asked. The
    // row action is destructive (GH #154 §6), matching the bulk Delete and
    // the dialog's confirm.
    let resp = client.get("/admin/users").await;
    let html = body_string(resp).await;
    assert!(
        html.contains(&format!("delete={id}")),
        "list should link the delete dialog for the row, got {html}"
    );
    let row_delete = {
        let at = html.find(&format!("delete={id}")).unwrap();
        let start = html[..at].rfind("<a ").unwrap();
        let end = html[at..].find('>').unwrap() + at;
        &html[start..end]
    };
    assert!(
        row_delete.contains("bg-destructive"),
        "row Delete must be destructive, got {row_delete}"
    );
    assert!(
        !html.contains("role=\"alertdialog\""),
        "no dialog without ?delete=, got {html}"
    );

    // ?delete=<id> renders the alert dialog on the list page: destructive
    // confirm, Cancel, and the confirmed POST target (GH #151).
    let resp = client.get(&format!("/admin/users?delete={id}")).await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    let action = format!("action=\"/admin/users/{id}/delete\"");
    for needle in [
        "role=\"alertdialog\"",
        "Delete this record?",
        "data-dialog-close",
        "bg-destructive",
        action.as_str(),
        "name=\"confirm\"",
    ] {
        assert!(html.contains(needle), "dialog missing {needle} in {html}");
    }

    // dialog.js mirrors Escape/backdrop dismissal into the URL; the server
    // honors it so a reload stays closed.
    let resp = client
        .get(&format!("/admin/users?delete={id}&open=false"))
        .await;
    let html = body_string(resp).await;
    assert!(
        !html.contains("role=\"alertdialog\""),
        "?open=false must keep the dialog closed, got {html}"
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

    // Check DB: user should be gone
    let mut db_check = db.clone();
    let count = User::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 2, "should have 2 after delete, got {}", count);
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
    let db = seeded_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;
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

    // `R::query(cx)` is the seam every load (find_by_key, the tx fetch)
    // consults, so a counter on the override proves "no find_by_key query
    // observed" (GH #144 acceptance) instead of inferring it from a status.
    static QUERIES: AtomicUsize = AtomicUsize::new(0);
    fn counted_query(_cx: &topcoat::context::Cx) -> toasty::stmt::Query<toasty::stmt::List<Dummy>> {
        QUERIES.fetch_add(1, Ordering::SeqCst);
        toasty::stmt::Query::<toasty::stmt::List<Dummy>>::all()
    }

    #[derive(Debug, toasty::Model)]
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
        .build();
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
async fn delete_policy_deny() {
    use argentum_core::{Resource, Schema, Table, TextColumn, TextInput};

    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct DenyDeleteResource;
    impl Resource for DenyDeleteResource {
        type Model = DummyUser;
        fn can_view_any(_cx: &topcoat::context::Cx) -> bool {
            true
        }
        fn can_view(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
            true
        }
        fn can_delete(_cx: &topcoat::context::Cx, _r: &DummyUser) -> bool {
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

    let mut db = Db::builder()
        .models(toasty::models!(DummyUser))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let rec = toasty::create!(DummyUser {
        name: "x".to_string()
    })
    .exec(&mut db)
    .await
    .unwrap();
    let router = argentum_core::Panel::new("admin")
        .app_context(db.clone())
        .auth(argentum_core::Auth::disabled())
        .resource::<DenyDeleteResource>()
        .build();
    let client = TestClient::new(&router);
    let slug = DenyDeleteResource::slug();
    let delete_url = format!("/admin/{}/{}/delete", slug, rec.id);
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(&delete_url, format!("confirm=1&csrf_token={csrf}"))
        .await;
    assert_eq!(
        resp.status(),
        403,
        "delete should be 403 when denied, got {}",
        resp.status()
    );
    // Check not deleted
    let mut db_check = db.clone();
    let count = DummyUser::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(count, 1, "should not delete when denied");
}
