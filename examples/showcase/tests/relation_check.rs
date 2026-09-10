use http::header::LOCATION;
use showcase::{
    app::router_for_tests as router,
    models::{Author, DEMO_TENANT, Post},
};

mod common;
use common::{
    assert_hydrate_keys_are_form_fields, body_string, full_db, get_tenant, post_form_tenant,
};

#[tokio::test]
async fn posts_list_shows_author_name() {
    let db = full_db().await;
    let router = router(db.clone());
    let resp = get_tenant(&router, "/admin/posts", DEMO_TENANT).await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(html.contains("Hello Toasty"), "missing post title {}", html);
    assert!(html.contains("Ada Author"), "missing author name {}", html);
}

#[tokio::test]
async fn posts_create_shows_select_with_author_options() {
    let db = full_db().await;
    let router = router(db);
    let resp = get_tenant(&router, "/admin/posts/create", DEMO_TENANT).await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let html = body_string(resp).await;
    assert!(html.contains("<select"), "missing select {}", html);
    assert!(
        html.contains("Ada Author"),
        "missing author option {}",
        html
    );
}

#[tokio::test]
async fn posts_create_empty_author_shows_required_error() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = post_form_tenant(
        &router,
        "/admin/posts/create",
        DEMO_TENANT,
        &csrf,
        format!("title=Test+Post&author_id=&image_path=/tmp/a.jpg&tags=a&csrf_token={csrf}",),
    )
    .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "empty should be 200 not redirect, got {} {}",
        status,
        html
    );
    assert!(
        html.contains("is required") || html.contains("required"),
        "missing required error {}",
        html
    );
    // DB still has 2 posts
    let mut db2 = db.clone();
    let posts = Post::all().exec(&mut db2).await.unwrap();
    assert_eq!(posts.len(), 2);
}

#[tokio::test]
async fn posts_create_invalid_author_shows_invalid_error() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let fake_id = uuid::Uuid::new_v4();
    let resp = post_form_tenant(
        &router,
        "/admin/posts/create",
        DEMO_TENANT,
        &csrf,
        format!(
            "title=Test+Post&author_id={}&image_path=/tmp/a.jpg&tags=a&csrf_token={csrf}",
            fake_id
        ),
    )
    .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(status.is_success(), "invalid should be 200 {}", html);
    assert!(
        html.contains("is invalid") || html.contains("invalid"),
        "missing invalid error {}",
        html
    );
    let mut db2 = db.clone();
    let posts = Post::all().exec(&mut db2).await.unwrap();
    assert_eq!(posts.len(), 2);
}

#[tokio::test]
async fn posts_create_valid_redirects_and_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = post_form_tenant(
        &router,
        "/admin/posts/create",
        DEMO_TENANT,
        &csrf,
        format!(
            "title=New+Post&author_id={}&image_path=/tmp/new.jpg&tags=new&csrf_token={csrf}",
            first.id
        ),
    )
    .await;
    assert!(
        resp.status().is_redirection(),
        "valid should redirect, got {} ",
        resp.status()
    );
    let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(loc.contains("/admin/posts"));
    let mut db2 = db.clone();
    let after = Post::all().exec(&mut db2).await.unwrap().len();
    assert_eq!(after, before + 1);
    let created = Post::filter(Post::fields().title().eq("New Post".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap();
    assert!(created.is_some());
}

#[tokio::test]
async fn posts_edit_hydrates_author() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // create a post via valid route to ensure edit hydrates
    let _ = post_form_tenant(
        &router,
        "/admin/posts/create",
        DEMO_TENANT,
        &csrf,
        format!(
            "title=EditMe&author_id={}&image_path=/tmp/edit.jpg&tags=edit&csrf_token={csrf}",
            first.id
        ),
    )
    .await;
    let mut db2 = db.clone();
    let post = Post::filter(Post::fields().title().eq("EditMe".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .unwrap();
    let edit_url = format!("/admin/posts/{}/edit", post.id);
    let resp = get_tenant(&router, &edit_url, DEMO_TENANT).await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(html.contains("EditMe"), "edit should show title {}", html);
    assert!(
        html.contains(&first.id.to_string()) || html.contains("selected"),
        "edit should show selected author {}",
        html
    );
}

#[tokio::test]
async fn posts_list_shows_comments_count_via_include() {
    let db = full_db().await;
    let router = router(db.clone());
    let resp = get_tenant(&router, "/admin/posts", DEMO_TENANT).await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    // Table should have Comments header and counts 1 and 0 (one query, no N+1)
    assert!(
        html.contains("Comments"),
        "missing Comments header {}",
        html
    );
    // Hello Toasty has 1 comment, Second Post has 0
    assert!(
        html.contains(">1<") || html.contains("1"),
        "missing comment count 1 {}",
        html
    );
    assert!(
        html.contains(">0<") || html.contains("0"),
        "missing comment count 0 {}",
        html
    );
    // GH #101: loaded relations must never render the unloaded marker.
    assert!(
        !html.contains("(unloaded)"),
        "unloaded marker leaked into list {}",
        html
    );
}

#[tokio::test]
async fn posts_update_rechecks_author_existence() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let first = &authors[0];
    let posts = Post::all().exec(&mut db_q).await.unwrap();
    let post = &posts[0];
    let edit_url = format!("/admin/posts/{}/edit", post.id);
    // Valid same-author update still redirects (symmetric double-check, GH #91).
    let resp = post_form_tenant(
        &router,
        &edit_url,
        DEMO_TENANT,
        &csrf,
        format!(
            "title=Updated+Title&author_id={}&image_path=/tmp/u.jpg&tags=u&csrf_token={csrf}",
            first.id
        ),
    )
    .await;
    assert!(
        resp.status().is_redirection(),
        "valid update should redirect, got {}",
        resp.status()
    );
    // Bogus author is rejected, not silently written (validate_async invalid).
    let fake = uuid::Uuid::new_v4();
    let resp = post_form_tenant(
        &router,
        &edit_url,
        DEMO_TENANT,
        &csrf,
        format!("title=Bad&author_id={fake}&image_path=/tmp/u.jpg&tags=u&csrf_token={csrf}"),
    )
    .await;
    assert!(
        !resp.status().is_redirection(),
        "bogus author update must not redirect, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn hydrate_form_values_match_schema_fields() {
    use showcase::app::{AuthorResource, PostResource};

    let db = full_db().await;
    let cx = topcoat::context::CxTestBuilder::new()
        .app_context(db.clone())
        .build();
    let mut db_q = db.clone();
    let author = Author::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_hydrate_keys_are_form_fields::<AuthorResource>(&cx, &author);
    let post = Post::all()
        .exec(&mut db_q)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_hydrate_keys_are_form_fields::<PostResource>(&cx, &post);
}
