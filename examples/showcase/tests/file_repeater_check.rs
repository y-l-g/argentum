use http::{Method, Request, header::{CONTENT_TYPE, COOKIE}};
use http_body_util::BodyExt;
use showcase::{
    app::router_for_tests as router,
    models::{Author, DEMO_TENANT, Post, seed, seed_phase2},
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
                .header("x-tenant-id", DEMO_TENANT.to_string())
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
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // Missing image_path and tags (both required)
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(format!(
                    "title=Test&author_id={}&image_path=&tags=&csrf_token={csrf}",
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
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(format!(
                    "title=Valid+With+Files&author_id={}&image_path=/tmp/valid.jpg&tags=valid,tags&csrf_token={csrf}",
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

#[tokio::test]
async fn posts_create_form_is_multipart() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("enctype=\"multipart/form-data\""),
        "file form must be multipart, got {}",
        &html[..html.len().min(2000)]
    );
    assert!(
        html.contains("type=\"file\""),
        "missing file input {}",
        &html[..html.len().min(2000)]
    );
}

#[tokio::test]
async fn users_create_form_stays_urlencoded() {
    let db = full_db().await;
    let router = router(db);
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/users/create")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(resp.status().is_success());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        !html.contains("multipart/form-data"),
        "plain form must stay urlencoded, got {}",
        &html[..html.len().min(2000)]
    );
}

#[tokio::test]
async fn posts_create_multipart_file_stores_filename() {
    let db = full_db().await;
    let router = router(db.clone());
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let boundary = "----TestBoundary789";
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nMultipart Upload\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"author_id\"\r\n\r\n{id}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"upload.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nFAKEBYTES\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nmultipart,tags\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{csrf}\r\n\
         --{b}--\r\n",
        b = boundary,
        id = first.id
    );
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .method(Method::POST)
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "multipart valid should redirect, got {}",
        resp.status()
    );
    let mut db2 = db.clone();
    let after = Post::all().exec(&mut db2).await.unwrap().len();
    assert_eq!(after, before + 1);
    let created = Post::filter(Post::fields().title().eq("Multipart Upload".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("multipart post");
    // v1 stores the filename, not the bytes (FileUpload contract).
    assert_eq!(created.image_path, "upload.jpg");
    assert_eq!(created.tags, "multipart,tags");
}

#[tokio::test]
async fn posts_edit_untouched_file_keeps_stored_path() {
    // GH #90: the edit form renders an empty file input, so an empty submit
    // means "keep" — it must not blank the stored path or trip required.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let post = Post::filter(showcase::models::Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("seeded post");
    assert_eq!(post.image_path, "/images/hello.jpg");
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/posts/{}/edit", post.id))
                .header("x-tenant-id", DEMO_TENANT.to_string())
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .body(Body::from(format!(
                    "title=Renamed&author_id={}&image_path=&tags=rust&csrf_token={csrf}",
                    authors[0].id
                )))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "untouched-file edit must redirect, got {}",
        resp.status()
    );
    let kept = Post::filter(showcase::models::Post::fields().title().eq("Renamed".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("renamed post");
    assert_eq!(kept.image_path, "/images/hello.jpg", "stored path must survive untouched edit");
}

#[tokio::test]
async fn posts_edit_explicit_clear_flag_skips_preservation() {
    // GH #90: `clear_<field>=1` opts back into clearing. On the required
    // image field that surfaces as the inline required error (not a silent
    // keep), with the stored path untouched.
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let post = Post::filter(showcase::models::Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("seeded post");
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = router
        .handle(
            Request::builder()
                .uri(format!("/admin/posts/{}/edit", post.id))
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .header("x-tenant-id", showcase::models::DEMO_TENANT.to_string())
                .body(Body::from(format!(
                    "title=Kept&author_id={}&image_path=&tags=rust&clear_image_path=1&csrf_token={csrf}",
                    authors[0].id
                )))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_success(),
        "explicit clear on a required file must re-render, got {}",
        resp.status()
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("is required"),
        "cleared required file must error inline, got {html}"
    );
    let kept = Post::filter(showcase::models::Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db_q)
        .await
        .unwrap()
        .expect("post still titled");
    assert_eq!(kept.image_path, "/images/hello.jpg", "failed edit must not touch storage");
}

#[tokio::test]
async fn multipart_body_limit_matches_urlencoded_cap() {
    // GH #90: the BodyLimit layer gives multipart the same 10 MiB cap as
    // urlencoded (Topcoat's 2 MiB default would 413 uploads we accept).
    let db = full_db().await;
    let router = router(db.clone());
    let mut db_q = db.clone();
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let csrf = uuid::Uuid::new_v4().to_string();
    let boundary = "----LimitTest";
    // 3 MiB streams fine (above Topcoat's 2 MiB default).
    let big_ok = "a".repeat(3 * 1024 * 1024);
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nT\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"author_id\"\r\n\r\n{id}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"big.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n{blob}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nt\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{csrf}\r\n\
         --{b}--\r\n",
        b = boundary,
        id = authors[0].id,
        blob = big_ok,
    );
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .header("x-tenant-id", showcase::models::DEMO_TENANT.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "3 MiB multipart must pass the 10 MiB cap, got {}",
        resp.status()
    );
    // 11 MiB is a 413 without buffering the whole body first.
    let big_no = "a".repeat(11 * 1024 * 1024);
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{blob}\r\n\
         --{b}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{csrf}\r\n\
         --{b}--\r\n",
        b = boundary,
        blob = big_no,
    );
    let resp = router
        .handle(
            Request::builder()
                .uri("/admin/posts/create")
                .method(Method::POST)
                .header(CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
                .header(COOKIE, format!("argentum_csrf={csrf}"))
                .header("x-tenant-id", showcase::models::DEMO_TENANT.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), 413, "11 MiB multipart must be rejected");
}
