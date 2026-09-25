use showcase::{
    app::router_for_tests as router,
    models::{Comment, Post},
};

use crate::common::{
    body_string, demo_client, form_body, full_db, input_value, response_cookies, row_link_key,
    tenanted_db,
};

#[tokio::test]
async fn comments_list_shows_body_and_post_title() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/comments").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(html.contains("Comments</h1>"), "missing heading: {html}");
    assert!(
        html.contains("Clear write-up"),
        "missing seeded comment body: {html}"
    );
    assert!(
        html.contains("Hello Toasty"),
        "missing parent post title via include: {html}"
    );
    assert!(
        !html.contains("(unloaded)"),
        "unloaded marker leaked into list: {html}"
    );
}

#[tokio::test]
async fn comments_list_offers_row_and_bulk_delete() {
    // GH #184: the queue moderates. then made chrome opt-in, so
    // `CommentResource` declares `deletable()`/`editable()` — the row Delete
    // control and the bulk bar render, and `can_delete`, `delete_record` and
    // `bulk_delete_records` stop being unreachable.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/comments").await;
    let html = body_string(resp).await;
    assert!(
        html.contains("Bulk Delete"),
        "the moderation queue must offer bulk delete: {html}"
    );
    // The row control is a `?delete=<key>` link that opens the confirmation
    // dialog; the confirmed POST is what removes the row.
    assert!(
        html.contains("delete="),
        "the moderation queue must offer row delete: {html}"
    );
    assert!(
        html.contains(">Edit<"),
        "queue must keep edit links: {html}"
    );
}

#[tokio::test]
async fn comments_row_delete_removes_the_comment() {
    // The chrome above is only worth anything if the write behind it lands.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let before = Comment::all().exec(&mut db.clone()).await.unwrap().len();
    assert!(before > 0, "the fixture must seed comments");

    let resp = client.get("/admin/comments").await;
    let html = body_string(resp).await;
    let csrf = input_value(&html, "csrf_token").expect("the list carries csrf");
    // Follow the row control the moderator actually clicks: identity is two
    // projections, and the delete route takes the record key the
    // `?delete=` link carries — not the table's display key.
    let key = row_link_key(&html, "delete").expect("a row delete control");

    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/comments/{key}/delete"),
            form_body(&[("confirm", "1"), ("csrf_token", &csrf)]),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "a confirmed delete must redirect, got {}",
        resp.status()
    );

    let after = Comment::all().exec(&mut db.clone()).await.unwrap().len();
    assert_eq!(after, before - 1, "the comment must be gone");
}

#[tokio::test]
async fn comments_create_form_shows_post_select() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/comments/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(html.contains("name=\"body\""), "missing body input: {html}");
    assert!(
        html.contains("name=\"post_id\""),
        "missing post select: {html}"
    );
    assert!(html.contains("Hello Toasty"), "missing post option: {html}");
}

/// GH #298: a Comment form's Post options are loaded through the resource's
/// needs-aware query, which asks for no relation includes, so the option load
/// selects the posts' own columns and not every comment of every post. This
/// pins the branch the loader runs: the empty set leaves both relations
/// unloaded, while the list/detail `query` keeps the includes its columns and
/// `view_relations` read.
#[tokio::test]
async fn post_options_do_not_load_every_posts_comments() {
    use showcase::app::PostResource;
    use tablo_core::{IncludeNeeds, Resource, Tenant, db::db as db_handle};
    use topcoat::context::CxTestBuilder;

    let (db, t1, _t2) = tenanted_db().await;
    let cx = CxTestBuilder::new()
        .app_context(db.clone())
        .request_context(Tenant(t1))
        .build();
    let mut handle = db_handle(&cx);

    let option_row = <PostResource as Resource>::query_with(&cx, &IncludeNeeds::default())
        .first()
        .exec(&mut handle)
        .await
        .unwrap()
        .expect("the tenant seeds a post");
    assert!(
        option_row.comments.is_unloaded() && option_row.author.is_unloaded(),
        "the option-load branch must not carry the resource's relation includes"
    );

    let list_row = <PostResource as Resource>::query(&cx)
        .first()
        .exec(&mut handle)
        .await
        .unwrap()
        .expect("the tenant seeds a post");
    assert!(
        !list_row.comments.is_unloaded() && !list_row.author.is_unloaded(),
        "the list/detail query keeps the includes its columns and view_relations read"
    );
}

