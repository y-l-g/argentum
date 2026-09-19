use showcase::{
    app::router_for_tests as router,
    models::{Comment, Post},
};

mod common;
use common::{
    body_string, demo_client, form_body, full_db, input_value, response_cookies, tenanted_db,
};

#[tokio::test]
async fn comments_list_shows_body_and_post_title() {
    let db = full_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let resp = client.get("/admin/comments").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(html.contains("Discussion</h1>"), "missing heading: {html}");
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
async fn comments_list_hides_delete_chrome() {
    // deletable() == false: the queue shows no row Delete buttons and no
    // bulk bar, so moderators never reach a 403 after a confirmation.
    let db = full_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
    let resp = client.get("/admin/comments").await;
    let html = body_string(resp).await;
    assert!(
        !html.contains("Bulk Delete"),
        "read-only queue must not offer bulk delete: {html}"
    );
    assert!(
        !html.contains("/delete"),
        "read-only queue must not offer row delete: {html}"
    );
    assert!(
        html.contains(">Edit<"),
        "queue must keep edit links: {html}"
    );
}

#[tokio::test]
async fn comments_create_form_shows_post_select() {
    let db = full_db().await;
    let router = router(db);
    let client = demo_client(&router).await;
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

#[tokio::test]
async fn comments_create_valid_redirects_and_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router).await;

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
    use argentum_core::{Resource, Tenant, db::db as db_handle};
    use showcase::app::{CommentResource, PostResource};
    use std::collections::HashMap;
    use topcoat::context::CxTestBuilder;
    use topcoat::router::response::IntoResponse;

    let (db, t1, t2) = tenanted_db().await;
    let cx = CxTestBuilder::new()
        .app_context(db.clone())
        .request_context(Tenant(t1))
        .build();

    // A post that exists — in the other tenant.
    let cx_t2 = cx.with(Tenant(t2));
    let foreign = PostResource::query(&cx_t2)
        .first()
        .exec(&mut db_handle(&cx_t2))
        .await
        .unwrap()
        .expect("t2 seeds one post");
    // ...and one in this tenant, as the positive control.
    let own = PostResource::query(&cx)
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
    // exactly like unknown id" is the contract here (GH #86/#169).
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
