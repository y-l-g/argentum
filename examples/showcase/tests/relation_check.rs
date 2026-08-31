use http::{
    Method, Request,
    header::{CONTENT_TYPE, LOCATION},
};
use http_body_util::BodyExt;
use showcase::{
    app::router_for_tests as router,
    models::{Author, Post, seed, seed_phase2},
};
use toasty::Db;
use topcoat::router::Body;

async fn full_db() -> Db {
    let mut db = Db::builder()
        .models(toasty::models!(
            showcase::models::User,
            showcase::models::Author,
            showcase::models::Post,
            showcase::models::Comment
        ))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    seed(&mut db).await.unwrap();
    seed_phase2(&mut db).await.unwrap();
    db
}

#[tokio::test]
async fn posts_list_shows_author_name() {
    let db = full_db().await;
    let router = router(db.clone());
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Hello Toasty"), "missing post title {}", html);
    assert!(html.contains("Ada Author"), "missing author name {}", html);
}

#[tokio::test]
async fn posts_create_shows_select_with_author_options() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("title=Test+Post&author_id="))
                .unwrap(),
        )
        .await;
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
    let fake_id = uuid::Uuid::new_v4();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("title=Test+Post&author_id={}", fake_id)))
                .unwrap(),
        )
        .await;
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
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
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("title=New+Post&author_id={}", first.id)))
                .unwrap(),
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
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // create a post via valid route to ensure edit hydrates
    let _ = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("title=EditMe&author_id={}", first.id)))
                .unwrap(),
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
    let resp = router
        .handle(
            Request::builder()
                .uri(edit_url)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("EditMe"), "edit should show title {}", html);
    assert!(
        html.contains(&first.id.to_string()) || html.contains("selected"),
        "edit should show selected author {}",
        html
    );
}
