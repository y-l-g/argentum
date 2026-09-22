use showcase::{
    app::router_for_tests as router,
    models::{Author, Post},
};

use crate::common::{body_string, demo_client, file_input_tag, full_db, post_count};

#[tokio::test]
async fn posts_create_shows_fileupload_and_repeater() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
        html.contains("data-slot=\"field\""),
        "missing field wrapper {}",
        html
    );
    // Repeater should render nested schema with Tags label and inner Tag input
    assert!(html.contains("Tags"), "missing Repeater label {}", html);
    assert!(
        html.contains("for=\"tags\"") || html.contains("name=\"tags\""),
        "missing tags input {}",
        html
    );
    // Content/Group/Tabs composition: sectioned story fields, grouped
    // metadata grid, tabbed media.
    assert!(html.contains("Content"), "missing Content section {}", html);
    assert!(
        html.contains("name=\"status\"") && html.contains("name=\"featured\""),
        "missing lifecycle selects {}",
        html
    );
    assert!(
        html.contains("field-group"),
        "missing Group container {}",
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
    let client = demo_client(&router, &db).await;
    let before = post_count(&db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    // Missing image_path (a required FileUpload). The optional Tags group is
    // empty, which is absent — not an error — since GH #147.
    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=Test&author_id={}&image_path=&tags=&csrf_token={csrf}",
                first.id
            ),
        )
        .await;
    let status = resp.status();
    let html = body_string(resp).await;
    assert!(
        status.is_success(),
        "invalid should be 200, got {} {}",
        status,
        html
    );
    assert!(
        html.contains("Image path is required"),
        "missing required error for the file field, got {html}"
    );
    assert_eq!(
        post_count(&db).await,
        before,
        "an invalid create must not add a post"
    );
}

#[tokio::test]
async fn posts_create_valid_fileupload_repeater_creates() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let first = &authors[0];
    let before = Post::all().exec(&mut db2).await.unwrap().len();
    let resp = client.csrf(&csrf).post_form("/admin/posts/create", format!(
            "title=Valid+With+Files&author_id={}&image_path=/tmp/valid.jpg&tags=valid,tags&csrf_token={csrf}",
            first.id
        ))
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

