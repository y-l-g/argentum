use http::{Method, Request, header::CONTENT_TYPE};
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
async fn posts_create_shows_fileupload_and_repeater() {
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
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    // FileUpload should be an input type="file" with for/id linking and Tokens
    assert!(
        html.contains("type=\"file\""),
        "missing file input {}",
        html
    );
    assert!(
        html.contains("for=\"image_path\"") || html.contains("for=\"image\""),
        "missing for/id linking {}",
        html
    );
    assert!(
        html.contains("grid gap-1.5"),
        "missing grid gap-1.5 {}",
        html
    );
    assert!(
        html.contains("border-border"),
        "missing border-border {}",
        html
    );
    // Repeater should render nested schema with Tags label and inner Tag input
    assert!(html.contains("Tags"), "missing Repeater label {}", html);
    assert!(
        html.contains("for=\"tags\"") || html.contains("name=\"tags\""),
        "missing tags input {}",
        html
    );
    // Section/Grid composition
    assert!(
        html.contains("Post Details") || html.contains("Section"),
        "missing Section {}",
        html
    );
    assert!(
        html.contains("grid grid-cols-2") || html.contains("grid-cols-2"),
        "missing Grid {}",
        html
    );
}

#[tokio::test]
async fn posts_create_invalid_fileupload_repeater_shows_errors() {
    let db = full_db().await;
    let router = router(db.clone());
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // Missing image_path and tags (both required)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "title=Test&author_id={}&image_path=&tags=",
                    first.id
                )))
                .unwrap(),
        )
        .await;
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        status.is_success(),
        "invalid should be 200, got {} {}",
        status,
        html
    );
    assert!(
        html.contains("is required") || html.contains("required"),
        "missing required error {}",
        html
    );
    // Should not create
    let mut db2 = db.clone();
    let posts = Post::all().exec(&mut db2).await.unwrap();
    assert_eq!(posts.len(), 2, "should not create on invalid");
}

#[tokio::test]
async fn posts_create_valid_fileupload_repeater_creates() {
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
                .body(Body::from(format!(
                    "title=Valid+With+Files&author_id={}&image_path=/tmp/valid.jpg&tags=valid,tags",
                    first.id
                )))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "valid should redirect, got {} ",
        resp.status()
    );
    let mut db2 = db.clone();
    let after = Post::all().exec(&mut db2).await.unwrap().len();
    assert_eq!(after, before + 1);
    let created = Post::filter(Post::fields().title().eq("Valid With Files".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap();
    assert!(created.is_some());
    let post = created.unwrap();
    assert_eq!(post.image_path, "/tmp/valid.jpg");
    assert_eq!(post.tags, "valid,tags");
}
