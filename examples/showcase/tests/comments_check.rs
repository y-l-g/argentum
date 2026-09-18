use showcase::{
    app::router_for_tests as router,
    models::{Comment, Post},
};

mod common;
use common::{body_string, demo_client, form_body, full_db, input_value, response_cookies};

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