/// An optional Repeater with a `required` inner input must not fail an empty
/// submit (GH #147): group-empty means "absent". The shipped `/admin/posts`
/// form is exactly that shape (optional `Tags` over a required inner input),
/// so an empty Tags group submits cleanly. A partially filled group still
/// enforces inner `required` — pinned at the schema level, where the shape is
/// expressible (a single-entry repeater submits one entry, so the values
/// cannot distinguish "group absent" from "group present" in this form).
#[tokio::test]
async fn posts_create_with_empty_optional_tags_group_submits() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let csrf = uuid::Uuid::new_v4().to_string();
    let mut db2 = db.clone();
    let authors = Author::all().exec(&mut db2).await.unwrap();
    let before = Post::all().exec(&mut db2).await.unwrap().len();

    let resp = client
        .csrf(&csrf)
        .post_form(
            "/admin/posts/create",
            format!(
                "title=No+Tags&author_id={}&image_path=/tmp/notags.jpg&tags=&csrf_token={csrf}",
                authors[0].id
            ),
        )
        .await;
    let status = resp.status();
    assert!(
        status.is_redirection(),
        "an empty optional Tags group must not fail the submit, got {status} {}",
        body_string(resp).await
    );
    let mut db2 = db.clone();
    assert_eq!(
        Post::all().exec(&mut db2).await.unwrap().len(),
        before + 1,
        "the post is created"
    );
    let created = Post::filter(Post::fields().title().eq("No Tags".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("created post");
    assert_eq!(created.tags, "", "the empty group stores empty");
}

#[tokio::test]
async fn posts_create_form_is_multipart() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/users/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
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
    let client = demo_client(&router, &db).await;
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
    let resp = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
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
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let post = Post::filter(
        showcase::models::Post::fields()
            .title()
            .eq("Hello Toasty".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("seeded post");
    assert_eq!(post.image_path, "hello-toasty.jpg");
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/posts/{}/edit", post.id),
            format!(
                "title=Renamed&author_id={}&image_path=&tags=rust&csrf_token={csrf}",
                authors[0].id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "untouched-file edit must redirect, got {}",
        resp.status()
    );
    let kept = Post::filter(
        showcase::models::Post::fields()
            .title()
            .eq("Renamed".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("renamed post");
    assert_eq!(
        kept.image_path, "hello-toasty.jpg",
        "stored path must survive untouched edit"
    );
}

#[tokio::test]
async fn posts_edit_explicit_clear_flag_skips_preservation() {
    // GH #90: `clear_<field>=1` opts back into clearing. On the required
    // image field that surfaces as the inline required error (not a silent
    // keep), with the stored path untouched.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let mut db_q = db.clone();
    let post = Post::filter(
        showcase::models::Post::fields()
            .title()
            .eq("Hello Toasty".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("seeded post");
    let authors = Author::all().exec(&mut db_q).await.unwrap();
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client.csrf(&csrf).post_form(&format!("/admin/posts/{}/edit", post.id), format!(
            "title=Kept&author_id={}&image_path=&tags=rust&clear_image_path=1&csrf_token={csrf}",
            authors[0].id
        ))
    .await;
    assert!(
        resp.status().is_success(),
        "explicit clear on a required file must re-render, got {}",
        resp.status()
    );
    let html = body_string(resp).await;
    assert!(
        html.contains("is required"),
        "cleared required file must error inline, got {html}"
    );
    let kept = Post::filter(
        showcase::models::Post::fields()
            .title()
            .eq("Hello Toasty".to_string()),
    )
    .first()
    .exec(&mut db_q)
    .await
    .unwrap()
    .expect("post still titled");
    assert_eq!(
        kept.image_path, "hello-toasty.jpg",
        "failed edit must not touch storage"
    );
}

#[tokio::test]
async fn multipart_body_limit_matches_urlencoded_cap() {
    // GH #90: the BodyLimit layer gives multipart the same 10 MiB cap as
    // urlencoded (Topcoat's 2 MiB default would 413 uploads we accept).
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
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
    let resp = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
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
    let resp = client
        .csrf(&csrf)
        .post_multipart("/admin/posts/create", boundary, body)
        .await;
    assert_eq!(resp.status(), 413, "11 MiB multipart must be rejected");
}

#[tokio::test]
async fn posts_author_select_is_searchable() {
    // GH #91: the relationship select carries the client-side filter hook.
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;
    let resp = client.get("/admin/posts/create").await;
    assert!(resp.status().is_success());
    let html = body_string(resp).await;
    assert!(
        html.contains("data-options-filter"),
        "author select must render the filter hook, got {html}"
    );
}

/// The reported bug (GH #184): the edit form's file input rendered `required`
/// with no visible stored value, so the browser refused to submit a save that
/// left the control untouched — the image path looked empty and the field
/// blocked the edit. The edit must now surface the stored path, drop the
/// native `required`, and let an untouched submit through while the server
/// preserves the stored value.
#[tokio::test]
async fn posts_edit_without_reupload_keeps_the_stored_image() {
    let db = full_db().await;
    let router = router(db.clone());
    let client = demo_client(&router, &db).await;

    let mut db2 = db.clone();
    let post = Post::filter(Post::fields().title().eq("Hello Toasty".to_string()))
        .first()
        .exec(&mut db2)
        .await
        .unwrap()
        .expect("the seeded post");
    let original_image = post.image_path.clone();
    assert!(
        !original_image.is_empty(),
        "the fixture must store an image path for this test to mean anything"
    );

    // The edit form: stored path visible, control not requiring a re-upload.
    let resp = client.get(&format!("/admin/posts/{}/edit", post.id)).await;
    assert_eq!(resp.status(), 200);
    let html = body_string(resp).await;
    assert!(
        html.contains(&format!("data-file-current=\"{original_image}\"")),
        "the edit must show the stored image path, got {html}"
    );
    assert!(
        html.contains("Leave empty to keep the current file."),
        "the edit must explain that an empty control keeps the file, got {html}"
    );
    // The regression itself: a `required` file input is unsubmittable when
    // empty, which is what made an untouched edit impossible in a browser.
    let tag = file_input_tag(&html);
    assert!(
        !tag.contains("required"),
        "the edit's file control must not be natively required, got {tag}"
    );

    // Save with the file control left untouched (empty), as a browser does
    // when the user does not pick a new file: title only, no `image_path`.
    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = client
        .csrf(&csrf)
        .post_form(
            &format!("/admin/posts/{}/edit", post.id),
            format!(
                "title=Hello+Toasty+Edited&author_id={}&image_path=&tags=rust,async&body=Edited+body&status=published&featured=true&csrf_token={csrf}",
                post.author_id
            ),
        )
        .await;
    assert!(
        resp.status().is_redirection(),
        "an untouched file input must not block the save, got {}",
        resp.status()
    );

    let mut db3 = db.clone();
    let saved = Post::filter(Post::fields().id().eq(post.id))
        .first()
        .exec(&mut db3)
        .await
        .unwrap()
        .expect("the post still exists");
    assert_eq!(
        saved.image_path, original_image,
        "an untouched file input must preserve the stored path (GH #90)"
    );
    assert_eq!(saved.title, "Hello Toasty Edited");
}