#[tokio::test]
async fn comments_create_valid_redirects_and_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let page = client.get("/admin/comments/create").await;
    let html = body_string(page).await;
    let csrf = input_value(&html, "csrf_token").expect("create form carries csrf");

    let mut db_q = db.clone();
    let post = Post::all().exec(&mut db_q).await.unwrap().remove(0);
    let before = Comment::all().exec(&mut db_q).await.unwrap().len();

    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/comments/create",
            form_body(&[
                ("body", "A thoughtful follow-up"),
                ("post_id", &post.id.to_string()),
                ("csrf_token", &csrf),
            ]),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid create must redirect, got {}",
        resp.status()
    );
    let mut db_check = db.clone();
    let after = Comment::all().exec(&mut db_check).await.unwrap().len();
    assert_eq!(after, before + 1, "comment must be created");

    let loc = resp
        .headers()
        .get(http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let followed = client.cookies(&response_cookies(&resp)).get(&loc).await;
    let html = body_string(followed).await;
    assert!(html.contains("Created"), "missing created toast: {html}");
    assert!(
        html.contains("A thoughtful follow-up"),
        "new comment must render on the list: {html}"
    );
}

/// GH #178: the pre-tx option-set validation is not a write-time guarantee.
/// A direct `create_record` / `update_record` caller (or a policy flip between
/// validation and the write) must still be stopped by the transaction itself —
/// and an update must not be able to re-point a comment at another tenant's
/// post.
#[tokio::test]
async fn comment_writes_recheck_the_parent_post_tenant_inside_the_transaction() {
    use std::collections::HashMap;

    use showcase::app::{CommentResource, PostResource};
    use tablo_core::{Resource, Tenant, db::db as db_handle, scoped_query};
    use topcoat::{context::CxTestBuilder, router::response::IntoResponse};

    let (db, t1, t2) = tenanted_db().await;
    let cx = CxTestBuilder::new()
        .app_context(db.clone())
        .request_context(Tenant(t1))
        .build();

    // The posts are read through `scoped_query`, the framework's tenant-scoped
    // entry point: plain `PostResource::query` is the unscoped base
    // now, so it could hand back either tenant's post and this test would be
    // asserting nothing.
    // A post that exists — in the other tenant.
    let cx_t2 = cx.with(Tenant(t2));
    let foreign = scoped_query::<PostResource>(&cx_t2)
        .unwrap()
        .first()
        .exec(&mut db_handle(&cx_t2))
        .await
        .unwrap()
        .expect("t2 seeds one post");
    // ...and one in this tenant, as the positive control.
    let own = scoped_query::<PostResource>(&cx)
        .unwrap()
        .first()
        .exec(&mut db_handle(&cx))
        .await
        .unwrap()
        .expect("t1 seeds one post");

    let values = |post_id: uuid::Uuid| {
        let mut v = HashMap::new();
        v.insert("body".to_string(), "moderated".to_string());
        v.insert("post_id".to_string(), post_id.to_string());
        v
    };

    // Create against the foreign post: refused inside the tx.
    let mut handle = db_handle(&cx);
    let mut tx = handle.transaction().await.unwrap();
    let refused =
        <CommentResource as Resource>::create_record(&cx, values(foreign.id), &mut tx).await;
    let error = refused.expect_err("a cross-tenant post must not accept a comment");
    drop(tx);
    // The guard's own 404, not a driver or FK failure: "wrong tenant looks
    // exactly like unknown id" is the contract here (#169).
    let refusal = error
        .into_response(&cx)
        .expect("the refusal renders a response");
    assert_eq!(
        refusal.status(),
        http::StatusCode::NOT_FOUND,
        "a cross-tenant parent must read as not found"
    );

    // Create against this tenant's post: accepted, so the guard is not
    // blanket-denying.
    let mut handle = db_handle(&cx);
    let mut tx = handle.transaction().await.unwrap();
    <CommentResource as Resource>::create_record(&cx, values(own.id), &mut tx)
        .await
        .expect("the tenant's own post accepts a comment");
    tx.commit().await.unwrap();

    // Re-pointing that comment at the foreign post is refused too, and the
    // stored row keeps its original parent.
    let stored = CommentResource::query(&cx)
        .first()
        .exec(&mut db_handle(&cx))
        .await
        .unwrap()
        .expect("the comment was written");
    let original_post = stored.post_id;
    let mut handle = db_handle(&cx);
    let mut tx = handle.transaction().await.unwrap();
    let repointed =
        <CommentResource as Resource>::update_record(&cx, stored, values(foreign.id), &mut tx)
            .await;
    assert!(
        repointed.is_err(),
        "an update must not re-point a comment at another tenant's post"
    );
    drop(tx);

    let after = CommentResource::query(&cx)
        .first()
        .exec(&mut db_handle(&cx))
        .await
        .unwrap()
        .expect("the comment survives the refused update");
    assert_eq!(
        after.post_id, original_post,
        "a refused re-point must not move the comment"
    );
}
